//! A workspace's enrollment tokens (`/api/v1/workspaces/{id}/enrollment-tokens`,
//! its admins), and the machines that enroll with them as the workspace's
//! runners (`/api/v1/runner/enroll`, with the token as the bearer token):
//! docs/architecture.md#enrollment-tokens, ADR 0071.

use crate::ctx::Ctx;
use crate::db::{Count, b, batch_changes, int, int_bool, n, on, opt_int, os, q, s};
use crate::error::{AppError, AppResult, not_found};
use crate::model::{RUNNER_SELECT, RunnerRow, RunnerView, workspace_ref};
use crate::{http, internal, workspaces as ws};
use fctl_domain::enrollment::{self, Refusal};
use fctl_domain::tokens::{SecretKind, hash_secret, new_secret};
use fctl_domain::verify::Sensitive;
use fctl_proto::api::{
    CreateEnrollmentTokenRequest, CreatedEnrollmentToken, EnrollRunnerRequest, EnrollmentInfo,
    EnrollmentToken, EnrollmentTokenList, RunnerRegistration,
};
use fctl_proto::session::{UserRef, WorkspaceRef};
use fctl_proto::{ApiError, EnrollmentTokenId, ErrorCode, UserId};
use serde::Deserialize;
use worker::{Request, Response};

/// An enrollment token's row, with who made and revoked it and the
/// runners it enrolled that are still there.
#[derive(Deserialize)]
struct TokenRow {
    id: String,
    workspace_id: String,
    name: String,
    #[serde(deserialize_with = "int_bool")]
    approval: bool,
    #[serde(deserialize_with = "opt_int", default)]
    max_runners: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    remove_offline_after_hours: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    expires_at: Option<i64>,
    created_by: Option<String>,
    creator_name: Option<String>,
    #[serde(deserialize_with = "int")]
    created_at: i64,
    #[serde(deserialize_with = "opt_int", default)]
    last_used_at: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    revoked_at: Option<i64>,
    revoked_by: Option<String>,
    revoker_name: Option<String>,
    #[serde(deserialize_with = "int", default)]
    runners: i64,
    #[serde(deserialize_with = "int", default)]
    pending: i64,
}

const TOKEN_SELECT: &str = "SELECT t.id, t.workspace_id, t.name, t.approval, t.max_runners, \
    t.remove_offline_after_hours, t.expires_at, t.created_by, cu.name AS creator_name, t.created_at, \
    t.last_used_at, t.revoked_at, t.revoked_by, ru.name AS revoker_name, \
    (SELECT COUNT(*) FROM runners x WHERE x.enrollment_id = t.id) AS runners, \
    (SELECT COUNT(*) FROM runners x WHERE x.enrollment_id = t.id AND x.pending_until IS NOT NULL) AS pending \
    FROM enrollment_tokens t LEFT JOIN users cu ON cu.id = t.created_by \
    LEFT JOIN users ru ON ru.id = t.revoked_by";

fn user_ref(id: &Option<String>, name: &Option<String>) -> Option<UserRef> {
    match (id, name) {
        (Some(id), Some(name)) => Some(UserRef {
            id: UserId(id.clone()),
            name: name.clone(),
            avatar_url: None,
        }),
        _ => None,
    }
}

fn count(n: Option<i64>) -> Option<u32> {
    n.and_then(|n| u32::try_from(n).ok())
}

impl TokenRow {
    fn to_token(&self) -> EnrollmentToken {
        EnrollmentToken {
            id: EnrollmentTokenId(self.id.clone()),
            name: self.name.clone(),
            approval: self.approval,
            max_runners: count(self.max_runners),
            remove_offline_after_hours: count(self.remove_offline_after_hours),
            expires_at: self.expires_at,
            created_at: self.created_at,
            created_by: user_ref(&self.created_by, &self.creator_name),
            last_used_at: self.last_used_at,
            revoked_at: self.revoked_at,
            revoked_by: user_ref(&self.revoked_by, &self.revoker_name),
            runners: count(Some(self.runners)).unwrap_or_default(),
            pending: count(Some(self.pending)).unwrap_or_default(),
        }
    }

    fn state(&self) -> enrollment::State {
        enrollment::State {
            revoked_at: self.revoked_at,
            expires_at: self.expires_at,
            max_runners: count(self.max_runners),
            runners: count(Some(self.runners)).unwrap_or_default(),
        }
    }
}

async fn token_row(ctx: &Ctx, workspace_id: &str, id: &str) -> AppResult<TokenRow> {
    q(
        &ctx.db,
        &format!("{TOKEN_SELECT} WHERE t.id = ? AND t.workspace_id = ?"),
        &[s(id), s(workspace_id)],
    )?
    .first()
    .await?
    .ok_or_else(|| not_found("Enrollment token"))
}

