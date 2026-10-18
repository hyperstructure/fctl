//! Authentication: passwords, tokens, the session cookie, and `Me`.

use crate::ctx::{Authed, Ctx};
use crate::db::{n, on, os, q, s};
use crate::error::{AppError, AppResult};
use crate::http::{self, SESSION_COOKIE};
use crate::model::{CredentialRow, IdentityRow, USAGE_JSON, UserRow, user_columns};
use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use fctl_domain::tokens::{SecretKind, hash_secret, new_secret};
use fctl_proto::ApiError;
use fctl_proto::api::{Me, TokenKind};
use worker::Request;

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Web sessions expire this long after their last use.
pub const WEB_SESSION_TTL_MS: i64 = 30 * DAY_MS;
/// CLI tokens expire this long after their last use.
pub const CLI_TOKEN_TTL_MS: i64 = 180 * DAY_MS;

// ───────────────────────────── passwords ─────────────────────────────

/// argon2id with the OWASP-recommended parameters (19 MiB, 2 passes, 1 lane).
fn argon2() -> Argon2<'static> {
    Argon2::default()
}

pub fn hash_password(password: &str) -> AppResult<String> {
    let salt: [u8; 16] = fctl_domain::random_bytes();
    let hash = argon2()
        .hash_password_with_salt(password.as_bytes(), &salt)
        .map_err(|e| crate::error::internal(format!("argon2: {e}")))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => argon2()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Burns the same time as a real verification, so unknown emails and wrong
/// passwords are indistinguishable by timing.
pub fn dummy_verify(password: &str) {
    let _ = hash_password(password);
}

// ───────────────────────────── tokens ─────────────────────────────

/// Creates a web session, CLI token or PAT and returns the secret. Web
/// sessions also set the cookie on `ctx`.
pub async fn issue_token(
    ctx: &mut Ctx,
    user_id: &str,
    kind: TokenKind,
    name: Option<&str>,
    expires_at: Option<i64>,
) -> AppResult<(String, String)> {
    let secret_kind = match kind {
        TokenKind::Web => SecretKind::WebSession,
        TokenKind::Cli => SecretKind::Cli,
        TokenKind::Pat => SecretKind::Pat,
    };
    let secret = new_secret(secret_kind);
    let id = fctl_domain::ids::token_id(ctx.now);
    let expires_at = expires_at.or(match kind {
        TokenKind::Web => Some(ctx.now + WEB_SESSION_TTL_MS),
        TokenKind::Cli => Some(ctx.now + CLI_TOKEN_TTL_MS),
        TokenKind::Pat => None,
    });
    q(
        &ctx.db,
        "INSERT INTO tokens (id, hash, kind, user_id, name, user_agent, ip, created_at, last_used_at, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            s(id.as_str()),
            s(&hash_secret(&secret)),
            s(kind.as_str()),
            s(user_id),
            os(name),
            os(ctx.user_agent.as_deref()),
            os(ctx.ip.as_deref()),
            n(ctx.now),
            n(ctx.now),
            on(expires_at),
        ],
    )?
    .run()
    .await?;
    if kind == TokenKind::Web {
        let max_age = (WEB_SESSION_TTL_MS / 1000) as u64;
        ctx.cookies.push(http::set_cookie(
            SESSION_COOKIE,
            &secret,
            max_age,
            ctx.cfg.secure_cookies(),
        ));
    }
    Ok((id.0, secret))
}

/// Signs in `user` in the browser: a new web session, and `last_login_at`.
/// The request then counts as theirs (for the audit log).
pub async fn start_web_session(ctx: &mut Ctx, user: &UserRow) -> AppResult<()> {
    let (token_id, _) = issue_token(ctx, &user.id, TokenKind::Web, None, None).await?;
    q(
        &ctx.db,
        "UPDATE users SET last_login_at = ? WHERE id = ?",
        &[n(ctx.now), s(&user.id)],
    )?
    .run()
    .await?;
    ctx.auth = Some(Authed::signed_in(user, token_id, ctx.now));
    Ok(())
}

pub fn clear_session_cookie(ctx: &mut Ctx) {
    ctx.cookies.push(http::set_cookie(
        SESSION_COOKIE,
        "",
        0,
        ctx.cfg.secure_cookies(),
    ));
}

#[derive(serde::Deserialize)]
struct TokenLookup {
    token_id: String,
    kind: String,
    #[serde(deserialize_with = "crate::db::int")]
    token_created_at: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    expires_at: Option<i64>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    last_used_at: Option<i64>,
    /// The instance's settings (JSON), for its two-factor policy.
    #[serde(default)]
    instance_settings: Option<String>,
    #[serde(flatten)]
    user: UserRow,
}

/// The bearer token or session cookie of a request, if any.
pub fn presented_secret(req: &Request) -> Option<(String, bool)> {
    if let Some(h) = http::header(req, "Authorization") {
        let token = h
            .strip_prefix("Bearer ")
            .or_else(|| h.strip_prefix("bearer "))?;
        return Some((token.trim().to_owned(), false));
    }
    http::cookie(req, SESSION_COOKIE).map(|c| (c, true))
}

