//! The GitHub accounts workspaces work in (docs/architecture.md#github):
//! `/api/v1/workspaces/{id}/github`, `/api/v1/github/installed` (the
//! GitHub App's setup URL) and `/api/v1/admin/github`.
//!
//! The instance has one GitHub App; installing it on an account lets the
//! instance mint tokens for that account's repositories. A workspace uses
//! an account once it is connected: an instance admin connects it, or
//! approves a workspace admin's request, since anyone could claim an
//! installation's id and its tokens can write.

use crate::ctx::Ctx;
use crate::db::{n, os, q, s};
use crate::error::{AppResult, forbidden, not_found, validation};
use crate::github::{self, InstallationInfo};
use crate::http;
use crate::model::workspace_ref;
use crate::workspaces::{self as ws, CONNECTION_SELECT, ConnectionRow, WorkspaceRow};
use fctl_domain::repos;
use fctl_proto::api::*;
use fctl_proto::session::WorkspaceRole;
use serde::{Deserialize, Serialize};
use worker::{Request, Response};

/// How long the state of an installation started from a workspace works.
const STATE_TTL_MS: i64 = 60 * 60 * 1000;

/// What the GitHub App's setup URL gets back: who installed it for which
/// workspace (signed with the master key).
#[derive(Serialize, Deserialize)]
struct InstallState {
    purpose: String,
    workspace: String,
    user: String,
    exp: i64,
}

const PURPOSE: &str = "github-install";

/// Installs the app on another account and comes back to `workspace`.
fn install_url(ctx: &Ctx, slug: &str, workspace_id: &str) -> AppResult<String> {
    let base = format!("https://github.com/apps/{slug}/installations/new");
    let Ok(keys) = ctx.keys() else {
        return Ok(base);
    };
    let state = keys.sign(&serde_json::to_vec(&InstallState {
        purpose: PURPOSE.into(),
        workspace: workspace_id.to_owned(),
        user: ctx.user()?.id.clone(),
        exp: ctx.now + STATE_TTL_MS,
    })?);
    let encoded: String = url::form_urlencoded::byte_serialize(state.as_bytes()).collect();
    Ok(format!("{base}?state={encoded}"))
}

/// `GET /api/v1/workspaces/{id}/github`.
pub async fn workspace(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    let role = a.require_open()?;
    respond(ctx, &a.row, role).await
}

async fn respond(ctx: &Ctx, row: &WorkspaceRow, role: WorkspaceRole) -> AppResult<Response> {
    let connections = ws::connections(&ctx.db, &row.id).await?;
    let user = ctx.user()?;
    http::ok(&WorkspaceGithub {
        configured: ctx.cfg.github.is_some(),
        app_slug: ctx.cfg.github.as_ref().map(|g| g.slug.clone()),
        install_url: match (&ctx.cfg.github, role.is_admin()) {
            (Some(g), true) => Some(install_url(ctx, &g.slug, &row.id)?),
            _ => None,
        },
        connections: connections
            .iter()
            .map(ConnectionRow::to_connection)
            .collect(),
        approval_required: !user.is_admin(),
    })
}

