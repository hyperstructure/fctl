//! Google sign-in and account linking (OpenID Connect, authorization code +
//! PKCE): Google's part of signing in with a provider (`api::sign_in`).
//!
//! The browser goes to `/api/v1/auth/google/start?intent=login|link`, which
//! sets a signed, short-lived state cookie and redirects to Google. Google
//! redirects back to `/api/v1/auth/google/callback`, where the code is
//! exchanged server to server; the ID token arrives over that TLS connection
//! from Google, so its claims are trusted without a signature check (OIDC
//! Core §3.1.3.7) after checking issuer, audience, expiry and nonce. What
//! it says of the person then goes to `api::sign_in::finish`, which links,
//! signs in or signs up as the instance's sign-in rules say.

use crate::api::sign_in::{self, Intent, Provided, fail, safe_return};
use crate::crypto;
use crate::ctx::Ctx;
use crate::error::{AppError, AppResult, not_configured};
use crate::http;
use fctl_domain::cookies::Cookie;
use fctl_proto::ApiError;
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
    /// The domain of the Google Workspace (or Cloud Identity) account, for
    /// one: Google hosts its address (`fctl_domain::google`).
    hd: Option<String>,
    name: Option<String>,
    given_name: Option<String>,
    family_name: Option<String>,
    picture: Option<String>,
}

impl Claims {
    /// What they say of the person, as `api::sign_in` takes it.
    fn provided(self) -> Provided {
        let email = self.email.as_deref().map(str::to_lowercase);
        let verified = self.email_verified.unwrap_or(false);
        let vouched = email
            .clone()
            .filter(|e| fctl_domain::google::vouches_for(e, verified, self.hd.as_deref()));
        Provided {
            provider: fctl_domain::sign_in::GOOGLE,
            subject: self.sub,
            email,
            verified,
            vouched,
            name: self.name,
            given_name: self.given_name,
            family_name: self.family_name,
            picture: self.picture,
        }
    }
}

/// Where Google sends people back after they sign in: the instance's OAuth
/// client lists it as an authorized redirect URI (Admin → Sign-in shows
/// it).
pub fn redirect_url(cfg: &crate::config::Config) -> String {
    cfg.url("/api/v1/auth/google/callback")
}

fn redirect_uri(ctx: &Ctx) -> String {
    redirect_url(&ctx.cfg)
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
                return clear_and_fail(ctx, "/settings", "google_reauth");
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
    // The address the sign-in page was given, at a domain that signs in
    // with Google: Google offers its account first. Only a hint: what
    // Google answers is checked as ever.
    if let Some(hint) =
        http::query(req, "login_hint").and_then(|h| fctl_domain::validate::email(&h).ok())
    {
        url.query_pairs_mut().append_pair("login_hint", &hint);
    }
    http::redirect(url.as_str())
}

/// Forgets the sign-in's state and redirects back into the app with an
/// error the page can show.
fn clear_and_fail(ctx: &mut Ctx, return_to: &str, code: &str) -> AppResult<Response> {
    clear_state(ctx);
    fail(return_to, code, "")
}

fn clear_state(ctx: &mut Ctx) {
    ctx.cookies.push(http::set_cookie(
        Cookie::GoogleSignIn,
        "",
        0,
        ctx.cfg.secure_cookies(),
    ));
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
        return clear_and_fail(ctx, "/login", "google_expired");
    };
    let failure_page = if st.intent == "link" {
        "/settings"
    } else {
        "/login"
    };
    if http::query(req, "state").as_deref() != Some(st.state.as_str()) {
        return clear_and_fail(ctx, failure_page, "google_state");
    }
    if http::query(req, "error").is_some() {
        return clear_and_fail(ctx, failure_page, "google_cancelled");
    }
    let Some(code) = http::query(req, "code") else {
        return clear_and_fail(ctx, failure_page, "google_state");
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
        return clear_and_fail(ctx, failure_page, "google_exchange");
    }
    let tokens: TokenResponse = res.json().await?;
    let claims = tokens
        .id_token
        .as_deref()
        .and_then(|t| t.split('.').nth(1))
        .and_then(crypto::b64url_decode)
        .and_then(|b| serde_json::from_slice::<Claims>(&b).ok());
    let Some(claims) = claims else {
        return clear_and_fail(ctx, failure_page, "google_exchange");
    };
    let valid = (claims.iss == "https://accounts.google.com"
        || claims.iss == "accounts.google.com")
        && claims.aud == google.client_id
        && claims.exp * 1000 > ctx.now
        && claims.nonce.as_deref() == Some(st.nonce.as_str());
    if !valid {
        return clear_and_fail(ctx, failure_page, "google_invalid");
    }
    clear_state(ctx);
    let intent = match (st.intent.as_str(), st.user_id) {
        ("link", Some(user_id)) => Intent::Link {
            user_id,
            return_to: st.return_to,
        },
        ("link", None) => return fail("/settings", "google_state", ""),
        _ => Intent::Login {
            return_to: st.return_to,
        },
    };
    sign_in::finish(ctx, claims.provided(), intent).await
}
