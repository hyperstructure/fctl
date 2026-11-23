//! Google sign-in and account linking (OpenID Connect, authorization code + PKCE).
//!
//! The browser goes to `/api/v1/auth/google/start?intent=login|link`, which
//! sets a signed, short-lived state cookie and redirects to Google. Google
//! redirects back to `/api/v1/auth/google/callback`, where the code is
//! exchanged server to server; the ID token arrives over that TLS connection
//! from Google, so its claims are trusted without a signature check (OIDC
//! Core §3.1.3.7) after checking issuer, audience, expiry and nonce.

use crate::auth;
use crate::crypto;
use crate::ctx::Ctx;
use crate::db::{Count, n, os, q, s};
use crate::error::{AppError, AppResult, not_configured, not_found};
use crate::http;
use crate::model::{UserRow, user_columns};
use fctl_domain::cookies::Cookie;
use fctl_proto::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use worker::{D1Database, Fetch, Headers, Method, Request, RequestInit, Response};

const STATE_TTL_MS: i64 = 10 * 60 * 1000;

#[derive(Serialize, Deserialize)]
struct OAuthState {
    state: String,
    verifier: String,
    nonce: String,
    intent: String,
    return_to: String,
    user_id: Option<String>,
    exp: i64,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: String,
    exp: i64,
    nonce: Option<String>,
    email: Option<String>,
    email_verified: Option<bool>,
    /// The domain of the Google Workspace (or Cloud Identity) account, for
    /// one: Google hosts its address (`fctl_domain::google`).
    hd: Option<String>,
    name: Option<String>,
    given_name: Option<String>,
    family_name: Option<String>,
    picture: Option<String>,
}

/// `path` when it is a path of the app, else the home page.
fn safe_return(path: Option<String>) -> String {
    path.filter(|p| fctl_domain::validate::is_app_path(p))
        .unwrap_or_else(|| "/".to_owned())
}

fn redirect_uri(ctx: &Ctx) -> String {
    ctx.cfg.url("/api/v1/auth/google/callback")
}

pub async fn start(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let google = ctx
        .cfg
        .google
        .clone()
        .ok_or_else(|| not_configured("Google sign-in is not configured"))?;
    let intent = http::query(req, "intent").unwrap_or_else(|| "login".into());
    let user_id = match intent.as_str() {
        "link" => {
            let user_id = ctx.user()?.id.clone();
            // A linked Google account signs in without the password, and
            // usually without a code: only a browser that signed in within
            // the last 15 minutes links one, so that a stolen session or
            // token can't add a way in.
            if !crate::api::two_factor::fresh_sign_in(ctx) {
                return fail(ctx, "/settings", "google_reauth");
            }
            Some(user_id)
        }
        "login" => None,
        _ => {
            return Err(AppError(ApiError::bad_request(
                "intent must be login or link",
            )));
        }
    };
    let (verifier, challenge) = crypto::pkce_pair();
    let st = OAuthState {
        state: crypto::b64url(&fctl_domain::random_bytes::<24>()),
        verifier,
        nonce: crypto::b64url(&fctl_domain::random_bytes::<24>()),
        intent,
        return_to: safe_return(http::query(req, "return_to")),
        user_id,
        exp: ctx.now + STATE_TTL_MS,
    };
    let signed = ctx.keys()?.sign(&serde_json::to_vec(&st)?);
    ctx.cookies.push(http::set_cookie(
        Cookie::GoogleSignIn,
        &signed,
        (STATE_TTL_MS / 1000) as u64,
        ctx.cfg.secure_cookies(),
    ));
    let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
        .map_err(crate::error::internal)?;
    url.query_pairs_mut()
        .append_pair("client_id", &google.client_id)
        .append_pair("redirect_uri", &redirect_uri(ctx))
        .append_pair("response_type", "code")
        .append_pair("scope", "openid email profile")
        .append_pair("state", &st.state)
        .append_pair("nonce", &st.nonce)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", "select_account");
    http::redirect(url.as_str())
}

/// Redirects back into the app with an error the page can show.
fn fail(ctx: &mut Ctx, return_to: &str, code: &str) -> AppResult<Response> {
    ctx.cookies.push(http::set_cookie(
        Cookie::GoogleSignIn,
        "",
        0,
        ctx.cfg.secure_cookies(),
    ));
    let sep = if return_to.contains('?') { '&' } else { '?' };
    http::redirect(&format!("{return_to}{sep}error={code}"))
}

