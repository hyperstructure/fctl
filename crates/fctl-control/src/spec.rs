//! Building the [`SessionSpec`] a host needs, with fresh credentials.

use crate::config::Config;
use crate::crypto::Keys;
use crate::db::{n, on, q, s};
use crate::error::{AppError, AppResult, internal};
use crate::github;
use crate::model::{
    CredentialRow, CredentialSecret, PROJECT_SELECT, ProjectRow, REPO_SELECT, RepoRow,
    SESSION_SELECT, SessionRow, UserRow, user_columns,
};
use fctl_domain::image::ImageWant;
use fctl_domain::size::Want;
use fctl_proto::runner::{
    ClaudeCredential, ClaudeCredentialKind, GitIdentity, ProjectSpec, SessionCredentials,
    SessionRepo, SessionSpec,
};
use fctl_proto::session::{GithubAccess, ProjectSettings, PullRequestRef, RepoGrant, RepoRef};
use fctl_proto::{ApiError, CredentialId, ErrorCode, ProjectId};
use std::cell::RefCell;
use std::collections::HashMap;
use worker::{D1Database, Fetch, Headers, Method, Request, RequestInit};

/// Claude Code's OAuth client and token endpoint (as `claude` itself uses them).
const CLAUDE_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// Refresh when less than this is left on an access token.
const REFRESH_MARGIN_MS: i64 = 10 * 60 * 1000;

