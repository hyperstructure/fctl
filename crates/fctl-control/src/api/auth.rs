//! `/api/v1/instance`, `/api/v1/setup`, `/api/v1/auth/*`, `/api/v1/invites/*`.

use crate::api::two_factor::Pending;
use crate::auth::{self, DAY_MS};
use crate::ctx::Ctx;
use crate::db::{Count, batch, n, os, q, s};
use crate::error::{AppError, AppResult, conflict, not_found, validation};
use crate::http;
use crate::model::user_columns;
use fctl_domain::allowlist;
use fctl_domain::tokens::{self, SecretKind, hash_secret, new_secret};
use fctl_domain::two_factor::{self, Via};
use fctl_domain::validate;
use fctl_proto::api::*;
use fctl_proto::session::InstanceRole;
use fctl_proto::{ApiError, ErrorCode};
use worker::{Request, Response};

pub async fn instance(ctx: &Ctx) -> AppResult<Response> {
    let owners: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM users WHERE role = 'owner'",
        &[],
    )?
    .first()
    .await?;
    let settings = crate::instance::settings(&ctx.db).await?;
    http::ok(&InstanceInfo {
        name: crate::instance::name(&ctx.db, &ctx.cfg).await?,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        setup_required: owners.map_or(0, |c| c.n) == 0,
        features: Features {
            google: ctx.cfg.google.is_some(),
            email: ctx.cfg.email.is_some(),
            github: ctx.cfg.github.is_some(),
            publish_screenshots: settings.publish_screenshots,
            early_access: ctx.cfg.early_access,
        },
    })
}

/// Creates the owner with the one-time setup token.
pub async fn setup(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    ctx.rate_limit(&format!("setup:{}", ctx.ip_key())).await?;
    let body: SetupRequest = http::body(req).await?;
    let Some(expected) = ctx.cfg.setup_token.clone() else {
        return Err(AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "This instance has no setup token. Set FCTL_SETUP_TOKEN and try again.",
        )));
    };
    if !tokens::constant_time_eq(body.token.trim().as_bytes(), expected.as_bytes()) {
        return Err(AppError(ApiError::new(
            ErrorCode::Forbidden,
            "The setup link is not valid",
        )));
    }
    let email = validate::email(&body.email)?;
    let person = validate::person_name(
        body.given_name.as_deref(),
        body.family_name.as_deref(),
        body.name.as_deref(),
    )?;
    validate::password(&body.password, Some(&email))?;
    let owners: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM users WHERE role = 'owner'",
        &[],
    )?
    .first()
    .await?;
    if owners.map_or(0, |c| c.n) > 0 {
        return Err(conflict("Setup is already complete. Sign in instead."));
    }
    let hash = auth::hash_password(&body.password)?;
    let id = fctl_domain::ids::user_id(ctx.now);
    let allow_id = fctl_domain::ids::allow_id(ctx.now);
    // The guard in the INSERT makes concurrent setups create one owner.
    batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "INSERT INTO users (id, email, name, given_name, family_name, password_hash, role, status, created_at, updated_at) \
                 SELECT ?, ?, ?, ?, ?, ?, 'owner', 'active', ?, ? WHERE NOT EXISTS (SELECT 1 FROM users WHERE role = 'owner')",
                &[s(id.as_str()), s(&email), s(&person.full), os(person.given.as_deref()), os(person.family.as_deref()), s(&hash), n(ctx.now), n(ctx.now)],
            )?,
            q(
                &ctx.db,
                "INSERT OR IGNORE INTO allowlist (id, pattern, role, note, created_by, created_at) VALUES (?, ?, 'admin', 'Instance owner', ?, ?)",
                &[s(allow_id.as_str()), s(&email), s(id.as_str()), n(ctx.now)],
            )?,
            q(
                &ctx.db,
                "INSERT OR IGNORE INTO user_emails (email, user_id, created_at) \
                 SELECT ?, ?, ? WHERE EXISTS (SELECT 1 FROM users WHERE id = ?)",
                &[s(&email), s(id.as_str()), n(ctx.now), s(id.as_str())],
            )?,
        ],
    )
    .await?;
    let user = auth::user_by_id(ctx, id.as_str())
        .await?
        .ok_or_else(|| conflict("Setup is already complete. Sign in instead."))?;
    auth::start_web_session(ctx, &user).await?;
    ctx.audit("instance.setup", Some(&user.id), None);
    http::created(&signed_in(auth::me(ctx, &user).await?))
}

