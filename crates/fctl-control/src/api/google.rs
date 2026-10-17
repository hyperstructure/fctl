//! Google sign-in and account linking (OpenID Connect, authorization code + PKCE).
//!
//! The browser goes to `/api/v1/auth/google/start?intent=login|link`, which
//! sets a signed, short-lived state cookie and redirects to Google. Google
//! redirects back to `/api/v1/auth/google/callback`, where the code is
//! exchanged server to server; the ID token arrives over that TLS connection
//! from Google, so its claims are trusted without a signature check (OIDC
//! Core §3.1.3.7) after checking issuer, audience, expiry and nonce.

use crate::auth;
use crate::ctx::Ctx;
use crate::crypto;
use crate::db::{n, os, q, s, Count};
use crate::error::{not_configured, not_found, AppError, AppResult};
use crate::http::{self, OAUTH_COOKIE};
use crate::model::{user_columns, UserRow};
use fctl_proto::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use worker::{Fetch, Headers, Method, Request, RequestInit, Response};

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
    name: Option<String>,
    picture: Option<String>,
}

fn safe_return(path: Option<String>) -> String {
    match path {
        Some(p) if p.starts_with('/') && !p.starts_with("//") && !p.contains("\\") => p,
        _ => "/".to_owned(),
    }
}

fn redirect_uri(ctx: &Ctx) -> String {
    ctx.cfg.url("/api/v1/auth/google/callback")
}

pub async fn start(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let google = ctx.cfg.google.clone().ok_or_else(|| not_configured("Google sign-in is not configured"))?;
    let intent = http::query(req, "intent").unwrap_or_else(|| "login".into());
    let user_id = match intent.as_str() {
        "link" => Some(ctx.user()?.id.clone()),
        "login" => None,
        _ => return Err(AppError(ApiError::bad_request("intent must be login or link"))),
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
    ctx.cookies
        .push(http::set_cookie(OAUTH_COOKIE, &signed, (STATE_TTL_MS / 1000) as u64, ctx.cfg.secure_cookies()));
    let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").map_err(crate::error::internal)?;
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
    ctx.cookies.push(http::set_cookie(OAUTH_COOKIE, "", 0, ctx.cfg.secure_cookies()));
    let sep = if return_to.contains('?') { '&' } else { '?' };
    http::redirect(&format!("{return_to}{sep}error={code}"))
}

pub async fn callback(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let google = ctx.cfg.google.clone().ok_or_else(|| not_configured("Google sign-in is not configured"))?;
    let cookie = http::cookie(req, OAUTH_COOKIE);
    let st: Option<OAuthState> = cookie
        .and_then(|c| ctx.keys().ok()?.verify(&c))
        .and_then(|b| serde_json::from_slice(&b).ok());
    let Some(st) = st.filter(|s| s.exp > ctx.now) else {
        return fail(ctx, "/login", "google_expired");
    };
    let failure_page = if st.intent == "link" { "/settings" } else { "/login" };
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
    init.with_method(Method::Post).with_headers(headers).with_body(Some(form.into()));
    let mut res = Fetch::Request(Request::new_with_init("https://oauth2.googleapis.com/token", &init)?)
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
    let valid = (claims.iss == "https://accounts.google.com" || claims.iss == "accounts.google.com")
        && claims.aud == google.client_id
        && claims.exp * 1000 > ctx.now
        && claims.nonce.as_deref() == Some(st.nonce.as_str());
    if !valid {
        return fail(ctx, failure_page, "google_invalid");
    }
    let email = claims.email.as_deref().map(str::to_lowercase);
    let verified = claims.email_verified.unwrap_or(false);
    ctx.cookies.push(http::set_cookie(OAUTH_COOKIE, "", 0, ctx.cfg.secure_cookies()));

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
        if let Some(other) = &linked {
            if other.id != user_id {
                return fail(ctx, "/settings", "google_linked_elsewhere");
            }
            return http::redirect(&format!("{}?linked=google", st.return_to));
        }
        insert_identity(ctx, user_id, &claims, email.as_deref()).await?;
        if let Some(pic) = claims.picture.as_deref() {
            q(&ctx.db, "UPDATE users SET avatar_url = COALESCE(avatar_url, ?) WHERE id = ?", &[s(pic), s(user_id)])?
                .run()
                .await?;
        }
        ctx.audit("identity.linked", Some(user_id), Some(serde_json::json!({ "provider": "google" })));
        return http::redirect(&format!("{}?linked=google", safe_return(Some(st.return_to))));
    }

    // Sign in.
    if let Some(user) = linked {
        if !user.is_active() {
            return fail(ctx, "/login", "disabled");
        }
        auth::start_web_session(ctx, &user).await?;
        ctx.audit("user.login", Some(&user.id), Some(serde_json::json!({ "via": "google" })));
        return http::redirect(&st.return_to);
    }
    let Some(email) = email.filter(|_| verified) else {
        return fail(ctx, "/login", "google_unverified");
    };
    if auth::user_by_email(ctx, &email).await?.is_some() {
        // Linking by email silently would let a Google account take over a
        // password account; the owner links it from Settings instead.
        return fail(ctx, "/login", "google_link_required");
    }
    let Some((_, role)) = crate::api::auth::allow_match(ctx, &email).await? else {
        return fail(ctx, "/login", "not_allowed");
    };
    let id = fctl_domain::ids::user_id(ctx.now);
    let role = if role == fctl_proto::session::InstanceRole::Admin { "admin" } else { "member" };
    let name = claims.name.clone().unwrap_or_else(|| email.split('@').next().unwrap_or("New user").to_owned());
    q(
        &ctx.db,
        "INSERT INTO users (id, email, name, avatar_url, role, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'active', ?, ?)",
        &[s(id.as_str()), s(&email), s(&name), os(claims.picture.as_deref()), s(role), n(ctx.now), n(ctx.now)],
    )?
    .run()
    .await?;
    insert_identity(ctx, id.as_str(), &claims, Some(&email)).await?;
    let user = auth::user_by_id(ctx, id.as_str()).await?.ok_or_else(|| not_found("User"))?;
    auth::start_web_session(ctx, &user).await?;
    ctx.audit("user.signup", Some(&user.id), Some(serde_json::json!({ "via": "google" })));
    http::redirect(&st.return_to)
}

async fn insert_identity(ctx: &Ctx, user_id: &str, c: &Claims, email: Option<&str>) -> AppResult<()> {
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
    )?
    .run()
    .await?;
    Ok(())
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
    if user.password_hash.is_none() && others.map_or(0, |c| c.n) == 0 {
        return Err(AppError(ApiError::new(
            ErrorCode::Conflict,
            "Set a password first, or you would have no way to sign in",
        )));
    }
    let changed = q(&ctx.db, "DELETE FROM identities WHERE id = ? AND user_id = ?", &[s(id), s(&user.id)])?
        .run()
        .await?;
    if changed == 0 {
        return Err(not_found("Linked account"));
    }
    ctx.audit("identity.unlinked", Some(&user.id), None);
    http::ok(&auth::me(ctx, &user).await?)
}
