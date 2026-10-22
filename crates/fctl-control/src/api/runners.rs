//! `/api/v1/runners/*` (admin) and `/api/v1/runner/*` (hosts).

use crate::ctx::Ctx;
use crate::db::{q, s};
use crate::error::{AppError, AppResult, forbidden, not_found};
use crate::http;
use crate::internal::{self, FeedIdentity, LiveRunners};
use crate::model::{RUNNER_SELECT, RunnerRow, RunnerView};
use fctl_domain::tokens::{SecretKind, hash_secret, new_secret};
use fctl_domain::validate;
use fctl_proto::ApiError;
use fctl_proto::api::*;
use worker::{Request, Response};

async fn live(ctx: &Ctx) -> LiveRunners {
    match internal::fleet_stub(&ctx.env) {
        Ok(stub) => internal::get(&stub, "/runners").await.unwrap_or_default(),
        Err(_) => LiveRunners::default(),
    }
}

/// What the caller sees of runners: instance admins all of them; others
/// those of their workspaces, and their own.
async fn view(ctx: &Ctx) -> AppResult<(Option<Vec<String>>, String)> {
    crate::api::workspaces::runner_view_for(ctx).await
}

/// `GET /api/v1/runners`: the runners the caller sees.
pub async fn list(ctx: &Ctx) -> AppResult<Response> {
    let (workspaces, user_id) = view(ctx).await?;
    let view = RunnerView {
        user_id: Some(&user_id),
        workspaces: workspaces.as_deref(),
    };
    let rows: Vec<RunnerRow> = q(
        &ctx.db,
        &format!("{RUNNER_SELECT} ORDER BY r.created_at"),
        &[],
    )?
    .all()
    .await?;
    let live = live(ctx).await;
    http::ok(&RunnerList {
        runners: rows
            .iter()
            .filter(|r| r.visible_to(&view))
            .map(|r| r.to_runner(&live, &view))
            .collect(),
    })
}

/// `POST /api/v1/runners`: registers a runner for a workspace the caller
/// is in: for everyone's sessions there (its admins), or for the caller's
/// own (`api::workspaces::runs_whose`). Without `workspace`, the one they
/// are an admin of, else the one they are a member of. Instance admins may
/// register one for no workspace yet.
pub async fn create(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: CreateRunnerRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    let workspace = match body.workspace.as_deref().filter(|w| !w.trim().is_empty()) {
        Some(w) => Some(w.trim().to_owned()),
        None => {
            let mine = crate::workspaces::mine(&ctx.db, &user.id).await?;
            let admin_of: Vec<&str> = mine
                .iter()
                .filter(|w| {
                    w.role()
                        .is_some_and(fctl_proto::session::WorkspaceRole::is_admin)
                })
                .map(|w| w.id.as_str())
                .collect();
            match (admin_of.as_slice(), mine.as_slice()) {
                ([only], _) => Some((*only).to_owned()),
                ([], [only]) => Some(only.id.clone()),
                ([], []) if user.is_admin() => None,
                ([], []) => {
                    return Err(forbidden(
                        "A runner runs the sessions of a workspace you are in, and you are in none yet",
                    ));
                }
                _ => {
                    return Err(crate::error::validation(
                        "workspace",
                        "Choose the workspace whose sessions it runs (--workspace)",
                    ));
                }
            }
        }
    };
    // The same checks whether it is named or not: open to them (its
    // two-factor requirement), and whose sessions they may have it run.
    let (workspace, personal) = match workspace {
        Some(w) => {
            let a = crate::workspaces::load(ctx, &w).await?;
            let personal = crate::api::workspaces::runs_whose(&a, body.personal)?;
            (Some(a.row.id), personal)
        }
        None => (None, false),
    };
    let reg =
        crate::api::workspaces::register_runner(ctx, &body.name, workspace.as_deref(), personal)
            .await?;
    http::created(&reg)
}