#[derive(serde::Deserialize)]
pub struct InviteRow {
    pub id: String,
    pub email: String,
    role: String,
    #[serde(deserialize_with = "crate::db::int")]
    expires_at: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    accepted_at: Option<i64>,
    inviter: Option<String>,
}

async fn find_invite(ctx: &Ctx, token: &str) -> AppResult<Option<InviteRow>> {
    q(
        &ctx.db,
        "SELECT i.id, i.email, i.role, i.expires_at, i.accepted_at, u.name AS inviter \
         FROM invites i LEFT JOIN users u ON u.id = i.created_by WHERE i.token_hash = ?",
        &[s(&hash_secret(token))],
    )?
    .first()
    .await
}

/// An invitation link that still works.
pub async fn live_invite(ctx: &Ctx, token: &str) -> AppResult<Option<InviteRow>> {
    Ok(find_invite(ctx, token)
        .await?
        .filter(|i| i.accepted_at.is_none() && i.expires_at > ctx.now))
}

/// `GET /api/v1/invites/{token}` (public).
pub async fn invite_preview(ctx: &Ctx, token: &str) -> AppResult<Response> {
    let Some(invite) = live_invite(ctx, token).await? else {
        return Err(not_found("Invitation"));
    };
    let workspaces: Vec<crate::workspaces::InvitationRow> = q(
        &ctx.db,
        &format!(
            "{} WHERE i.email = ? AND i.expires_at > ? ORDER BY i.created_at",
            crate::workspaces::INVITATION_SELECT
        ),
        &[s(&invite.email), n(ctx.now)],
    )?
    .all()
    .await?;
    let early_access = crate::api::early_access::asked(ctx, &invite.email).await?;
    http::ok(&InvitePreview {
        email: invite.email,
        instance: crate::instance::name(&ctx.db, &ctx.cfg).await?,
        invited_by: invite.inviter,
        expires_at: invite.expires_at,
        workspaces: workspaces.iter().map(|w| w.workspace()).collect(),
        early_access,
    })
}

#[derive(serde::Deserialize)]
struct AllowPatternRow {
    pattern: String,
    role: String,
}

/// The allowlist entry that lets `email` in, if any.
pub async fn allow_match(ctx: &Ctx, email: &str) -> AppResult<Option<(String, InstanceRole)>> {
    let rows: Vec<AllowPatternRow> = q(&ctx.db, "SELECT pattern, role FROM allowlist", &[])?
        .all()
        .await?;
    Ok(
        allowlist::best_match(&rows, email, |r| r.pattern.as_str()).map(|r| {
            (
                r.pattern.clone(),
                InstanceRole::parse(&r.role).unwrap_or(InstanceRole::Member),
            )
        }),
    )
}

