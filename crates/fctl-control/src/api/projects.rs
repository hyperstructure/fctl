//! `/api/v1/projects/*` and `/api/v1/github/*`.

use crate::access;
use crate::ctx::Ctx;
use crate::db::{Count, batch, n, os, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, not_found, validation};
use crate::github;
use crate::http;
use crate::model::{PROJECT_ORDER, PROJECT_SELECT, PROJECT_VISIBLE, ProjectRow};
use crate::workspaces::{self as ws, ConnectionRow};
use fctl_domain::{naming, perm, published, settings, validate};
use fctl_proto::api::*;
use fctl_proto::session::{ProjectRole, ProjectSettings, RepoRef, UserRef, WorkspaceRole};
use fctl_proto::{ApiError, UserId};
use serde::Deserialize;
use worker::{Request, Response};

/// `GET /api/v1/projects[?workspace=…][&archived=1]`: the projects the
/// caller sees, in all their workspaces or one, in their order.
pub async fn list(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    let user = ctx.user()?;
    let settings = ctx.settings();
    let include_archived = http::query(req, "archived").is_some_and(|v| v == "1" || v == "true");
    let mut sql = format!("{PROJECT_SELECT} WHERE {PROJECT_VISIBLE}");
    let mut params = vec![s(&user.id)];
    if let Some(w) = http::query(req, "workspace") {
        params.push(s(&w));
        sql.push_str(" AND (w.id = ?2 OR w.slug = ?2)");
    }
    if !include_archived {
        sql.push_str(" AND p.archived_at IS NULL");
    }
    sql.push_str(&access::places_sql(ctx.limits(), &mut params));
    sql.push_str(&format!(" ORDER BY {PROJECT_ORDER}"));
    let rows: Vec<ProjectRow> = q(&ctx.db, &sql, &params)?.all().await?;
    http::ok(&ProjectList {
        projects: rows
            .iter()
            .filter_map(|r| {
                access::project_role(user, &settings, ctx.limits(), r)
                    .map(|role| r.to_project(Some(role)))
            })
            .collect(),
    })
}

pub async fn get(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    http::ok(&p.row.to_project(p.role))
}

/// Validates and normalizes project settings.
fn clean_settings(mut s: ProjectSettings) -> AppResult<ProjectSettings> {
    if let Some(img) = s.image.as_deref() {
        s.image = if img.trim().is_empty() {
            None
        } else {
            Some(validate::image(img)?)
        };
    }
    if let Some(b) = &s.image_build {
        s.image_build = Some(validate::image_build(b)?);
    }
    if s.image.is_some() && s.image_build.is_some() {
        // On the image's field, which forms from before builds show too.
        return Err(validation(
            "image",
            "Choose an image or a build from the repository, not both",
        ));
    }
    if let Some(m) = s.model.as_deref() {
        s.model = if m.trim().is_empty() {
            None
        } else {
            Some(validate::model(m)?)
        };
    }
    if let Some(e) = s.effort.as_deref() {
        s.effort = if e.trim().is_empty() {
            None
        } else {
            Some(validate::effort(e)?)
        };
    }
    if let Some(script) = s.setup_script.as_deref() {
        let t = validate::text("settings.setup_script", script, 20_000)?;
        s.setup_script = (!t.is_empty()).then_some(t);
    }
    if let Some(i) = s.instructions.as_deref() {
        let t = validate::text("settings.instructions", i, 20_000)?;
        s.instructions = (!t.is_empty()).then_some(t);
    }
    if let Some(c) = s.cpus
        && !(1..=64).contains(&c)
    {
        return Err(validation("settings.cpus", "Use 1–64 CPUs"));
    }
    if let Some(m) = s.memory_mb
        && !(512..=262_144).contains(&m)
    {
        return Err(validation("settings.memory_mb", "Use 512 MiB – 256 GiB"));
    }
    if let Some(t) = s.idle_timeout_minutes
        && !(1..=10_080).contains(&t)
    {
        return Err(validation(
            "settings.idle_timeout_minutes",
            "Use 1 minute – 7 days",
        ));
    }
    if let Some(d) = s.screenshot_days
        && !published::valid_days(d)
    {
        return Err(validation(
            "settings.screenshot_days",
            &format!("Use {} – {} days", published::MIN_DAYS, published::MAX_DAYS),
        ));
    }
    let mut env = std::collections::BTreeMap::new();
    for (k, v) in &s.env {
        let k = validate::env_name("settings.env", k)?;
        if v.len() > 10_000 {
            return Err(validation(
                "settings.env",
                "Values can be at most 10 000 characters",
            ));
        }
        env.insert(k, v.clone());
    }
    if env.len() > 100 {
        return Err(validation("settings.env", "Use at most 100 variables"));
    }
    s.env = env;
    Ok(s)
}