pub async fn callback(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let google = ctx
        .cfg
        .google
        .clone()
        .ok_or_else(|| not_configured("Google sign-in is not configured"))?;
    let cookie = http::cookie(req, Cookie::GoogleSignIn, ctx.cfg.secure_cookies());
    let st: Option<OAuthState> = cookie
        .and_then(|c| ctx.keys().ok()?.verify(&c))
        .and_then(|b| serde_json::from_slice(&b).ok());
    let Some(st) = st.filter(|s| s.exp > ctx.now) else {
        return fail(ctx, "/login", "google_expired");
    };
    let failure_page = if st.intent == "link" {
        "/settings"
    } else {
        "/login"
    };
    if http::query(req, "state").as_deref() != Some(st.state.as_str()) {
        return fail(ctx, failure_page, "google_state");
    }
    if http::query(req, "error").is_some() {
        return fail(ctx, failure_page, "google_cancelled");
    }
    let Some(code) = http::query(req, "code") else {
        return fail(ctx, failure_page, "google_state");
    };
    // Exchange the code.
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", &code)
        .append_pair("client_id", &google.client_id)
        .append_pair("client_secret", &google.client_secret)
        .append_pair("redirect_uri", &redirect_uri(ctx))
        .append_pair("grant_type", "authorization_code")
        .append_pair("code_verifier", &st.verifier)
        .finish();
    let headers = Headers::new();
    headers.set("Content-Type", "application/x-www-form-urlencoded")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(form.into()));
    let mut res = Fetch::Request(Request::new_with_init(
        "https://oauth2.googleapis.com/token",
        &init,
    )?)
    .send()
    .await?;
    if res.status_code() != 200 {
        worker::console_error!("google token exchange: HTTP {}", res.status_code());
        return fail(ctx, failure_page, "google_exchange");
    }
    let tokens: TokenResponse = res.json().await?;
    let claims = tokens
        .id_token
        .as_deref()
        .and_then(|t| t.split('.').nth(1))
        .and_then(crypto::b64url_decode)
        .and_then(|b| serde_json::from_slice::<Claims>(&b).ok());
    let Some(claims) = claims else {
        return fail(ctx, failure_page, "google_exchange");
    };
    let valid = (claims.iss == "https://accounts.google.com"
        || claims.iss == "accounts.google.com")
        && claims.aud == google.client_id
        && claims.exp * 1000 > ctx.now
        && claims.nonce.as_deref() == Some(st.nonce.as_str());
    if !valid {
        return fail(ctx, failure_page, "google_invalid");
    }
    let email = claims.email.as_deref().map(str::to_lowercase);
    let verified = claims.email_verified.unwrap_or(false);
    ctx.cookies.push(http::set_cookie(
        Cookie::GoogleSignIn,
        "",
        0,
        ctx.cfg.secure_cookies(),
    ));

    let linked: Option<UserRow> = q(
        &ctx.db,
        &format!(
            "SELECT {} FROM identities i JOIN users u ON u.id = i.user_id WHERE i.provider = 'google' AND i.subject = ?",
            user_columns("u")
        ),
        &[s(&claims.sub)],
    )?
    .first()
    .await?;

    if st.intent == "link" {
        let Some(user_id) = st.user_id.as_deref() else {
            return fail(ctx, "/settings", "google_state");
        };
        let Some(user) = auth::user_by_id(ctx, user_id)
            .await?
            .filter(UserRow::is_active)
        else {
            return fail(ctx, "/settings", "google_state");
        };
        if let Some(other) = &linked {
            if other.id != user_id {
                return fail(ctx, "/settings", "google_linked_elsewhere");
            }
            return http::redirect(&format!("{}?linked=google", st.return_to));
        }
        insert_identity(ctx, user_id, &claims, email.as_deref()).await?;
        let instance = crate::instance::name(&ctx.db, &ctx.cfg).await?;
        crate::email::notice(
            ctx,
            &user,
            "A Google account was linked to your account",
            format!(
                "The Google account {} now signs in to your {instance} account ({}).\n\nIf you didn't link it, unlink it in Settings now and change your password: {}",
                email.as_deref().unwrap_or("without an email address"),
                user.email,
                ctx.cfg.url("/settings?tab=security")
            ),
        );
        if user.avatar_url.is_none() {
            adopt_picture(ctx, user_id, &claims).await;
        }
        ctx.audit(
            "identity.linked",
            Some(user_id),
            Some(serde_json::json!({ "provider": "google" })),
        );
        return http::redirect(&format!(
            "{}?linked=google",
            safe_return(Some(st.return_to))
        ));
    }

    // Sign in.
    if let Some(user) = linked {
        return sign_in(ctx, user, &st.return_to).await;
    }
    let Some(email) = email.filter(|_| verified) else {
        return fail(ctx, "/login", "google_unverified");
    };
    // Google vouches for an address only where it hosts it, Gmail's or a
    // Google Workspace account's: any other a Google account shows was
    // verified once, when the account was made, and may be someone else's
    // now (a former employee's, a recycled one). Signing up with one takes
    // the invitation sent to it, which only its mailbox gets.
    if !fctl_domain::google::vouches_for(&email, verified, claims.hd.as_deref()) {
        return fail(ctx, "/login", "google_not_authoritative");
    }
    if let Some(user) = auth::user_by_email(ctx, &email).await? {
        return first_sign_in(ctx, user, &claims, &email, &st.return_to).await;
    }
    let Some((_, role)) = crate::api::auth::allow_match(ctx, &email).await? else {
        return fail(ctx, "/login", "not_allowed");
    };
    let id = fctl_domain::ids::user_id(ctx.now);
    let role = if role == fctl_proto::session::InstanceRole::Admin {
        "admin"
    } else {
        "member"
    };
    // Google's first and last name when it has them; else its full name or
    // the email's local part.
    let fallback = claims
        .name
        .clone()
        .unwrap_or_else(|| email.split('@').next().unwrap_or("New user").to_owned());
    let person = fctl_domain::validate::person_name(
        claims.given_name.as_deref(),
        claims.family_name.as_deref(),
        None,
    )
    .or_else(|_| fctl_domain::validate::person_name(None, None, Some(&fallback)))
    .or_else(|_| fctl_domain::validate::person_name(None, None, Some("New user")))?;
    // At once, and failing whole: an address another account took since
    // it was looked up (connecting it, say) makes no second account for it.
    crate::db::batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "INSERT INTO users (id, email, name, given_name, family_name, role, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, 'active', ?, ?)",
                &[
                    s(id.as_str()),
                    s(&email),
                    s(&person.full),
                    os(person.given.as_deref()),
                    os(person.family.as_deref()),
                    s(role),
                    n(ctx.now),
                    n(ctx.now),
                ],
            )?,
            q(
                &ctx.db,
                "INSERT INTO user_emails (email, user_id, created_at) VALUES (?, ?, ?)",
                &[s(&email), s(id.as_str()), n(ctx.now)],
            )?,
            identity_insert(ctx, id.as_str(), &claims, Some(&email))?,
        ],
    )
    .await?;
    let user = auth::user_by_id(ctx, id.as_str())
        .await?
        .ok_or_else(|| not_found("User"))?;
    auth::start_web_session(ctx, &user, false).await?;
    // Their Gravatar first, which only people who chose a picture have;
    // else Google's, which is a letter for those who didn't.
    let user = crate::api::auth::new_gravatar(ctx, user).await?;
    if user.avatar_url.is_none() {
        adopt_picture(ctx, &user.id, &claims).await;
    }
    ctx.audit(
        "user.signup",
        Some(&user.id),
        Some(serde_json::json!({ "via": "google" })),
    );
    crate::api::workspaces::joined_after_sign_up(ctx, &user, &email).await?;
    http::redirect(&st.return_to)
}