thread_local! {
    static BOT_EMAIL: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

pub async fn session_row(db: &D1Database, id: &str) -> AppResult<Option<SessionRow>> {
    q(
        db,
        &format!("{SESSION_SELECT} WHERE s.id = ?2"),
        &[s(""), s(id)],
    )?
    .first()
    .await
}

pub async fn project_row(db: &D1Database, id: &str) -> AppResult<Option<ProjectRow>> {
    q(
        db,
        &format!("{PROJECT_SELECT} WHERE p.id = ?2"),
        &[s(""), s(id)],
    )?
    .first()
    .await
}

/// Whether runner `runner_id` runs session `session_id` now: it is
/// enabled, was added to the session's workspace, its owner is still an
/// active member there (`serves!`), and it runs the sessions of the person
/// the session runs as (`fctl_domain::schedule::runs_for`: a runner added
/// for its owner's sessions runs only theirs, and someone who keeps their
/// sessions on their own runners has them run nowhere else). A runner that
/// no longer does gets nothing more for the sessions it still has there:
/// no credentials, uplinks, files or screenshots.
pub async fn runner_serves(db: &D1Database, runner_id: &str, session_id: &str) -> AppResult<bool> {
    #[derive(serde::Deserialize)]
    struct Row {
        owner: Option<String>,
        #[serde(deserialize_with = "crate::db::int_bool", default)]
        personal: bool,
        person: Option<String>,
        #[serde(deserialize_with = "crate::db::int_bool", default)]
        own_runners: bool,
    }
    let row: Option<Row> = q(
        db,
        concat!(
            "SELECT r.created_by AS owner, rw.personal AS personal, c.user_id AS person, \
             om.own_runners AS own_runners FROM sessions s JOIN projects p ON p.id = s.project_id \
             JOIN runner_workspaces rw ON rw.workspace_id = p.workspace_id AND rw.runner_id = ?1 \
             JOIN runners r ON r.id = rw.runner_id \
             LEFT JOIN claude_credentials c ON c.id = s.credential_id \
             LEFT JOIN workspace_members om ON om.workspace_id = p.workspace_id AND om.user_id = c.user_id \
             WHERE s.id = ?2 AND r.enabled = 1 AND ",
            crate::serves!()
        ),
        &[s(runner_id), s(session_id)],
    )?
    .first()
    .await?;
    Ok(row.is_some_and(|r| {
        fctl_domain::schedule::runs_for(
            r.owner.as_deref(),
            r.personal,
            r.person.as_deref(),
            r.own_runners,
        )
    }))
}

/// The error for a runner that no longer runs a session ([`runner_serves`]).
pub fn not_served() -> AppError {
    crate::error::forbidden("This runner no longer runs this session")
}

/// Whether Claude credential `credential_id` still runs session
/// `session_id`: its owner's account is active and they may still message
/// the session (`fctl_domain::perm::session_role`): they are a member of
/// its workspace, which its two-factor requirement doesn't close to them,
/// and they may start sessions in its project, or it was shared with them
/// to message or own. A session keeps the credential it last ran on, and a
/// message, a follow-up or a review's findings start a stopped one again,
/// so without this whoever can message it would run it on the subscription
/// of someone who left the workspace, was disabled, or lost their role in
/// the project, and on that person's own runners, which would get the
/// project's secrets and new GitHub tokens they no longer may.
pub async fn credential_serves(
    db: &D1Database,
    credential_id: &str,
    session_id: &str,
) -> AppResult<bool> {
    let owner: Option<UserRow> = q(
        db,
        &format!(
            "SELECT {} FROM claude_credentials c JOIN users u ON u.id = c.user_id WHERE c.id = ?",
            user_columns("u")
        ),
        &[s(credential_id)],
    )?
    .first()
    .await?;
    let Some(owner) = owner.filter(UserRow::is_active) else {
        return Ok(false);
    };
    let settings = crate::instance::settings(db).await?;
    let role = crate::access::session_role_now(db, &owner, &settings, None, session_id).await?;
    Ok(role.is_some_and(|r| r.can_message()))
}

/// The error for a session whose Claude credential no longer runs it
/// ([`credential_serves`]).
pub fn credential_not_lent() -> AppError {
    AppError(ApiError::new(
        ErrorCode::NotConfigured,
        "This session ran on the Claude subscription of someone who can no longer run it: they left its workspace or lost their role in its project, or their account is disabled. Resume it to run it on yours.",
    ))
}

/// The repositories a project's agents may use besides its primary one.
pub async fn grants(db: &D1Database, project_id: &str) -> AppResult<Vec<RepoGrant>> {
    let rows: Vec<RepoRow> = q(db, REPO_SELECT, &[s(project_id)])?.all().await?;
    Ok(rows.iter().map(RepoRow::grant).collect())
}

/// What a queued session asks of a runner.
#[derive(Clone, Debug, Default)]
pub struct SessionWants {
    /// CPUs and memory.
    pub size: Want,
    /// The image it starts from.
    pub image: ImageWant,
    /// Its workspace: only the workspace's runners take it.
    pub workspace_id: String,
    /// Whom it runs as: the owner of its Claude connection. Runners added
    /// for their owner's sessions take only theirs.
    pub person: Option<String>,
    /// Their name, for why it waits.
    pub person_name: String,
    /// They keep their sessions in its workspace on their own runners.
    pub own_runners: bool,
    /// It is a review agent's session: only runners that run them take it.
    pub review: bool,
}

/// The CPUs, memory and image each of these sessions asks for (its
/// project's sandbox settings), and whom each runs as, in one query.
pub async fn session_wants(
    db: &D1Database,
    ids: &[&str],
) -> AppResult<HashMap<String, SessionWants>> {
    #[derive(serde::Deserialize)]
    struct Row {
        id: String,
        settings: String,
        repo_owner: Option<String>,
        repo_name: Option<String>,
        workspace_id: Option<String>,
        person: Option<String>,
        person_name: Option<String>,
        #[serde(deserialize_with = "crate::db::int_bool", default)]
        own_runners: bool,
        #[serde(deserialize_with = "crate::db::int_bool", default)]
        review: bool,
    }
    let rows: Vec<Row> = q(
        db,
        "SELECT s.id AS id, p.settings AS settings, p.repo_owner AS repo_owner, p.repo_name AS repo_name, \
         p.workspace_id AS workspace_id, c.user_id AS person, u.name AS person_name, \
         om.own_runners AS own_runners, rv.session_id IS NOT NULL AS review \
         FROM sessions s JOIN projects p ON p.id = s.project_id \
         LEFT JOIN claude_credentials c ON c.id = s.credential_id LEFT JOIN users u ON u.id = c.user_id \
         LEFT JOIN workspace_members om ON om.workspace_id = p.workspace_id AND om.user_id = c.user_id \
         LEFT JOIN reviews rv ON rv.session_id = s.id \
         WHERE s.id IN (SELECT value FROM json_each(?))",
        &[s(&serde_json::to_string(ids)?)],
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let settings: ProjectSettings = serde_json::from_str(&r.settings).unwrap_or_default();
            let repo = match (r.repo_owner, r.repo_name) {
                (Some(owner), Some(name)) => Some(RepoRef { owner, name }),
                _ => None,
            };
            let wants = SessionWants {
                size: Want::of(&settings),
                image: ImageWant::of(&settings, repo.as_ref()),
                workspace_id: r.workspace_id.unwrap_or_default(),
                person: r.person,
                person_name: r.person_name.unwrap_or_default(),
                own_runners: r.own_runners,
                review: r.review,
            };
            (r.id, wants)
        })
        .collect())
}

