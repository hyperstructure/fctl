//! Two-factor sign-in (docs/architecture.md#two-factor-sign-in).
//!
//! - A sign-in that asks for the second factor (`fctl_domain::two_factor::asks`)
//!   starts no web session. It leaves a challenge, which the browser holds in
//!   the `__Host-fctl-2fa` cookie for ten minutes, and
//!   `POST /auth/two-factor/verify` finishes it with a code from the
//!   authenticator app or a recovery code. Wrong codes in a row make
//!   sign-ins wait, longer each time.
//! - People add an authenticator app in Settings: its secret travels sealed
//!   for them until the app's first code confirms it.
//! - While the instance requires two-factor of an account that has none,
//!   the account can only set it up ([`open_while_required`]).

use crate::auth;
use crate::ctx::Ctx;
use crate::db::{Count, Query, batch, n, os, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, internal, not_found, validation};
use crate::http;
use crate::model::UserRow;
use fctl_domain::cookies::Cookie;
use fctl_domain::tokens::{SecretKind, hash_secret, new_secret};
use fctl_domain::totp;
use fctl_domain::two_factor::{
    self as policy, CHALLENGE_TTL_MS, FRESH_SIGN_IN_MS, Factors, SETUP_TTL_MS,
};
use fctl_proto::api::*;
use fctl_proto::session::InstanceRole;
use fctl_proto::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use worker::{Method, Request, Response};

// ───────────────────────────── status ─────────────────────────────

#[derive(Deserialize)]
struct StatusRow {
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    since: Option<i64>,
    #[serde(deserialize_with = "crate::db::int", default)]
    passkeys: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    used_at: Option<i64>,
    #[serde(deserialize_with = "crate::db::int", default)]
    codes: i64,
    #[serde(default)]
    settings: Option<String>,
}

/// `Me.two_factor` for `user`, from the database as it is now.
pub async fn status(ctx: &Ctx, user: &UserRow) -> AppResult<TwoFactor> {
    let row: Option<StatusRow> = q(
        &ctx.db,
        &format!(
            "SELECT (SELECT created_at FROM authenticator_apps WHERE user_id = ?1) AS since, \
             (SELECT last_used_at FROM authenticator_apps WHERE user_id = ?1) AS used_at, \
             (SELECT COUNT(*) FROM recovery_codes WHERE user_id = ?1 AND used_at IS NULL) AS codes, \
             (SELECT COUNT(*) FROM passkeys WHERE user_id = ?1) AS passkeys, \
             (SELECT value FROM settings WHERE key = '{}') AS settings",
            crate::instance::KEY
        ),
        &[s(&user.id)],
    )?
    .first()
    .await?;
    let row = row.ok_or_else(|| internal("two-factor status: no row"))?;
    let instance = crate::instance::parse(row.settings.as_deref());
    // Workspaces that require it require it of their members as the
    // instance would; only the instance's requirement closes the account.
    let settings = user.two_factor_settings(&instance);
    let has_password = user.has_password();
    let factors = Factors {
        authenticator: row.since.is_some(),
        passkeys: row.passkeys > 0,
    };
    Ok(TwoFactor {
        authenticator_since: row.since,
        authenticator_used_at: row.used_at,
        recovery_codes: row.codes.max(0) as u32,
        google: user.two_factor_google,
        google_required: settings.two_factor_google,
        required: policy::required(&settings, has_password),
        setup_required: policy::setup_required(&instance, has_password, factors),
        on: policy::on(&settings, has_password, factors),
    })
}

/// `GET /api/v1/me/two-factor`.
pub async fn get(ctx: &Ctx) -> AppResult<Response> {
    let user = ctx.user()?;
    http::ok(&status(ctx, user).await?)
}

/// `PATCH /api/v1/me/two-factor`.
pub async fn update(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: UpdateTwoFactorRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    if let Some(google) = body.google {
        q(
            &ctx.db,
            "UPDATE users SET two_factor_google = ?, updated_at = ? WHERE id = ?",
            &[crate::db::b(google), n(ctx.now), s(&user.id)],
        )?
        .run()
        .await?;
        ctx.audit(
            "two_factor.updated",
            Some(&user.id),
            Some(serde_json::json!({ "google": google })),
        );
    }
    me(ctx, &user.id).await
}