/// Signs in `user`, whom this Google account signs in, and goes on to
/// `return_to`: by way of the page that asks for a code, when their
/// two-factor sign-in asks for one on Google sign-ins.
async fn sign_in(ctx: &mut Ctx, user: UserRow, return_to: &str) -> AppResult<Response> {
    if !user.is_active() {
        return fail(ctx, "/login", "disabled");
    }
    let settings = user.two_factor_settings(&crate::instance::settings(&ctx.db).await?);
    if fctl_domain::two_factor::asks(
        &settings,
        fctl_domain::two_factor::Via::Google,
        user.has_password(),
        user.factors(),
        user.two_factor_google,
    ) {
        // The page that asks for the code finishes it, then goes on to
        // `return_to`.
        crate::api::two_factor::challenge(
            ctx,
            &user,
            fctl_proto::api::ChallengeReason::Google,
            crate::api::two_factor::Pending {
                return_to: Some(return_to.to_owned()),
                ..Default::default()
            },
        )
        .await?;
        return http::redirect("/two-factor");
    }
    auth::start_web_session(ctx, &user, false).await?;
    ctx.audit(
        "user.login",
        Some(&user.id),
        Some(serde_json::json!({ "via": "google" })),
    );
    http::redirect(return_to)
}

/// A Google sign-in with the address of `user`, whom no Google account
/// signs in yet; Google vouches that the person holds the address.
///
/// Linking it by address would let a Google account take over an account
/// someone signs in to another way: they sign in that way, then link
/// Google from Settings. An account with no way to sign in at all (no
/// password, passkey or linked account), though, has nobody to take it
/// from: an admin token made it for this address (`api::provision`), and
/// nobody has signed in to it. Its first Google sign-in links Google to it.
async fn first_sign_in(
    ctx: &mut Ctx,
    user: UserRow,
    claims: &Claims,
    email: &str,
    return_to: &str,
) -> AppResult<Response> {
    if !user.is_active() {
        return fail(ctx, "/login", "disabled");
    }
    if !link_first(&ctx.db, &user.id, claims, email, ctx.now).await? {
        return fail(ctx, "/login", link_required(&user));
    }
    let instance = crate::instance::name(&ctx.db, &ctx.cfg).await?;
    crate::email::notice(
        ctx,
        &user,
        "A Google account was linked to your account",
        format!(
            "You signed in to your {instance} account ({}) for the first time, with the Google account {email}: it signs you in from now on.\n\nIf it wasn't you, tell an admin of {instance} now. To sign in another way too, add a password or a passkey in Settings: {}",
            user.email,
            ctx.cfg.url("/settings?tab=security")
        ),
    );
    if user.avatar_url.is_none() {
        adopt_picture(ctx, &user.id, claims).await;
    }
    let response = sign_in(ctx, user.clone(), return_to).await;
    // After the sign-in, which makes it theirs in the log; linked whether
    // or not it went through.
    ctx.audit(
        "identity.linked",
        Some(&user.id),
        Some(serde_json::json!({ "provider": "google", "via": "first_sign_in" })),
    );
    response
}