/// The caller may manage a runner as a machine (rename, disable, drain,
/// remove): its owner, or an instance admin. A new token is for its owner
/// alone ([`rotate_token`]).
async fn require_manager(ctx: &Ctx, id: &str) -> AppResult<()> {
    let user = ctx.user()?;
    if user.is_admin() {
        return Ok(());
    }
    #[derive(serde::Deserialize)]
    struct Owner {
        created_by: Option<String>,
    }
    let owner: Option<Owner> = q(
        &ctx.db,
        "SELECT created_by FROM runners WHERE id = ?",
        &[s(id)],
    )?
    .first()
    .await?;
    match owner {
        Some(o) if o.created_by.as_deref() == Some(user.id.as_str()) => Ok(()),
        Some(_) => Err(forbidden(
            "Only the runner's owner and instance admins can do this",
        )),
        None => Err(not_found("Runner")),
    }
}

/// One runner as the caller sees it.
async fn one(ctx: &Ctx, id: &str, live: &LiveRunners) -> AppResult<Runner> {
    let (workspaces, user_id) = view(ctx).await?;
    let view = RunnerView {
        user_id: Some(&user_id),
        workspaces: workspaces.as_deref(),
    };
    let rows: Vec<RunnerRow> = q(
        &ctx.db,
        &format!("{RUNNER_SELECT} WHERE r.id = ?"),
        &[s(id)],
    )?
    .all()
    .await?;
    Ok(rows
        .first()
        .ok_or_else(|| not_found("Runner"))?
        .to_runner(live, &view))
}

pub async fn update(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdateRunnerRequest = http::body(req).await?;
    require_manager(ctx, id).await?;
    let name = body
        .name
        .as_deref()
        .map(|v| validate::name("name", v))
        .transpose()?;
    let changed = q(
        &ctx.db,
        "UPDATE runners SET name = COALESCE(?, name), enabled = COALESCE(?, enabled) WHERE id = ?",
        &[
            crate::db::os(name.as_deref()),
            body.enabled.map_or(crate::db::null(), crate::db::b),
            s(id),
        ],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(not_found("Runner"));
    }
    if body.enabled == Some(false) {
        let stub = internal::fleet_stub(&ctx.env)?;
        let _: serde_json::Value = internal::post(
            &stub,
            "/runner-disabled",
            &serde_json::json!({ "runner_id": id }),
        )
        .await?;
    }
    if let Some(draining) = body.draining {
        let name: Option<NameRow> = q(&ctx.db, "SELECT name FROM runners WHERE id = ?", &[s(id)])?
            .first()
            .await?;
        let by = ctx.user()?.to_ref();
        drain(
            ctx,
            id,
            &name.map(|n| n.name).unwrap_or_default(),
            draining,
            false,
            Some(by),
        )
        .await?;
    }
    ctx.audit("runner.updated", Some(id), None);
    let live = live(ctx).await;
    http::ok(&one(ctx, id, &live).await?)
}

pub async fn delete(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    require_manager(ctx, id).await?;
    let stub = internal::fleet_stub(&ctx.env)?;
    let _: serde_json::Value = internal::post(
        &stub,
        "/runner-disabled",
        &serde_json::json!({ "runner_id": id }),
    )
    .await?;
    let changed = q(&ctx.db, "DELETE FROM runners WHERE id = ?", &[s(id)])?
        .run()
        .await?;
    if changed == 0 {
        return Err(not_found("Runner"));
    }
    ctx.audit("runner.deleted", Some(id), None);
    http::ok(&Ok { ok: true })
}