/// A build comes from the project's repository.
fn needs_repo() -> AppError {
    validation(
        "settings.image_build",
        "Link a repository first: the image is built from its files",
    )
}

async fn slug_taken(ctx: &Ctx, slug: &str) -> AppResult<bool> {
    let c: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM projects WHERE slug = ?",
        &[s(slug)],
    )?
    .first()
    .await?;
    Ok(c.map_or(0, |c| c.n) > 0)
}

/// The workspace a request is about: the one named, or the caller's only
/// one; with `admin`, for a new project, the only one they are an admin of.
async fn target_workspace(
    ctx: &Ctx,
    named: Option<&str>,
    admin: bool,
) -> AppResult<ws::WorkspaceAccess> {
    if let Some(w) = named.filter(|w| !w.trim().is_empty()) {
        return ws::load(ctx, w.trim()).await;
    }
    let user = ctx.user()?;
    let mine = ws::mine(&ctx.db, &user.id).await?;
    let candidates: Vec<&ws::WorkspaceRow> = mine
        .iter()
        .filter(|w| !admin || w.role().is_some_and(WorkspaceRole::is_admin))
        .collect();
    match (candidates.as_slice(), mine.is_empty()) {
        ([only], _) => ws::load(ctx, &only.id).await,
        ([], true) => Err(validation(
            "workspace",
            "Create a workspace first, or ask to join one: projects belong to workspaces",
        )),
        // A member of workspaces they don't administer.
        ([], false) => Err(forbidden(
            "Only a workspace's admins create its projects, and you are an admin of none of yours. Ask one of them to create it, or create a workspace of your own.",
        )),
        _ => Err(validation(
            "workspace",
            "Choose the workspace the project belongs to",
        )),
    }
}

/// Checks that a workspace may use `repo`, and returns the installation
/// that mints its tokens.
async fn check_repo(
    ctx: &Ctx,
    gh: &github::Github<'_>,
    connections: &[ConnectionRow],
    repo: &RepoRef,
) -> AppResult<u64> {
    fctl_domain::repos::check_scope(
        ws::scope(connections, &repo.owner).as_ref(),
        &repo.owner,
        Some(&repo.name),
    )?;
    ws::installation(&ctx.db, gh, connections, &repo.owner).await
}