/// `GET /api/v1/workspaces/{id}/enrollment-tokens` (its admins): newest first.
pub async fn list(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let rows: Vec<TokenRow> = q(
        &ctx.db,
        &format!("{TOKEN_SELECT} WHERE t.workspace_id = ? ORDER BY t.created_at DESC"),
        &[s(a.id())],
    )?
    .all()
    .await?;
    http::ok(&EnrollmentTokenList {
        tokens: rows.iter().map(TokenRow::to_token).collect(),
    })
}

/// `POST /api/v1/workspaces/{id}/enrollment-tokens` (its admins): a new
/// token, whose secret the answer shows once. Making one trusts whatever
/// machines enroll with it with everyone's sessions there, as adding a
/// runner for everyone's sessions does: it takes confirming it's you, and
/// the workspace open to them (its two-factor requirement).
pub async fn create(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: CreateEnrollmentTokenRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let new = enrollment::check_new(&body, ctx.now).map_err(AppError)?;
    ctx.verified(Sensitive::NewEnrollmentToken)?;
    let secret = new_secret(SecretKind::Enrollment);
    let token_id = fctl_domain::ids::enrollment_token_id(ctx.now);
    let user = ctx.user()?.id.clone();
    q(
        &ctx.db,
        "INSERT INTO enrollment_tokens (id, workspace_id, name, token_hash, approval, max_runners, \
         remove_offline_after_hours, expires_at, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            s(token_id.as_str()),
            s(a.id()),
            s(&new.name),
            s(&hash_secret(&secret)),
            b(new.approval),
            on(new.max_runners.map(i64::from)),
            on(new.remove_offline_after_hours.map(i64::from)),
            on(new.expires_at),
            s(&user),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(a.id()),
        "workspace.enrollment_token_created",
        Some(token_id.as_str()),
        Some(serde_json::json!({
            "name": new.name,
            "approval": new.approval,
            "max_runners": new.max_runners,
            "remove_offline_after_hours": new.remove_offline_after_hours,
            "expires_at": new.expires_at,
        })),
    );
    let row = token_row(ctx, a.id(), token_id.as_str()).await?;
    http::created(&CreatedEnrollmentToken {
        token: row.to_token(),
        secret,
    })
}

/// `DELETE /api/v1/workspaces/{id}/enrollment-tokens/{token}` (its admins):
/// revokes it. It enrolls no more runners; those it enrolled stay, waiting
/// ones too, which its admins still approve or reject.
pub async fn revoke(ctx: &mut Ctx, id: &str, token: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let user = ctx.user()?.id.clone();
    let changed = q(
        &ctx.db,
        "UPDATE enrollment_tokens SET revoked_at = ?, revoked_by = ? \
         WHERE id = ? AND workspace_id = ? AND revoked_at IS NULL",
        &[n(ctx.now), s(&user), s(token), s(a.id())],
    )?
    .run()
    .await?;
    let row = token_row(ctx, a.id(), token).await?;
    if changed > 0 {
        ctx.audit_in(
            Some(a.id()),
            "workspace.enrollment_token_revoked",
            Some(token),
            Some(serde_json::json!({ "name": row.name })),
        );
    }
    http::ok(&row.to_token())
}

/// The enrollment token a host presents, with its workspace.
struct Presented {
    token: TokenRow,
    workspace: WorkspaceRef,
}

/// The enrollment token in the request's `Authorization` header. One the
/// control plane doesn't know counts towards the caller's rate limit, so
/// that nobody guesses one; the ones it knows don't, so that a fleet behind
/// one address enrolls at once.
async fn presented(ctx: &Ctx, req: &Request) -> AppResult<Presented> {
    let secret = http::header(req, "Authorization")
        .and_then(|h| h.strip_prefix("Bearer ").map(str::to_owned))
        .filter(|t| t.starts_with(SecretKind::Enrollment.prefix()));
    let unknown = || {
        AppError(ApiError::new(
            ErrorCode::Unauthorized,
            "Unknown enrollment token: copy it again from the workspace's Runners tab, or ask one of its admins for another.",
        ))
    };
    let Some(secret) = secret else {
        return Err(unknown());
    };
    let row: Option<TokenRow> = q(
        &ctx.db,
        &format!("{TOKEN_SELECT} WHERE t.token_hash = ?"),
        &[s(&hash_secret(&secret))],
    )?
    .first()
    .await?;
    let Some(token) = row else {
        ctx.rate_limit(&format!("enroll:{}", ctx.ip_key())).await?;
        return Err(unknown());
    };
    #[derive(Deserialize)]
    struct W {
        id: String,
        slug: String,
        name: String,
        color: Option<String>,
        #[serde(deserialize_with = "opt_int", default)]
        avatar_updated_at: Option<i64>,
    }
    let w: W = q(
        &ctx.db,
        "SELECT id, slug, name, color, avatar_updated_at FROM workspaces WHERE id = ?",
        &[s(&token.workspace_id)],
    )?
    .first()
    .await?
    .ok_or_else(unknown)?;
    let workspace = workspace_ref(
        &w.id,
        &w.slug,
        &w.name,
        w.color.as_deref(),
        w.avatar_updated_at,
    );
    Ok(Presented { token, workspace })
}

