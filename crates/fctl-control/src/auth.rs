//! Authentication: passwords, tokens, the session cookie, and `Me`.

use crate::ctx::{Authed, Ctx};
use crate::db::{n, on, os, q, s};
use crate::error::{AppError, AppResult};
use crate::http;
use crate::model::{CredentialRow, IdentityRow, USAGE_JSON, UserRow, user_columns};
use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use fctl_domain::cookies::Cookie;
use fctl_domain::limits::Limits;
use fctl_domain::tokens::{SecretKind, hash_secret, new_secret};
use fctl_proto::ApiError;
use fctl_proto::api::{InstanceSettings, Me, TokenKind};
use worker::{D1Database, Request};

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
/// sessions also set the cookie on `ctx`. Only a PAT may have `limits`.
/// `verified_at`: the sign-in it comes from confirmed it's its person
/// (`fctl_domain::verify`).
pub async fn issue_token(
    ctx: &mut Ctx,
    user_id: &str,
    kind: TokenKind,
    name: Option<&str>,
    expires_at: Option<i64>,
    limits: Option<&Limits>,
    verified_at: Option<i64>,
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
    let scope = limits
        .filter(|_| kind == TokenKind::Pat)
        .map(Limits::to_json);
    q(
        &ctx.db,
        "INSERT INTO tokens (id, hash, kind, user_id, name, user_agent, ip, created_at, last_used_at, expires_at, scope, verified_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            s(id.as_str()),
            s(&hash_secret(&secret)),
            s(kind.as_str()),
            s(user_id),
            os(name),
            os(ctx.user_agent.as_deref()),
            os(ctx.ip.as_deref()),
            n(ctx.now),
            // An access token is used once something uses it.
            on((kind != TokenKind::Pat).then_some(ctx.now)),
            on(expires_at),
            os(scope.as_deref()),
            on(verified_at),
        ],
    )?
    .run()
    .await?;
    if kind == TokenKind::Web {
        let max_age = (WEB_SESSION_TTL_MS / 1000) as u64;
        ctx.cookies.push(http::set_cookie(
            Cookie::Session,
            &secret,
            max_age,
            ctx.cfg.secure_cookies(),
        ));
    }
    Ok((id.0, secret))
}

/// Signs in `user` in the browser: a new web session, and `last_login_at`
/// and `last_active_at`. The request then counts as theirs (for the audit
/// log). `confirmed`: the sign-in took a passkey or a second factor, so the
/// browser counts as having confirmed it's them before sensitive actions
/// (`fctl_domain::verify`) for the next 15 minutes.
pub async fn start_web_session(ctx: &mut Ctx, user: &UserRow, confirmed: bool) -> AppResult<()> {
    let verified_at = confirmed.then_some(ctx.now);
    let (token_id, _) =
        issue_token(ctx, &user.id, TokenKind::Web, None, None, None, verified_at).await?;
    q(
        &ctx.db,
        "UPDATE users SET last_login_at = ?1, last_active_at = ?1 WHERE id = ?2",
        &[n(ctx.now), s(&user.id)],
    )?
    .run()
    .await?;
    let settings = crate::instance::settings(&ctx.db).await?;
    ctx.auth = Some(Authed::signed_in(
        user,
        token_id,
        ctx.now,
        settings,
        verified_at,
    ));
    Ok(())
}

/// Records that `user_id` started a session themselves, at `now`, as when
/// they were last active (`users.last_active_at`, which the instance
/// admins' list of people shows). Sessions that review agents, schedules
/// and limited tokens start for them don't count.
pub async fn mark_active(db: &D1Database, user_id: &str, now: i64) -> AppResult<()> {
    q(
        db,
        "UPDATE users SET last_active_at = ?1 WHERE id = ?2 AND (last_active_at IS NULL OR last_active_at < ?1)",
        &[n(now), s(user_id)],
    )?
    .run()
    .await?;
    Ok(())
}