/// Password sign-up needs an invitation for the email: without email
/// verification, an allowlist match alone would let anyone claim an address.
pub async fn signup(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    ctx.rate_limit(&format!("signup:{}", ctx.ip_key())).await?;
    let body: SignupRequest = http::body(req).await?;
    let email = validate::email(&body.email)?;
    let person = validate::person_name(
        body.given_name.as_deref(),
        body.family_name.as_deref(),
        body.name.as_deref(),
    )?;
    validate::password(&body.password, Some(&email))?;
    // The invitation first: without one for this address, the answer must
    // not say whether it has an account (docs/architecture.md#early-access).
    let invite = match body.invite.as_deref() {
        Some(token) => live_invite(ctx, token).await?,
        None => None,
    };
    let Some(invite) = invite else {
        return Err(AppError(ApiError::new(
            ErrorCode::AccessDenied,
            "Signing up needs an invitation link. Ask an admin of this instance for one.",
        )));
    };
    if invite.email != email {
        return Err(validation(
            "email",
            "This invitation is for a different email address",
        ));
    }
    if auth::user_by_email(ctx, &email).await?.is_some() {
        return Err(conflict(
            "An account with this email already exists. Sign in instead.",
        ));
    }
    if allow_match(ctx, &email).await?.is_none() {
        return Err(AppError(ApiError::new(
            ErrorCode::AccessDenied,
            "This email is no longer allowed on this instance.",
        )));
    }
    let role = match InstanceRole::parse(&invite.role) {
        Some(InstanceRole::Admin) => "admin",
        _ => "member",
    };
    let hash = auth::hash_password(&body.password)?;
    let id = fctl_domain::ids::user_id(ctx.now);
    batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "INSERT INTO users (id, email, name, given_name, family_name, password_hash, role, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)",
                &[s(id.as_str()), s(&email), s(&person.full), os(person.given.as_deref()), os(person.family.as_deref()), s(&hash), s(role), n(ctx.now), n(ctx.now)],
            )?,
            q(
                &ctx.db,
                "UPDATE invites SET accepted_at = ? WHERE id = ?",
                &[n(ctx.now), s(&invite.id)],
            )?,
            q(
                &ctx.db,
                "INSERT INTO user_emails (email, user_id, created_at) VALUES (?, ?, ?)",
                &[s(&email), s(id.as_str()), n(ctx.now)],
            )?,
        ],
    )
    .await?;
    let user = auth::user_by_id(ctx, id.as_str())
        .await?
        .ok_or_else(|| crate::error::internal("user vanished after insert"))?;
    auth::start_web_session(ctx, &user).await?;
    ctx.audit(
        "user.signup",
        Some(&user.id),
        Some(serde_json::json!({ "via": "invite" })),
    );
    crate::api::workspaces::joined_after_sign_up(ctx, &user, &email).await?;
    http::created(&signed_in(auth::me(ctx, &user).await?))
}

pub async fn login(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: LoginRequest = http::body(req).await?;
    let email = body.email.trim().to_lowercase();
    ctx.rate_limit(&format!("login:{}:{email}", ctx.ip_key()))
        .await?;
    ctx.rate_limit(&format!("login-email:{email}")).await?;
    let user = auth::user_by_email(ctx, &email).await?;
    let ok = match user.as_ref().and_then(|u| u.password_hash.as_deref()) {
        Some(phc) => auth::verify_password(&body.password, phc),
        None => {
            auth::dummy_verify(&body.password);
            false
        }
    };
    let Some(user) = user.filter(|_| ok) else {
        return Err(AppError(ApiError::new(
            ErrorCode::InvalidCredentials,
            "That email and password don't match an account",
        )));
    };
    if !user.is_active() {
        return Err(AppError(ApiError::new(
            ErrorCode::AccessDenied,
            "Your account is disabled. Ask an admin for access.",
        )));
    }
    let settings = user.two_factor_settings(&crate::instance::settings(&ctx.db).await?);
    if two_factor::asks(
        &settings,
        Via::Password,
        user.has_password(),
        user.factors(),
        user.two_factor_google,
    ) {
        let challenge = crate::api::two_factor::challenge(
            ctx,
            &user,
            ChallengeReason::Password,
            Pending::default(),
        )
        .await?;
        return http::ok(&waits_for(challenge));
    }
    auth::start_web_session(ctx, &user).await?;
    ctx.audit("user.login", Some(&user.id), None);
    http::ok(&signed_in(auth::me(ctx, &user).await?))
}

/// The answer to a sign-in that is done.
pub fn signed_in(me: Me) -> AuthResponse {
    AuthResponse {
        me: Some(me),
        two_factor: None,
        next: None,
        recovery_codes_left: None,
    }
}