/// `Me` for `user_id` as it is now.
async fn me(ctx: &Ctx, user_id: &str) -> AppResult<Response> {
    let user = auth::user_by_id(ctx, user_id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::ok(&auth::me(ctx, &user).await?)
}

// ───────────────────────────── the requirement ─────────────────────────────

/// While the instance requires two-factor of an account that has none, the
/// requests it may still make: everything that needs no sign-in, who it is,
/// signing out, and setting up an authenticator app or a passkey.
pub fn open_while_required(method: &Method, segs: &[&str]) -> bool {
    use Method::*;
    matches!(
        (method, segs),
        (
            Get,
            ["instance"] | ["health"] | ["invites", _] | ["published", ..]
        ) | (Head, ["published", ..])
            | (Post, ["setup"])
            | (Post, ["auth", "signup" | "login" | "logout"])
            | (Post, ["auth", "password", "forgot" | "reset"])
            | (Get, ["auth", "me"] | ["auth", "google", ..])
            | (Get | Post, ["auth", "two-factor", ..])
            | (Post, ["auth", "device"] | ["auth", "device", "token"])
            | (Get | Patch, ["me", "two-factor"])
            | (Post, ["me", "two-factor", "authenticator"])
            | (Post, ["me", "two-factor", "authenticator", "confirm"])
            | (Post, ["auth", "passkey"] | ["auth", "passkey", "options"])
            | (Post, ["me", "passkeys"] | ["me", "passkeys", "options"])
    )
}

/// What every other request answers meanwhile.
pub fn setup_required_error(ctx: &Ctx) -> AppError {
    AppError(ApiError::new(
        ErrorCode::TwoFactorSetupRequired,
        format!(
            "This instance requires two-factor sign-in, and your account doesn't have it yet. Set it up at {} or with `fctl account two-factor enable`, then try again.",
            ctx.cfg.public_url
        ),
    ))
}

// ───────────────────────────── waiting sign-ins ─────────────────────────────

/// What a waiting sign-in does once its code checks out, beyond signing in.
#[derive(Default)]
pub struct Pending {
    /// A Google sign-in: the page it started from.
    pub return_to: Option<String>,
    /// A password reset: the link's token hash, and the new password.
    pub reset_hash: Option<String>,
    pub password_hash: Option<String>,
}

/// Leaves `user`'s sign-in waiting for its second factor: stores the
/// challenge, sets its cookie, and returns what the page shows.
pub async fn challenge(
    ctx: &mut Ctx,
    user: &UserRow,
    reason: ChallengeReason,
    pending: Pending,
) -> AppResult<TwoFactorChallenge> {
    let secret = new_secret(SecretKind::SignInChallenge);
    let expires_at = ctx.now + CHALLENGE_TTL_MS;
    q(
        &ctx.db,
        "INSERT INTO sign_in_challenges (hash, user_id, reason, return_to, reset_hash, password_hash, created_at, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            s(&hash_secret(&secret)),
            s(&user.id),
            s(reason.as_str()),
            os(pending.return_to.as_deref()),
            os(pending.reset_hash.as_deref()),
            os(pending.password_hash.as_deref()),
            n(ctx.now),
            n(expires_at),
        ],
    )?
    .run()
    .await?;
    ctx.cookies.push(http::set_cookie(
        Cookie::SecondFactor,
        &secret,
        (CHALLENGE_TTL_MS / 1000) as u64,
        ctx.cfg.secure_cookies(),
    ));
    describe(ctx, user, reason, expires_at).await
}

async fn describe(
    ctx: &Ctx,
    user: &UserRow,
    reason: ChallengeReason,
    expires_at: i64,
) -> AppResult<TwoFactorChallenge> {
    let app = app(ctx, &user.id).await?;
    let codes = unused_codes(ctx, &user.id).await?;
    let mut methods = Vec::new();
    if app.is_some() {
        methods.push(SecondFactor::Totp);
    }
    if user.passkeys > 0 {
        methods.push(SecondFactor::Passkey);
    }
    if codes > 0 {
        methods.push(SecondFactor::RecoveryCode);
    }
    Ok(TwoFactorChallenge {
        name: user.name.clone(),
        email: user.email.clone(),
        avatar_url: user.avatar_url.clone(),
        reason,
        methods,
        expires_at,
        locked_until: app.and_then(|a| a.locked_until).filter(|t| *t > ctx.now),
    })
}

#[derive(Deserialize)]
struct ChallengeRow {
    user_id: String,
    reason: String,
    return_to: Option<String>,
    reset_hash: Option<String>,
    password_hash: Option<String>,
    #[serde(deserialize_with = "crate::db::int")]
    expires_at: i64,
}

fn expired() -> AppError {
    AppError(ApiError::new(
        ErrorCode::Unauthorized,
        "This sign-in took too long. Start again.",
    ))
}

/// The request's waiting sign-in, by its cookie: its hash and row.
async fn waiting(ctx: &Ctx, req: &Request) -> AppResult<(String, ChallengeRow)> {
    let Some(secret) = http::cookie(req, Cookie::SecondFactor, ctx.cfg.secure_cookies()) else {
        return Err(expired());
    };
    let hash = hash_secret(&secret);
    let row: Option<ChallengeRow> = q(
        &ctx.db,
        "SELECT user_id, reason, return_to, reset_hash, password_hash, expires_at FROM sign_in_challenges WHERE hash = ?",
        &[s(&hash)],
    )?
    .first()
    .await?;
    let row = row.filter(|r| r.expires_at > ctx.now).ok_or_else(expired)?;
    Ok((hash, row))
}

