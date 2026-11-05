//! `/api/v1/runners/*` (people) and `/api/v1/runner/*` (hosts, with their
//! runner token).

use crate::api::workspaces::Viewer;
use crate::ctx::Ctx;
use crate::db::{n, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, not_found};
use crate::http;
use crate::internal::{self, FeedIdentity, LiveRunners};
use crate::model::{RUNNER_SELECT, RunnerRow, RunnerView};
use crate::workspaces as ws;
use fctl_domain::tokens::{SecretKind, hash_secret, new_secret};
use fctl_domain::validate;
use fctl_domain::verify::Sensitive;
use fctl_domain::workspace::RunnerOwner;
use fctl_proto::ApiError;
use fctl_proto::api::*;
use worker::{Request, Response};

async fn live(ctx: &Ctx) -> LiveRunners {
    match internal::fleet_stub(&ctx.env) {
        Ok(stub) => internal::get(&stub, "/runners").await.unwrap_or_default(),
        Err(_) => LiveRunners::default(),
    }
}

/// `GET /api/v1/runners`: the runners the caller sees: instance admins all
/// of them; others those of their workspaces, and their own.
pub async fn list(ctx: &Ctx) -> AppResult<Response> {
    let viewer = Viewer::instance(ctx).await?;
    let view = viewer.view();
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

/// A runner, as the checks of who may manage it need it.
#[derive(serde::Deserialize)]
pub(crate) struct Managed {
    pub name: String,
    created_by: Option<String>,
    /// The workspace it belongs to, when it enrolled in one.
    pub workspace_id: Option<String>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    pub pending_until: Option<i64>,
}

impl Managed {
    /// Whom it belongs to, as the caller sees it: for a workspace's runner,
    /// whether they are an admin there, and it is open to them.
    async fn owner(&self, ctx: &Ctx) -> AppResult<RunnerOwner> {
        let user = ctx.user()?;
        Ok(match &self.workspace_id {
            Some(w) => RunnerOwner::Workspace {
                admin: match ws::load(ctx, w).await {
                    Ok(a) => a.require_admin().is_ok(),
                    Err(_) => false,
                },
            },
            None => RunnerOwner::Person {
                mine: self.created_by.as_deref() == Some(user.id.as_str()),
            },
        })
    }
}

async fn managed(ctx: &Ctx, id: &str) -> AppResult<Managed> {
    q(
        &ctx.db,
        "SELECT name, created_by, workspace_id, pending_until FROM runners WHERE id = ?",
        &[s(id)],
    )?
    .first()
    .await?
    .ok_or_else(|| not_found("Runner"))
}

/// The caller may manage a runner as a machine (rename, disable, drain,
/// remove): who registered it, the admins of the workspace it belongs to,
/// or an instance admin (`fctl_domain::workspace::can_manage_runner`). A
/// new token is for its owners alone ([`rotate_token`]).
async fn require_manager(ctx: &Ctx, id: &str) -> AppResult<Managed> {
    let runner = managed(ctx, id).await?;
    let owner = runner.owner(ctx).await?;
    if fctl_domain::workspace::can_manage_runner(owner, ctx.user()?.is_admin()) {
        return Ok(runner);
    }
    Err(forbidden(match owner {
        RunnerOwner::Workspace { .. } => {
            "Only the admins of the workspace this runner belongs to, and instance admins, can do this"
        }
        RunnerOwner::Person { .. } => "Only the runner's owner and instance admins can do this",
    }))
}

/// One runner as the caller sees it.
async fn one(ctx: &Ctx, id: &str, live: &LiveRunners) -> AppResult<Runner> {
    let viewer = Viewer::instance(ctx).await?;
    let view = viewer.view();
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

/// Tells FleetObject that a runner's workspaces changed (after the
/// response): feeds update, queued sessions may start on it, and the
/// sessions it runs where it no longer serves stop. `approved`: an admin
/// of its workspace approved it, which its host hears.
pub(crate) fn changed(ctx: &Ctx, runner_id: &str, approved: bool) {
    let env = ctx.env.clone();
    let runner_id = runner_id.to_owned();
    ctx.defer(async move {
        let stub = internal::fleet_stub(&env)?;
        let _: serde_json::Value = internal::post(
            &stub,
            "/runner-changed",
            &internal::RunnerChanged {
                runner_id,
                approved,
            },
        )
        .await?;
        Ok(())
    });
}

/// Tells FleetObject that a runner is gone: its control connection and
/// uplinks close, the sessions it ran stop, saying `reason`, and lists
/// drop it.
pub(crate) async fn removed(env: &worker::Env, runner_id: &str, reason: &str) -> AppResult<()> {
    let stub = internal::fleet_stub(env)?;
    let _: serde_json::Value = internal::post(
        &stub,
        "/runner-removed",
        &internal::RunnerRemoved {
            runner_id: runner_id.to_owned(),
            reason: reason.to_owned(),
        },
    )
    .await?;
    Ok(())
}

pub async fn update(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdateRunnerRequest = http::body(req).await?;
    let runner = require_manager(ctx, id).await?;
    let name = body
        .name
        .as_deref()
        .map(|v| validate::name("name", v))
        .transpose()?;
    let updated = q(
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
    if updated == 0 {
        return Err(not_found("Runner"));
    }
    if body.enabled == Some(false) {
        disconnect(ctx, id).await?;
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
    if let Some(w) = &runner.workspace_id {
        ctx.audit_in(
            Some(w),
            "workspace.runner_updated",
            Some(id),
            Some(serde_json::json!({
                "name": name.as_deref().unwrap_or(&runner.name),
                "enabled": body.enabled,
                "draining": body.draining,
            })),
        );
    }
    // Feeds show its new name.
    if name.is_some() {
        changed(ctx, id, false);
    }
    let live = live(ctx).await;
    http::ok(&one(ctx, id, &live).await?)
}

/// `DELETE /api/v1/runners/{id}`: removes a runner: its token stops
/// working, and the sessions it ran stop and resume elsewhere.
pub async fn delete(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let runner = require_manager(ctx, id).await?;
    // FleetObject first, so that a failure there leaves the runner as it
    // was, for another try, rather than gone with its connection open.
    removed(
        &ctx.env,
        id,
        &format!("Its runner {} was removed.", runner.name),
    )
    .await?;
    let changed = q(&ctx.db, "DELETE FROM runners WHERE id = ?", &[s(id)])?
        .run()
        .await?;
    if changed == 0 {
        return Err(not_found("Runner"));
    }
    ctx.audit("runner.deleted", Some(id), None);
    if let Some(w) = &runner.workspace_id {
        ctx.audit_in(
            Some(w),
            "workspace.runner_removed",
            Some(id),
            Some(serde_json::json!({ "name": runner.name })),
        );
    }
    http::ok(&Ok { ok: true })
}

/// A workspace's runner that waits for approval, for the admins of the
/// workspace it belongs to, who approve or reject it.
async fn waiting_for_caller(ctx: &Ctx, id: &str) -> AppResult<(Managed, String)> {
    let runner = managed(ctx, id).await?;
    let Some(w) = runner.workspace_id.clone() else {
        return Err(conflict(
            "Only runners that enrolled with an enrollment token wait for approval",
        ));
    };
    // People outside the workspace are told it doesn't exist.
    let a = ws::load(ctx, &w).await.map_err(|_| not_found("Runner"))?;
    a.require_admin().map_err(|_| {
        forbidden("Only the admins of the workspace this runner enrolled in approve or reject it")
    })?;
    Ok((runner, w))
}

/// `POST /api/v1/runners/{id}/approve`: an admin of the workspace a runner
/// enrolled in lets it run everyone's sessions there. Its host takes
/// sessions at once, without a restart. Approving one that is approved
/// already changes nothing.
pub async fn approve(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let (runner, workspace_id) = waiting_for_caller(ctx, id).await?;
    if runner.pending_until.is_some() {
        // It gets the workspace's sessions, with their code, secrets and
        // tokens, as a runner added for everyone's sessions does.
        ctx.verified(Sensitive::ApproveRunner)?;
        let me = ctx.user()?.id.clone();
        let changes = crate::db::batch_changes(
            &ctx.db,
            vec![
                q(
                    &ctx.db,
                    "UPDATE runners SET pending_until = NULL WHERE id = ? AND pending_until IS NOT NULL",
                    &[s(id)],
                )?,
                q(
                    &ctx.db,
                    "INSERT OR IGNORE INTO runner_workspaces (runner_id, workspace_id, added_by, created_at, personal) \
                     SELECT r.id, r.workspace_id, ?3, ?4, 0 FROM runners r \
                     WHERE r.id = ?1 AND r.workspace_id = ?2 AND r.pending_until IS NULL",
                    &[s(id), s(&workspace_id), s(&me), n(ctx.now)],
                )?,
            ],
        )
        .await?;
        if changes.first().copied().unwrap_or(0) > 0 {
            ctx.audit_in(
                Some(&workspace_id),
                "workspace.runner_approved",
                Some(id),
                Some(serde_json::json!({ "name": runner.name })),
            );
            changed(ctx, id, true);
        }
    }
    let live = live(ctx).await;
    http::ok(&one(ctx, id, &live).await?)
}

/// `POST /api/v1/runners/{id}/reject`: an admin of the workspace a runner
/// enrolled in turns it away: it is removed, and its token stops working.
/// It no longer counts against its enrollment token's limit.
pub async fn reject(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let (runner, workspace_id) = waiting_for_caller(ctx, id).await?;
    if runner.pending_until.is_none() {
        return Err(conflict(&format!(
            "{} was approved already: remove it instead, from the Runners tab or with `fctl runners remove {id}`.",
            runner.name
        )));
    }
    // FleetObject first, as for `delete`: it gets nothing but a welcome
    // while it waits, so a race with an approval only drops its connection
    // for its host to open again.
    removed(&ctx.env, id, "Its runner was rejected.").await?;
    let changed = q(
        &ctx.db,
        "DELETE FROM runners WHERE id = ? AND pending_until IS NOT NULL",
        &[s(id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(conflict(&format!(
            "{} was approved or removed meanwhile",
            runner.name
        )));
    }
    ctx.audit_in(
        Some(&workspace_id),
        "workspace.runner_rejected",
        Some(id),
        Some(serde_json::json!({ "name": runner.name })),
    );
    http::ok(&Ok { ok: true })
}

/// `POST /api/v1/runners/{id}/token`: a new token for a runner, for the
/// person who registered it, or the admins of the workspace it belongs to
/// (`fctl_domain::workspace::check_replace_runner_token`).
pub async fn rotate_token(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let user_id = ctx.user()?.id.clone();
    let runner = managed(ctx, id).await?;
    fctl_domain::workspace::check_replace_runner_token(runner.owner(ctx).await?)?;
    if let Some(w) = runner.workspace_id.clone() {
        if runner.pending_until.is_some() {
            return Err(conflict(&format!(
                "{} waits for approval: approve or reject it first",
                runner.name
            )));
        }
        // Its workspace is open to them as its admin, as `owner` checked:
        // the new token runs its sessions, as approving it does.
        ctx.verified(Sensitive::AddRunner)?;
        let token = new_secret(SecretKind::Runner);
        let changed = q(
            &ctx.db,
            "UPDATE runners SET token_hash = ? WHERE id = ? AND workspace_id = ?",
            &[s(&hash_secret(&token)), s(id), s(&w)],
        )?
        .run()
        .await?;
        if changed == 0 {
            return Err(not_found("Runner"));
        }
        disconnect(ctx, id).await?;
        ctx.audit("runner.token_rotated", Some(id), None);
        ctx.audit_in(
            Some(&w),
            "workspace.runner_token_rotated",
            Some(id),
            Some(serde_json::json!({ "name": runner.name })),
        );
        let runner = one(ctx, id, &LiveRunners::default()).await?;
        return http::ok(&RunnerRegistration { runner, token });
    }
    // The new token gets the sessions of the workspaces the runner serves,
    // as adding it to them would: not while one of them is closed to its
    // owner by its two-factor requirement, which a password alone would
    // otherwise get round, and confirming it's them where the instance
    // asks for that.
    #[derive(serde::Deserialize)]
    struct Served {
        name: String,
        #[serde(deserialize_with = "crate::db::int_bool")]
        require_two_factor: bool,
    }
    let served: Vec<Served> = q(
        &ctx.db,
        "SELECT w.name AS name, w.require_two_factor AS require_two_factor FROM runner_workspaces rw \
         JOIN workspaces w ON w.id = rw.workspace_id \
         JOIN workspace_members m ON m.workspace_id = rw.workspace_id AND m.user_id = ? \
         WHERE rw.runner_id = ?",
        &[s(&user_id), s(id)],
    )?
    .all()
    .await?;
    let user = ctx.user()?;
    if let Some(closed) = served
        .iter()
        .find(|w| crate::workspaces::blocked_for(ctx, user, w.require_two_factor))
    {
        return Err(forbidden(&format!(
            "This runner runs the sessions of {}, which requires two-factor sign-in: set it up in Settings before you replace the runner's token.",
            closed.name
        )));
    }
    if !served.is_empty() {
        ctx.verified(Sensitive::AddRunner)?;
    }
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
    disconnect(ctx, id).await?;
    ctx.audit("runner.token_rotated", Some(id), None);
    let runner = one(ctx, id, &LiveRunners::default()).await?;
    http::ok(&RunnerRegistration { runner, token })
}

/// Closes a runner's control connection and its sessions' uplinks: it was
/// disabled, or its token replaced.
async fn disconnect(ctx: &Ctx, id: &str) -> AppResult<()> {
    let stub = internal::fleet_stub(&ctx.env)?;
    let _: serde_json::Value = internal::post(
        &stub,
        "/runner-disabled",
        &serde_json::json!({ "runner_id": id }),
    )
    .await?;
    Ok(())
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
    // Its host sees every workspace it serves, and what fctl knows of the
    // Claude token it keeps.
    let row = rows.first().ok_or_else(|| not_found("Runner"))?;
    let mut runner = row.to_runner(&live, &RunnerView::default());
    runner.claude_token = row.hello().and_then(|h| h.claude_token);
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

/// `DELETE /api/v1/runner`: a runner that enrolled in a workspace removes
/// itself, as its host uninstalls (`fctl host uninstall`), so that machines
/// autoscaling destroys don't pile up. A person's runner is theirs to
/// remove.
pub async fn remove_this_runner(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let r = runner_auth(ctx, req).await?;
    let Some(w) = r.workspace_id.clone() else {
        return Err(forbidden(
            "Only runners that enrolled with an enrollment token remove themselves. Its owner removes this one, in the app or with `fctl runners remove`.",
        ));
    };
    // FleetObject first, as for `delete`: the host tries again otherwise.
    removed(
        &ctx.env,
        &r.id,
        &format!("Its runner {} was uninstalled.", r.name),
    )
    .await?;
    q(&ctx.db, "DELETE FROM runners WHERE id = ?", &[s(&r.id)])?
        .run()
        .await?;
    ctx.audit_in(
        Some(&w),
        "workspace.runner_removed",
        Some(&r.id),
        Some(serde_json::json!({ "name": r.name, "by": "host" })),
    );
    http::ok(&Ok { ok: true })
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
    /// The workspace it belongs to, when it enrolled in one.
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    /// While it waits for approval there: until when.
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    pub pending_until: Option<i64>,
    /// Its enrollment token removes it once offline this long.
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    pub remove_offline_after_minutes: Option<i64>,
}

/// Authenticates a host by its runner token.
pub(crate) async fn runner_auth(ctx: &Ctx, req: &Request) -> AppResult<RunnerAuth> {
    let token = http::header(req, "Authorization")
        .and_then(|h| h.strip_prefix("Bearer ").map(str::to_owned))
        .filter(|t| t.starts_with(SecretKind::Runner.prefix()))
        .ok_or_else(|| AppError(ApiError::unauthorized()))?;
    let row: Option<RunnerAuth> = q(
        &ctx.db,
        "SELECT r.id, r.name, r.workspace_id, w.name AS workspace_name, r.pending_until, \
         r.remove_offline_after_minutes FROM runners r LEFT JOIN workspaces w ON w.id = r.workspace_id \
         WHERE r.token_hash = ? AND r.enabled = 1",
        &[s(&hash_secret(&token))],
    )?
    .first()
    .await?;
    row.ok_or_else(|| {
        AppError(ApiError::new(
            fctl_proto::ErrorCode::Unauthorized,
            "Unknown or disabled runner token: the runner was removed, rejected or disabled, or its token replaced",
        ))
    })
}

/// `GET /api/v1/runner/connect`: the host's control connection. A runner
/// that waits for approval connects as any other, and gets no sessions
/// until it is approved.
pub async fn connect(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    if !http::is_websocket_upgrade(req) {
        return Err(AppError(ApiError::bad_request(
            "Expected a WebSocket upgrade",
        )));
    }
    let r = runner_auth(ctx, req).await?;
    let stub = internal::fleet_stub(&ctx.env)?;
    let ident = serde_json::to_string(&internal::RunnerIdent {
        id: r.id.clone(),
        name: r.name.clone(),
        pending: r
            .pending_until
            .map(|until| fctl_proto::runner::PendingApproval {
                workspace: r.workspace_name.clone().unwrap_or_default(),
                until,
            }),
        remove_after_ms: r
            .remove_offline_after_minutes
            .map(|m| m.saturating_mul(fctl_domain::enrollment::MINUTE_MS)),
    })?;
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
    crate::spec::check_runs(&ctx.db, &r.id, session_id).await?;
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