/// The answer to a sign-in that waits for its second factor.
fn waits_for(challenge: TwoFactorChallenge) -> AuthResponse {
    AuthResponse {
        me: None,
        two_factor: Some(challenge),
        next: None,
        recovery_codes_left: None,
    }
}

pub async fn logout(ctx: &mut Ctx) -> AppResult<Response> {
    // Signed in, the request came from the app (the origin check), so the
    // cookie goes; without the cookie, as from a page of another site, it
    // stays. One that no longer signs anyone in is cleared already.
    if let Some(a) = ctx.auth.clone() {
        q(
            &ctx.db,
            "DELETE FROM tokens WHERE id = ?",
            &[s(&a.token_id)],
        )?
        .run()
        .await?;
        if a.via_cookie {
            auth::clear_session_cookie(ctx);
        }
    }
    http::ok(&Ok { ok: true })
}

pub async fn get_me(ctx: &Ctx) -> AppResult<Response> {
    let user = ctx.user()?;
    http::ok(&auth::me(ctx, user).await?)
}

pub async fn update_me(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: UpdateMeRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    let person = if body.given_name.is_some() || body.name.is_some() {
        validate::person_name(
            body.given_name.as_deref(),
            body.family_name.as_deref(),
            body.name.as_deref(),
        )?
    } else {
        validate::PersonName {
            full: user.name.clone(),
            given: user.given_name.clone(),
            family: user.family_name.clone(),
        }
    };
    let avatar = match body.avatar_url.as_deref().map(str::trim) {
        Some("") => None,
        Some(url) if url.starts_with("https://") && url.len() <= 500 => Some(url.to_owned()),
        Some(_) => return Err(validation("avatar_url", "Use an https:// image URL")),
        None => user.avatar_url.clone(),
    };
    // The primary address's commit email (Settings → Email addresses sets
    // the others').
    if let Some(e) = body.commit_email.as_deref() {
        let commit_email = validate::commit_email(e)?;
        crate::api::emails::set_commit_email(ctx, &user, &user.email, commit_email.as_deref())
            .await?;
    }
    q(
        &ctx.db,
        "UPDATE users SET name = ?, given_name = ?, family_name = ?, avatar_url = ?, updated_at = ? WHERE id = ?",
        &[
            s(&person.full),
            os(person.given.as_deref()),
            os(person.family.as_deref()),
            os(avatar.as_deref()),
            n(ctx.now),
            s(&user.id),
        ],
    )?
    .run()
    .await?;
    let user = auth::user_by_id(ctx, &user.id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::ok(&auth::me(ctx, &user).await?)
}

pub async fn change_password(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: ChangePasswordRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    ctx.rate_limit(&format!("password:{}", user.id)).await?;
    if let Some(phc) = user.password_hash.as_deref() {
        let current = body.current_password.as_deref().unwrap_or_default();
        if !auth::verify_password(current, phc) {
            return Err(validation(
                "current_password",
                "That isn't your current password",
            ));
        }
    } else if !crate::api::two_factor::fresh_sign_in(ctx) {
        // A password is a new way in: without one to confirm, only a browser
        // that signed in within the last 15 minutes sets it, so that a
        // stolen session or token can't.
        return Err(validation(
            "new_password",
            "Setting a password takes a sign-in from the last 15 minutes. Sign out and in again, then set it.",
        ));
    }
    validate::password(&body.new_password, Some(&user.email))?;
    let hash = auth::hash_password(&body.new_password)?;
    q(
        &ctx.db,
        "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?",
        &[s(&hash), n(ctx.now), s(&user.id)],
    )?
    .run()
    .await?;
    let keep = ctx.auth.as_ref().map(|a| a.token_id.clone());
    auth::revoke_tokens(ctx, &user.id, keep.as_deref()).await?;
    // Sign-ins with the old password that wait for a code go too.
    q(
        &ctx.db,
        "DELETE FROM sign_in_challenges WHERE user_id = ?",
        &[s(&user.id)],
    )?
    .run()
    .await?;
    ctx.audit("user.password_changed", Some(&user.id), None);
    let instance = crate::instance::name(&ctx.db, &ctx.cfg).await?;
    crate::email::notice(
        ctx,
        &user,
        "Your password was changed",
        format!(
            "The password of your {instance} account ({}) was {}, and its other sessions were signed out.\n\nIf you didn't do this, reset your password from the sign-in page now: {}",
            user.email,
            if user.has_password() {
                "changed"
            } else {
                "set"
            },
            ctx.cfg.url("/login")
        ),
    );
    http::ok(&Ok { ok: true })
}

/// Always answers the same way, so it cannot be used to probe for accounts:
/// the link is made and sent after the response, so neither how long the
/// answer takes nor whether the email could be sent says whether the
/// address has one.
pub async fn forgot_password(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: ForgotPasswordRequest = http::body(req).await?;
    ctx.rate_limit(&format!("forgot:{}", ctx.ip_key())).await?;
    let email = body.email.trim().to_lowercase();
    if let (Some(mail), Some(user)) = (
        ctx.cfg.email.clone(),
        auth::user_by_email(ctx, &email).await?,
    ) && user.is_active()
    {
        let secret = new_secret(SecretKind::PasswordReset);
        let url = reset_url(ctx, &secret);
        let text = format!(
            "Someone asked to reset the password of your {} account.\n\nChoose a new password: {url}\n\nThe link works for one hour. If it wasn't you, ignore this email.",
            ctx.cfg.instance_name
        );
        let db = crate::db::handle(&ctx.db);
        let now = ctx.now;
        ctx.defer(async move {
            store_reset(&db, &secret, &user.id, None, now).await?;
            // To the address they typed: any of the account's.
            crate::email::send(&mail, &email, "Reset your password", &text).await
        });
    }
    http::ok(&Ok { ok: true })
}

/// Where the reset link with `secret` opens.
fn reset_url(ctx: &Ctx, secret: &str) -> String {
    ctx.cfg.url(&format!("/reset#token={secret}"))
}

/// Keeps a one-hour password reset link's secret (its SHA-256) for `user_id`.
async fn store_reset(
    db: &worker::D1Database,
    secret: &str,
    user_id: &str,
    created_by: Option<&str>,
    now: i64,
) -> AppResult<()> {
    q(
        db,
        "INSERT INTO password_resets (token_hash, user_id, created_by, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
        &[s(&hash_secret(secret)), s(user_id), os(created_by), n(now), n(now + 60 * 60 * 1000)],
    )?
    .run()
    .await?;
    Ok(())
}

/// A one-hour password reset link for `user_id`.
pub async fn create_reset_link(
    ctx: &Ctx,
    user_id: &str,
    created_by: Option<&str>,
) -> AppResult<String> {
    let secret = new_secret(SecretKind::PasswordReset);
    store_reset(&ctx.db, &secret, user_id, created_by, ctx.now).await?;
    Ok(reset_url(ctx, &secret))
}

#[derive(serde::Deserialize)]
struct ResetRow {
    user_id: String,
    #[serde(deserialize_with = "crate::db::int")]
    expires_at: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    used_at: Option<i64>,
}

pub async fn reset_password(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: ResetPasswordRequest = http::body(req).await?;
    ctx.rate_limit(&format!("reset:{}", ctx.ip_key())).await?;
    let hash = hash_secret(body.token.trim());
    let row: Option<ResetRow> = q(
        &ctx.db,
        "SELECT user_id, expires_at, used_at FROM password_resets WHERE token_hash = ?",
        &[s(&hash)],
    )?
    .first()
    .await?;
    let Some(row) = row.filter(|r| r.used_at.is_none() && r.expires_at > ctx.now) else {
        return Err(AppError(ApiError::new(
            ErrorCode::Forbidden,
            "This reset link has expired. Ask for a new one.",
        )));
    };
    let user = auth::user_by_id(ctx, &row.user_id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    if !user.is_active() {
        return Err(AppError(ApiError::new(
            ErrorCode::AccessDenied,
            "Your account is disabled.",
        )));
    }
    validate::password(&body.new_password, Some(&user.email))?;
    let phc = auth::hash_password(&body.new_password)?;
    // With two-factor, the link alone changes nothing: the new password
    // takes effect once a code checks out (`api::two_factor::verify`).
    let settings = user.two_factor_settings(&crate::instance::settings(&ctx.db).await?);
    if two_factor::on(&settings, user.has_password(), user.factors()) {
        let challenge = crate::api::two_factor::challenge(
            ctx,
            &user,
            ChallengeReason::PasswordReset,
            Pending {
                reset_hash: Some(hash),
                password_hash: Some(phc),
                ..Pending::default()
            },
        )
        .await?;
        return http::ok(&waits_for(challenge));
    }
    batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?",
                &[s(&phc), n(ctx.now), s(&user.id)],
            )?,
            q(
                &ctx.db,
                "UPDATE password_resets SET used_at = ? WHERE token_hash = ?",
                &[n(ctx.now), s(&hash)],
            )?,
            q(
                &ctx.db,
                "DELETE FROM tokens WHERE user_id = ?",
                &[s(&user.id)],
            )?,
        ],
    )
    .await?;
    auth::start_web_session(ctx, &user).await?;
    ctx.audit("user.password_reset", Some(&user.id), None);
    password_reset_notice(ctx, &user).await?;
    http::ok(&signed_in(auth::me(ctx, &user).await?))
}