pub async fn create(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    // Read as JSON first: a new project's settings start with connectors off
    // unless the request sets them (see `settings::for_new_project`).
    let raw: serde_json::Value = http::body(req).await?;
    let body = CreateProjectRequest::deserialize(&raw)
        .map_err(|e| AppError(ApiError::bad_request(format!("Invalid request body: {e}"))))?;
    let user = ctx.user()?.clone();
    let workspace = target_workspace(ctx, body.workspace.as_deref(), true).await?;
    fctl_domain::workspace::check_create_project(Some(workspace.require_open()?))?;
    let connections = ws::connections(&ctx.db, workspace.id()).await?;
    let name = validate::name("name", &body.name)?;
    let description = body
        .description
        .as_deref()
        .map(|d| validate::text("description", d, 500))
        .transpose()?
        .filter(|d| !d.is_empty());
    let slug = match body.slug.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(sl) => {
            let sl = validate::slug(sl)?;
            if slug_taken(ctx, &sl).await? {
                return Err(validation("slug", "Another project uses this name"));
            }
            sl
        }
        None => {
            let base = naming::project_slug(&name);
            let base = validate::slug(&base).unwrap_or_else(|_| {
                format!("project-{}", &fctl_domain::ids::ulid(ctx.now)[20..]).to_lowercase()
            });
            let mut candidate = base.clone();
            let mut i = 2;
            while slug_taken(ctx, &candidate).await? {
                candidate = format!("{base}-{i}");
                i += 1;
                if i > 50 {
                    return Err(conflict("Pick a different project name"));
                }
            }
            candidate
        }
    };
    let mut settings = body.settings.clone().unwrap_or_default();
    settings::for_new_project(
        &mut settings,
        raw.get("settings").unwrap_or(&serde_json::Value::Null),
    );
    let settings = clean_settings(settings)?;
    // The repository: an existing one, a new one, or none.
    let mut repo: Option<RepoRef> = None;
    let mut default_branch = body
        .default_branch
        .as_deref()
        .map(|b| validate::branch("default_branch", b))
        .transpose()?;
    if let Some(create) = &body.create_repo {
        let gh = github::from_config(&ctx.cfg, ctx.now)?;
        let owner = new_repo_owner(&gh, &connections, create.owner.as_deref())?;
        // The account the workspace connected, while its login is still its.
        ws::installation(&ctx.db, &gh, &connections, &owner).await?;
        let repo_name = naming::slugify(&create.name, 100);
        if repo_name.is_empty() {
            return Err(validation("create_repo.name", "Enter a repository name"));
        }
        let created = gh
            .create_repo(
                &owner,
                &repo_name,
                create.private,
                create.description.as_deref(),
            )
            .await?;
        default_branch = default_branch.or(Some(created.default_branch.clone()));
        repo = Some(RepoRef {
            owner: created.owner,
            name: created.name,
        });
    } else if let Some(r) = body.repo.as_deref().filter(|r| !r.trim().is_empty()) {
        let parsed = RepoRef::parse(r)
            .ok_or_else(|| validation("repo", "Use owner/name, like hyperstructure/fctl"))?;
        if ctx.cfg.github.is_some() {
            let gh = github::from_config(&ctx.cfg, ctx.now)?;
            check_repo(ctx, &gh, &connections, &parsed).await?;
            let found = gh.get_repo(&parsed).await?;
            // GitHub answers for a repository by its old name too, once it
            // was renamed or moved to another account: the one it names
            // must be the workspace's to use as well.
            fctl_domain::repos::check_scope(
                ws::scope(&connections, &found.owner).as_ref(),
                &found.owner,
                Some(&found.name),
            )?;
            default_branch = default_branch.or(Some(found.default_branch.clone()));
            repo = Some(RepoRef {
                owner: found.owner,
                name: found.name,
            });
        } else {
            repo = Some(parsed);
        }
    }
    if settings.image_build.is_some() && repo.is_none() {
        return Err(needs_repo());
    }
    let id = fctl_domain::ids::project_id(ctx.now);
    batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "INSERT INTO projects (id, workspace_id, slug, name, description, repo_owner, repo_name, default_branch, settings, created_by, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                &[
                    s(id.as_str()),
                    s(workspace.id()),
                    s(&slug),
                    s(&name),
                    os(description.as_deref()),
                    os(repo.as_ref().map(|r| r.owner.as_str())),
                    os(repo.as_ref().map(|r| r.name.as_str())),
                    s(default_branch.as_deref().unwrap_or("main")),
                    s(&serde_json::to_string(&settings)?),
                    s(&user.id),
                    n(ctx.now),
                    n(ctx.now),
                ],
            )?,
            q(
                &ctx.db,
                "INSERT INTO project_members (project_id, user_id, role, added_by, created_at) VALUES (?, ?, 'owner', ?, ?)",
                &[s(id.as_str()), s(&user.id), s(&user.id), n(ctx.now)],
            )?,
        ],
    )
    .await?;
    ctx.audit_in(
        Some(workspace.id()),
        "project.created",
        Some(id.as_str()),
        Some(serde_json::json!({ "slug": slug })),
    );
    let p = access::project(ctx, id.as_str()).await?;
    http::created(&p.row.to_project(p.role))
}