/// Links the Google account of `c` to the account `user_id` as its first
/// way to sign in, as one statement that checks again that it has none and
/// is active: a password, a passkey or a Google account given it since it
/// was read (another tab's first sign-in) leaves it as it is. Whether it
/// linked it.
async fn link_first(
    db: &D1Database,
    user_id: &str,
    c: &Claims,
    email: &str,
    now: i64,
) -> AppResult<bool> {
    let linked = q(
        db,
        "INSERT OR IGNORE INTO identities (id, user_id, provider, subject, email, name, avatar_url, created_at) \
         SELECT ?1, ?2, 'google', ?3, ?4, ?5, ?6, ?7 \
         WHERE EXISTS (SELECT 1 FROM users u WHERE u.id = ?2 AND u.status = 'active' AND u.password_hash IS NULL) \
         AND NOT EXISTS (SELECT 1 FROM passkeys p WHERE p.user_id = ?2) \
         AND NOT EXISTS (SELECT 1 FROM identities i WHERE i.user_id = ?2)",
        &[
            s(fctl_domain::ids::identity_id(now).as_str()),
            s(user_id),
            s(&c.sub),
            s(email),
            os(c.name.as_deref()),
            os(c.picture.as_deref()),
            n(now),
        ],
    )?
    .run()
    .await?;
    Ok(linked == 1)
}

/// Why a Google sign-in didn't link to `user`, who signs in another way,
/// as the sign-in page tells them: sign in that way, then link Google
/// from Settings.
fn link_required(user: &UserRow) -> &'static str {
    if user.has_password() {
        "google_link_required"
    } else if user.passkeys > 0 {
        "google_link_passkey"
    } else {
        // Another Google account, or this one a moment ago in another tab.
        "google_link_other"
    }
}