/// `POST /api/v1/runners/{id}/token`: a new token for a runner, for the
/// person who registered it (`fctl_domain::workspace::check_replace_runner_token`).
pub async fn rotate_token(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let user_id = ctx.user()?.id.clone();
    #[derive(serde::Deserialize)]
    struct Owner {
        created_by: Option<String>,
    }
    let owner: Option<Owner> = q(
        &ctx.db,
        "SELECT created_by FROM runners WHERE id = ?",
        &[s(id)],
    )?
    .first()
    .await?;
    let owner = owner.ok_or_else(|| not_found("Runner"))?;
    fctl_domain::workspace::check_replace_runner_token(
        owner.created_by.as_deref() == Some(user_id.as_str()),
    )?;
    let token = new_secret(SecretKind::Runner);
    // Only while it is still theirs.
    let changed = q(
        &ctx.db,
        "UPDATE runners SET token_hash = ? WHERE id = ? AND created_by = ?",
        &[s(&hash_secret(&token)), s(id), s(&user_id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(not_found("Runner"));
    }
    let stub = internal::fleet_stub(&ctx.env)?;
    let _: serde_json::Value = internal::post(
        &stub,
        "/runner-disabled",
        &serde_json::json!({ "runner_id": id }),
    )
    .await?;
    ctx.audit("runner.token_rotated", Some(id), None);
    let runner = one(ctx, id, &LiveRunners::default()).await?;
    http::ok(&RunnerRegistration { runner, token })
}

#[derive(serde::Deserialize)]
struct NameRow {
    name: String,
}

/// Drains a runner or stops draining it (`by`: the admin, or `None` for its
/// host), and records who did. With `stop_sessions` it also stops the
/// sessions the runner runs, which then resume elsewhere.
async fn drain(
    ctx: &Ctx,
    id: &str,
    name: &str,
    draining: bool,
    stop_sessions: bool,
    by: Option<fctl_proto::session::UserRef>,
) -> AppResult<()> {
    let stub = internal::fleet_stub(&ctx.env)?;
    let _: serde_json::Value = internal::post(
        &stub,
        "/drain",
        &internal::Drain {
            runner_id: id.to_owned(),
            runner_name: name.to_owned(),
            draining,
            by: by.clone(),
            stop_sessions: draining && stop_sessions,
        },
    )
    .await?;
    ctx.audit(
        if draining {
            "runner.drained"
        } else {
            "runner.undrained"
        },
        Some(id),
        Some(serde_json::json!({
            "by": if by.is_some() { "admin" } else { "host" },
            "stop_sessions": draining && stop_sessions,
        })),
    );
    Ok(())
}

/// The runner and the sessions it runs, as its host asks for them.
async fn detail(ctx: &Ctx, id: &str) -> AppResult<RunnerDetail> {
    #[derive(serde::Deserialize)]
    struct Row {
        id: String,
        project_slug: String,
        #[serde(deserialize_with = "crate::db::int")]
        number: i64,
        title: String,
        status: String,
        #[serde(deserialize_with = "crate::db::opt_int", default)]
        resume_at: Option<i64>,
    }
    let rows: Vec<RunnerRow> = q(
        &ctx.db,
        &format!("{RUNNER_SELECT} WHERE r.id = ?"),
        &[s(id)],
    )?
    .all()
    .await?;
    let live = live(ctx).await;
    // Its host sees every workspace it serves.
    let runner = rows
        .first()
        .ok_or_else(|| not_found("Runner"))?
        .to_runner(&live, &RunnerView::default());
    let sessions: Vec<Row> = q(
        &ctx.db,
        "SELECT s.id, p.slug AS project_slug, s.number, s.title, s.status, s.resume_at FROM sessions s \
         JOIN projects p ON p.id = s.project_id WHERE s.runner_id = ? \
         AND s.status IN ('provisioning','preparing','running','idle','paused','stopping') ORDER BY s.created_at",
        &[s(id)],
    )?
    .all()
    .await?;
    Ok(RunnerDetail {
        runner,
        sessions: sessions
            .into_iter()
            .filter_map(|r| {
                Some(RunnerSession {
                    id: fctl_proto::SessionId(r.id),
                    project_slug: r.project_slug,
                    number: r.number.max(0) as u32,
                    title: r.title,
                    status: fctl_proto::session::SessionStatus::parse(&r.status)?,
                    resume_at: r.resume_at,
                })
            })
            .collect(),
    })
}

/// `GET /api/v1/runner`: the runner whose token this is, and its sessions.
pub async fn this_runner(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    let r = runner_auth(ctx, req).await?;
    http::ok(&detail(ctx, &r.id).await?)
}

/// `POST /api/v1/runner/drain`: the host drains its runner (`fctl host
/// drain`), or stops draining it.
pub async fn drain_this_runner(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let r = runner_auth(ctx, req).await?;
    let body: DrainRequest = http::body(req).await?;
    let stop = body.stop_sessions.unwrap_or(false);
    drain(ctx, &r.id, &r.name, body.draining, stop, None).await?;
    http::ok(&detail(ctx, &r.id).await?)
}

#[derive(serde::Deserialize)]
pub(crate) struct RunnerAuth {
    pub id: String,
    pub name: String,
}

/// Authenticates a host by its runner token.
pub(crate) async fn runner_auth(ctx: &Ctx, req: &Request) -> AppResult<RunnerAuth> {
    let token = http::header(req, "Authorization")
        .and_then(|h| h.strip_prefix("Bearer ").map(str::to_owned))
        .filter(|t| t.starts_with(SecretKind::Runner.prefix()))
        .ok_or_else(|| AppError(ApiError::unauthorized()))?;
    let row: Option<RunnerAuth> = q(
        &ctx.db,
        "SELECT id, name FROM runners WHERE token_hash = ? AND enabled = 1",
        &[s(&hash_secret(&token))],
    )?
    .first()
    .await?;
    row.ok_or_else(|| {
        AppError(ApiError::new(
            fctl_proto::ErrorCode::Unauthorized,
            "Unknown or disabled runner token",
        ))
    })
}

/// `GET /api/v1/runner/connect`: the host's control connection.
pub async fn connect(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    if !http::is_websocket_upgrade(req) {
        return Err(AppError(ApiError::bad_request(
            "Expected a WebSocket upgrade",
        )));
    }
    let r = runner_auth(ctx, req).await?;
    let stub = internal::fleet_stub(&ctx.env)?;
    let ident = serde_json::json!({ "id": r.id, "name": r.name }).to_string();
    internal::upgrade(&stub, "/runner", &[(internal::HDR_RUNNER, &ident)]).await
}

/// `GET /api/v1/runner/sessions/{id}/uplink`: a session's event stream.
pub async fn uplink(ctx: &Ctx, req: &Request, session_id: &str) -> AppResult<Response> {
    if !http::is_websocket_upgrade(req) {
        return Err(AppError(ApiError::bad_request(
            "Expected a WebSocket upgrade",
        )));
    }
    let r = runner_auth(ctx, req).await?;
    #[derive(serde::Deserialize)]
    struct Assigned {
        runner_id: Option<String>,
    }
    let row: Option<Assigned> = q(
        &ctx.db,
        "SELECT runner_id FROM sessions WHERE id = ?",
        &[s(session_id)],
    )?
    .first()
    .await?;
    let assigned = row.ok_or_else(|| not_found("Session"))?;
    if assigned.runner_id.as_deref() != Some(r.id.as_str()) {
        return Err(forbidden("This session is not assigned to this runner"));
    }
    if !crate::spec::runner_serves(&ctx.db, &r.id, session_id).await? {
        return Err(crate::spec::not_served());
    }
    let stub = internal::session_stub(&ctx.env, session_id)?;
    internal::upgrade(&stub, "/uplink", &[(internal::HDR_RUNNER, &r.id)]).await
}

/// `GET /api/v1/feed/ws`: live updates for dashboards.
pub async fn feed(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    if !http::is_websocket_upgrade(req) {
        return Err(AppError(ApiError::bad_request(
            "Expected a WebSocket upgrade",
        )));
    }
    if ctx.auth.as_ref().is_some_and(|a| a.via_cookie) {
        http::check_same_origin(req, ctx.cfg.origin(), ctx.cfg.dev)?;
    }
    let user = ctx.user()?;
    let mut ident: FeedIdentity =
        crate::workspaces::feed_identity(&ctx.db, user, &ctx.settings()).await?;
    ident.token_id = ctx.auth.as_ref().map(|a| a.token_id.clone());
    ident.checked_at = Some(ctx.now);
    let stub = internal::fleet_stub(&ctx.env)?;
    let json = serde_json::to_string(&ident)?;
    internal::upgrade(&stub, "/feed", &[(internal::HDR_VIEWER, &json)]).await
}