pub async fn update(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    // Read as JSON first: what an update keeps depends on which settings it
    // sent (see `settings::keep_unsent`).
    let raw: serde_json::Value = http::body(req).await?;
    let body = UpdateProjectRequest::deserialize(&raw)
        .map_err(|e| AppError(ApiError::bad_request(format!("Invalid request body: {e}"))))?;
    let p = access::project(ctx, id).await?;
    p.require_maintainer()?;
    let row = &p.row;
    let name = body
        .name
        .as_deref()
        .map(|v| validate::name("name", v))
        .transpose()?
        .unwrap_or_else(|| row.name.clone());
    let description = match body.description.as_deref() {
        Some(d) => Some(validate::text("description", d, 500)?).filter(|d| !d.is_empty()),
        None => row.description.clone(),
    };
    let (mut owner, mut repo_name) = (row.repo_owner.clone(), row.repo_name.clone());
    if let Some(r) = body.repo.as_deref() {
        if r.trim().is_empty() {
            owner = None;
            repo_name = None;
        } else {
            let parsed = RepoRef::parse(r).ok_or_else(|| validation("repo", "Use owner/name"))?;
            // The project's other repositories share its account.
            let grants = crate::spec::grants(&ctx.db, &row.id).await?;
            fctl_domain::repos::check_primary(&parsed, &grants)?;
            if ctx.cfg.github.is_some() {
                let gh = github::from_config(&ctx.cfg, ctx.now)?;
                let connections = ws::connections(&ctx.db, p.workspace_id()).await?;
                check_repo(ctx, &gh, &connections, &parsed).await?;
                gh.get_repo(&parsed).await?;
            }
            owner = Some(parsed.owner);
            repo_name = Some(parsed.name);
        }
    }
    let default_branch = body
        .default_branch
        .as_deref()
        .map(|b| validate::branch("default_branch", b))
        .transpose()?
        .unwrap_or_else(|| row.default_branch.clone());
    let settings = match body.settings {
        Some(st) => {
            let mut st = clean_settings(st)?;
            let sent = raw.get("settings").unwrap_or(&serde_json::Value::Null);
            settings::keep_unsent(&mut st, &row.settings(), sent);
            st
        }
        None => row.settings(),
    };
    if settings.image_build.is_some() && (owner.is_none() || repo_name.is_none()) {
        return Err(needs_repo());
    }
    let archived_at = match body.archived {
        Some(true) => row.archived_at.or(Some(ctx.now)),
        Some(false) => None,
        None => row.archived_at,
    };
    batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "UPDATE projects SET name = ?, description = ?, repo_owner = ?, repo_name = ?, default_branch = ?, settings = ?, archived_at = ?, updated_at = ? WHERE id = ?",
                &[
                    s(&name),
                    os(description.as_deref()),
                    os(owner.as_deref()),
                    os(repo_name.as_deref()),
                    s(&default_branch),
                    s(&serde_json::to_string(&settings)?),
                    crate::db::on(archived_at),
                    n(ctx.now),
                    s(&row.id),
                ],
            )?,
            // The primary repository is not listed again among the others.
            q(
                &ctx.db,
                "DELETE FROM project_repos WHERE project_id = ? AND owner = ? AND name = ?",
                &[
                    s(&row.id),
                    os(owner.as_deref()),
                    os(repo_name.as_deref()),
                ],
            )?,
        ],
    )
    .await?;
    ctx.audit_in(
        Some(p.workspace_id()),
        "project.updated",
        Some(&row.id),
        None,
    );
    let p = access::project(ctx, &row.id).await?;
    http::ok(&p.row.to_project(p.role))
}