/// The Google account's picture as the avatar of someone who has none: a
/// copy (`api::avatars::adopt_google`). Without it, they keep their
/// initials until they upload one.
async fn adopt_picture(ctx: &Ctx, user_id: &str, c: &Claims) {
    if let Some(picture) = c.picture.as_deref()
        && let Err(e) = crate::api::avatars::adopt_google(ctx, user_id, picture).await
    {
        worker::console_log!("google picture not copied: {e}");
    }
}

async fn insert_identity(
    ctx: &Ctx,
    user_id: &str,
    c: &Claims,
    email: Option<&str>,
) -> AppResult<()> {
    identity_insert(ctx, user_id, c, email)?.run().await?;
    Ok(())
}

/// The statement that links the Google account of `c` to `user_id`.
fn identity_insert(
    ctx: &Ctx,
    user_id: &str,
    c: &Claims,
    email: Option<&str>,
) -> AppResult<crate::db::Query> {
    q(
        &ctx.db,
        "INSERT INTO identities (id, user_id, provider, subject, email, name, avatar_url, created_at) VALUES (?, ?, 'google', ?, ?, ?, ?, ?)",
        &[
            s(fctl_domain::ids::identity_id(ctx.now).as_str()),
            s(user_id),
            s(&c.sub),
            os(email),
            os(c.name.as_deref()),
            os(c.picture.as_deref()),
            n(ctx.now),
        ],
    )
}