/// Emails `user` that a reset link changed their password: links come by
/// email, and from admins.
pub async fn password_reset_notice(ctx: &Ctx, user: &crate::model::UserRow) -> AppResult<()> {
    let instance = crate::instance::name(&ctx.db, &ctx.cfg).await?;
    crate::email::notice(
        ctx,
        user,
        "Your password was reset",
        format!(
            "A password reset link set a new password for your {instance} account ({}), and its other sessions were signed out.\n\nIf you didn't do this, reset your password from the sign-in page now, and tell the instance's admins: {}",
            user.email,
            ctx.cfg.url("/login")
        ),
    );
    Ok(())
}

// ───────────────────────────── tokens ─────────────────────────────

#[derive(serde::Deserialize)]
struct TokenRow {
    id: String,
    kind: String,
    name: Option<String>,
    user_agent: Option<String>,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    last_used_at: Option<i64>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    expires_at: Option<i64>,
}

pub async fn list_tokens(ctx: &Ctx) -> AppResult<Response> {
    let user = ctx.user()?;
    let current = ctx
        .auth
        .as_ref()
        .map(|a| a.token_id.clone())
        .unwrap_or_default();
    let rows: Vec<TokenRow> = q(
        &ctx.db,
        "SELECT id, kind, name, user_agent, created_at, last_used_at, expires_at FROM tokens \
         WHERE user_id = ? AND (expires_at IS NULL OR expires_at > ?) ORDER BY COALESCE(last_used_at, created_at) DESC",
        &[s(&user.id), n(ctx.now)],
    )?
    .all()
    .await?;
    http::ok(&TokenList {
        tokens: rows
            .into_iter()
            .map(|r| TokenInfo {
                current: r.id == current,
                id: fctl_proto::TokenId(r.id),
                kind: TokenKind::parse(&r.kind).unwrap_or(TokenKind::Pat),
                name: r.name,
                created_at: r.created_at,
                last_used_at: r.last_used_at,
                expires_at: r.expires_at,
                user_agent: r.user_agent,
            })
            .collect(),
    })
}

