//! A workspace's enrollment tokens (`/api/v1/workspaces/{id}/enrollment-tokens`:
//! its admins', for everyone's sessions, and its members', for their own),
//! and the machines that enroll with them as runners of the workspace
//! (`/api/v1/runner/enroll`, with the token as the bearer token):
//! docs/architecture.md#enrollment-tokens, ADRs 0071 and 0072.

use crate::ctx::Ctx;
use crate::db::{Count, b, batch_changes, int, int_bool, n, on, opt_int, os, q, s};
use crate::error::{AppError, AppResult, not_found};
use crate::model::{RUNNER_SELECT, RunnerRow, RunnerView, workspace_ref};
use crate::{http, internal, workspaces as ws};
use fctl_domain::enrollment::{self, Refusal};
use fctl_domain::tokens::{SecretKind, hash_secret, new_secret};
use fctl_domain::verify::Sensitive;
use fctl_domain::workspace as policy;
use fctl_proto::api::{
    CreateEnrollmentTokenRequest, CreatedEnrollmentToken, EnrollRunnerRequest, EnrollmentInfo,
    EnrollmentToken, EnrollmentTokenList, Placement, RunnerRegistration,
    UpdateEnrollmentTokenRequest,
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
    /// For its maker's own sessions: the runners it enrolls are theirs.
    #[serde(deserialize_with = "int_bool", default)]
    personal: bool,
    /// Its maker is an active member of its workspace, without which a
    /// token for their own sessions enrolls nothing.
    #[serde(deserialize_with = "int_bool", default)]
    maker_member: bool,
    /// Its maker is an active admin of its workspace, without which a
    /// token for everyone's sessions enrolls nothing.
    #[serde(deserialize_with = "int_bool", default)]
    maker_admin: bool,
    #[serde(deserialize_with = "int_bool")]
    approval: bool,
    #[serde(deserialize_with = "opt_int", default)]
    max_runners: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    remove_offline_after_minutes: Option<i64>,
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
    /// Its runners' labels (JSON) and placement: they are a pool.
    #[serde(default)]
    labels: Option<String>,
    #[serde(default)]
    placement: Option<String>,
}