/// The request's waiting sign-in: its hash and whose it is.
pub async fn waiting_sign_in(ctx: &Ctx, req: &Request) -> AppResult<(String, String)> {
    let (hash, row) = waiting(ctx, req).await?;
    Ok((hash, row.user_id))
}

async fn active_user(ctx: &Ctx, id: &str) -> AppResult<UserRow> {
    let user = auth::user_by_id(ctx, id).await?.ok_or_else(expired)?;
    if !user.is_active() {
        return Err(AppError(ApiError::new(
            ErrorCode::AccessDenied,
            "Your account is disabled. Ask an admin for access.",
        )));
    }
    Ok(user)
}

fn clear_cookie(ctx: &mut Ctx) {
    ctx.cookies.push(http::set_cookie(
        Cookie::SecondFactor,
        "",
        0,
        ctx.cfg.secure_cookies(),
    ));
}

/// `GET /api/v1/auth/two-factor/challenge`: the sign-in this browser waits
/// to finish, for the page that asks for the code.
pub async fn current(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    let (_, row) = waiting(ctx, req).await?;
    let user = active_user(ctx, &row.user_id).await?;
    let reason = ChallengeReason::parse(&row.reason).ok_or_else(expired)?;
    http::ok(&describe(ctx, &user, reason, row.expires_at).await?)
}

/// `POST /api/v1/auth/two-factor/cancel`: forget the waiting sign-in ("Not
/// you?", "Start again").
pub async fn cancel(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    http::check_same_origin(req, ctx.cfg.origin(), ctx.cfg.dev)?;
    if let Some(secret) = http::cookie(req, Cookie::SecondFactor, ctx.cfg.secure_cookies()) {
        q(
            &ctx.db,
            "DELETE FROM sign_in_challenges WHERE hash = ?",
            &[s(&hash_secret(&secret))],
        )?
        .run()
        .await?;
    }
    clear_cookie(ctx);
    http::ok(&Ok { ok: true })
}

/// `POST /api/v1/auth/two-factor/verify`: finishes the waiting sign-in with
/// a code from the authenticator app or a recovery code.
pub async fn verify(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    // The challenge cookie is a credential: only the app itself may use it.
    http::check_same_origin(req, ctx.cfg.origin(), ctx.cfg.dev)?;
    let body: VerifyTwoFactorRequest = http::body(req).await?;
    ctx.rate_limit(&format!("2fa:{}", ctx.ip_key())).await?;
    let (hash, row) = waiting(ctx, req).await?;
    ctx.rate_limit(&format!("2fa-user:{}", row.user_id)).await?;
    let user = active_user(ctx, &row.user_id).await?;
    let reason = ChallengeReason::parse(&row.reason).ok_or_else(expired)?;
    // A passkey is checked and used up here; a code only checked, and used
    // up once the challenge is claimed.
    let valid = match (&body.passkey, body.code.as_deref()) {
        (Some(passkey), _) => Valid::Passkey {
            name: crate::api::passkeys::second_factor(ctx, &user, &hash, passkey).await?,
        },
        (None, Some(code)) => check_code(ctx, &user, code).await?,
        (None, None) => return Err(validation("code", "Enter a code")),
    };
    // One sign-in per challenge, however many requests race for it; the
    // code is used up only by the request that got the challenge.
    let spent = q(
        &ctx.db,
        "DELETE FROM sign_in_challenges WHERE hash = ?",
        &[s(&hash)],
    )?
    .run()
    .await?;
    if spent == 0 {
        return Err(expired());
    }
    clear_cookie(ctx);
    let used = use_code(ctx, &user, valid).await?;
    if reason == ChallengeReason::PasswordReset {
        apply_reset(ctx, &user, &row).await?;
        crate::api::auth::password_reset_notice(ctx, &user).await?;
    }
    auth::start_web_session(ctx, &user, true).await?;
    let (factor, passkey) = match &used {
        CodeUse::Totp => ("totp", None),
        CodeUse::Recovery { .. } => ("recovery_code", None),
        CodeUse::Passkey { name } => ("passkey", Some(name.clone())),
    };
    ctx.audit(
        if reason == ChallengeReason::PasswordReset {
            "user.password_reset"
        } else {
            "user.login"
        },
        Some(&user.id),
        Some(
            serde_json::json!({ "via": reason.as_str(), "two_factor": factor, "passkey": passkey }),
        ),
    );
    let recovery_codes_left = match used {
        CodeUse::Recovery { left } => {
            let name = crate::instance::name(&ctx.db, &ctx.cfg).await?;
            recovery_code_notice(ctx, &user, &name, left);
            Some(left)
        }
        CodeUse::Totp | CodeUse::Passkey { .. } => None,
    };
    http::ok(&AuthResponse {
        me: Some(auth::me(ctx, &user).await?),
        two_factor: None,
        next: row.return_to,
        recovery_codes_left,
    })
}

