//! `/api/v1/projects/{id}/repos`: the repositories a project's agents may
//! use besides its primary one. Changes apply to sessions that start
//! afterwards; running sessions lose a removed repository when their tokens
//! are next refreshed, within the hour.

use crate::access::{self, ProjectAccess};
use crate::ctx::Ctx;
use crate::db::{n, q, s};
use crate::error::{AppResult, not_found, validation};
use crate::github;
use crate::http;
use crate::model::{ALL_REPOS, REPO_SELECT, RepoRow};
use fctl_domain::repos;
use fctl_proto::api::{AddProjectRepoRequest, ProjectRepo, ProjectRepoList};
use fctl_proto::session::{RepoAccess, RepoGrant, RepoRef};
use worker::{Request, Response};

async fn rows(ctx: &Ctx, project_id: &str) -> AppResult<Vec<RepoRow>> {
    q(&ctx.db, REPO_SELECT, &[s(project_id)])?.all().await
}

async fn respond(ctx: &Ctx, p: &ProjectAccess) -> AppResult<Response> {
    let primary = p.row.repo().map(|r| ProjectRepo {
        owner: r.owner,
        name: Some(r.name),
        access: RepoAccess::Write,
        primary: true,
        added_by: None,
        added_at: None,
    });
    let others = rows(ctx, &p.row.id).await?;
    http::ok(&ProjectRepoList {
        repos: primary
            .into_iter()
            .chain(others.iter().map(RepoRow::to_repo))
            .collect(),
    })
}

pub async fn list(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    respond(ctx, &p).await
}

pub async fn add(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: AddProjectRepoRequest = http::body(req).await?;
    let p = access::project(ctx, id).await?;
    p.require_maintainer()?;
    let (owner, name) = repos::parse(&body.repo)?;
    let mut grant = RepoGrant {
        owner,
        name,
        access: body.access.unwrap_or_default(),
    };
    let grants: Vec<RepoGrant> = rows(ctx, &p.row.id)
        .await?
        .iter()
        .map(RepoRow::grant)
        .collect();
    let primary = p.row.repo();
    repos::check(primary.as_ref(), &grants, &grant)?;
    // The workspace must work in the account, and may use the repository.
    let connections = crate::workspaces::connections(&ctx.db, p.workspace_id()).await?;
    repos::check_scope(
        crate::workspaces::scope(&connections, &grant.owner).as_ref(),
        &grant.owner,
        grant.name.as_deref(),
    )?;
    // With a GitHub App, the repository must exist and be reachable; GitHub's
    // spelling of the names is the one kept.
    if let Some(app) = ctx.cfg.github.as_ref() {
        let gh = github::from_config(&ctx.cfg, ctx.now)?;
        match grant.repo() {
            Some(r) => {
                let found = gh.get_repo(&r).await?;
                grant.owner = found.owner;
                grant.name = Some(found.name);
            }
            None => {
                let login = gh
                    .installation_on(&grant.owner)
                    .await?
                    .map(|i| i.account)
                    .ok_or_else(|| {
                        validation(
                            "repo",
                            &format!(
                                "The {} GitHub App is not installed on {}. Install it there first.",
                                app.slug, grant.owner
                            ),
                        )
                    })?;
                grant.owner = login;
            }
        }
        // GitHub's names (a renamed repository answers to its old one).
        repos::check(primary.as_ref(), &grants, &grant)?;
    }
    q(
        &ctx.db,
        "INSERT INTO project_repos (project_id, owner, name, access, added_by, created_at) VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT (project_id, owner, name) DO UPDATE SET access = excluded.access",
        &[
            s(&p.row.id),
            s(&grant.owner),
            s(grant.name.as_deref().unwrap_or(ALL_REPOS)),
            s(grant.access.as_str()),
            s(&ctx.user()?.id),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(p.workspace_id()),
        "project.repo_added",
        Some(&p.row.id),
        Some(serde_json::json!({ "repo": grant.label(), "access": grant.access.as_str() })),
    );
    respond(ctx, &p).await
}

pub async fn remove(ctx: &mut Ctx, id: &str, owner: &str, name: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    p.require_maintainer()?;
    // `*` arrives as is or percent-encoded.
    let name = if name.eq_ignore_ascii_case("%2A") {
        ALL_REPOS
    } else {
        name
    };
    if p.row.repo().is_some_and(|r| {
        r.same(&RepoRef {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }) {
        return Err(validation(
            "repo",
            "This is the project's primary repository. Link the project to another one instead.",
        ));
    }
    let removed = q(
        &ctx.db,
        "DELETE FROM project_repos WHERE project_id = ? AND owner = ? AND name = ?",
        &[s(&p.row.id), s(owner), s(name)],
    )?
    .run()
    .await?;
    if removed == 0 {
        return Err(not_found("Repository"));
    }
    ctx.audit_in(
        Some(p.workspace_id()),
        "project.repo_removed",
        Some(&p.row.id),
        Some(serde_json::json!({ "repo": format!("{owner}/{name}") })),
    );
    respond(ctx, &p).await
}
