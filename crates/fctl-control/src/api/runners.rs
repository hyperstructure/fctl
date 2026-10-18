//! `/api/v1/runners/*` (admin) and `/api/v1/runner/*` (hosts).

use crate::ctx::Ctx;
use crate::db::{n, q, s};
use crate::error::{AppError, AppResult, forbidden, not_found};
use crate::http;
use crate::internal::{self, FeedIdentity, LiveRunners};
use crate::model::{RUNNER_SELECT, RunnerRow};
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

pub async fn list(ctx: &Ctx) -> AppResult<Response> {
    ctx.user()?;
    let rows: Vec<RunnerRow> = q(
        &ctx.db,
        &format!("{RUNNER_SELECT} ORDER BY r.created_at"),
        &[],
    )?
    .all()
    .await?;
    let live = live(ctx).await;
    http::ok(&RunnerList {
        runners: rows.iter().map(|r| r.to_runner(&live)).collect(),
    })
}

pub async fn create(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: CreateRunnerRequest = http::body(req).await?;
    let actor = ctx.admin()?.clone();
    let name = validate::name("name", &body.name)?;
    let token = new_secret(SecretKind::Runner);
    let id = fctl_domain::ids::runner_id(ctx.now);
    q(
        &ctx.db,
        "INSERT INTO runners (id, name, token_hash, enabled, created_by, created_at) VALUES (?, ?, ?, 1, ?, ?)",
        &[s(id.as_str()), s(&name), s(&hash_secret(&token)), s(&actor.id), n(ctx.now)],
    )?
    .run()
    .await?;
    ctx.audit(
        "runner.created",
        Some(id.as_str()),
        Some(serde_json::json!({ "name": name })),
    );
    let row: Option<RunnerRow> = q(
        &ctx.db,
        &format!("{RUNNER_SELECT} WHERE r.id = ?"),
        &[s(id.as_str())],
    )?
    .first()
    .await?;
    let runner = row
        .ok_or_else(|| not_found("Runner"))?
        .to_runner(&LiveRunners::default());
    http::created(&RunnerRegistration { runner, token })
}

pub async fn update(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdateRunnerRequest = http::body(req).await?;
    ctx.admin()?;
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
    let rows: Vec<RunnerRow> = q(
        &ctx.db,
        &format!("{RUNNER_SELECT} WHERE r.id = ?"),
        &[s(id)],
    )?
    .all()
    .await?;
    let live = live(ctx).await;
    let r = rows.first().ok_or_else(|| not_found("Runner"))?;
    http::ok(&r.to_runner(&live))
}

pub async fn delete(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    ctx.admin()?;
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

pub async fn rotate_token(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    ctx.admin()?;
    let token = new_secret(SecretKind::Runner);
    let changed = q(
        &ctx.db,
        "UPDATE runners SET token_hash = ? WHERE id = ?",
        &[s(&hash_secret(&token)), s(id)],
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
    let rows: Vec<RunnerRow> = q(
        &ctx.db,
        &format!("{RUNNER_SELECT} WHERE r.id = ?"),
        &[s(id)],
    )?
    .all()
    .await?;
    let runner = rows
        .first()
        .ok_or_else(|| not_found("Runner"))?
        .to_runner(&LiveRunners::default());
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
    let runner = rows
        .first()
        .ok_or_else(|| not_found("Runner"))?
        .to_runner(&live);
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
    #[derive(serde::Deserialize)]
    struct P {
        project_id: String,
    }
    let projects: Vec<P> = q(
        &ctx.db,
        "SELECT project_id FROM project_members WHERE user_id = ?",
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let ident = FeedIdentity {
        user_id: user.id.clone(),
        admin: user.is_admin(),
        projects: projects.into_iter().map(|p| p.project_id).collect(),
    };
    let stub = internal::fleet_stub(&ctx.env)?;
    let json = serde_json::to_string(&ident)?;
    internal::upgrade(&stub, "/feed", &[(internal::HDR_VIEWER, &json)]).await
}