const TOKEN_SELECT: &str = "SELECT t.id, t.workspace_id, t.name, t.personal, t.approval, t.max_runners, \
    t.remove_offline_after_minutes, t.expires_at, t.created_by, cu.name AS creator_name, t.created_at, \
    t.labels, t.placement, \
    EXISTS (SELECT 1 FROM workspace_members m WHERE m.workspace_id = t.workspace_id \
      AND m.user_id = t.created_by AND cu.status = 'active') AS maker_member, \
    EXISTS (SELECT 1 FROM workspace_members m WHERE m.workspace_id = t.workspace_id \
      AND m.user_id = t.created_by AND m.role IN ('owner', 'admin') AND cu.status = 'active') AS maker_admin, \
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
            personal: self.personal,
            approval: self.approval,
            max_runners: count(self.max_runners),
            remove_offline_after_minutes: count(self.remove_offline_after_minutes),
            expires_at: self.expires_at,
            created_at: self.created_at,
            created_by: user_ref(&self.created_by, &self.creator_name),
            last_used_at: self.last_used_at,
            revoked_at: self.revoked_at,
            revoked_by: user_ref(&self.revoked_by, &self.revoker_name),
            runners: count(Some(self.runners)).unwrap_or_default(),
            pending: count(Some(self.pending)).unwrap_or_default(),
            labels: self.labels(),
            placement: self.placement(),
        }
    }

    fn labels(&self) -> Vec<String> {
        crate::model::pool_labels(self.labels.as_deref())
    }

    fn placement(&self) -> Placement {
        self.placement
            .as_deref()
            .and_then(Placement::parse)
            .unwrap_or_default()
    }

    /// The person whose runners it enrolls, for a token for its maker's
    /// own sessions.
    fn owner(&self) -> Option<&str> {
        self.created_by.as_deref().filter(|_| self.personal)
    }

    /// Whether it enrolls runners at all, whatever its state: a token for
    /// its maker's own sessions doesn't once they are no longer an active
    /// member of its workspace, or have no account, and one for everyone's
    /// sessions doesn't once they are no longer an active admin there, as
    /// an invitation counts only while the admin who sent it is still one
    /// (`workspaces::INVITER_STILL_ADMIN`): an admin who is removed,
    /// demoted or disabled can't enroll a machine of theirs for everyone's
    /// sessions with a token they kept.
    fn usable(&self, workspace: &str) -> AppResult<()> {
        if self.personal && !(self.owner().is_some() && self.maker_member) {
            return Err(AppError(enrollment::maker_gone(workspace)));
        }
        if !self.personal && !self.maker_admin {
            return Err(AppError(enrollment::maker_no_admin(workspace)));
        }
        Ok(())
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

/// Whether the caller may see and revoke token `t` of workspace `a`: its
/// admins every one, a member those they made for their own sessions.
fn manages(ctx: &Ctx, a: &ws::WorkspaceAccess, t: &TokenRow) -> AppResult<bool> {
    let role = a.require_open()?;
    let me = ctx.user()?.id.as_str();
    Ok(policy::can_manage_enrollment_token(
        Some(role),
        t.owner() == Some(me),
    ))
}

/// `GET /api/v1/workspaces/{id}/enrollment-tokens`: newest first, every one
/// for its admins, and for a member those they made for their own sessions.
pub async fn list(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_open()?;
    let rows: Vec<TokenRow> = q(
        &ctx.db,
        &format!("{TOKEN_SELECT} WHERE t.workspace_id = ? ORDER BY t.created_at DESC"),
        &[s(a.id())],
    )?
    .all()
    .await?;
    let mut tokens = Vec::new();
    for t in &rows {
        if manages(ctx, &a, t)? {
            tokens.push(t.to_token());
        }
    }
    http::ok(&EnrollmentTokenList { tokens })
}

/// `POST /api/v1/workspaces/{id}/enrollment-tokens`: a new token, whose
/// secret the answer shows once, for everyone's sessions (its admins) or
/// for the caller's own (any member). Making one trusts whatever machines
/// enroll with it with those sessions, as adding a runner for them does:
/// it takes confirming it's you, and the workspace open to them (its
/// two-factor requirement).
pub async fn create(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: CreateEnrollmentTokenRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    let role = a.require_open()?;
    let personal = policy::enrollment_for_own(Some(role), body.personal).map_err(AppError)?;
    let new = enrollment::check_new(&body, personal, ctx.now).map_err(AppError)?;
    ctx.verified(Sensitive::NewEnrollmentToken)?;
    let secret = new_secret(SecretKind::Enrollment);
    let token_id = fctl_domain::ids::enrollment_token_id(ctx.now);
    let user = ctx.user()?.id.clone();
    q(
        &ctx.db,
        "INSERT INTO enrollment_tokens (id, workspace_id, name, token_hash, personal, approval, max_runners, \
         remove_offline_after_minutes, expires_at, created_by, created_at, labels, placement) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            s(token_id.as_str()),
            s(a.id()),
            s(&new.name),
            s(&hash_secret(&secret)),
            b(new.personal),
            b(new.approval),
            on(new.max_runners.map(i64::from)),
            on(new.remove_offline_after_minutes.map(i64::from)),
            on(new.expires_at),
            s(&user),
            n(ctx.now),
            os(labels_json(&new.labels).as_deref()),
            s(new.placement.as_str()),
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
            "personal": new.personal,
            "approval": new.approval,
            "max_runners": new.max_runners,
            "remove_offline_after_minutes": new.remove_offline_after_minutes,
            "expires_at": new.expires_at,
            "labels": new.labels,
            "placement": new.placement,
        })),
    );
    let row = token_row(ctx, a.id(), token_id.as_str()).await?;
    http::created(&CreatedEnrollmentToken {
        token: row.to_token(),
        secret,
    })
}