pub async fn build(
    db: &D1Database,
    cfg: &Config,
    keys: &Keys,
    session_id: &str,
    now: i64,
) -> AppResult<SessionSpec> {
    let row = session_row(db, session_id)
        .await?
        .ok_or_else(|| internal("session vanished"))?;
    let project = project_row(db, &row.project_id)
        .await?
        .ok_or_else(|| internal("project vanished"))?;
    let cred_id = row.credential_id.clone().ok_or_else(|| {
        AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "The session has no Claude credential",
        ))
    })?;
    if !credential_serves(db, &cred_id, &row.id).await? {
        return Err(credential_not_lent());
    }
    let claude = claude_credential(db, keys, &cred_id, now).await?;
    if let Some(review) = crate::reviews::of_session(db, &row.id).await? {
        return review_spec(db, cfg, &row, &project, &review, claude, now).await;
    }
    let repo = project.repo();
    let grants = grants(db, &project.id).await?;
    let workspace_id = project.workspace_id().to_owned();
    // What it started with, within what its project allows now.
    let github_access = row.github_access();
    let tokens = github_tokens(
        cfg,
        db,
        &workspace_id,
        repo.as_ref(),
        &grants,
        github_access,
        now,
    )
    .await?;
    let repos = (!grants.is_empty()).then(|| {
        grants
            .iter()
            .map(|g| {
                let label = g.name.as_deref().unwrap_or("*");
                let refused = tokens.refused.iter().any(|r| r.eq_ignore_ascii_case(label));
                let unreachable = g
                    .name
                    .as_ref()
                    .is_some_and(|n| tokens.unreachable.iter().any(|u| u.eq_ignore_ascii_case(n)));
                SessionRepo {
                    owner: g.owner.clone(),
                    name: g.name.clone(),
                    access: g.access,
                    unavailable: if refused {
                        Some(refused_notice(g))
                    } else if unreachable {
                        Some(unreachable_notice(cfg, g))
                    } else {
                        None
                    },
                }
            })
            .collect()
    });
    let secrets = crate::api::projects::decrypted_secrets(db, keys, &project.id).await?;
    let git_author = bot_identity(cfg, now).await;
    let mut settings = project.settings();
    // A project that credits nobody never sends a commit email to a sandbox.
    let author = if settings.credit_people {
        creator(db, &workspace_id, &row.created_by).await?
    } else {
        None
    };
    // Agents from before `claude_ai_connectors` ignore it, but every agent
    // passes the project's environment on to Claude. Not stored.
    settings.env = fctl_domain::settings::session_env(&settings);
    // Agents offer `fctl publish` only where it works: the instance must
    // allow it too. Hosts from before the setting leave it out of the
    // agent's configuration, and so never get a screenshot to upload.
    let instance = crate::instance::settings(db).await?;
    settings.publish_screenshots =
        fctl_domain::published::allowed(instance.publish_screenshots, &settings);
    // Each run starts with the project's memory as it is then. Hosts from
    // before it leave both out of the agent's configuration, so their
    // agents offer neither `fctl memory` nor `fctl retro`.
    let memory = if settings.project_memory {
        Some(crate::api::memories::for_spec(db, &project.id).await?)
    } else {
        None
    };
    let retrospectives = settings.retrospectives;
    Ok(SessionSpec {
        session_id: row.session_id(),
        number: row.number.max(0) as u32,
        title: row.title.clone(),
        project: ProjectSpec {
            id: ProjectId(project.id.clone()),
            slug: project.slug.clone(),
            name: project.name.clone(),
            repo,
            default_branch: project.default_branch.clone(),
            settings,
            repos,
        },
        branch: row
            .branch
            .clone()
            .unwrap_or_else(|| format!("fctl/{}", row.number)),
        base_branch: row
            .base_branch
            .clone()
            .unwrap_or_else(|| project.default_branch.clone()),
        prompt: if row.started_at.is_some() {
            None
        } else {
            Some(row.prompt.clone())
        },
        task: Some(row.prompt.clone()),
        task_attachments: row
            .prompt_attachments
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default(),
        author: row.creator_ref(),
        claude_session_id: row.claude_session_id.clone(),
        resume: row.started_at.is_some(),
        model: row.model.clone(),
        effort: row.effort.clone(),
        secrets,
        credentials: SessionCredentials {
            claude,
            github: tokens.write,
            github_read: tokens.read,
        },
        git_author,
        co_authors: Vec::new(),
        co_authored_by: author.into_iter().collect(),
        web_url: cfg.url(&format!("/s/{}", row.id)),
        idle_timeout_secs: fctl_domain::settings::idle_timeout_secs(&project.settings()),
        run_id: None,
        // Agents follow them; their checks are for the session's viewers.
        pull_requests: row
            .to_session(None)
            .pull_requests
            .into_iter()
            .map(|pr| PullRequestRef { checks: None, ..pr })
            .collect(),
        memory,
        retrospectives,
        github_access: (!github_access.pushes()).then_some(github_access),
        review: None,
    })
}

