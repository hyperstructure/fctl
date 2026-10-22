//! Confirming it's you before sensitive actions
//! (docs/architecture.md#confirming-its-you, `fctl_domain::verify`).
//!
//! Where the instance asks for it, the handlers of the actions in
//! `fctl_domain::verify::Sensitive` call `Ctx::verified` right before they
//! change anything: a browser, CLI or token that hasn't confirmed it's its
//! person within the last 15 minutes gets `verification_required`. It
//! confirms here, with a passkey (`POST /auth/verify/passkey`, then
//! `POST /auth/verify`), a code from the authenticator app or a recovery
//! code, and tries again. A CLI can't use passkeys: it asks its person to
//! confirm in the browser instead (`POST /auth/verify/device`), as it signs
//! in, and approving its code confirms it.

use crate::ctx::Ctx;
use crate::db::{n, q, s};
use crate::error::{AppError, AppResult, validation};
use crate::http;
use crate::model::UserRow;
use fctl_domain::verify;
use fctl_proto::ApiError;
use fctl_proto::api::*;
use serde::Deserialize;
use worker::{Request, Response};

#[derive(Deserialize)]
struct CodesRow {
    #[serde(deserialize_with = "crate::db::int", default)]
    codes: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    locked_until: Option<i64>,
}

/// Whether sensitive actions ask `user` to confirm it's them, what with,
/// and until when the token that confirmed at `verified_at` counts.
async fn verification(
    ctx: &Ctx,
    user: &UserRow,
    verified_at: Option<i64>,
) -> AppResult<Verification> {
    let factors = user.factors();
    let row: Option<CodesRow> = if factors.authenticator {
        q(
            &ctx.db,
            "SELECT (SELECT COUNT(*) FROM recovery_codes WHERE user_id = ?1 AND used_at IS NULL) AS codes, \
             (SELECT locked_until FROM authenticator_apps WHERE user_id = ?1) AS locked_until",
            &[s(&user.id)],
        )?
        .first()
        .await?
    } else {
        None
    };
    let (codes, locked_until) = row.map_or((0, None), |r| (r.codes, r.locked_until));
    Ok(Verification {
        required: verify::required(&ctx.settings(), factors),
        verified_until: verify::until(verified_at, ctx.now),
        methods: verify::methods(factors, u32::try_from(codes).unwrap_or(0)),
        locked_until: locked_until.filter(|t| *t > ctx.now),
    })
}

/// `GET /api/v1/auth/verify`: whether sensitive actions ask the caller to
/// confirm it's them, and what they can confirm it with.
pub async fn status(ctx: &Ctx) -> AppResult<Response> {
    let user = ctx.user()?;
    let verified_at = ctx.auth.as_ref().and_then(|a| a.verified_at);
    http::ok(&verification(ctx, user, verified_at).await?)
}

/// `POST /api/v1/auth/verify`: confirms it's the caller, with a passkey's
/// answer to `POST /auth/verify/passkey`, a code from their authenticator
/// app or a recovery code. The browser, CLI or token that asked counts as
/// confirmed for the next 15 minutes.
pub async fn verify(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: VerifyRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    ctx.rate_limit(&format!("2fa:{}", ctx.ip_key())).await?;
    ctx.rate_limit(&format!("2fa-user:{}", user.id)).await?;
    let code = body
        .code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let (via, passkey) = match (&body.passkey, code) {
        (Some(answer), _) => (
            "passkey",
            Some(crate::api::passkeys::confirm(ctx, &user, answer).await?),
        ),
        (None, Some(code)) => {
            if !user.factors().authenticator {
                return Err(validation(
                    "code",
                    "You have no authenticator app. Confirm it's you with your passkey.",
                ));
            }
            // Used up, and counted towards the wait when it's wrong, as
            // the codes that sign in are.
            match crate::api::two_factor::confirm_code(ctx, &user, code).await? {
                crate::api::two_factor::CodeUse::Totp => ("totp", None),
                crate::api::two_factor::CodeUse::Recovery { .. } => ("recovery_code", None),
                crate::api::two_factor::CodeUse::Passkey { name } => ("passkey", Some(name)),
            }
        }
        (None, None) => return Err(validation("code", "Enter a code")),
    };
    let token_id = ctx
        .auth
        .as_ref()
        .map(|a| a.token_id.clone())
        .ok_or_else(|| AppError(ApiError::unauthorized()))?;
    q(
        &ctx.db,
        "UPDATE tokens SET verified_at = ? WHERE id = ? AND user_id = ?",
        &[n(ctx.now), s(&token_id), s(&user.id)],
    )?
    .run()
    .await?;
    let kind = ctx.auth.as_mut().map(|a| {
        a.verified_at = Some(ctx.now);
        a.kind
    });
    ctx.audit(
        "user.verified",
        Some(&user.id),
        Some(serde_json::json!({
            "via": via,
            "passkey": passkey,
            "token": kind.map(TokenKind::as_str),
        })),
    );
    http::ok(&verification(ctx, &user, Some(ctx.now)).await?)
}

/// `POST /api/v1/auth/verify/device`: a CLI asks its person to confirm
/// it's them in the browser, where passkeys work, as it signs in: the
/// answer is a code to approve there (`/device`), which confirms the CLI's
/// token, and whose state `POST /api/v1/auth/device/token` reports
/// (`verified`, not a new token). Only for the CLI and access tokens; a
/// browser confirms with `POST /auth/verify`.
pub async fn device(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: DeviceStartRequest = http::body(req).await?;
    let (user_id, token_id) = match ctx.auth.as_ref() {
        Some(a) if a.kind != TokenKind::Web => (a.user.id.clone(), a.token_id.clone()),
        Some(_) => {
            return Err(AppError(ApiError::bad_request(
                "A browser confirms it's you itself, with POST /api/v1/auth/verify",
            )));
        }
        None => return Err(AppError(ApiError::unauthorized())),
    };
    ctx.rate_limit(&format!("device:{}", ctx.ip_key())).await?;
    let start =
        crate::api::auth::new_device_code(ctx, &body.client_name, Some((&user_id, &token_id)))
            .await?;
    http::created(&start)
}
