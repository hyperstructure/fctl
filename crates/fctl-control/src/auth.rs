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
use fctl_domain::sign_in::SignedIn;
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

/// What a new token is, besides its person and kind.
#[derive(Default)]
pub struct NewToken<'a> {
    pub name: Option<&'a str>,
    pub expires_at: Option<i64>,
    /// What it is limited to: only a PAT may be.
    pub limits: Option<&'a Limits>,
    /// The sign-in it comes from confirmed it's its person
    /// (`fctl_domain::verify`).
    pub verified_at: Option<i64>,
    /// How the sign-in it comes from was made (`fctl_domain::sign_in`).
    pub how: SignedIn,
}

/// Creates a web session, CLI token or PAT and returns its id and secret.
/// Web sessions also set the cookie on `ctx`.
pub async fn issue_token(
    ctx: &mut Ctx,
    user_id: &str,
    kind: TokenKind,
    new: NewToken<'_>,
) -> AppResult<(String, String)> {
    let NewToken {
        name,
        expires_at,
        limits,
        verified_at,
        how,
    } = new;
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
        "INSERT INTO tokens (id, hash, kind, user_id, name, user_agent, ip, created_at, last_used_at, expires_at, scope, verified_at, auth_method, auth_domain) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            os(how.method_str()),
            os(how.domain.as_deref()),
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
/// (`fctl_domain::verify`) for the next 15 minutes. `how`: how it was made,
/// which the session keeps for the instance's sign-in rules.
pub async fn start_web_session(
    ctx: &mut Ctx,
    user: &UserRow,
    confirmed: bool,
    how: SignedIn,
) -> AppResult<()> {
    let verified_at = confirmed.then_some(ctx.now);
    let (token_id, _) = issue_token(
        ctx,
        &user.id,
        TokenKind::Web,
        NewToken {
            verified_at,
            how: how.clone(),
            ..NewToken::default()
        },
    )
    .await?;
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
        how,
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
    /// How its sign-in was made (`fctl_domain::sign_in::SignedIn`).
    #[serde(default)]
    auth_method: Option<String>,
    #[serde(default)]
    auth_domain: Option<String>,
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
         t.auth_method AS auth_method, t.auth_domain AS auth_domain, \
         (SELECT value FROM settings WHERE key = '{}') AS instance_settings, {} \
         FROM tokens t JOIN users u ON u.id = t.user_id WHERE t.{column} = ?",
        crate::instance::KEY,
        user_columns("u")
    )
}