#[derive(Deserialize)]
struct ResetRow {
    user_id: String,
    #[serde(deserialize_with = "crate::db::int")]
    expires_at: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    used_at: Option<i64>,
}

/// A reset link's new password takes effect once its code checked out:
/// until then, the link alone changed nothing.
async fn apply_reset(ctx: &Ctx, user: &UserRow, row: &ChallengeRow) -> AppResult<()> {
    let (Some(reset_hash), Some(phc)) = (row.reset_hash.as_deref(), row.password_hash.as_deref())
    else {
        return Err(expired());
    };
    let reset: Option<ResetRow> = q(
        &ctx.db,
        "SELECT user_id, expires_at, used_at FROM password_resets WHERE token_hash = ?",
        &[s(reset_hash)],
    )?
    .first()
    .await?;
    if !reset.is_some_and(|r| r.user_id == user.id && r.used_at.is_none() && r.expires_at > ctx.now)
    {
        return Err(AppError(ApiError::new(
            ErrorCode::Forbidden,
            "This reset link has expired. Ask for a new one.",
        )));
    }
    batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?",
                &[s(phc), n(ctx.now), s(&user.id)],
            )?,
            q(
                &ctx.db,
                "UPDATE password_resets SET used_at = ? WHERE token_hash = ? OR (user_id = ? AND used_at IS NULL)",
                &[n(ctx.now), s(reset_hash), s(&user.id)],
            )?,
            q(
                &ctx.db,
                "DELETE FROM tokens WHERE user_id = ?",
                &[s(&user.id)],
            )?,
            crate::auth::refuse_device_codes(&ctx.db, &user.id)?,
            q(
                &ctx.db,
                "DELETE FROM sign_in_challenges WHERE user_id = ?",
                &[s(&user.id)],
            )?,
        ],
    )
    .await
}

// ───────────────────────────── codes ─────────────────────────────

#[derive(Deserialize)]
struct AppRow {
    secret_enc: String,
    #[serde(deserialize_with = "crate::db::int")]
    last_step: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    locked_until: Option<i64>,
}

async fn app(ctx: &Ctx, user_id: &str) -> AppResult<Option<AppRow>> {
    q(
        &ctx.db,
        "SELECT secret_enc, last_step, locked_until FROM authenticator_apps WHERE user_id = ?",
        &[s(user_id)],
    )?
    .first()
    .await
}

fn totp_aad(user_id: &str) -> String {
    format!("totp:{user_id}")
}

/// A second factor that checked out, not used up yet ([`use_code`]).
enum Valid {
    Totp {
        step: i64,
    },
    Recovery {
        hash: String,
    },
    /// A passkey, by name: its challenge is used up already.
    Passkey {
        name: String,
    },
}

/// Which second factor it was, once used.
pub enum CodeUse {
    Totp,
    Recovery { left: u32 },
    Passkey { name: String },
}

fn locked(until: i64, now: i64) -> AppError {
    let secs = ((until - now) / 1000).max(1);
    let wait = match secs {
        ..=60 => "a minute".to_owned(),
        61..=3599 => format!("{} minutes", (secs + 59) / 60),
        _ => "an hour".to_owned(),
    };
    AppError(
        ApiError::new(
            ErrorCode::RateLimited,
            format!("Too many wrong codes. Try again in {wait}, or use a recovery code."),
        )
        .with_retry_after(secs as u32),
    )
}

/// Checks a code against `user`'s authenticator app or recovery codes,
/// without using it up. Wrong codes count towards the wait, which holds back
/// codes from the app: six digits can be guessed, recovery codes can't, so
/// whoever has them can always sign in.
async fn check_code(ctx: &Ctx, user: &UserRow, typed: &str) -> AppResult<Valid> {
    let Some(app) = app(ctx, &user.id).await? else {
        return Err(AppError(ApiError::new(
            ErrorCode::Conflict,
            "Two-factor sign-in is off for this account. Start again.",
        )));
    };
    if let Some(code) = totp::normalize(typed) {
        if let Some(until) = app.locked_until.filter(|t| *t > ctx.now) {
            return Err(locked(until, ctx.now));
        }
        let secret = ctx.keys()?.open(&totp_aad(&user.id), &app.secret_enc)?;
        return match totp::check(&secret, &code, ctx.now, app.last_step) {
            totp::Check::Valid(step) => Ok(Valid::Totp { step }),
            totp::Check::Used => Err(code_used()),
            totp::Check::Wrong => Err(wrong(ctx, user).await),
        };
    }
    if let Some(hash) = policy::hash_recovery_code(typed) {
        let unused: Option<Count> = q(
            &ctx.db,
            "SELECT COUNT(*) AS n FROM recovery_codes WHERE user_id = ? AND code_hash = ? AND used_at IS NULL",
            &[s(&user.id), s(&hash)],
        )?
        .first()
        .await?;
        if unused.is_some_and(|c| c.n > 0) {
            return Ok(Valid::Recovery { hash });
        }
    }
    Err(wrong(ctx, user).await)
}