/// A review agent's spec (`crate::reviews`): the reviewed repository is
/// its working directory, checked out at the pull request's commit, and
/// the project's others, its primary one too, it reads. It reads alone on
/// GitHub, gets none of the project's secrets, setup script, memory or
/// claude.ai connectors, and is told how to review. Its session stops once
/// the review is posted and the agent's turn has ended, and a minute after
/// its last turn without one (`idle_timeout_secs`).
async fn review_spec(
    db: &D1Database,
    cfg: &Config,
    row: &SessionRow,
    project: &ProjectRow,
    review: &crate::reviews::ReviewRow,
    claude: ClaudeCredential,
    now: i64,
) -> AppResult<SessionSpec> {
    let reviewed = review
        .repo()
        .ok_or_else(|| internal("a review without a repository"))?;
    let mut grants: Vec<RepoGrant> = grants(db, &project.id)
        .await?
        .into_iter()
        .filter(|g| g.repo().is_none_or(|r| !r.same(&reviewed)))
        .collect();
    if let Some(primary) = project.repo().filter(|p| !p.same(&reviewed)) {
        grants.insert(
            0,
            RepoGrant {
                owner: primary.owner,
                name: Some(primary.name),
                access: fctl_proto::session::RepoAccess::Read,
            },
        );
    }
    let tokens = github_tokens(
        cfg,
        db,
        project.workspace_id(),
        Some(&reviewed),
        &grants,
        GithubAccess::Read,
        now,
    )
    .await?;
    let repos = (!grants.is_empty()).then(|| {
        grants
            .iter()
            .map(|g| {
                let label = g.name.as_deref().unwrap_or("*");
                let refused = tokens.refused.iter().any(|r| r.eq_ignore_ascii_case(label));
                let unreachable = g
                    .name
                    .as_ref()
                    .is_some_and(|n| tokens.unreachable.iter().any(|u| u.eq_ignore_ascii_case(n)));
                SessionRepo {
                    owner: g.owner.clone(),
                    name: g.name.clone(),
                    access: fctl_proto::session::RepoAccess::Read,
                    unavailable: if refused {
                        Some(refused_notice(g))
                    } else if unreachable {
                        Some(unreachable_notice(cfg, g))
                    } else {
                        None
                    },
                }
            })
            .collect()
    });
    let p = project.settings();
    let mut settings = ProjectSettings {
        image: p.image.clone(),
        image_build: p.image_build.clone(),
        setup_script: None,
        // What a pull request steers its agent to download must not reach
        // the project's other sessions.
        package_caches: false,
        model: p.model.clone(),
        effort: p.effort.clone(),
        instructions: None,
        cpus: p.cpus,
        memory_mb: p.memory_mb,
        idle_timeout_minutes: Some(1),
        auto_pull_request: false,
        pull_request_descriptions: Default::default(),
        remote_control: false,
        claude_ai_connectors: false,
        credit_people: false,
        credit_claude: p.credit_claude,
        publish_screenshots: false,
        screenshot_days: None,
        project_memory: false,
        retrospectives: false,
        github_access: Some(GithubAccess::Read),
        follow_ups: Default::default(),
        env: p.env.clone(),
    };
    settings.env = fctl_domain::settings::session_env(&settings);
    let conf = crate::reviews::settings(db, &project.id).await?;
    let branch = row
        .branch
        .clone()
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| format!("pull/{}", review.number));
    let base = row
        .base_branch
        .clone()
        .unwrap_or_else(|| project.default_branch.clone());
    let spec_review = crate::reviews::spec_of(
        review,
        &branch,
        &base,
        conf.as_ref().and_then(|c| c.instructions.as_deref()),
    );
    Ok(SessionSpec {
        session_id: row.session_id(),
        number: row.number.max(0) as u32,
        title: row.title.clone(),
        project: ProjectSpec {
            id: ProjectId(project.id.clone()),
            slug: project.slug.clone(),
            name: project.name.clone(),
            repo: Some(reviewed),
            default_branch: base.clone(),
            settings,
            repos,
        },
        branch,
        base_branch: base,
        prompt: if row.started_at.is_some() {
            None
        } else {
            Some(row.prompt.clone())
        },
        task: Some(row.prompt.clone()),
        task_attachments: Vec::new(),
        author: row.creator_ref(),
        claude_session_id: row.claude_session_id.clone(),
        resume: row.started_at.is_some(),
        model: row.model.clone(),
        effort: row.effort.clone(),
        secrets: Default::default(),
        credentials: SessionCredentials {
            claude,
            github: tokens.write,
            github_read: tokens.read,
        },
        git_author: bot_identity(cfg, now).await,
        co_authors: Vec::new(),
        co_authored_by: Vec::new(),
        web_url: cfg.url(&format!("/s/{}", row.id)),
        idle_timeout_secs: 60,
        run_id: None,
        pull_requests: row
            .to_session(None)
            .pull_requests
            .into_iter()
            .map(|pr| PullRequestRef { checks: None, ..pr })
            .collect(),
        memory: None,
        retrospectives: false,
        github_access: Some(GithubAccess::Read),
        review: spec_review,
    })
}

