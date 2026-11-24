//! `/api/v1/admin/tokens`: the instance's admin tokens
//! (docs/architecture.md#admin-tokens), which programs provision people
//! with (`api::provision`), where the deployment turns them on
//! (`FCTL_ADMIN_TOKENS`). Instance admins make, list and revoke them,
//! signed in; an admin token reaches none of this.

use crate::ctx::Ctx;
use crate::db::{n, on, q, s};
use crate::error::{AppError, AppResult, not_found};
use crate::http;
use fctl_domain::tokens::{SecretKind, hash_secret, new_secret};
use fctl_domain::validate;
use fctl_domain::verify::Sensitive;
use fctl_proto::api::{AdminToken, AdminTokenList, CreateAdminTokenRequest, CreatedAdminToken, Ok};
use fctl_proto::session::UserRef;
use fctl_proto::{AdminTokenId, ApiError, ErrorCode, UserId};
use serde::Deserialize;
use worker::{Request, Response};

#[derive(Deserialize)]
struct Row {
    id: String,
    name: String,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    last_used_at: Option<i64>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    expires_at: Option<i64>,
    user_id: String,
    user_name: String,
    #[serde(default)]
    user_avatar: Option<String>,
}

const SELECT: &str = "SELECT t.id, t.name, t.created_at, t.last_used_at, t.expires_at, \
    u.id AS user_id, u.name AS user_name, u.avatar_url AS user_avatar \
    FROM admin_tokens t JOIN users u ON u.id = t.user_id";

impl Row {
    fn into_token(self) -> AdminToken {
        AdminToken {
            id: AdminTokenId(self.id),
            name: self.name,
            created_by: UserRef {
                id: UserId(self.user_id),
                name: self.user_name,
                avatar_url: self.user_avatar,
            },
            created_at: self.created_at,
            last_used_at: self.last_used_at,
            expires_at: self.expires_at,
        }
    }
}

/// What making one answers on an instance that doesn't take them.
fn not_taken() -> AppError {
    AppError(ApiError::new(
        ErrorCode::NotConfigured,
        "This instance doesn't take admin tokens: its deployment turns them on with FCTL_ADMIN_TOKENS=1.",
    ))
}

/// `GET /api/v1/admin/tokens`: every admin token of the instance that
/// hasn't expired, whoever made it, newest first.
pub async fn list(ctx: &Ctx) -> AppResult<Response> {
    ctx.admin()?;
    let rows: Vec<Row> = q(
        &ctx.db,
        &format!(
            "{SELECT} WHERE t.expires_at IS NULL OR t.expires_at > ? ORDER BY t.created_at DESC"
        ),
        &[n(ctx.now)],
    )?
    .all()
    .await?;
    http::ok(&AdminTokenList {
        enabled: ctx.cfg.admin_tokens,
        tokens: rows.into_iter().map(Row::into_token).collect(),
    })
}

/// `POST /api/v1/admin/tokens`: an admin token that acts as the admin who
/// makes it, and its secret, which nobody sees again.
pub async fn create(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: CreateAdminTokenRequest = http::body(req).await?;
    let actor = ctx.admin()?.clone();
    if !ctx.cfg.admin_tokens {
        return Err(not_taken());
    }
    let name = validate::name("name", &body.name)?;
    let expires_at =
        fctl_domain::admin_tokens::expires_at(body.expires_in_days, ctx.now).map_err(AppError)?;
    // Whoever holds one creates accounts and changes people's addresses.
    ctx.verified(Sensitive::NewAdminToken)?;
    let secret = new_secret(SecretKind::Admin);
    let id = fctl_domain::ids::admin_token_id(ctx.now);
    // How the sign-in that makes it was made, which a domain's rule holds it
    // to as it holds its admin's sign-ins.
    let how = ctx
        .auth
        .as_ref()
        .map(|a| a.signed_in_with.clone())
        .unwrap_or_default();
    q(
        &ctx.db,
        "INSERT INTO admin_tokens (id, hash, name, user_id, created_at, expires_at, auth_method, auth_domain) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            s(id.as_str()),
            s(&hash_secret(&secret)),
            s(&name),
            s(&actor.id),
            n(ctx.now),
            on(expires_at),
            crate::db::os(how.method_str()),
            crate::db::os(how.domain.as_deref()),
        ],
    )?
    .run()
    .await?;
    ctx.audit(
        "admin_token.created",
        Some(id.as_str()),
        Some(serde_json::json!({ "name": name, "expires_at": expires_at })),
    );
    http::created(&CreatedAdminToken {
        token: AdminToken {
            id,
            name,
            created_by: actor.to_ref(),
            created_at: ctx.now,
            last_used_at: None,
            expires_at,
        },
        secret,
    })
}

/// `DELETE /api/v1/admin/tokens/{id}`: any instance admin revokes any of
/// them, and it stops working at once.
pub async fn revoke(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    ctx.admin()?;
    #[derive(Deserialize)]
    struct Revoked {
        name: String,
        user_id: String,
    }
    let revoked: Option<Revoked> = q(
        &ctx.db,
        "DELETE FROM admin_tokens WHERE id = ? RETURNING name, user_id",
        &[s(id)],
    )?
    .first()
    .await?;
    let Some(revoked) = revoked else {
        return Err(not_found("Admin token"));
    };
    ctx.audit(
        "admin_token.revoked",
        Some(id),
        Some(serde_json::json!({ "name": revoked.name, "created_by": revoked.user_id })),
    );
    http::ok(&Ok { ok: true })
}