/// Labels as D1 keeps them: JSON, or NULL for none.
fn labels_json(labels: &[String]) -> Option<String> {
    (!labels.is_empty())
        .then(|| serde_json::to_string(labels).ok())
        .flatten()
}

/// `PATCH /api/v1/workspaces/{id}/enrollment-tokens/{token}`: renames a
/// pool, or changes its labels, its placement or how many runners it
/// keeps, for those who may revoke it: its runners carry the new labels
/// and placement at FleetDO's next dispatch, revoked tokens' too.
pub async fn update(
    ctx: &mut Ctx,
    req: &mut Request,
    id: &str,
    token: &str,
) -> AppResult<Response> {
    let body: UpdateEnrollmentTokenRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    a.require_open()?;
    let row = token_row(ctx, a.id(), token).await?;
    if !manages(ctx, &a, &row)? {
        return Err(not_found("Enrollment token"));
    }
    let change = enrollment::check_change(&body).map_err(AppError)?;
    if change.is_empty() {
        return http::ok(&row.to_token());
    }
    apply(&ctx.db, a.id(), token, &change).await?;
    let mut data = serde_json::Map::new();
    data.insert(
        "name".into(),
        serde_json::json!(change.name.as_deref().unwrap_or(&row.name)),
    );
    if let Some(labels) = &change.labels {
        data.insert("labels".into(), serde_json::json!(labels));
    }
    if let Some(p) = change.placement {
        data.insert("placement".into(), serde_json::json!(p));
    }
    if let Some(max) = change.max_runners {
        data.insert("max_runners".into(), serde_json::json!(max));
    }
    ctx.audit_in(
        Some(a.id()),
        "workspace.enrollment_token_changed",
        Some(token),
        Some(serde_json::Value::Object(data)),
    );
    // Its runners' labels and placement, and the names sessions wait for.
    crate::api::runners::pool_changed(ctx, token);
    http::ok(&token_row(ctx, a.id(), token).await?.to_token())
}

/// Changes token `token` of workspace `workspace_id` as `change` says.
async fn apply(
    db: &worker::D1Database,
    workspace_id: &str,
    token: &str,
    change: &enrollment::TokenChange,
) -> AppResult<()> {
    let mut sets: Vec<&str> = Vec::new();
    let mut params = Vec::new();
    if let Some(name) = &change.name {
        sets.push("name = ?");
        params.push(s(name));
    }
    if let Some(labels) = &change.labels {
        sets.push("labels = ?");
        params.push(os(labels_json(labels).as_deref()));
    }
    if let Some(p) = change.placement {
        sets.push("placement = ?");
        params.push(s(p.as_str()));
    }
    if let Some(max) = change.max_runners {
        sets.push("max_runners = ?");
        params.push(on(max.map(i64::from)));
    }
    if sets.is_empty() {
        return Ok(());
    }
    params.push(s(token));
    params.push(s(workspace_id));
    q(
        db,
        &format!(
            "UPDATE enrollment_tokens SET {} WHERE id = ? AND workspace_id = ?",
            sets.join(", ")
        ),
        &params,
    )?
    .run()
    .await?;
    Ok(())
}