/// The person who started the session, as its commits credit them: with
/// the commit email they chose for the address they are a member of the
/// workspace as, never a sign-in email. `None` without one, and then
/// commits don't name them.
async fn creator(
    db: &D1Database,
    workspace_id: &str,
    user_id: &str,
) -> AppResult<Option<GitIdentity>> {
    #[derive(serde::Deserialize)]
    struct U {
        name: String,
        commit_email: Option<String>,
    }
    let u: Option<U> = q(
        db,
        "SELECT u.name AS name, e.commit_email AS commit_email FROM users u \
         JOIN workspace_members m ON m.user_id = u.id AND m.workspace_id = ? \
         LEFT JOIN user_emails e ON e.email = m.email AND e.user_id = u.id WHERE u.id = ?",
        &[s(workspace_id), s(user_id)],
    )?
    .first()
    .await?;
    Ok(u.and_then(|U { name, commit_email }| {
        commit_email
            .filter(|e| !e.is_empty())
            .map(|email| GitIdentity { name, email })
    }))
}

/// Commits are authored by the GitHub App's bot account.
async fn bot_identity(cfg: &Config, _now: i64) -> GitIdentity {
    let Some(gh) = cfg.github.as_ref() else {
        return GitIdentity {
            name: "fctl agent".into(),
            email: "agent@fctl.invalid".into(),
        };
    };
    let login = format!("{}[bot]", gh.slug);
    if let Some((name, email)) = BOT_EMAIL.with(|b| b.borrow().clone()) {
        return GitIdentity { name, email };
    }
    let mut email = format!("{login}@users.noreply.github.com");
    if let Ok(req) = Request::new(
        &format!("https://api.github.com/users/{}%5Bbot%5D", gh.slug),
        Method::Get,
    ) {
        let mut req = req;
        if let Ok(h) = req.headers_mut() {
            let _ = h.set("User-Agent", "fctl-control");
            let _ = h.set("Accept", "application/vnd.github+json");
        }
        if let Ok(mut res) = Fetch::Request(req).send().await
            && let Ok(v) = res.json::<serde_json::Value>().await
            && let Some(id) = v.get("id").and_then(serde_json::Value::as_u64)
        {
            email = format!("{id}+{login}@users.noreply.github.com");
        }
    }
    BOT_EMAIL.with(|b| *b.borrow_mut() = Some((login.clone(), email.clone())));
    GitIdentity { name: login, email }
}