/// The admin token whose hash is `?`, its maker and the instance's
/// settings, as [`TokenLookup`]: one query.
fn admin_token_sql() -> String {
    format!(
        "SELECT t.id AS token_id, 'admin' AS kind, t.created_at AS token_created_at, t.expires_at AS expires_at, \
         t.last_used_at AS last_used_at, NULL AS verified_at, NULL AS scope, \
         NULL AS auth_method, NULL AS auth_domain, \
         (SELECT value FROM settings WHERE key = '{}') AS instance_settings, {} \
         FROM admin_tokens t JOIN users u ON u.id = t.user_id WHERE t.hash = ?",
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
/// active, its sign-in meets the instance's sign-in rules, and it may do
/// more than set up two-factor sign-in. For sockets opened with the token,
/// which check again while they stay open, so that signing out, revoking
/// the token, disabling the account or requiring a provider of its domain
/// closes them too.
pub async fn by_token_id(db: &D1Database, token_id: &str, now: i64) -> AppResult<Option<Holder>> {
    let found: Option<TokenLookup> = q(db, &token_lookup_sql("id"), &[s(token_id)])?
        .first()
        .await?;
    Ok(found.and_then(|f| {
        if f.expires_at.is_some_and(|e| e <= now) || !f.user.is_active() {
            return None;
        }
        let settings = crate::instance::parse(f.instance_settings.as_deref());
        let how = SignedIn::read(f.auth_method.as_deref(), f.auth_domain.as_deref());
        if f.user.admits(&settings, &how).is_err() {
            return None;
        }
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
    // Runner and enrollment tokens are checked by the endpoints for hosts
    // themselves.
    if matches!(kind, SecretKind::Runner | SecretKind::Enrollment) {
        return Ok(());
    }
    if kind == SecretKind::Admin {
        // A bearer token's alone: no browser gets one in a cookie.
        if via_cookie {
            clear_session_cookie(ctx);
            return Ok(());
        }
        return admin_token(ctx, &secret).await;
    }
    if kind == SecretKind::Service {
        if via_cookie {
            clear_session_cookie(ctx);
            return Ok(());
        }
        return crate::service_accounts::authenticate(ctx, &secret).await;
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
    let settings = crate::instance::parse(found.instance_settings.as_deref());
    // A domain of theirs signs in with a provider only: a sign-in made
    // another way, or before fctl kept how, no longer lets them in. A
    // browser's is signed out, and the sign-in page shows the way in.
    let how = SignedIn::read(found.auth_method.as_deref(), found.auth_domain.as_deref());
    if let Err(required) = found.user.admits(&settings, &how) {
        if via_cookie {
            clear_session_cookie(ctx);
            return Ok(());
        }
        return Err(AppError(crate::sign_in::token_refused(&required, kind)));
    }
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
        signed_in_with: how,
        two_factor_setup_required,
        settings,
        limits,
        autopilot: None,
        admin_token: false,
        service_account: None,
    });
    Ok(())
}

/// Resolves an admin token (`fctl_adm_…`, `fctl_domain::admin_tokens`) to
/// the admin who made it, while the instance takes admin tokens, the token
/// hasn't expired and they are still an active admin of the instance.
async fn admin_token(ctx: &mut Ctx, secret: &str) -> AppResult<()> {
    if !ctx.cfg.admin_tokens {
        return Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::Unauthorized,
            "This instance doesn't take admin tokens: its deployment turns them on with FCTL_ADMIN_TOKENS=1.",
        )));
    }
    let found: Option<TokenLookup> = q(&ctx.db, &admin_token_sql(), &[s(&hash_secret(secret))])?
        .first()
        .await?;
    let Some(found) = found else {
        return Err(AppError(ApiError::unauthorized()));
    };
    if found.expires_at.is_some_and(|e| e <= ctx.now) {
        return Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::Unauthorized,
            "This admin token has expired. An instance admin makes another under Admin → Settings → Admin tokens.",
        )));
    }
    if !found.user.is_active() || !found.user.is_admin() {
        return Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::AccessDenied,
            "This admin token stopped working: the admin who made it isn't an active admin of the instance any more.",
        )));
    }
    if found
        .last_used_at
        .is_none_or(|t| ctx.now - t > 10 * 60 * 1000)
    {
        let db = crate::db::handle(&ctx.db);
        let (id, now) = (found.token_id.clone(), ctx.now);
        ctx.defer(async move {
            q(
                &db,
                "UPDATE admin_tokens SET last_used_at = ? WHERE id = ?",
                &[n(now), s(&id)],
            )?
            .run()
            .await?;
            Ok(())
        });
    }
    let settings = crate::instance::parse(found.instance_settings.as_deref());
    // An admin the instance requires two-factor sign-in of, who hasn't set
    // it up, does nothing else until they have, with their tokens too.
    let two_factor_setup_required = fctl_domain::two_factor::setup_required(
        &settings,
        found.user.has_password(),
        found.user.factors(),
    );
    ctx.auth = Some(Authed {
        user: found.user,
        token_id: found.token_id,
        // Not one of `TokenKind`'s, which clients read: `admin_token` says
        // what it is, and what it reaches knows no other kind.
        kind: TokenKind::Pat,
        via_cookie: false,
        signed_in_at: found.token_created_at,
        verified_at: None,
        // A program's token: the sign-in rules check people's sign-ins,
        // and its admin's account stays what stops it.
        signed_in_with: SignedIn::default(),
        two_factor_setup_required,
        settings,
        limits: None,
        autopilot: None,
        admin_token: true,
        service_account: None,
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
        avatar_source: user.picture().source.filter(|_| user.avatar_url.is_some()),
        gravatar: ctx.settings().gravatar.then_some(user.gravatar),
        claude_stand_in: user.claude_stand_in,
        export: Some(data_export(ctx, user)),
        sign_in: crate::sign_in::requirement(user, &ctx.settings()),
    })
}

