//! Early access (docs/architecture.md#early-access): where the deployment
//! turns it on (`FCTL_EARLY_ACCESS`, as app.fctl.sh's does), people without
//! an account ask for it on the sign-in page, and the instance's admins
//! invite them from the list. Inviting goes through the invitations of
//! `api::admin`, which mark the request invited.

use crate::ctx::Ctx;
use crate::db::{Count, n, os, q, s};
use crate::error::{AppError, AppResult, not_found};
use crate::http;
use fctl_domain::validate;
use fctl_proto::api::{AccessRequest, AccessRequestList, Ok, RequestAccessRequest};
use fctl_proto::session::UserRef;
use fctl_proto::{AccessRequestId, ApiError, ErrorCode, UserId};
use worker::{Request, Response};

/// The requests that wait or were invited: those whose address has an
/// account are done (the hourly cron trigger deletes them).
const OPEN: &str = "NOT EXISTS (SELECT 1 FROM user_emails e WHERE e.email = r.email) \
    AND NOT EXISTS (SELECT 1 FROM users u WHERE u.email = r.email)";

/// `POST /api/v1/access-requests` (public): someone asks for early access.
/// The answer is the same whatever the address is, so that it tells nobody
/// who asked, was invited or has an account: an address with an account
/// isn't recorded, and asking again moves `asked_at`.
pub async fn request(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    if !ctx.cfg.early_access {
        return Err(AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "This instance takes no requests for access. Ask one of its admins for an invitation.",
        )));
    }
    ctx.rate_limit(&format!("early-access:{}", ctx.ip_key()))
        .await?;
    let body: RequestAccessRequest = http::body(req).await?;
    let email = validate::email(&body.email)?;
    // A SELECT with a WHERE, so that SQLite reads ON CONFLICT as the upsert.
    q(
        &ctx.db,
        "INSERT INTO access_requests (id, email, created_at, asked_at) SELECT ?1, ?2, ?3, ?3 \
         WHERE NOT EXISTS (SELECT 1 FROM user_emails WHERE email = ?2) \
         AND NOT EXISTS (SELECT 1 FROM users WHERE email = ?2) \
         ON CONFLICT (email) DO UPDATE SET asked_at = excluded.asked_at",
        &[
            s(fctl_domain::ids::access_request_id(ctx.now).as_str()),
            s(&email),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    http::ok(&Ok { ok: true })
}

#[derive(serde::Deserialize)]
struct Row {
    id: String,
    email: String,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    #[serde(deserialize_with = "crate::db::int")]
    asked_at: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    invited_at: Option<i64>,
    invited_by_id: Option<String>,
    invited_by_name: Option<String>,
    invited_by_avatar: Option<String>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    invite_expires_at: Option<i64>,
}

impl Row {
    fn into_request(self) -> AccessRequest {
        AccessRequest {
            id: AccessRequestId(self.id),
            email: self.email,
            created_at: self.created_at,
            asked_at: self.asked_at,
            invited_at: self.invited_at,
            invited_by: match (self.invited_by_id, self.invited_by_name) {
                (Some(id), Some(name)) => Some(UserRef {
                    id: UserId(id),
                    name,
                    avatar_url: self.invited_by_avatar,
                }),
                _ => None,
            },
            invite_expires_at: self.invite_expires_at,
        }
    }
}

#[derive(serde::Deserialize)]
struct Counts {
    #[serde(deserialize_with = "crate::db::int")]
    waiting: i64,
    #[serde(deserialize_with = "crate::db::int")]
    invited: i64,
}

/// `GET /api/v1/admin/access-requests`: who asked and has no account yet,
/// those waiting first (longest waiting first), then those invited (latest
/// first).
pub async fn list(ctx: &Ctx) -> AppResult<Response> {
    ctx.admin()?;
    let rows: Vec<Row> = q(
        &ctx.db,
        &format!(
            "SELECT r.id, r.email, r.created_at, r.asked_at, r.invited_at, \
             u.id AS invited_by_id, u.name AS invited_by_name, u.avatar_url AS invited_by_avatar, \
             (SELECT MAX(i.expires_at) FROM invites i WHERE i.email = r.email AND i.accepted_at IS NULL \
              AND i.expires_at > ?1) AS invite_expires_at \
             FROM access_requests r LEFT JOIN users u ON u.id = r.invited_by WHERE {OPEN} \
             ORDER BY r.invited_at IS NOT NULL, \
             CASE WHEN r.invited_at IS NULL THEN r.created_at ELSE -r.invited_at END, r.id \
             LIMIT ?2"
        ),
        &[n(ctx.now), n(i64::from(AccessRequestList::LIMIT))],
    )?
    .all()
    .await?;
    let counts: Option<Counts> = q(
        &ctx.db,
        &format!(
            "SELECT COALESCE(SUM(CASE WHEN r.invited_at IS NULL THEN 1 ELSE 0 END), 0) AS waiting, \
             COALESCE(SUM(CASE WHEN r.invited_at IS NULL THEN 0 ELSE 1 END), 0) AS invited \
             FROM access_requests r WHERE {OPEN}"
        ),
        &[],
    )?
    .first()
    .await?;
    let count = |v: i64| u32::try_from(v.max(0)).unwrap_or(u32::MAX);
    http::ok(&AccessRequestList {
        requests: rows.into_iter().map(Row::into_request).collect(),
        waiting: counts.as_ref().map_or(0, |c| count(c.waiting)),
        invited: counts.as_ref().map_or(0, |c| count(c.invited)),
    })
}

/// `DELETE /api/v1/admin/access-requests/{id}`: an admin removes a request.
/// Asking again adds it back.
pub async fn remove(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    ctx.admin()?;
    #[derive(serde::Deserialize)]
    struct Removed {
        email: String,
    }
    let removed: Option<Removed> = q(
        &ctx.db,
        "DELETE FROM access_requests WHERE id = ? RETURNING email",
        &[s(id)],
    )?
    .first()
    .await?;
    let removed = removed.ok_or_else(|| not_found("Request"))?;
    ctx.audit("access_request.removed", Some(&removed.email), None);
    http::ok(&Ok { ok: true })
}

/// Records that `email` was invited, when it asked for early access: from
/// the list, or any other way an admin invites it.
pub async fn invited(ctx: &Ctx, email: &str) -> AppResult<()> {
    q(
        &ctx.db,
        "UPDATE access_requests SET invited_at = ?, invited_by = ? WHERE email = ?",
        &[
            n(ctx.now),
            os(ctx.auth.as_ref().map(|a| a.user.id.as_str())),
            s(email),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// Whether `email` asked for early access (an invitation to it answers
/// the request).
pub async fn asked(ctx: &Ctx, email: &str) -> AppResult<bool> {
    let row: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM access_requests WHERE email = ?",
        &[s(email)],
    )?
    .first()
    .await?;
    Ok(row.is_some_and(|c| c.n > 0))
}