pub async fn delete(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    p.require_owner()?;
    #[derive(serde::Deserialize)]
    struct Active {
        #[serde(deserialize_with = "crate::db::int")]
        sessions: i64,
        #[serde(deserialize_with = "crate::db::int")]
        reviews: i64,
    }
    let active: Option<Active> = q(
        &ctx.db,
        "SELECT COUNT(*) - COUNT(rv.session_id) AS sessions, COUNT(rv.session_id) AS reviews FROM sessions s \
         LEFT JOIN reviews rv ON rv.session_id = s.id WHERE s.project_id = ? \
         AND s.status IN ('queued','provisioning','preparing','running','idle','paused','stopping')",
        &[s(&p.row.id)],
    )?
    .first()
    .await?;
    match active {
        Some(a) if a.sessions > 0 => {
            return Err(conflict("Stop the project's sessions before deleting it"));
        }
        Some(a) if a.reviews > 0 => {
            return Err(conflict(
                "A review agent is reviewing a pull request in this project: stop it under Reviews, or let it finish, before deleting the project",
            ));
        }
        _ => {}
    }
    ctx.verified(fctl_domain::verify::Sensitive::DeleteProject)?;
    crate::reviews::cancel_project(ctx, &p.row.id).await;
    // The rows cascade; the sessions' event logs and the project's files
    // live in Durable Objects, so they are queued for purging in the same batch.
    batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "INSERT OR IGNORE INTO session_purges (session_id, queued_at) SELECT id, ? FROM sessions WHERE project_id = ?",
                &[n(ctx.now), s(&p.row.id)],
            )?,
            q(
                &ctx.db,
                "INSERT OR IGNORE INTO file_purges (project_id, queued_at) VALUES (?, ?)",
                &[s(&p.row.id), n(ctx.now)],
            )?,
            q(
                &ctx.db,
                "DELETE FROM projects WHERE id = ?",
                &[s(&p.row.id)],
            )?,
        ],
    )
    .await?;
    let env = ctx.env.clone();
    let db = crate::db::handle(&ctx.db);
    ctx.defer(async move { crate::housekeeping::purge_deleted(&env, &db).await });
    ctx.audit_in(
        Some(p.workspace_id()),
        "project.deleted",
        Some(&p.row.id),
        Some(serde_json::json!({ "slug": p.row.slug })),
    );
    http::ok(&Ok { ok: true })
}

// ───────────────────────────── members ─────────────────────────────

#[derive(serde::Deserialize)]
struct MemberRow {
    user_id: String,
    name: String,
    email: String,
    avatar_url: Option<String>,
    role: String,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
}

pub async fn members(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    let rows: Vec<MemberRow> = q(
        &ctx.db,
        "SELECT m.user_id, u.name, COALESCE(wm.email, u.email) AS email, u.avatar_url, m.role, m.created_at FROM project_members m \
         JOIN users u ON u.id = m.user_id JOIN projects p ON p.id = m.project_id \
         LEFT JOIN workspace_members wm ON wm.workspace_id = p.workspace_id AND wm.user_id = m.user_id \
         WHERE m.project_id = ? ORDER BY m.created_at",
        &[s(&p.row.id)],
    )?
    .all()
    .await?;
    http::ok(&MemberList {
        members: rows
            .into_iter()
            .map(|r| ProjectMember {
                user: UserRef {
                    id: UserId(r.user_id),
                    name: r.name,
                    avatar_url: r.avatar_url,
                },
                email: r.email,
                role: ProjectRole::parse(&r.role).unwrap_or(ProjectRole::Viewer),
                added_at: r.created_at,
            })
            .collect(),
    })
}

pub async fn add_member(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: AddMemberRequest = http::body(req).await?;
    let p = access::project(ctx, id).await?;
    if !perm::can_assign_project_role(p.role, body.role) {
        return Err(forbidden("You cannot give this role"));
    }
    let target = crate::api::auth::member_by_email(ctx, p.workspace_id(), &body.email)
        .await?
        .ok_or_else(|| {
            validation(
                "email",
                "No one in this workspace has this address. Invite them to the workspace first.",
            )
        })?;
    // Adding someone who is a member already changes their role: the same
    // checks as changing it.
    if let Some(current) = member_role(ctx, &p.row.id, &target.id).await? {
        check_role_change(ctx, &p, current, body.role).await?;
    }
    q(
        &ctx.db,
        "INSERT INTO project_members (project_id, user_id, role, added_by, created_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT (project_id, user_id) DO UPDATE SET role = excluded.role",
        &[s(&p.row.id), s(&target.id), s(body.role.as_str()), s(&ctx.user()?.id), n(ctx.now)],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(p.workspace_id()),
        "project.member_added",
        Some(&p.row.id),
        Some(serde_json::json!({ "user": target.id, "role": body.role.as_str() })),
    );
    crate::workspaces::changed(ctx, vec![target.id.clone()]);
    members(ctx, &p.row.id).await
}