/// How often `user` can export their data, and when next.
pub fn data_export(ctx: &Ctx, user: &UserRow) -> fctl_proto::api::DataExport {
    let hours = ctx.settings().export_interval_hours;
    fctl_proto::api::DataExport {
        interval_hours: hours,
        last_at: user.exported_at,
        next_at: fctl_domain::account::next_export_at(user.exported_at, hours, ctx.now),
    }
}

/// Revokes every token of a user (password change or reset), except `keep`:
/// their admin tokens too ([`sign_out_everywhere`]).
pub async fn revoke_tokens(ctx: &Ctx, user_id: &str, keep: Option<&str>) -> AppResult<()> {
    crate::db::batch(&ctx.db, sign_out_everywhere(&ctx.db, user_id, keep)?).await
}

/// What revokes every token of `user_id` but `keep`: their browsers' and
/// CLIs' sign-ins, their access tokens, their admin tokens, and the CLI
/// sign-ins they approved that no CLI picked up yet. Whatever signs them
/// out everywhere (a new password, a reset, a disabled account) runs all
/// of it, so that nothing made while someone else held the account
/// outlasts it.
pub fn sign_out_everywhere(
    db: &D1Database,
    user_id: &str,
    keep: Option<&str>,
) -> AppResult<Vec<crate::db::Query>> {
    Ok(vec![
        q(
            db,
            "DELETE FROM tokens WHERE user_id = ? AND id != ?",
            &[s(user_id), s(keep.unwrap_or(""))],
        )?,
        q(
            db,
            "DELETE FROM admin_tokens WHERE user_id = ? AND id != ?",
            &[s(user_id), s(keep.unwrap_or(""))],
        )?,
        refuse_device_codes(db, user_id)?,
    ])
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

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::db::{Count, batch};
    use crate::testing;
    use wasm_bindgen_test::wasm_bindgen_test;

    async fn count(db: &D1Database, sql: &str) -> AppResult<i64> {
        let c: Option<Count> = q(db, sql, &[])?.first().await?;
        Ok(c.map_or(0, |c| c.n))
    }

    /// What a password reset, a new password or a disabled account signs
    /// out: an admin token made while someone else held the account too.
    #[wasm_bindgen_test]
    async fn signing_out_everywhere_takes_admin_tokens_too() -> AppResult<()> {
        let db = testing::database();
        for sql in [
            "INSERT INTO users (id, email, name, role, created_at, updated_at) VALUES \
             ('usr_ada', 'ada@example.com', 'Ada', 'admin', 0, 0), ('usr_bob', 'bob@example.com', 'Bob', 'admin', 0, 0)",
            "INSERT INTO tokens (id, hash, kind, user_id, created_at) VALUES \
             ('tok_web', 'h1', 'web', 'usr_ada', 0), ('tok_pat', 'h2', 'pat', 'usr_ada', 0), ('tok_bob', 'h3', 'pat', 'usr_bob', 0)",
            "INSERT INTO admin_tokens (id, hash, name, user_id, created_at) VALUES \
             ('adt_ada', 'h4', 'Okta', 'usr_ada', 0), ('adt_bob', 'h5', 'HR sync', 'usr_bob', 0)",
        ] {
            q(&db, sql, &[])?.run().await?;
        }
        // The browser that changed the password stays signed in.
        batch(&db, sign_out_everywhere(&db, "usr_ada", Some("tok_web"))?).await?;
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM tokens WHERE user_id = 'usr_ada'"
            )
            .await?,
            1
        );
        assert_eq!(
            count(&db, "SELECT COUNT(*) AS n FROM tokens WHERE id = 'tok_web'").await?,
            1
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM admin_tokens WHERE user_id = 'usr_ada'"
            )
            .await?,
            0
        );
        // A reset keeps nothing; Bob's are his.
        batch(&db, sign_out_everywhere(&db, "usr_ada", None)?).await?;
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM tokens WHERE user_id = 'usr_ada'"
            )
            .await?,
            0
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM tokens WHERE user_id = 'usr_bob'"
            )
            .await?,
            1
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM admin_tokens WHERE user_id = 'usr_bob'"
            )
            .await?,
            1
        );
        Ok(())
    }
}