pub async fn create_token(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: CreateTokenRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    let name = validate::name("name", &body.name)?;
    let expires_at = match body.expires_in_days {
        Some(0) | None => None,
        Some(d) if d <= 3650 => Some(ctx.now + d as i64 * DAY_MS),
        Some(_) => return Err(validation("expires_in_days", "Use at most 3650 days")),
    };
    let (id, secret) =
        auth::issue_token(ctx, &user.id, TokenKind::Pat, Some(&name), expires_at).await?;
    ctx.audit(
        "token.created",
        Some(&id),
        Some(serde_json::json!({ "name": name })),
    );
    http::created(&CreatedToken {
        token: TokenInfo {
            id: fctl_proto::TokenId(id),
            kind: TokenKind::Pat,
            name: Some(name),
            created_at: ctx.now,
            last_used_at: None,
            expires_at,
            user_agent: None,
            current: false,
        },
        secret,
    })
}

pub async fn delete_token(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let changed = q(
        &ctx.db,
        "DELETE FROM tokens WHERE id = ? AND user_id = ?",
        &[s(id), s(&user.id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(not_found("Token"));
    }
    if ctx
        .auth
        .as_ref()
        .is_some_and(|a| a.token_id == id && a.via_cookie)
    {
        auth::clear_session_cookie(ctx);
    }
    http::ok(&Ok { ok: true })
}

// ───────────────────────────── device flow ─────────────────────────────

const DEVICE_TTL_MS: i64 = 10 * 60 * 1000;
const DEVICE_INTERVAL_S: u32 = 2;

pub async fn device_start(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    ctx.rate_limit(&format!("device:{}", ctx.ip_key())).await?;
    let body: DeviceStartRequest = http::body(req).await?;
    let client_name = validate::name("client_name", &body.client_name)?;
    let device_code = new_secret(SecretKind::DeviceCode);
    // Retry on the (unlikely) user code collision.
    let mut user_code = tokens::new_user_code();
    for _ in 0..3 {
        let taken: Option<Count> = q(
            &ctx.db,
            "SELECT COUNT(*) AS n FROM device_codes WHERE user_code = ?",
            &[s(&user_code)],
        )?
        .first()
        .await?;
        if taken.map_or(0, |c| c.n) == 0 {
            break;
        }
        user_code = tokens::new_user_code();
    }
    q(
        &ctx.db,
        "DELETE FROM device_codes WHERE expires_at < ?",
        &[n(ctx.now - DAY_MS)],
    )?
    .run()
    .await?;
    q(
        &ctx.db,
        "INSERT INTO device_codes (device_hash, user_code, client_name, status, ip, location, user_agent, created_at, expires_at) \
         VALUES (?, ?, ?, 'pending', ?, ?, ?, ?, ?)",
        &[
            s(&hash_secret(&device_code)),
            s(&user_code),
            s(&client_name),
            os(ctx.ip.as_deref()),
            os(ctx.location.as_deref()),
            os(ctx.user_agent.as_deref()),
            n(ctx.now),
            n(ctx.now + DEVICE_TTL_MS),
        ],
    )?
    .run()
    .await?;
    http::created(&DeviceStartResponse {
        verification_uri: ctx.cfg.url("/device"),
        verification_uri_complete: ctx.cfg.url(&format!("/device?code={user_code}")),
        device_code,
        user_code,
        expires_in: (DEVICE_TTL_MS / 1000) as u32,
        interval: DEVICE_INTERVAL_S,
    })
}

#[derive(serde::Deserialize)]
struct DeviceRow {
    device_hash: String,
    user_code: String,
    client_name: String,
    status: String,
    user_id: Option<String>,
    ip: Option<String>,
    location: Option<String>,
    user_agent: Option<String>,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    #[serde(deserialize_with = "crate::db::int")]
    expires_at: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    last_poll_at: Option<i64>,
}

pub async fn device_poll(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: DevicePollRequest = http::body(req).await?;
    let hash = hash_secret(body.device_code.trim());
    let row: Option<DeviceRow> = q(
        &ctx.db,
        "SELECT * FROM device_codes WHERE device_hash = ?",
        &[s(&hash)],
    )?
    .first()
    .await?;
    let Some(row) = row else {
        return http::ok(&DevicePollResponse::Expired);
    };
    if row.expires_at <= ctx.now || row.status == "consumed" {
        return http::ok(&DevicePollResponse::Expired);
    }
    if row.status == "denied" {
        return http::ok(&DevicePollResponse::Denied);
    }
    let too_fast = row
        .last_poll_at
        .is_some_and(|t| ctx.now - t < (DEVICE_INTERVAL_S as i64 * 1000) - 250);
    q(
        &ctx.db,
        "UPDATE device_codes SET last_poll_at = ? WHERE device_hash = ?",
        &[n(ctx.now), s(&hash)],
    )?
    .run()
    .await?;
    if row.status == "pending" {
        return http::ok(if too_fast {
            &DevicePollResponse::SlowDown
        } else {
            &DevicePollResponse::Pending
        });
    }
    // Approved: issue the CLI token exactly once.
    let claimed = q(
        &ctx.db,
        "UPDATE device_codes SET status = 'consumed' WHERE device_hash = ? AND status = 'approved'",
        &[s(&row.device_hash)],
    )?
    .run()
    .await?;
    let Some(user_id) = row.user_id.filter(|_| claimed == 1) else {
        return http::ok(&DevicePollResponse::Expired);
    };
    let user = auth::user_by_id(ctx, &user_id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    if !user.is_active() {
        return http::ok(&DevicePollResponse::Denied);
    }
    let (_, token) =
        auth::issue_token(ctx, &user.id, TokenKind::Cli, Some(&row.client_name), None).await?;
    let me = auth::me(ctx, &user).await?;
    http::ok(&DevicePollResponse::Approved {
        token,
        me: Box::new(me),
    })
}

async fn device_by_user_code(ctx: &Ctx, code: &str) -> AppResult<DeviceRow> {
    let code = tokens::normalize_user_code(code)
        .ok_or_else(|| validation("code", "Check the code and try again"))?;
    let row: Option<DeviceRow> = q(
        &ctx.db,
        "SELECT * FROM device_codes WHERE user_code = ?",
        &[s(&code)],
    )?
    .first()
    .await?;
    row.filter(|r| r.expires_at > ctx.now && r.status == "pending")
        .ok_or_else(|| {
            validation(
                "code",
                "This code has expired or was already used. Run fctl login again.",
            )
        })
}

pub async fn device_info(ctx: &Ctx, code: &str) -> AppResult<Response> {
    let user = ctx.user()?;
    // Codes are short enough to type: nobody tries many of them.
    ctx.rate_limit(&format!("device-code:{}", user.id)).await?;
    let row = device_by_user_code(ctx, code).await?;
    http::ok(&DeviceInfo {
        user_code: row.user_code,
        client_name: row.client_name,
        created_at: row.created_at,
        expires_at: row.expires_at,
        ip: row.ip,
        location: row.location,
        user_agent: row.user_agent,
    })
}

pub async fn device_decide(ctx: &mut Ctx, req: &mut Request, code: &str) -> AppResult<Response> {
    let body: DeviceDecisionRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    ctx.rate_limit(&format!("device-code:{}", user.id)).await?;
    let row = device_by_user_code(ctx, code).await?;
    let status = if body.approve { "approved" } else { "denied" };
    q(
        &ctx.db,
        "UPDATE device_codes SET status = ?, user_id = ? WHERE device_hash = ? AND status = 'pending'",
        &[s(status), s(&user.id), s(&row.device_hash)],
    )?
    .run()
    .await?;
    ctx.audit(
        if body.approve {
            "device.approved"
        } else {
            "device.denied"
        },
        Some(&user.id),
        Some(serde_json::json!({ "client": row.client_name })),
    );
    http::ok(&Ok { ok: true })
}

/// A member of a workspace by the address they are a member as, for
/// sharing and project membership dialogs: only the workspace's people,
/// and only by the address it knows them by.
pub async fn member_by_email(
    ctx: &Ctx,
    workspace_id: &str,
    email: &str,
) -> AppResult<Option<crate::model::UserRow>> {
    let email = validate::email(email)?;
    q(
        &ctx.db,
        &format!(
            "SELECT {} FROM workspace_members m JOIN users u ON u.id = m.user_id \
             WHERE m.workspace_id = ? AND m.email = ? AND u.status = 'active'",
            user_columns("u")
        ),
        &[s(workspace_id), s(&email)],
    )?
    .first()
    .await
}