/// A session's GitHub tokens (none without a GitHub App or repositories),
/// within what its workspace may use, for an agent that may do `access` on
/// GitHub. Listed repositories the workspace may not use, or the GitHub App
/// cannot reach, are left out; without the primary repository the session
/// cannot start.
pub async fn github_tokens(
    cfg: &Config,
    db: &D1Database,
    workspace_id: &str,
    primary: Option<&RepoRef>,
    grants: &[RepoGrant],
    access: GithubAccess,
    now: i64,
) -> AppResult<github::SessionTokens> {
    let (Some(gh), Some(plan)) = (
        cfg.github.as_ref(),
        fctl_domain::repos::token_plan(primary, grants),
    ) else {
        return Ok(github::SessionTokens::default());
    };
    let connections = crate::workspaces::connections(db, workspace_id).await?;
    let scope = crate::workspaces::scope(&connections, &plan.owner);
    let (plan, refused) = fctl_domain::repos::narrow(plan, scope.as_ref());
    // An agent that only reads gets one read-only token for everything.
    let plan = if access == GithubAccess::Read {
        plan.reading_only()
    } else {
        plan
    };
    if let Some(p) = primary
        && refused.iter().any(|n| n.eq_ignore_ascii_case(&p.name))
    {
        return Err(AppError(ApiError::new(
            ErrorCode::NotConfigured,
            format!(
                "This project's workspace may not use {}. Its admins connect {} with it, under the workspace's GitHub tab.",
                p.full_name(),
                p.owner
            ),
        )));
    }
    let mut tokens = if plan.write.is_empty() && plan.read == fctl_domain::repos::ReadScope::None {
        github::SessionTokens::default()
    } else {
        // Through the installation on the account the workspace connected,
        // while its login is still that account's.
        let gh = github::from_config(cfg, now)?;
        let inst = crate::workspaces::installation(db, &gh, &connections, &plan.owner).await?;
        gh.session_tokens(inst, &plan, access).await?
    };
    tokens.refused = refused;
    if let Some(p) = primary
        && tokens
            .unreachable
            .iter()
            .any(|n| n.eq_ignore_ascii_case(&p.name))
    {
        return Err(AppError(ApiError::new(
            ErrorCode::NotConfigured,
            format!(
                "The {} GitHub App can't reach {}. Add the repository to the app's installation on {}, or link the project to another one.",
                gh.slug,
                p.full_name(),
                p.owner
            ),
        )));
    }
    Ok(tokens)
}

/// What the agent tells people about a listed repository its workspace may
/// not use.
fn refused_notice(g: &RepoGrant) -> String {
    format!(
        "This project's workspace may not use {}, so this session works without it. Its admins connect {} with it under the workspace's GitHub tab, or remove the repository from the project.",
        g.label(),
        g.owner
    )
}