/// Whether the caller may make a member whose role is `current` a `to`:
/// they may give both roles, and a project keeps an owner.
async fn check_role_change(
    ctx: &Ctx,
    p: &access::ProjectAccess,
    current: ProjectRole,
    to: ProjectRole,
) -> AppResult<()> {
    if !perm::can_assign_project_role(p.role, to) || !perm::can_assign_project_role(p.role, current)
    {
        return Err(forbidden("You cannot change this member's role"));
    }
    if current == ProjectRole::Owner
        && to != ProjectRole::Owner
        && owner_count(ctx, &p.row.id).await? <= 1
    {
        return Err(conflict("A project needs at least one owner"));
    }
    Ok(())
}

async fn owner_count(ctx: &Ctx, project_id: &str) -> AppResult<i64> {
    let c: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM project_members WHERE project_id = ? AND role = 'owner'",
        &[s(project_id)],
    )?
    .first()
    .await?;
    Ok(c.map_or(0, |c| c.n))
}

async fn member_role(ctx: &Ctx, project_id: &str, user_id: &str) -> AppResult<Option<ProjectRole>> {
    #[derive(serde::Deserialize)]
    struct R {
        role: String,
    }
    let r: Option<R> = q(
        &ctx.db,
        "SELECT role FROM project_members WHERE project_id = ? AND user_id = ?",
        &[s(project_id), s(user_id)],
    )?
    .first()
    .await?;
    Ok(r.and_then(|r| ProjectRole::parse(&r.role)))
}

pub async fn set_member_role(
    ctx: &mut Ctx,
    req: &mut Request,
    id: &str,
    user_id: &str,
) -> AppResult<Response> {
    let body: SetRoleRequest = http::body(req).await?;
    let p = access::project(ctx, id).await?;
    let current = member_role(ctx, &p.row.id, user_id)
        .await?
        .ok_or_else(|| not_found("Member"))?;
    check_role_change(ctx, &p, current, body.role).await?;
    q(
        &ctx.db,
        "UPDATE project_members SET role = ? WHERE project_id = ? AND user_id = ?",
        &[s(body.role.as_str()), s(&p.row.id), s(user_id)],
    )?
    .run()
    .await?;
    crate::workspaces::changed(ctx, vec![user_id.to_owned()]);
    members(ctx, &p.row.id).await
}

pub async fn remove_member(ctx: &mut Ctx, id: &str, user_id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    let me = ctx.user()?.id.clone();
    let current = member_role(ctx, &p.row.id, user_id)
        .await?
        .ok_or_else(|| not_found("Member"))?;
    let leaving = me == user_id;
    if !leaving && !perm::can_assign_project_role(p.role, current) {
        return Err(forbidden("You cannot remove this member"));
    }
    if current == ProjectRole::Owner && owner_count(ctx, &p.row.id).await? <= 1 {
        return Err(conflict("A project needs at least one owner"));
    }
    q(
        &ctx.db,
        "DELETE FROM project_members WHERE project_id = ? AND user_id = ?",
        &[s(&p.row.id), s(user_id)],
    )?
    .run()
    .await?;
    crate::workspaces::changed(ctx, vec![user_id.to_owned()]);
    if leaving {
        return http::ok(&Ok { ok: true });
    }
    members(ctx, &p.row.id).await
}

// ───────────────────────────── secrets ─────────────────────────────

fn secret_aad(project_id: &str, name: &str) -> String {
    format!("secret:{project_id}:{name}")
}

#[derive(serde::Deserialize)]
struct SecretRow {
    name: String,
    #[serde(deserialize_with = "crate::db::int")]
    updated_at: i64,
    updated_by: Option<String>,
    updater_name: Option<String>,
}

pub async fn secrets(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    p.require_maintainer()?;
    let rows: Vec<SecretRow> = q(
        &ctx.db,
        "SELECT s.name, s.updated_at, s.updated_by, u.name AS updater_name FROM project_secrets s \
         LEFT JOIN users u ON u.id = s.updated_by WHERE s.project_id = ? ORDER BY s.name",
        &[s(&p.row.id)],
    )?
    .all()
    .await?;
    http::ok(&SecretList {
        secrets: rows
            .into_iter()
            .map(|r| SecretInfo {
                name: r.name,
                updated_at: r.updated_at,
                updated_by: match (r.updated_by, r.updater_name) {
                    (Some(id), Some(name)) => Some(UserRef {
                        id: UserId(id),
                        name,
                        avatar_url: None,
                    }),
                    _ => None,
                },
            })
            .collect(),
    })
}