/// `GET /api/v1/runner/enroll`, with an enrollment token: the workspace it
/// enrolls in and how, or why it enrolls no runner now.
pub async fn info(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    let p = presented(ctx, req).await?;
    enrollment::check(&p.token.state(), ctx.now)
        .map_err(|r| AppError(r.error(&p.workspace.name)))?;
    http::ok(&EnrollmentInfo {
        workspace: p.workspace,
        name: p.token.name.clone(),
        approval: p.token.approval,
        expires_at: p.token.expires_at,
        max_runners: count(p.token.max_runners),
        runners: count(Some(p.token.runners)).unwrap_or_default(),
        remove_offline_after_hours: count(p.token.remove_offline_after_hours),
    })
}

/// `POST /api/v1/runner/enroll`, with an enrollment token: registers the
/// machine as a runner of the token's workspace, which belongs to the
/// workspace, and answers with its own runner token. With a token that
/// requires approval, the runner waits for an admin to approve it, and the
/// workspace's admins are told.
pub async fn enroll(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let p = presented(ctx, req).await?;
    let body: EnrollRunnerRequest = http::body(req).await?;
    let name = fctl_domain::validate::name("name", &body.name).map_err(AppError)?;
    let facts = body.facts.map(enrollment::facts);
    let workspace = p.workspace.name.clone();
    let refuse = |r: Refusal| AppError(r.error(&workspace));
    enrollment::check(&p.token.state(), ctx.now).map_err(refuse)?;
    let runner_token = new_secret(SecretKind::Runner);
    let runner_id = fctl_domain::ids::runner_id(ctx.now);
    let pending_until = p.token.approval.then(|| enrollment::pending_until(ctx.now));
    // One statement checks the token again and counts its runners as it
    // inserts, so that two machines that enroll at once can't both take
    // its last place. A runner that needs no approval runs the workspace's
    // sessions from the start.
    let changes = batch_changes(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "INSERT INTO runners (id, name, token_hash, enabled, created_by, created_at, workspace_id, \
                 enrollment_id, enrolled_ip, enrolled_location, facts, pending_until, remove_offline_after_hours) \
                 SELECT ?1, ?2, ?3, 1, NULL, ?4, t.workspace_id, t.id, ?5, ?6, ?7, ?8, t.remove_offline_after_hours \
                 FROM enrollment_tokens t WHERE t.id = ?9 AND t.revoked_at IS NULL \
                 AND (t.expires_at IS NULL OR t.expires_at > ?4) \
                 AND (t.max_runners IS NULL OR (SELECT COUNT(*) FROM runners x WHERE x.enrollment_id = t.id) < t.max_runners)",
                &[
                    s(runner_id.as_str()),
                    s(&name),
                    s(&hash_secret(&runner_token)),
                    n(ctx.now),
                    os(ctx.ip.as_deref()),
                    os(ctx.location.as_deref()),
                    os(facts
                        .as_ref()
                        .and_then(|f| serde_json::to_string(f).ok())
                        .as_deref()),
                    on(pending_until),
                    s(&p.token.id),
                ],
            )?,
            q(
                &ctx.db,
                "INSERT INTO runner_workspaces (runner_id, workspace_id, added_by, created_at, personal) \
                 SELECT r.id, r.workspace_id, NULL, ?2, 0 FROM runners r \
                 WHERE r.id = ?1 AND r.pending_until IS NULL AND r.workspace_id IS NOT NULL",
                &[s(runner_id.as_str()), n(ctx.now)],
            )?,
            q(
                &ctx.db,
                "UPDATE enrollment_tokens SET last_used_at = ?2 WHERE id = ?1 \
                 AND EXISTS (SELECT 1 FROM runners WHERE id = ?3)",
                &[s(&p.token.id), n(ctx.now), s(runner_id.as_str())],
            )?,
        ],
    )
    .await?;
    if changes.first().copied().unwrap_or(0) == 0 {
        // Revoked, expired or full since it was read: say which.
        let row = token_row(ctx, &p.token.workspace_id, &p.token.id).await?;
        return Err(match enrollment::check(&row.state(), ctx.now) {
            Err(r) => refuse(r),
            Ok(()) => AppError(ApiError::new(
                ErrorCode::Unavailable,
                "The runner could not be enrolled just now. Try again.",
            )),
        });
    }
    let workspace_id = p.token.workspace_id.clone();
    ctx.audit_in(
        Some(&workspace_id),
        "workspace.runner_enrolled",
        Some(runner_id.as_str()),
        Some(serde_json::json!({
            "name": name,
            "enrollment_token": p.token.id,
            "enrollment_token_name": p.token.name,
            "pending": pending_until.is_some(),
        })),
    );
    crate::api::runners::changed(ctx, runner_id.as_str(), false);
    let row: RunnerRow = q(
        &ctx.db,
        &format!("{RUNNER_SELECT} WHERE r.id = ?"),
        &[s(runner_id.as_str())],
    )?
    .first()
    .await?
    .ok_or_else(|| not_found("Runner"))?;
    // Its host sees all of it, as `GET /api/v1/runner` shows it.
    let runner = row.to_runner(&internal::LiveRunners::default(), &RunnerView::default());
    if pending_until.is_some() {
        tell_admins(ctx, &p, &runner);
    }
    http::created(&RunnerRegistration {
        runner,
        token: runner_token,
    })
}