pub fn clear_session_cookie(ctx: &mut Ctx) {
    ctx.cookies.push(http::set_cookie(
        Cookie::Session,
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
    /// When it last confirmed it's its person (`fctl_domain::verify`).
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    verified_at: Option<i64>,
    /// What the token is limited to (`fctl_domain::limits::Limits`, JSON).
    #[serde(default)]
    scope: Option<String>,
    /// The instance's settings (JSON), for its two-factor policy.
    #[serde(default)]
    instance_settings: Option<String>,
    #[serde(flatten)]
    user: UserRow,
}

/// The bearer token or session cookie of a request, if any (`secure`:
/// `Config::secure_cookies`).
pub fn presented_secret(req: &Request, secure: bool) -> Option<(String, bool)> {
    if let Some(h) = http::header(req, "Authorization") {
        let token = h
            .strip_prefix("Bearer ")
            .or_else(|| h.strip_prefix("bearer "))?;
        return Some((token.trim().to_owned(), false));
    }
    http::cookie(req, Cookie::Session, secure).map(|c| (c, true))
}

/// The token whose `column` (`hash` or `id`) is `?`, its user, and the
/// instance's settings, which say whether the user may do anything before
/// setting up two-factor: one query.
fn token_lookup_sql(column: &str) -> String {
    format!(
        "SELECT t.id AS token_id, t.kind AS kind, t.created_at AS token_created_at, t.expires_at AS expires_at, \
         t.last_used_at AS last_used_at, t.verified_at AS verified_at, t.scope AS scope, \
         (SELECT value FROM settings WHERE key = '{}') AS instance_settings, {} \
         FROM tokens t JOIN users u ON u.id = t.user_id WHERE t.{column} = ?",
        crate::instance::KEY,
        user_columns("u")
    )
}

/// Who a token signs in, as sockets opened with it check again while they
/// stay open ([`by_token_id`]).
pub struct Holder {
    pub user: UserRow,
    /// The instance's settings.
    pub settings: InstanceSettings,
    /// What the token is limited to, when it is.
    pub limits: Option<Limits>,
}

/// The account behind the token `token_id`, with the instance's settings
/// and the token's limits, while the token still lets it in as
/// [`authenticate`] does: it exists and hasn't expired, the account is
/// active, and it may do more than set up two-factor sign-in. For sockets
/// opened with the token, which check again while they stay open, so that
/// signing out, revoking the token or disabling the account closes them
/// too.
pub async fn by_token_id(db: &D1Database, token_id: &str, now: i64) -> AppResult<Option<Holder>> {
    let found: Option<TokenLookup> = q(db, &token_lookup_sql("id"), &[s(token_id)])?
        .first()
        .await?;
    Ok(found.and_then(|f| {
        if f.expires_at.is_some_and(|e| e <= now) || !f.user.is_active() {
            return None;
        }
        let settings = crate::instance::parse(f.instance_settings.as_deref());
        let setup_required = fctl_domain::two_factor::setup_required(
            &settings,
            f.user.has_password(),
            f.user.factors(),
        );
        (!setup_required).then(|| Holder {
            limits: Limits::parse(f.scope.as_deref()),
            user: f.user,
            settings,
        })
    }))
}

/// Resolves the caller from the bearer token or the session cookie. Invalid
/// or expired credentials are treated as signed out (and the cookie cleared).
pub async fn authenticate(ctx: &mut Ctx, req: &Request) -> AppResult<()> {
    let Some((secret, via_cookie)) = presented_secret(req, ctx.cfg.secure_cookies()) else {
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
    let found: Option<TokenLookup> = q(
        &ctx.db,
        &token_lookup_sql("hash"),
        &[s(&hash_secret(&secret))],
    )?
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
                Cookie::Session,
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
        found.user.factors(),
    );
    // However it comes: in a cookie, a limited token stays limited.
    let limits = Limits::parse(found.scope.as_deref());
    ctx.auth = Some(Authed {
        user: found.user,
        token_id: found.token_id,
        kind,
        via_cookie,
        signed_in_at: found.token_created_at,
        verified_at: found.verified_at,
        two_factor_setup_required,
        settings,
        limits,
    });
    Ok(())
}

// ───────────────────────────── users ─────────────────────────────

/// The account an address belongs to: its primary one, or one connected
/// to it.
pub async fn user_by_email(ctx: &Ctx, email: &str) -> AppResult<Option<UserRow>> {
    q(
        &ctx.db,
        &format!(
            "SELECT {} FROM users u WHERE u.email = ?1 OR u.id = (SELECT user_id FROM user_emails WHERE email = ?1)",
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
    // Without one chosen, sessions run on the newest working one
    // (`api::sessions::credential_for`).
    let default_id = user.default_credential_id.clone().or_else(|| {
        creds
            .iter()
            .rev()
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
        commit_email: crate::api::emails::primary_commit_email(ctx, user).await?,
        two_factor: crate::api::two_factor::status(ctx, user).await?,
        passkeys: crate::api::passkeys::list(ctx, &user.id).await?,
        emails: crate::api::emails::list(ctx, user).await?,
        token: None,
    })
}

/// Revokes every token of a user (password change or reset), except `keep`.
pub async fn revoke_tokens(ctx: &Ctx, user_id: &str, keep: Option<&str>) -> AppResult<()> {
    crate::db::batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "DELETE FROM tokens WHERE user_id = ? AND id != ?",
                &[s(user_id), s(keep.unwrap_or(""))],
            )?,
            refuse_device_codes(&ctx.db, user_id)?,
        ],
    )
    .await
}

/// Refuses the CLI sign-ins `user_id` approved that no CLI has picked up
/// yet (`device_codes`), with their tokens: an approval from before would
/// otherwise still sign a CLI in, after a password change or reset.
pub fn refuse_device_codes(db: &D1Database, user_id: &str) -> AppResult<crate::db::Query> {
    q(
        db,
        "UPDATE device_codes SET status = 'denied' WHERE user_id = ? AND status IN ('pending', 'approved')",
        &[s(user_id)],
    )
}