/// What the agent tells people about a listed repository it has no token for.
fn unreachable_notice(cfg: &Config, g: &RepoGrant) -> String {
    let app = cfg.github.as_ref().map_or("fctl", |gh| gh.slug.as_str());
    format!(
        "The {app} GitHub App can't reach {}, so this session works without it. Add the repository to the app's installation on {}, or remove it from the project.",
        g.label(),
        g.owner
    )
}

/// The credential with a valid access token, refreshing an OAuth login when
/// it is about to expire. Callers serialize refreshes per credential.
pub async fn claude_credential(
    db: &D1Database,
    keys: &Keys,
    id: &str,
    now: i64,
) -> AppResult<ClaudeCredential> {
    let row: Option<CredentialRow> = q(
        db,
        "SELECT * FROM claude_credentials WHERE id = ?",
        &[s(id)],
    )?
    .first()
    .await?;
    let row = row.ok_or_else(|| {
        AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "The Claude credential was removed",
        ))
    })?;
    if row.status != "active" {
        return Err(AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "The Claude credential is no longer valid. Connect Claude again in Settings.",
        )));
    }
    let aad = CredentialRow::aad(&row.id, &row.user_id);
    let mut secret: CredentialSecret = keys.open_json(&aad, &row.secret_enc)?;
    let kind = row.kind();
    if kind == ClaudeCredentialKind::Oauth
        && secret
            .expires_at
            .is_some_and(|e| e - now < REFRESH_MARGIN_MS)
    {
        secret = refresh(db, keys, &row, &secret, now).await?;
    }
    q(
        db,
        "UPDATE claude_credentials SET last_used_at = ? WHERE id = ?",
        &[n(now), s(&row.id)],
    )?
    .run()
    .await?;
    Ok(ClaudeCredential {
        id: CredentialId(row.id.clone()),
        kind,
        access_token: secret.access_token,
        scopes: row.scopes(),
        expires_at: secret.expires_at,
        subscription_type: row.subscription_type.clone(),
        account: row.account(),
    })
}

async fn refresh(
    db: &D1Database,
    keys: &Keys,
    row: &CredentialRow,
    secret: &CredentialSecret,
    now: i64,
) -> AppResult<CredentialSecret> {
    let refresh_token = secret.refresh_token.clone().ok_or_else(|| {
        AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "This Claude login cannot be refreshed. Connect Claude again.",
        ))
    })?;
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLAUDE_CLIENT_ID,
        "scope": row.scopes().join(" "),
    });
    let headers = Headers::new();
    headers.set("Content-Type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(body.to_string().into()));
    let mut res = Fetch::Request(Request::new_with_init(CLAUDE_TOKEN_URL, &init)?)
        .send()
        .await?;
    let status = res.status_code();
    let v: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
    if status == 400 || status == 401 {
        // invalid_grant: the login was revoked or used elsewhere.
        q(
            db,
            "UPDATE claude_credentials SET status = 'invalid', updated_at = ? WHERE id = ?",
            &[n(now), s(&row.id)],
        )?
        .run()
        .await?;
        return Err(AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "Claude rejected the stored login. Connect Claude again in Settings.",
        )));
    }
    if status != 200 {
        return Err(AppError(ApiError::new(
            ErrorCode::Unavailable,
            format!("Claude's token service answered {status}"),
        )));
    }
    let access_token = v
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| internal("token response without access_token"))?
        .to_owned();
    let new = CredentialSecret {
        access_token,
        refresh_token: v
            .get("refresh_token")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .or(secret.refresh_token.clone()),
        expires_at: v
            .get("expires_in")
            .and_then(serde_json::Value::as_i64)
            .map(|s| now + s * 1000),
    };
    let sealed = keys.seal_json(&CredentialRow::aad(&row.id, &row.user_id), &new)?;
    q(
        db,
        "UPDATE claude_credentials SET secret_enc = ?, expires_at = ?, updated_at = ? WHERE id = ?",
        &[s(&sealed), on(new.expires_at), n(now), s(&row.id)],
    )?
    .run()
    .await?;
    Ok(new)
}