/// Connects `info`'s account to a workspace, narrowed to `repos`: right
/// away for instance admins, otherwise as a request for them. Returns
/// whether it is connected.
async fn connect_account(
    ctx: &Ctx,
    row: &WorkspaceRow,
    info: &InstallationInfo,
    repos: Option<Vec<String>>,
) -> AppResult<bool> {
    let user = ctx.user()?.clone();
    let connections = ws::connections(&ctx.db, &row.id).await?;
    let existing = connections
        .iter()
        .find(|c| c.account.eq_ignore_ascii_case(&info.account));
    let repos_json = repos.as_ref().map(serde_json::to_string).transpose()?;
    if let Some(c) = existing.filter(|c| c.active()) {
        if c.repos() == repos {
            return Ok(true);
        }
        if !user.is_admin() {
            return Err(forbidden(
                "An instance admin changes which repositories a workspace may use. Remove the account and ask again to change them.",
            ));
        }
    }
    let active = user.is_admin();
    q(
        &ctx.db,
        "INSERT INTO workspace_github (workspace_id, account, installation_id, repos, status, requested_by, approved_by, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (workspace_id, account) DO UPDATE SET account = excluded.account, \
         installation_id = excluded.installation_id, repos = excluded.repos, status = excluded.status, \
         requested_by = COALESCE(workspace_github.requested_by, excluded.requested_by), \
         approved_by = excluded.approved_by, updated_at = excluded.updated_at",
        &[
            s(&row.id),
            s(&info.account),
            n(info.id as i64),
            os(repos_json.as_deref()),
            s(if active { "active" } else { "requested" }),
            s(&user.id),
            os(active.then_some(user.id.as_str())),
            n(ctx.now),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    let data = serde_json::json!({ "account": info.account, "repos": repos });
    ctx.audit_in(
        Some(&row.id),
        if active {
            "workspace.github_connected"
        } else {
            "workspace.github_requested"
        },
        Some(&info.account),
        Some(data.clone()),
    );
    if active {
        ctx.audit(
            "github.connected",
            Some(&info.account),
            Some(serde_json::json!({ "workspace": row.id, "repos": repos })),
        );
        ws::changed(ctx, ws::member_ids(&ctx.db, &row.id).await?);
    } else {
        ctx.audit(
            "github.requested",
            Some(&info.account),
            Some(serde_json::json!({ "workspace": row.id, "repos": repos })),
        );
        tell_admins(ctx, row, &info.account).await?;
    }
    Ok(active)
}

/// Tells the instance's admins that a workspace asks to use an account: in
/// their feeds, and by email when it is set up.
async fn tell_admins(ctx: &Ctx, row: &WorkspaceRow, account: &str) -> AppResult<()> {
    #[derive(Deserialize)]
    struct Admin {
        id: String,
        email: String,
    }
    let admins: Vec<Admin> = q(
        &ctx.db,
        "SELECT id, email FROM users WHERE role IN ('owner', 'admin') AND status = 'active'",
        &[],
    )?
    .all()
    .await?;
    ws::changed(ctx, admins.iter().map(|a| a.id.clone()).collect());
    if let Some(mail) = ctx.cfg.email.clone() {
        let who = ctx.user()?.name.clone();
        let text = format!(
            "{who} asks for {} to work in {account} on GitHub, where the instance's GitHub App is installed.\n\n\
             Approve or decline it: {}",
            row.name,
            ctx.cfg.url("/admin?tab=github"),
        );
        let subject = format!("{} asks to use {account}", row.name);
        for a in admins {
            let (mail, text, subject) = (mail.clone(), text.clone(), subject.clone());
            ctx.defer(async move { crate::email::send(&mail, &a.email, &subject, &text).await });
        }
    }
    Ok(())
}

/// `POST /api/v1/workspaces/{id}/github` (admins).
pub async fn connect(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: ConnectGithubRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    let role = a.require_admin()?;
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    let account = repos::account(&body.account)?;
    let slug = ctx.cfg.github.as_ref().map_or("fctl", |g| g.slug.as_str());
    let info = gh.installation_on(&account).await?.ok_or_else(|| {
        validation(
            "account",
            &format!("The {slug} GitHub App isn't installed on {account}. Install it there first."),
        )
    })?;
    let repos = body
        .repos
        .as_deref()
        .map(|r| repos::scope_names(&info.account, r))
        .transpose()?;
    connect_account(ctx, &a.row, &info, repos).await?;
    respond(ctx, &a.row, role).await
}

/// `DELETE /api/v1/workspaces/{id}/github/{account}`: the workspace's
/// admins and instance admins take an account away from a workspace (or a
/// request back). Its projects' sessions get no more tokens for it.
pub async fn disconnect(ctx: &mut Ctx, id: &str, account: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let a = ws::load(ctx, id).await?;
    let role = a.require_open()?;
    if !role.is_admin() && !user.is_admin() {
        return Err(forbidden("Only the workspace's admins can do this"));
    }
    let removed = q(
        &ctx.db,
        "DELETE FROM workspace_github WHERE workspace_id = ? AND account = ?",
        &[s(a.id()), s(account)],
    )?
    .run()
    .await?;
    if removed == 0 {
        return Err(not_found("GitHub account"));
    }
    ctx.audit_in(
        Some(a.id()),
        "workspace.github_disconnected",
        Some(account),
        None,
    );
    ws::changed(ctx, ws::member_ids(&ctx.db, a.id()).await?);
    respond(ctx, &a.row, role).await
}

/// `GET /api/v1/github/installed`: the GitHub App's setup URL, where GitHub
/// sends people after they install or configure it. With the state of an
/// installation started from a workspace, the account is connected to it
/// (or asked for), and the browser goes back to the workspace.
pub async fn installed(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        let here = req.url()?;
        let next: String = url::form_urlencoded::byte_serialize(
            format!("{}?{}", here.path(), here.query().unwrap_or_default()).as_bytes(),
        )
        .collect();
        return http::redirect(&format!("/login?next={next}"));
    };
    let state = http::query(req, "state")
        .and_then(|st| ctx.keys().ok()?.verify(&st))
        .and_then(|b| serde_json::from_slice::<InstallState>(&b).ok())
        .filter(|st| st.purpose == PURPOSE && st.exp > ctx.now && st.user == user.id);
    let Some(state) = state else {
        // Installed from GitHub, or the state ran out: instance admins
        // connect it to workspaces.
        return http::redirect(if user.is_admin() {
            "/admin?tab=github"
        } else {
            "/"
        });
    };
    let a = ws::load(ctx, &state.workspace).await?;
    let back = |outcome: &str, account: &str| {
        let account: String = url::form_urlencoded::byte_serialize(account.as_bytes()).collect();
        format!(
            "/w/{}?tab=github&github={outcome}&account={account}",
            a.row.slug
        )
    };
    if http::query(req, "setup_action").as_deref() == Some("request") {
        // An organization's member asked its owners to install the app.
        return http::redirect(&back("asked_owner", ""));
    }
    a.require_admin()?;
    let installation = http::query(req, "installation_id")
        .and_then(|i| i.parse::<u64>().ok())
        .ok_or_else(|| validation("installation_id", "GitHub sent no installation"))?;
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    let info = gh.installation(installation).await?;
    let connected = connect_account(ctx, &a.row, &info, None).await?;
    http::redirect(&back(
        if connected { "connected" } else { "requested" },
        &info.account,
    ))
}

// ───────────────────────────── instance admins ─────────────────────────────

#[derive(Deserialize)]
struct AdminConnectionRow {
    #[serde(flatten)]
    connection: ConnectionRow,
    slug: String,
    name: String,
    color: Option<String>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    avatar_updated_at: Option<i64>,
}

/// `GET /api/v1/admin/github`: every account the app is installed on, and
/// the workspaces that use it or ask to.
pub async fn admin_list(ctx: &Ctx) -> AppResult<Response> {
    ctx.admin()?;
    let rows: Vec<AdminConnectionRow> = q(
        &ctx.db,
        &format!(
            "SELECT * FROM ({CONNECTION_SELECT}) c JOIN (SELECT id AS wid, slug, name, color, avatar_updated_at FROM workspaces) w \
             ON w.wid = c.workspace_id ORDER BY c.created_at"
        ),
        &[],
    )?
    .all()
    .await?;
    let (installations, slug) = match ctx.cfg.github.as_ref() {
        Some(g) => (
            github::from_config(&ctx.cfg, ctx.now)?
                .installation_list()
                .await
                .unwrap_or_default(),
            Some(g.slug.clone()),
        ),
        None => (Vec::new(), None),
    };
    let connection = |r: &AdminConnectionRow| GithubWorkspaceConnection {
        workspace: workspace_ref(
            &r.connection.workspace_id,
            &r.slug,
            &r.name,
            r.color.as_deref(),
            r.avatar_updated_at,
        ),
        repos: r.connection.repos(),
        status: r.connection.status(),
        requested_by: r.connection.requester(),
        created_at: r.connection.created_at,
    };
    let mut out: Vec<GithubInstallation> = installations
        .iter()
        .map(|i| GithubInstallation {
            account: i.account.clone(),
            account_type: i.account_type.clone(),
            installation_id: Some(i.id),
            repository_selection: i.repository_selection.clone(),
            installed_at: i.created_at,
            workspaces: rows
                .iter()
                .filter(|r| r.connection.account.eq_ignore_ascii_case(&i.account))
                .map(connection)
                .collect(),
        })
        .collect();
    // Accounts workspaces name where the app isn't installed (any more).
    for r in &rows {
        if let Some(entry) = out
            .iter_mut()
            .find(|i| i.account.eq_ignore_ascii_case(&r.connection.account))
        {
            if entry.installation_id.is_none() {
                entry.workspaces.push(connection(r));
            }
            continue;
        }
        out.push(GithubInstallation {
            account: r.connection.account.clone(),
            account_type: None,
            installation_id: None,
            repository_selection: None,
            installed_at: None,
            workspaces: vec![connection(r)],
        });
    }
    out.sort_by_key(|i| i.account.to_lowercase());
    http::ok(&GithubInstallations {
        configured: slug.is_some(),
        install_url: slug
            .as_ref()
            .map(|s| format!("https://github.com/apps/{s}/installations/new")),
        app_slug: slug,
        installations: out,
    })
}

/// `PUT /api/v1/admin/github/{account}/workspaces/{workspace}`: the
/// workspace uses the account, with these repositories (approving its
/// request, when it asked).
pub async fn admin_connect(
    ctx: &mut Ctx,
    req: &mut Request,
    account: &str,
    workspace: &str,
) -> AppResult<Response> {
    let body: SetGithubConnectionRequest = http::body(req).await?;
    let admin = ctx.admin()?.clone();
    let row = ws::find(&ctx.db, &admin.id, workspace)
        .await?
        .ok_or_else(|| not_found("Workspace"))?;
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    let account = repos::account(account)?;
    let info = gh.installation_on(&account).await?.ok_or_else(|| {
        validation(
            "account",
            &format!("The GitHub App isn't installed on {account}"),
        )
    })?;
    let repos = body
        .repos
        .as_deref()
        .map(|r| repos::scope_names(&info.account, r))
        .transpose()?;
    connect_account(ctx, &row, &info, repos).await?;
    admin_list(ctx).await
}

/// `DELETE /api/v1/admin/github/{account}/workspaces/{workspace}`: the
/// workspace no longer uses the account, or its request is declined.
pub async fn admin_disconnect(
    ctx: &mut Ctx,
    account: &str,
    workspace: &str,
) -> AppResult<Response> {
    let admin = ctx.admin()?.clone();
    let row = ws::find(&ctx.db, &admin.id, workspace)
        .await?
        .ok_or_else(|| not_found("Workspace"))?;
    let removed = q(
        &ctx.db,
        "DELETE FROM workspace_github WHERE workspace_id = ? AND account = ? RETURNING status",
        &[s(&row.id), s(account)],
    )?
    .all::<serde_json::Value>()
    .await?;
    let Some(status) = removed
        .first()
        .and_then(|r| r.get("status"))
        .and_then(serde_json::Value::as_str)
    else {
        return Err(not_found("GitHub account"));
    };
    let action = if status == "requested" {
        "github.declined"
    } else {
        "github.disconnected"
    };
    ctx.audit(
        action,
        Some(account),
        Some(serde_json::json!({ "workspace": row.id })),
    );
    ctx.audit_in(
        Some(&row.id),
        if status == "requested" {
            "workspace.github_declined"
        } else {
            "workspace.github_disconnected"
        },
        Some(account),
        None,
    );
    ws::changed(ctx, ws::member_ids(&ctx.db, &row.id).await?);
    admin_list(ctx).await
}