pub async fn unlink(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let others: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM identities WHERE user_id = ? AND id != ?",
        &[s(&user.id), s(id)],
    )?
    .first()
    .await?;
    if user.password_hash.is_none() && user.passkeys == 0 && others.map_or(0, |c| c.n) == 0 {
        return Err(AppError(ApiError::new(
            ErrorCode::Conflict,
            "Set a password or add a passkey first, or you would have no way to sign in",
        )));
    }
    let changed = q(
        &ctx.db,
        "DELETE FROM identities WHERE id = ? AND user_id = ?",
        &[s(id), s(&user.id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(not_found("Linked account"));
    }
    ctx.audit("identity.unlinked", Some(&user.id), None);
    http::ok(&auth::me(ctx, &user).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Count;
    use crate::testing;
    use wasm_bindgen_test::wasm_bindgen_test;

    const NOW: i64 = 1_800_000_000_000;

    /// Bob, whom an admin token made as `api::provision::create` does: an
    /// active member, with an allowlist entry of his own and no way to sign
    /// in. Ada signs in with a password, Cy with a passkey and Dee with
    /// another Google account; Eve's account, made as Bob's was, is
    /// disabled.
    async fn world(db: &D1Database) -> AppResult<()> {
        for sql in [
            "INSERT INTO users (id, email, name, password_hash, role, status, created_at, updated_at) VALUES \
             ('usr_ada', 'ada@acme.dev', 'Ada', 'hash', 'admin', 'active', 0, 0), \
             ('usr_bob', 'bob@acme.dev', 'Bob', NULL, 'member', 'active', 0, 0), \
             ('usr_cy', 'cy@acme.dev', 'Cy', NULL, 'member', 'active', 0, 0), \
             ('usr_dee', 'dee@acme.dev', 'Dee', NULL, 'member', 'active', 0, 0), \
             ('usr_eve', 'eve@acme.dev', 'Eve', NULL, 'member', 'disabled', 0, 0)",
            "INSERT INTO user_emails (email, user_id, created_at) VALUES ('ada@acme.dev', 'usr_ada', 0), \
             ('bob@acme.dev', 'usr_bob', 0), ('cy@acme.dev', 'usr_cy', 0), ('dee@acme.dev', 'usr_dee', 0), \
             ('eve@acme.dev', 'usr_eve', 0)",
            "INSERT INTO allowlist (id, pattern, role, note, created_by, created_at) VALUES \
             ('alw_bob', 'bob@acme.dev', 'member', 'Admin token', 'usr_ada', 0), \
             ('alw_eve', 'eve@acme.dev', 'member', 'Admin token', 'usr_ada', 0)",
            "INSERT INTO passkeys (id, user_id, credential_id, public_key, algorithm, name, created_at) VALUES \
             ('psk_cy', 'usr_cy', 'cred-cy', 'key', -7, 'Laptop', 0)",
            "INSERT INTO identities (id, user_id, provider, subject, email, created_at) VALUES \
             ('idn_dee', 'usr_dee', 'google', 'g-dee-personal', 'dee@gmail.com', 0)",
        ] {
            q(db, sql, &[])?.run().await?;
        }
        Ok(())
    }

    /// What Google's ID token says of the account with `sub` and `email`.
    fn claims(sub: &str, email: &str) -> Claims {
        Claims {
            iss: "https://accounts.google.com".into(),
            aud: "client".into(),
            sub: sub.into(),
            exp: NOW / 1000 + 3600,
            nonce: None,
            email: Some(email.into()),
            email_verified: Some(true),
            hd: Some("acme.dev".into()),
            name: Some("Someone".into()),
            given_name: None,
            family_name: None,
            picture: None,
        }
    }

    async fn user(db: &D1Database, id: &str) -> AppResult<UserRow> {
        q(
            db,
            &format!("SELECT {} FROM users u WHERE u.id = ?", user_columns("u")),
            &[s(id)],
        )?
        .first()
        .await?
        .ok_or_else(|| not_found("User"))
    }

    /// The Google accounts that sign in to `user_id`.
    async fn googles(db: &D1Database, user_id: &str) -> AppResult<Vec<String>> {
        #[derive(Deserialize)]
        struct Subject {
            subject: String,
        }
        let rows: Vec<Subject> = q(
            db,
            "SELECT subject FROM identities WHERE user_id = ? AND provider = 'google' ORDER BY subject",
            &[s(user_id)],
        )?
        .all()
        .await?;
        Ok(rows.into_iter().map(|r| r.subject).collect())
    }

    #[wasm_bindgen_test]
    async fn a_provisioned_account_takes_its_first_google_sign_in() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        let bob = claims("g-bob", "bob@acme.dev");
        assert!(link_first(&db, "usr_bob", &bob, "bob@acme.dev", NOW).await?);
        assert_eq!(googles(&db, "usr_bob").await?, vec!["g-bob".to_owned()]);
        // Then it is his way in: another Google account with his address
        // links to it only from his Settings.
        let other = claims("g-bob-2", "bob@acme.dev");
        assert!(!link_first(&db, "usr_bob", &other, "bob@acme.dev", NOW + 1).await?);
        assert_eq!(googles(&db, "usr_bob").await?, vec!["g-bob".to_owned()]);
        assert_eq!(
            link_required(&user(&db, "usr_bob").await?),
            "google_link_other"
        );
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn an_account_someone_signs_in_to_keeps_to_its_way_in() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        for (id, email, code) in [
            ("usr_ada", "ada@acme.dev", "google_link_required"),
            ("usr_cy", "cy@acme.dev", "google_link_passkey"),
            ("usr_dee", "dee@acme.dev", "google_link_other"),
        ] {
            let c = claims(&format!("g-{id}"), email);
            assert!(!link_first(&db, id, &c, email, NOW).await?, "{id}");
            assert_eq!(link_required(&user(&db, id).await?), code, "{id}");
        }
        assert!(googles(&db, "usr_ada").await?.is_empty());
        assert!(googles(&db, "usr_cy").await?.is_empty());
        assert_eq!(
            googles(&db, "usr_dee").await?,
            vec!["g-dee-personal".to_owned()]
        );
        // A disabled account takes none, though nobody signs in to it.
        let eve = claims("g-eve", "eve@acme.dev");
        assert!(!link_first(&db, "usr_eve", &eve, "eve@acme.dev", NOW).await?);
        assert!(googles(&db, "usr_eve").await?.is_empty());
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn two_first_sign_ins_at_once_link_one_google_account() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        let (one, two) = (
            claims("g-bob", "bob@acme.dev"),
            claims("g-bob-2", "bob@acme.dev"),
        );
        let (a, b) = futures::join!(
            link_first(&db, "usr_bob", &one, "bob@acme.dev", NOW),
            link_first(&db, "usr_bob", &two, "bob@acme.dev", NOW),
        );
        assert_ne!(a?, b?);
        let linked: Option<Count> = q(
            &db,
            "SELECT COUNT(*) AS n FROM identities WHERE user_id = 'usr_bob'",
            &[],
        )?
        .first()
        .await?;
        assert_eq!(linked.map(|c| c.n), Some(1));
        Ok(())
    }
}