pub async fn set_secret(
    ctx: &mut Ctx,
    req: &mut Request,
    id: &str,
    name: &str,
) -> AppResult<Response> {
    let body: SetSecretRequest = http::body(req).await?;
    let p = access::project(ctx, id).await?;
    p.require_maintainer()?;
    let name = validate::env_name("name", name)?;
    if body.value.len() > 64 * 1024 {
        return Err(validation("value", "Secrets can be at most 64 KiB"));
    }
    let sealed = ctx
        .keys()?
        .seal(&secret_aad(&p.row.id, &name), body.value.as_bytes())?;
    q(
        &ctx.db,
        "INSERT INTO project_secrets (project_id, name, value_enc, updated_by, updated_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT (project_id, name) DO UPDATE SET value_enc = excluded.value_enc, updated_by = excluded.updated_by, updated_at = excluded.updated_at",
        &[s(&p.row.id), s(&name), s(&sealed), s(&ctx.user()?.id), n(ctx.now)],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(p.workspace_id()),
        "project.secret_set",
        Some(&p.row.id),
        Some(serde_json::json!({ "name": name })),
    );
    secrets(ctx, &p.row.id).await
}

pub async fn delete_secret(ctx: &mut Ctx, id: &str, name: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    p.require_maintainer()?;
    q(
        &ctx.db,
        "DELETE FROM project_secrets WHERE project_id = ? AND name = ?",
        &[s(&p.row.id), s(name)],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(p.workspace_id()),
        "project.secret_deleted",
        Some(&p.row.id),
        Some(serde_json::json!({ "name": name })),
    );
    secrets(ctx, &p.row.id).await
}

/// Decrypted secrets for a session spec.
pub async fn decrypted_secrets(
    db: &worker::D1Database,
    keys: &crate::crypto::Keys,
    project_id: &str,
) -> AppResult<std::collections::BTreeMap<String, String>> {
    #[derive(serde::Deserialize)]
    struct R {
        name: String,
        value_enc: String,
    }
    let rows: Vec<R> = q(
        db,
        "SELECT name, value_enc FROM project_secrets WHERE project_id = ?",
        &[s(project_id)],
    )?
    .all()
    .await?;
    let mut out = std::collections::BTreeMap::new();
    for r in rows {
        let bytes = keys.open(&secret_aad(project_id, &r.name), &r.value_enc)?;
        out.insert(r.name, String::from_utf8_lossy(&bytes).into_owned());
    }
    Ok(out)
}

// ───────────────────────────── GitHub ─────────────────────────────

/// The organization a new repository goes into: the one asked for, the
/// instance's default one, or the workspace's first organization; it must
/// be one the workspace works in, with all its repositories.
fn new_repo_owner(
    gh: &github::Github<'_>,
    connections: &[ConnectionRow],
    asked: Option<&str>,
) -> AppResult<String> {
    let usable = |owner: &str| {
        matches!(
            ws::scope(connections, owner),
            Some(fctl_domain::repos::Scope::All)
        )
    };
    if let Some(owner) = asked.map(str::trim).filter(|o| !o.is_empty()) {
        if !usable(owner) {
            return Err(validation(
                "create_repo.owner",
                &format!(
                    "This workspace works in only some of {owner}'s repositories, so it can't create new ones there"
                ),
            ));
        }
        return Ok(owner.to_owned());
    }
    if let Some(default) = gh.default_owner().filter(|o| usable(o)) {
        return Ok(default.to_owned());
    }
    connections
        .iter()
        .find(|c| c.active() && c.repos().is_none())
        .map(|c| c.account.clone())
        .ok_or_else(|| {
            validation(
                "create_repo",
                "To create repositories, this workspace needs a GitHub account it works in every repository of",
            )
        })
}