/// Uses up a code that checked out; fails when another request used it
/// first.
async fn use_code(ctx: &Ctx, user: &UserRow, valid: Valid) -> AppResult<CodeUse> {
    match valid {
        Valid::Totp { step } => {
            // Conditional, so that two requests can't both use one code.
            let changed = q(
                &ctx.db,
                "UPDATE authenticator_apps SET last_step = ?, failures = 0, locked_until = NULL, last_used_at = ? \
                 WHERE user_id = ? AND last_step < ?",
                &[n(step), n(ctx.now), s(&user.id), n(step)],
            )?
            .run()
            .await?;
            if changed == 0 {
                return Err(code_used());
            }
            Ok(CodeUse::Totp)
        }
        Valid::Recovery { hash } => {
            let changed = q(
                &ctx.db,
                "UPDATE recovery_codes SET used_at = ? WHERE user_id = ? AND code_hash = ? AND used_at IS NULL",
                &[n(ctx.now), s(&user.id), s(&hash)],
            )?
            .run()
            .await?;
            if changed == 0 {
                return Err(validation("code", "That recovery code was just used."));
            }
            Ok(CodeUse::Recovery {
                left: unused_codes(ctx, &user.id).await?,
            })
        }
        Valid::Passkey { name } => Ok(CodeUse::Passkey { name }),
    }
}

/// Checks a code and uses it up, to confirm a change to two-factor sign-in.
pub async fn confirm_code(ctx: &Ctx, user: &UserRow, typed: &str) -> AppResult<CodeUse> {
    let valid = check_code(ctx, user, typed).await?;
    use_code(ctx, user, valid).await
}

fn code_used() -> AppError {
    validation(
        "code",
        "That code was just used. Enter the next one the app shows.",
    )
}

/// Counts a wrong code, and makes codes from the app wait when it was one
/// too many: the error to answer with.
async fn wrong(ctx: &Ctx, user: &UserRow) -> AppError {
    match count_wrong(ctx, user).await {
        Result::Ok(e) | Err(e) => e,
    }
}

#[derive(Deserialize)]
struct Failures {
    #[serde(deserialize_with = "crate::db::int")]
    failures: i64,
}

async fn count_wrong(ctx: &Ctx, user: &UserRow) -> AppResult<AppError> {
    // In one statement, so that wrong codes sent at once all count.
    let row: Option<Failures> = q(
        &ctx.db,
        "UPDATE authenticator_apps SET failures = failures + 1 WHERE user_id = ? RETURNING failures",
        &[s(&user.id)],
    )?
    .first()
    .await?;
    let failures = row.map_or(1, |r| r.failures.clamp(1, i64::from(u32::MAX))) as u32;
    let Some(until) = policy::lockout_ms(failures).map(|ms| ctx.now + ms) else {
        return Ok(validation(
            "code",
            "That code isn't right. Check the code your authenticator app shows for this account.",
        ));
    };
    // The longest wait wins when wrong codes race.
    q(
        &ctx.db,
        "UPDATE authenticator_apps SET locked_until = MAX(COALESCE(locked_until, 0), ?) WHERE user_id = ?",
        &[n(until), s(&user.id)],
    )?
    .run()
    .await?;
    ctx.audit(
        "two_factor.locked",
        Some(&user.id),
        Some(serde_json::json!({ "failures": failures, "until": until })),
    );
    if failures == policy::FREE_FAILURES {
        // Whoever it was had the password: the person should know.
        let name = crate::instance::name(&ctx.db, &ctx.cfg).await?;
        let place = ctx
            .location
            .as_deref()
            .map(|l| format!(" from near {l}"))
            .unwrap_or_default();
        crate::email::notice(
            ctx,
            user,
            "Wrong two-factor codes for your account",
            format!(
                "Someone signed in to {name} as {}{place} with your password, then entered {failures} wrong codes from an authenticator app. Sign-ins now wait before they take codes from the app again; recovery codes still work.\n\nIf it wasn't you, change your password now: whoever it was knows it. {}",
                user.email,
                ctx.cfg.url("/settings")
            ),
        );
    }
    Ok(locked(until, ctx.now))
}

async fn unused_codes(ctx: &Ctx, user_id: &str) -> AppResult<u32> {
    let c: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM recovery_codes WHERE user_id = ? AND used_at IS NULL",
        &[s(user_id)],
    )?
    .first()
    .await?;
    Ok(c.map_or(0, |c| c.n.max(0) as u32))
}