/// `DELETE /api/v1/workspaces/{id}/enrollment-tokens/{token}`: revokes it,
/// for its admins, or a member who made it for their own sessions. It
/// enrolls no more runners; those it enrolled stay, waiting ones too, which
/// are still approved or rejected.
pub async fn revoke(ctx: &mut Ctx, id: &str, token: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_open()?;
    if !manages(ctx, &a, &token_row(ctx, a.id(), token).await?)? {
        // Members are told only of their own.
        return Err(not_found("Enrollment token"));
    }
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

/// `GET /api/v1/workspaces/{id}/runners/demand`: what the workspace's pools
/// that the caller manages need now: every one for its admins, and so for
/// its service accounts, which read as their keeper; a member's own for
/// them. `?pool=` narrows it to one, by its id or name.
pub async fn workspace_demand(ctx: &Ctx, req: &Request, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_open()?;
    let rows: Vec<TokenRow> = q(
        &ctx.db,
        &format!("{TOKEN_SELECT} WHERE t.workspace_id = ? ORDER BY t.created_at"),
        &[s(a.id())],
    )?
    .all()
    .await?;
    let wanted = http::query(req, "pool");
    let mut ids = Vec::new();
    for t in &rows {
        let named = wanted
            .as_deref()
            .is_none_or(|w| w.trim() == t.id || w.trim() == t.name);
        if named && manages(ctx, &a, t)? {
            ids.push(t.id.clone());
        }
    }
    if let Some(w) = &wanted
        && ids.is_empty()
    {
        return Err(not_found(&format!("Pool {w}")));
    }
    let mut demand = crate::pools::workspace_demand(ctx, a.id(), Some(&ids)).await?;
    // Every session that waits is the admins' to see; a member sees those
    // their pools count.
    if !policy::can_manage_enrollment_token(Some(a.require_open()?), false) {
        demand.waiting = fctl_proto::api::PoolWaiting::default();
        for p in &demand.pools {
            let w = &p.waiting;
            demand.waiting.sessions += w.sessions;
            demand.waiting.since = match (demand.waiting.since, w.since) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            for (k, n) in &w.reasons {
                *demand.waiting.reasons.entry(k.clone()).or_default() += n;
            }
        }
        demand.unpooled = 0;
    }
    http::ok(&demand)
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
/// enrolls in, for whose sessions and how, or why it enrolls no runner now.
pub async fn info(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    let p = presented(ctx, req).await?;
    p.token.usable(&p.workspace.name)?;
    enrollment::check(&p.token.state(), ctx.now)
        .map_err(|r| AppError(r.error(&p.workspace.name, p.token.personal)))?;
    http::ok(&EnrollmentInfo {
        owner: p
            .token
            .owner()
            .and_then(|_| user_ref(&p.token.created_by, &p.token.creator_name)),
        workspace: p.workspace,
        name: p.token.name.clone(),
        approval: p.token.approval,
        expires_at: p.token.expires_at,
        max_runners: count(p.token.max_runners),
        runners: count(Some(p.token.runners)).unwrap_or_default(),
        remove_offline_after_minutes: count(p.token.remove_offline_after_minutes),
        labels: p.token.labels(),
        placement: p.token.placement(),
    })
}

/// `GET /api/v1/runner/enroll/demand`, with an enrollment token: what its
/// pool needs now, for whatever scales it (`fctl runners demand`, KEDA's
/// metrics-api scaler, the CloudWatch exporter). Counts only. A token that
/// enrolls nothing any more (revoked, expired, or its maker no longer one
/// who may make it) reads nothing either.
pub async fn demand(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    let p = presented(ctx, req).await?;
    p.token.usable(&p.workspace.name)?;
    let state = p.token.state();
    if let Err(r @ (Refusal::Revoked { .. } | Refusal::Expired { .. })) =
        enrollment::check(&state, ctx.now)
    {
        return Err(AppError(r.error(&p.workspace.name, p.token.personal)));
    }
    let demand = crate::pools::workspace_demand(
        ctx,
        &p.token.workspace_id,
        Some(std::slice::from_ref(&p.token.id)),
    )
    .await?;
    match demand.pools.into_iter().next() {
        Some(pool) => http::ok(&pool),
        None => Err(not_found("Pool")),
    }
}

/// `POST /api/v1/runner/enroll`, with an enrollment token: registers the
/// machine as a runner of the token's workspace, and answers with its own
/// runner token. The runner belongs to the workspace and runs everyone's
/// sessions there, or, with a token a member made for their own sessions,
/// belongs to them and runs only theirs. With a token that requires
/// approval, it waits until an admin of the workspace approves it, or its
/// owner, who are told.
pub async fn enroll(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let p = presented(ctx, req).await?;
    let body: EnrollRunnerRequest = http::body(req).await?;
    let name = fctl_domain::validate::name("name", &body.name).map_err(AppError)?;
    let facts = body.facts.map(enrollment::facts);
    let workspace = p.workspace.name.clone();
    let personal = p.token.personal;
    let refuse = |r: Refusal| AppError(r.error(&workspace, personal));
    p.token.usable(&workspace)?;
    enrollment::check(&p.token.state(), ctx.now).map_err(refuse)?;
    let runner_token = new_secret(SecretKind::Runner);
    let runner_id = fctl_domain::ids::runner_id(ctx.now);
    let pending_until = p.token.approval.then(|| enrollment::pending_until(ctx.now));
    // One statement checks the token again and counts its runners as it
    // inserts, so that two machines that enroll at once can't both take
    // its last place, and a token for its maker's own sessions enrolls
    // only while they are an active member, one for everyone's only while
    // they are an active admin. A runner that needs no approval runs the
    // workspace's sessions from the start: everyone's, or its owner's
    // alone.
    let changes = batch_changes(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "INSERT INTO runners (id, name, token_hash, enabled, created_by, created_at, workspace_id, \
                 enrollment_id, enrolled_ip, enrolled_location, facts, pending_until, remove_offline_after_minutes) \
                 SELECT ?1, ?2, ?3, 1, CASE WHEN t.personal = 1 THEN t.created_by END, ?4, t.workspace_id, t.id, \
                 ?5, ?6, ?7, ?8, t.remove_offline_after_minutes \
                 FROM enrollment_tokens t WHERE t.id = ?9 AND t.revoked_at IS NULL \
                 AND (t.expires_at IS NULL OR t.expires_at > ?4) \
                 AND (t.max_runners IS NULL OR (SELECT COUNT(*) FROM runners x WHERE x.enrollment_id = t.id) < t.max_runners) \
                 AND EXISTS (SELECT 1 FROM workspace_members m JOIN users u ON u.id = m.user_id \
                   WHERE m.workspace_id = t.workspace_id AND m.user_id = t.created_by AND u.status = 'active' \
                   AND (t.personal = 1 OR m.role IN ('owner', 'admin')))",
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
                 SELECT r.id, r.workspace_id, r.created_by, ?2, r.created_by IS NOT NULL FROM runners r \
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
        // Revoked, expired or full since it was read, or its maker left
        // or is no longer an admin: say which.
        let row = token_row(ctx, &p.token.workspace_id, &p.token.id).await?;
        row.usable(&workspace)?;
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
            "personal": personal,
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
        tell_approvers(ctx, &p, &runner);
    }
    http::created(&RunnerRegistration {
        runner,
        token: runner_token,
    })
}

/// Tells those who approve a runner that it waits for them (after the
/// response): the workspace's admins, or its owner for a runner that is
/// theirs. A notification in each browser where they turned notifications
/// on, which the next one replaces, and, where the instance sends email,
/// an email when it is the only one that waits for them, so that a fleet
/// that enrolls at once sends one.
fn tell_approvers(ctx: &Ctx, p: &Presented, runner: &fctl_proto::api::Runner) {
    let cfg = ctx.cfg.clone();
    let db = crate::db::handle(&ctx.db);
    let now = ctx.now;
    let workspace = p.workspace.clone();
    let token = p.token.name.clone();
    let owner = p.token.owner().map(str::to_owned);
    let runner = runner.clone();
    ctx.defer(async move {
        #[derive(Deserialize)]
        struct Approver {
            id: String,
            email: String,
        }
        let approvers: Vec<Approver> = match &owner {
            Some(owner) => q(
                &db,
                "SELECT id, email FROM users WHERE id = ? AND status = 'active'",
                &[s(owner)],
            )?,
            None => q(
                &db,
                "SELECT u.id, u.email FROM workspace_members m JOIN users u ON u.id = m.user_id \
                 WHERE m.workspace_id = ? AND m.role IN ('owner', 'admin') AND u.status = 'active'",
                &[s(workspace.id.as_str())],
            )?,
        }
        .all()
        .await?;
        // The ones that wait for the same people.
        let waiting: Option<Count> = q(
            &db,
            "SELECT COUNT(*) AS n FROM runners WHERE workspace_id = ?1 AND pending_until IS NOT NULL \
             AND created_by IS ?2",
            &[s(workspace.id.as_str()), os(owner.as_deref())],
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
            yours: owner.is_some(),
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
        for to in &approvers {
            if let Err(e) = crate::push::deliver(&cfg, &db, &to.id, &push).await {
                worker::console_error!("telling {} a runner waits: {}", to.id, e.0.message);
            }
            if let Some((mail, (subject, text))) = &email
                && let Err(e) = crate::email::send(mail, &to.email, subject, text).await
            {
                worker::console_error!("emailing {} that a runner waits: {}", to.id, e.0.message);
            }
        }
        Ok(())
    });
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::testing;
    use wasm_bindgen_test::wasm_bindgen_test;

    async fn token(db: &worker::D1Database, id: &str) -> AppResult<EnrollmentToken> {
        let row: Option<TokenRow> = q(db, &format!("{TOKEN_SELECT} WHERE t.id = ?"), &[s(id)])?
            .first()
            .await?;
        row.map(|r| r.to_token())
            .ok_or_else(|| not_found("Enrollment token"))
    }

    /// A token's labels and placement read as they were stored, and an
    /// update changes what it names and leaves the rest.
    #[wasm_bindgen_test]
    async fn tokens_keep_their_pools_labels_and_placement() -> AppResult<()> {
        let db = testing::database();
        for sql in [
            "INSERT INTO users (id, email, name, created_at, updated_at) VALUES ('usr_ada', 'ada@example.com', 'Ada', 0, 0)",
            "INSERT INTO workspaces (id, slug, name, created_at, updated_at) VALUES ('wsp_acme', 'acme', 'Acme', 0, 0)",
            "INSERT INTO enrollment_tokens (id, workspace_id, name, token_hash, max_runners, created_by, created_at, labels, placement) \
             VALUES ('enr_large', 'wsp_acme', 'AWS large', 'h1', 20, 'usr_ada', 1, '[\"aws\",\"large\"]', 'packed')",
            // From before pools: no labels, no placement.
            "INSERT INTO enrollment_tokens (id, workspace_id, name, token_hash, created_by, created_at) \
             VALUES ('enr_old', 'wsp_acme', 'Build fleet', 'h2', 'usr_ada', 2)",
        ] {
            q(&db, sql, &[])?.run().await?;
        }
        let large = token(&db, "enr_large").await?;
        assert_eq!(large.labels, ["aws", "large"]);
        assert_eq!(large.placement, Placement::Packed);
        let old = token(&db, "enr_old").await?;
        assert!(old.labels.is_empty());
        assert_eq!(old.placement, Placement::Spread);
        // Its labels and limit go; its name and placement stay.
        let change = enrollment::check_change(&UpdateEnrollmentTokenRequest {
            labels: Some(vec![]),
            max_runners: Some(0),
            ..Default::default()
        })
        .map_err(AppError)?;
        apply(&db, "wsp_acme", "enr_large", &change).await?;
        let large = token(&db, "enr_large").await?;
        assert!(large.labels.is_empty());
        assert_eq!(large.max_runners, None);
        assert_eq!(
            (large.name.as_str(), large.placement),
            ("AWS large", Placement::Packed)
        );
        // Another workspace's id changes nothing.
        let change = enrollment::check_change(&UpdateEnrollmentTokenRequest {
            name: Some("GPU".into()),
            labels: Some(vec!["gpu".into()]),
            placement: Some(Placement::Spread),
            max_runners: Some(4),
        })
        .map_err(AppError)?;
        apply(&db, "wsp_other", "enr_old", &change).await?;
        assert_eq!(token(&db, "enr_old").await?.name, "Build fleet");
        apply(&db, "wsp_acme", "enr_old", &change).await?;
        let gpu = token(&db, "enr_old").await?;
        assert_eq!(
            (
                gpu.name.as_str(),
                gpu.labels.as_slice(),
                gpu.placement,
                gpu.max_runners
            ),
            (
                "GPU",
                ["gpu".to_owned()].as_slice(),
                Placement::Spread,
                Some(4)
            )
        );
        Ok(())
    }
}