/// `GET /api/v1/github`: the instance's GitHub App and the accounts the
/// caller's workspaces work in.
pub async fn github_info(ctx: &Ctx) -> AppResult<Response> {
    let user = ctx.user()?;
    let Some(cfg) = ctx.cfg.github.as_ref() else {
        return http::ok(&GithubInfo {
            configured: false,
            app_slug: None,
            install_url: None,
            accounts: vec![],
        });
    };
    #[derive(Deserialize)]
    struct A {
        account: String,
    }
    // Of the workspaces open to them: not one its two-factor requirement
    // closes to them.
    let open: Vec<String> = ws::mine(&ctx.db, &user.id)
        .await?
        .into_iter()
        .filter(|w| !ws::blocked_for(ctx, user, w.require_two_factor))
        .map(|w| w.id)
        .collect();
    let accounts: Vec<A> = q(
        &ctx.db,
        "SELECT DISTINCT g.account FROM workspace_github g \
         WHERE g.workspace_id IN (SELECT value FROM json_each(?)) AND g.status = 'active' ORDER BY g.account",
        &[s(&serde_json::to_string(&open)?)],
    )?
    .all()
    .await?;
    http::ok(&GithubInfo {
        configured: true,
        app_slug: Some(cfg.slug.clone()),
        install_url: Some(format!(
            "https://github.com/apps/{}/installations/new",
            cfg.slug
        )),
        accounts: accounts.into_iter().map(|a| a.account).collect(),
    })
}

/// `GET /api/v1/github/repos?workspace=…`: the repositories a workspace's
/// projects may use (the caller's only workspace without one), newest
/// push first, with the projects of it that use them.
pub async fn github_repos(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    let workspace = target_workspace(ctx, http::query(req, "workspace").as_deref(), false).await?;
    workspace.require_open()?;
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    let connections = ws::connections(&ctx.db, workspace.id()).await?;
    let mut repos = Vec::new();
    for c in connections.iter().filter(|c| c.active()) {
        let inst = match ws::installation(&ctx.db, &gh, &connections, &c.account).await {
            Ok(i) => i,
            Err(e) => {
                worker::console_log!("no installation on {}: {}", c.account, e.0.message);
                continue;
            }
        };
        let scope = fctl_domain::repos::Scope::of(c.repos());
        // One account GitHub won't list doesn't hide the others'.
        let theirs = match gh.repos_of(inst).await {
            Ok(r) => r,
            Err(e) => {
                worker::console_log!("listing {}'s repositories: {}", c.account, e.0.message);
                continue;
            }
        };
        repos.extend(
            theirs
                .into_iter()
                .filter(|r| r.owner.eq_ignore_ascii_case(&c.account) && scope.allows(&r.name)),
        );
    }
    repos.sort_by(|a, b| b.pushed_at.cmp(&a.pushed_at));
    #[derive(serde::Deserialize)]
    struct Linked {
        id: String,
        repo_owner: String,
        repo_name: String,
    }
    // Only the projects the caller sees: the others' names and repositories
    // aren't theirs to know (`PROJECT_VISIBLE`).
    let user = ctx.user()?;
    let linked: Vec<Linked> = q(
        &ctx.db,
        &format!(
            "SELECT p.id, p.repo_owner, p.repo_name FROM projects p JOIN workspaces w ON w.id = p.workspace_id \
             JOIN workspace_members wm ON wm.workspace_id = p.workspace_id AND wm.user_id = ?1 \
             LEFT JOIN project_members m ON m.project_id = p.id AND m.user_id = ?1 \
             WHERE p.repo_owner IS NOT NULL AND p.archived_at IS NULL AND p.workspace_id = ?2 AND {PROJECT_VISIBLE}"
        ),
        &[s(&user.id), s(workspace.id())],
    )?
    .all()
    .await?;
    for r in &mut repos {
        r.project_id = linked
            .iter()
            .find(|l| {
                l.repo_owner.eq_ignore_ascii_case(&r.owner)
                    && l.repo_name.eq_ignore_ascii_case(&r.name)
            })
            .map(|l| fctl_proto::ProjectId(l.id.clone()));
    }
    http::ok(&GithubRepoList { repos })
}