/// Statements that give `user_id` a new set of recovery codes, in place of
/// any they had.
fn recovery_statements(ctx: &Ctx, user_id: &str, codes: &[String]) -> AppResult<Vec<Query>> {
    let mut statements = vec![q(
        &ctx.db,
        "DELETE FROM recovery_codes WHERE user_id = ?",
        &[s(user_id)],
    )?];
    for code in codes {
        let hash = policy::hash_recovery_code(code).ok_or_else(|| internal("recovery code"))?;
        statements.push(q(
            &ctx.db,
            "INSERT INTO recovery_codes (user_id, code_hash, created_at) VALUES (?, ?, ?)",
            &[s(user_id), s(&hash), n(ctx.now)],
        )?);
    }
    Ok(statements)
}

// ───────────────────────────── setting it up ─────────────────────────────

/// A new authenticator app's secret, sealed for its user until the app's
/// first code confirms it.
#[derive(Serialize, Deserialize)]
struct SealedSetup {
    secret: String,
    exp: i64,
    /// A current code checked out: it may take the place of an app.
    #[serde(default)]
    replace: bool,
}

fn setup_aad(user_id: &str) -> String {
    format!("totp-setup:{user_id}")
}

/// Whether this browser signed in within the last 15 minutes. A CLI or
/// access token never counts: either can be made from a browser session
/// that signed in long ago.
pub fn fresh_sign_in(ctx: &Ctx) -> bool {
    ctx.auth
        .as_ref()
        .is_some_and(|a| a.kind == TokenKind::Web && a.signed_in_at >= ctx.now - FRESH_SIGN_IN_MS)
}

/// What setting up an app takes (`policy::setup_proof`): the password,
/// unless this browser signed in within the last 15 minutes. Without a
/// password, a first app takes that sign-in, which nothing stands in for.
fn setup_proof(ctx: &Ctx, user: &UserRow, replacing: bool) -> AppResult<policy::SetupProof> {
    match policy::setup_proof(replacing, user.has_password(), fresh_sign_in(ctx)) {
        policy::SetupProof::FreshSignIn => Err(validation(
            "sign_in",
            "Setting up two-factor sign-in takes a sign-in from the last 15 minutes. Sign out and in again in the browser, then set it up.",
        )),
        proof => Ok(proof),
    }
}

/// `POST /api/v1/me/two-factor/authenticator`: a new secret, its link and
/// QR code. Nothing changes until a code from the app confirms it.
/// Replacing an app takes a code from it, or a recovery code, as turning it
/// off does.
pub async fn start_authenticator(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: StartAuthenticatorRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    ctx.rate_limit(&format!("2fa-setup:{}", user.id)).await?;
    let keys = ctx.keys()?;
    let replace = app(ctx, &user.id).await?.is_some();
    if replace {
        let code = body
            .code
            .as_deref()
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| {
                validation(
                    "code",
                    "Enter a code from your current authenticator app, or a recovery code",
                )
            })?;
        confirm_code(ctx, &user, code).await?;
    }
    let proof = setup_proof(ctx, &user, replace)?;
    let secret = totp::base32(&totp::new_secret());
    let issuer = crate::instance::name(&ctx.db, &ctx.cfg).await?;
    let uri = totp::uri(&issuer, &user.email, &secret);
    let qr = fctl_domain::qr::rows(&uri).ok_or_else(|| internal("the setup link is too long"))?;
    let expires_at = ctx.now + SETUP_TTL_MS;
    let setup = keys.seal_json(
        &setup_aad(&user.id),
        &SealedSetup {
            secret: secret.clone(),
            exp: expires_at,
            replace,
        },
    )?;
    http::created(&AuthenticatorSetup {
        secret,
        uri,
        qr,
        setup,
        expires_at,
        password_required: proof == policy::SetupProof::Password,
    })
}