/// Resolves the caller from the bearer token or the session cookie. Invalid
/// or expired credentials are treated as signed out (and the cookie cleared).
pub async fn authenticate(ctx: &mut Ctx, req: &Request) -> AppResult<()> {
    let Some((secret, via_cookie)) = presented_secret(req) else {
        return Ok(());
    };
    let Some(kind) = SecretKind::of(&secret) else {
        return Err(AppError(ApiError::unauthorized()));
    };
    // Runner tokens are checked by the runner endpoints themselves.
    if kind == SecretKind::Runner {
        return Ok(());
    }
    if !matches!(
        kind,
        SecretKind::WebSession | SecretKind::Cli | SecretKind::Pat
    ) {
        return Err(AppError(ApiError::unauthorized()));
    }
    // One query: the token, its user, and the instance's settings, which
    // say whether the user may do anything before setting up two-factor.
    let sql = format!(
        "SELECT t.id AS token_id, t.kind AS kind, t.created_at AS token_created_at, t.expires_at AS expires_at, \
         t.last_used_at AS last_used_at, (SELECT value FROM settings WHERE key = '{}') AS instance_settings, {} \
         FROM tokens t JOIN users u ON u.id = t.user_id WHERE t.hash = ?",
        crate::instance::KEY,
        user_columns("u")
    );
    let found: Option<TokenLookup> = q(&ctx.db, &sql, &[s(&hash_secret(&secret))])?
        .first()
        .await?;
    let Some(found) = found else {
        if via_cookie {
            clear_session_cookie(ctx);
            return Ok(());
        }
        return Err(AppError(ApiError::unauthorized()));
    };
    if found.expires_at.is_some_and(|e| e <= ctx.now) {
        if via_cookie {
            clear_session_cookie(ctx);
            return Ok(());
        }
        return Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::Unauthorized,
            "Your token has expired. Sign in again.",
        )));
    }
    if !found.user.is_active() {
        return Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::AccessDenied,
            "Your account is disabled. Ask an admin for access.",
        )));
    }
    let kind = TokenKind::parse(&found.kind).unwrap_or(TokenKind::Pat);
    // Touch the token at most every 10 minutes; slide web and CLI expiry.
    let stale = found
        .last_used_at
        .is_none_or(|t| ctx.now - t > 10 * 60 * 1000);
    if stale {
        let new_expiry = match kind {
            TokenKind::Web => Some(ctx.now + WEB_SESSION_TTL_MS),
            TokenKind::Cli => Some(ctx.now + CLI_TOKEN_TTL_MS),
            TokenKind::Pat => found.expires_at,
        };
        let db = crate::db::handle(&ctx.db);
        let id = found.token_id.clone();
        let now = ctx.now;
        ctx.defer(async move {
            q(
                &db,
                "UPDATE tokens SET last_used_at = ?, expires_at = ? WHERE id = ?",
                &[n(now), on(new_expiry), s(&id)],
            )?
            .run()
            .await?;
            Ok(())
        });
        if kind == TokenKind::Web && via_cookie {
            let max_age = (WEB_SESSION_TTL_MS / 1000) as u64;
            ctx.cookies.push(http::set_cookie(
                SESSION_COOKIE,
                &secret,
                max_age,
                ctx.cfg.secure_cookies(),
            ));
        }
    }
    let settings = crate::instance::parse(found.instance_settings.as_deref());
    let two_factor_setup_required = fctl_domain::two_factor::setup_required(
        &settings,
        found.user.has_password(),
        found.user.has_second_factor(),
    );
    ctx.auth = Some(Authed {
        user: found.user,
        token_id: found.token_id,
        kind,
        via_cookie,
        signed_in_at: found.token_created_at,
        two_factor_setup_required,
    });
    Ok(())
}

// ───────────────────────────── users ─────────────────────────────

pub async fn user_by_email(ctx: &Ctx, email: &str) -> AppResult<Option<UserRow>> {
    q(
        &ctx.db,
        &format!(
            "SELECT {} FROM users u WHERE u.email = ?",
            user_columns("u")
        ),
        &[s(email)],
    )?
    .first()
    .await
}

pub async fn user_by_id(ctx: &Ctx, id: &str) -> AppResult<Option<UserRow>> {
    q(
        &ctx.db,
        &format!("SELECT {} FROM users u WHERE u.id = ?", user_columns("u")),
        &[s(id)],
    )?
    .first()
    .await
}

/// `GET /auth/me` for `user`.
pub async fn me(ctx: &Ctx, user: &UserRow) -> AppResult<Me> {
    let identities: Vec<IdentityRow> = q(
        &ctx.db,
        "SELECT id, provider, email, name, avatar_url, created_at FROM identities WHERE user_id = ? ORDER BY created_at",
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let creds: Vec<CredentialRow> = q(
        &ctx.db,
        &format!(
            "SELECT c.*, {USAGE_JSON} AS usage FROM claude_credentials c WHERE c.user_id = ? ORDER BY c.created_at"
        ),
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let default_id = user.default_credential_id.clone().or_else(|| {
        creds
            .iter()
            .find(|c| c.status == "active")
            .map(|c| c.id.clone())
    });
    Ok(Me {
        user: user.to_user(),
        identities: identities.iter().map(IdentityRow::to_identity).collect(),
        claude: creds
            .iter()
            .map(|c| c.to_info(default_id.as_deref()))
            .collect(),
        commit_email: user.commit_email.clone(),
        two_factor: crate::api::two_factor::status(ctx, user).await?,
    })
}

/// Revokes every token of a user (password change or reset), except `keep`.
pub async fn revoke_tokens(ctx: &Ctx, user_id: &str, keep: Option<&str>) -> AppResult<()> {
    q(
        &ctx.db,
        "DELETE FROM tokens WHERE user_id = ? AND id != ?",
        &[s(user_id), s(keep.unwrap_or(""))],
    )?
    .run()
    .await?;
    Ok(())
}