/// Tells the workspace's admins that a runner waits for their approval
/// (after the response): a notification in each browser where they turned
/// notifications on, which the next one replaces, and, where the instance
/// sends email, an email when it is the only one that waits, so that a
/// fleet that enrolls at once sends one.
fn tell_admins(ctx: &Ctx, p: &Presented, runner: &fctl_proto::api::Runner) {
    let cfg = ctx.cfg.clone();
    let db = crate::db::handle(&ctx.db);
    let now = ctx.now;
    let workspace = p.workspace.clone();
    let token = p.token.name.clone();
    let runner = runner.clone();
    ctx.defer(async move {
        #[derive(Deserialize)]
        struct Admin {
            id: String,
            email: String,
        }
        let admins: Vec<Admin> = q(
            &db,
            "SELECT u.id, u.email FROM workspace_members m JOIN users u ON u.id = m.user_id \
             WHERE m.workspace_id = ? AND m.role IN ('owner', 'admin') AND u.status = 'active'",
            &[s(workspace.id.as_str())],
        )?
        .all()
        .await?;
        let waiting: Option<Count> = q(
            &db,
            "SELECT COUNT(*) AS n FROM runners WHERE workspace_id = ? AND pending_until IS NOT NULL",
            &[s(workspace.id.as_str())],
        )?
        .first()
        .await?;
        let waiting = waiting.map_or(1, |c| u32::try_from(c.n).unwrap_or(1).max(1));
        let system = [
            runner
                .os
                .as_deref()
                .zip(runner.arch.as_deref())
                .map(|(os, arch)| format!("{os}/{arch}")),
            runner.driver.clone(),
            runner.version.as_ref().map(|v| format!("fctl {v}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        let address = runner.enrollment.as_ref().and_then(|e| e.address.clone());
        let location = runner.enrollment.as_ref().and_then(|e| e.location.clone());
        let w = enrollment::Waiting {
            runner_id: runner.id.as_str(),
            runner: &runner.name,
            hostname: runner.hostname.as_deref(),
            token: &token,
            workspace_id: workspace.id.as_str(),
            workspace_slug: &workspace.slug,
            workspace: &workspace.name,
            waiting,
            address: address.as_deref(),
            location: location.as_deref(),
            system: (!system.is_empty()).then_some(system.as_str()),
        };
        let push = enrollment::waiting_push(&w, now);
        let email = (waiting == 1)
            .then_some(cfg.email.as_ref())
            .flatten()
            .map(|mail| (mail, enrollment::waiting_email(&w, &cfg.public_url)));
        for admin in &admins {
            if let Err(e) = crate::push::deliver(&cfg, &db, &admin.id, &push).await {
                worker::console_error!("telling {} a runner waits: {}", admin.id, e.0.message);
            }
            if let Some((mail, (subject, text))) = &email
                && let Err(e) = crate::email::send(mail, &admin.email, subject, text).await
            {
                worker::console_error!("emailing {} that a runner waits: {}", admin.id, e.0.message);
            }
        }
        Ok(())
    });
}