/// `POST /api/v1/me/two-factor/authenticator/confirm`: the app's first code
/// turns it on, in place of the old one if there was one. The first app
/// comes with recovery codes.
pub async fn confirm_authenticator(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: ConfirmAuthenticatorRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    ctx.rate_limit(&format!("2fa-setup:{}", user.id)).await?;
    let keys = ctx.keys()?;
    let setup: SealedSetup = keys
        .open_json(&setup_aad(&user.id), body.setup.trim())
        .ok()
        .filter(|st: &SealedSetup| st.exp > ctx.now)
        .ok_or_else(|| validation("setup", "This setup took too long. Start again."))?;
    let replacing = app(ctx, &user.id).await?.is_some();
    if replacing && !setup.replace {
        return Err(validation(
            "setup",
            "Your two-factor sign-in changed meanwhile. Start again.",
        ));
    }
    // A replacement only while there is still an app to replace.
    if setup_proof(ctx, &user, setup.replace && replacing)? == policy::SetupProof::Password {
        let given = body.password.as_deref().unwrap_or_default();
        let right = user
            .password_hash
            .as_deref()
            .is_some_and(|phc| !given.is_empty() && auth::verify_password(given, phc));
        if !right {
            return Err(validation(
                "password",
                if given.is_empty() {
                    "Enter your password to confirm it's you"
                } else {
                    "That isn't your password"
                },
            ));
        }
    }
    let secret = totp::from_base32(&setup.secret).ok_or_else(|| internal("setup secret"))?;
    let totp::Check::Valid(step) = totp::check(&secret, &body.code, ctx.now, 0) else {
        return Err(validation(
            "code",
            "That code doesn't match. Enter the code the app shows for this account; if it still doesn't work, check that your phone sets its clock automatically.",
        ));
    };
    let codes = (unused_codes(ctx, &user.id).await? == 0).then(policy::new_recovery_codes);
    let mut statements = vec![q(
        &ctx.db,
        "INSERT INTO authenticator_apps (user_id, secret_enc, last_step, failures, locked_until, created_at, last_used_at) \
         VALUES (?, ?, ?, 0, NULL, ?, NULL) ON CONFLICT (user_id) DO UPDATE SET secret_enc = excluded.secret_enc, \
         last_step = excluded.last_step, failures = 0, locked_until = NULL, created_at = excluded.created_at, last_used_at = NULL",
        &[
            s(&user.id),
            s(&keys.seal(&totp_aad(&user.id), &secret)?),
            n(step),
            n(ctx.now),
        ],
    )?];
    if let Some(codes) = &codes {
        statements.extend(recovery_statements(ctx, &user.id, codes)?);
    }
    batch(&ctx.db, statements).await?;
    ctx.audit(
        if replacing {
            "two_factor.replaced"
        } else {
            "two_factor.enabled"
        },
        Some(&user.id),
        None,
    );
    // Workspaces that require it open to them now.
    crate::workspaces::changed(ctx, vec![user.id.clone()]);
    let name = crate::instance::name(&ctx.db, &ctx.cfg).await?;
    if replacing {
        crate::email::notice(
            ctx,
            &user,
            "Your authenticator app was replaced",
            format!(
                "Sign-ins to your {name} account ({}) now take codes from the authenticator app you just set up; the old one's codes no longer work. Your recovery codes still do.\n\nIf you didn't do this, change your password and ask an admin of {name} to reset two-factor sign-in for you.",
                user.email
            ),
        );
    } else {
        crate::email::notice(
            ctx,
            &user,
            "Two-factor sign-in is on",
            format!(
                "Signing in to your {name} account ({}) with your password now also takes a code from your authenticator app.\n\nKeep your recovery codes somewhere safe: each one signs you in once if you lose your phone.\n\nIf you didn't turn this on, change your password and ask an admin of {name} to reset two-factor sign-in for you.",
                user.email
            ),
        );
    }
    let fresh = auth::user_by_id(ctx, &user.id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::ok(&AuthenticatorAdded {
        me: auth::me(ctx, &fresh).await?,
        recovery_codes: codes,
    })
}

/// `POST /api/v1/me/two-factor/authenticator/remove`: turns two-factor
/// off, with a current code. Not while the instance requires it.
pub async fn remove_authenticator(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: TwoFactorCodeRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    ctx.rate_limit(&format!("2fa-user:{}", user.id)).await?;
    let instance = crate::instance::settings(&ctx.db).await?;
    if !policy::can_turn_off(&instance, user.has_password(), user.factors()) {
        return Err(conflict(
            "This instance requires two-factor sign-in, so it stays on. To move it to a new phone, replace the authenticator app instead.",
        ));
    }
    if !policy::can_turn_off(
        &user.two_factor_settings(&instance),
        user.has_password(),
        user.factors(),
    ) {
        return Err(conflict(
            "A workspace you are in requires two-factor sign-in, so it stays on. Add a passkey first, or leave the workspace. To move it to a new phone, replace the authenticator app instead.",
        ));
    }
    confirm_code(ctx, &user, &body.code).await?;
    batch(&ctx.db, forget_statements(ctx, &user.id)?).await?;
    ctx.audit("two_factor.disabled", Some(&user.id), None);
    crate::workspaces::changed(ctx, vec![user.id.clone()]);
    let name = crate::instance::name(&ctx.db, &ctx.cfg).await?;
    crate::email::notice(
        ctx,
        &user,
        "Two-factor sign-in is off",
        format!(
            "Signing in to your {name} account ({}) with your password no longer asks for a code.\n\nIf you didn't turn it off, change your password and turn it on again in Settings: {}",
            user.email,
            ctx.cfg.url("/settings")
        ),
    );
    me(ctx, &user.id).await
}

/// `POST /api/v1/me/two-factor/recovery-codes`: ten new recovery codes,
/// with a current code. The old ones stop working.
pub async fn new_recovery_codes(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: TwoFactorCodeRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    ctx.rate_limit(&format!("2fa-user:{}", user.id)).await?;
    confirm_code(ctx, &user, &body.code).await?;
    let codes = policy::new_recovery_codes();
    batch(&ctx.db, recovery_statements(ctx, &user.id, &codes)?).await?;
    ctx.audit("two_factor.recovery_codes", Some(&user.id), None);
    let fresh = auth::user_by_id(ctx, &user.id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::ok(&RecoveryCodes {
        codes,
        me: auth::me(ctx, &fresh).await?,
    })
}

/// Statements that remove `user_id`'s two-factor sign-in: the app, the
/// recovery codes, and the sign-ins waiting for them.
pub fn forget_statements(ctx: &Ctx, user_id: &str) -> AppResult<Vec<Query>> {
    ["authenticator_apps", "recovery_codes", "sign_in_challenges"]
        .iter()
        .map(|table| {
            q(
                &ctx.db,
                &format!("DELETE FROM {table} WHERE user_id = ?"),
                &[s(user_id)],
            )
        })
        .collect()
}

/// `DELETE /api/v1/admin/users/{id}/two-factor`: admins turn off someone's
/// two-factor sign-in, when they lost their phone and recovery codes.
pub async fn admin_reset(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let actor = ctx.admin()?.clone();
    let target = auth::user_by_id(ctx, id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    if target.id == actor.id {
        return Err(forbidden(
            "Turn your own two-factor sign-in off in Settings, with a code",
        ));
    }
    if target.role() == InstanceRole::Owner {
        return Err(forbidden(
            "Nobody can reset the primary instance owner's two-factor sign-in: they sign in with a recovery code",
        ));
    }
    if target.role() == InstanceRole::Admin && actor.role() != InstanceRole::Owner {
        return Err(forbidden(
            "Only the primary instance owner can reset an admin's two-factor sign-in",
        ));
    }
    let settings = crate::instance::settings(&ctx.db).await?;
    let required = policy::required(&settings, target.has_password());
    // Passkeys go too where they are the second factor the instance
    // requires: one on the lost phone must not stay the factor nobody has.
    // Not when they are the person's only way in, though.
    let identities: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM identities WHERE user_id = ?",
        &[s(&target.id)],
    )?
    .first()
    .await?;
    let other_way_in = target.has_password() || identities.is_some_and(|c| c.n > 0);
    let passkeys = required && target.passkeys > 0;
    if passkeys && !other_way_in && !target.has_second_factor() {
        return Err(conflict(
            "Their passkeys are their only way in. Send them a reset link to set a password first.",
        ));
    }
    let remove_passkeys = passkeys && other_way_in;
    if !target.has_second_factor() && !remove_passkeys {
        return Err(conflict("They have no two-factor sign-in to reset"));
    }
    ctx.verified(fctl_domain::verify::Sensitive::ResetTwoFactor)?;
    let mut statements = forget_statements(ctx, &target.id)?;
    if remove_passkeys {
        statements.push(q(
            &ctx.db,
            "DELETE FROM passkeys WHERE user_id = ?",
            &[s(&target.id)],
        )?);
    }
    batch(&ctx.db, statements).await?;
    ctx.audit(
        "two_factor.reset",
        Some(&target.id),
        Some(serde_json::json!({ "passkeys_removed": remove_passkeys })),
    );
    // Workspaces that require it close to them.
    crate::workspaces::changed(ctx, vec![target.id.clone()]);
    let name = crate::instance::name(&ctx.db, &ctx.cfg).await?;
    let again = if required {
        format!("{name} requires two-factor sign-in, so you set it up again when you next sign in.")
    } else {
        format!(
            "To use it again, set it up in Settings: {}",
            ctx.cfg.url("/settings")
        )
    };
    let gone = if remove_passkeys {
        "your authenticator app's codes, your recovery codes and your passkeys no longer work"
    } else {
        "your authenticator app's codes and your recovery codes no longer work"
    };
    crate::email::notice(
        ctx,
        &target,
        "Two-factor sign-in was reset",
        format!(
            "{} turned off two-factor sign-in for your {name} account ({}): {gone}. {again}\n\nIf you didn't ask for this, tell them, and change your password.",
            actor.name, target.email
        ),
    );
    let updated = auth::user_by_id(ctx, &target.id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::ok(&updated.to_user())
}

// ───────────────────────────── notices ─────────────────────────────

fn recovery_code_notice(ctx: &Ctx, user: &UserRow, instance: &str, left: u32) {
    let place = ctx
        .location
        .as_deref()
        .map(|l| format!(" from near {l}"))
        .unwrap_or_default();
    let left = match left {
        0 => "None are left: get new ones in Settings now.".to_owned(),
        1 => "1 is left.".to_owned(),
        n => format!("{n} are left."),
    };
    crate::email::notice(
        ctx,
        user,
        "A recovery code was used to sign in",
        format!(
            "Someone signed in to {instance} as {}{place} with one of your recovery codes. {left}\n\nIf it wasn't you, change your password and get new recovery codes in Settings: {}",
            user.email,
            ctx.cfg.url("/settings")
        ),
    );
}
