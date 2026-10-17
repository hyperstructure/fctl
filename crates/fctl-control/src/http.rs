//! Request and response helpers: JSON bodies, cookies, security headers.

use crate::error::{AppError, AppResult};
use fctl_proto::{ApiError, ErrorCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use worker::{Headers, Method, Request, Response};

/// Largest JSON body accepted.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

pub const SESSION_COOKIE: &str = "__Host-fctl";
pub const OAUTH_COOKIE: &str = "__Host-fctl-oauth";

/// Headers on every API response.
pub fn api_headers() -> Headers {
    let h = Headers::new();
    let _ = h.set("Content-Type", "application/json; charset=utf-8");
    let _ = h.set("Cache-Control", "no-store");
    let _ = h.set("X-Content-Type-Options", "nosniff");
    let _ = h.set("Referrer-Policy", "same-origin");
    let _ = h.set("X-Frame-Options", "DENY");
    let _ = h.set("Cross-Origin-Resource-Policy", "same-origin");
    h
}

pub fn json<T: Serialize>(status: u16, body: &T) -> AppResult<Response> {
    let text = serde_json::to_string(body)?;
    Ok(Response::ok(text)?
        .with_status(status)
        .with_headers(api_headers()))
}

pub fn ok<T: Serialize>(body: &T) -> AppResult<Response> {
    json(200, body)
}

pub fn created<T: Serialize>(body: &T) -> AppResult<Response> {
    json(201, body)
}

pub fn no_content() -> AppResult<Response> {
    Ok(Response::empty()?.with_status(204).with_headers(api_headers()))
}

pub fn error_response(e: &ApiError) -> Response {
    let status = e.code.http_status();
    let body = serde_json::to_string(e).unwrap_or_else(|_| {
        r#"{"code":"internal","message":"Something went wrong"}"#.to_owned()
    });
    let headers = api_headers();
    if let Some(s) = e.retry_after {
        let _ = headers.set("Retry-After", &s.to_string());
    }
    match Response::ok(body) {
        Ok(r) => r.with_status(status).with_headers(headers),
        Err(_) => fallback_response(status),
    }
}

/// `Response::empty` cannot fail in practice; this keeps error paths total.
#[allow(clippy::unwrap_used)]
fn fallback_response(status: u16) -> Response {
    Response::empty().unwrap().with_status(status)
}

pub fn redirect(location: &str) -> AppResult<Response> {
    let h = Headers::new();
    h.set("Location", location)?;
    h.set("Cache-Control", "no-store")?;
    Ok(Response::empty()?.with_status(302).with_headers(h))
}

/// Parses a JSON body, rejecting non-JSON content types and huge bodies.
pub async fn body<T: DeserializeOwned>(req: &mut Request) -> AppResult<T> {
    let ct = req.headers().get("Content-Type")?.unwrap_or_default();
    if !ct.to_ascii_lowercase().starts_with("application/json") {
        return Err(AppError(ApiError::bad_request(
            "Send the body as application/json",
        )));
    }
    let bytes = req.bytes().await?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(AppError(ApiError::bad_request("The request body is too large")));
    }
    serde_json::from_slice(&bytes).map_err(|e| {
        AppError(ApiError::bad_request(format!("Invalid request body: {e}")))
    })
}

/// A query parameter.
pub fn query(req: &Request, name: &str) -> Option<String> {
    let url = req.url().ok()?;
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
}

pub fn header(req: &Request, name: &str) -> Option<String> {
    req.headers().get(name).ok().flatten().filter(|v| !v.is_empty())
}

/// Reads a cookie by name.
pub fn cookie(req: &Request, name: &str) -> Option<String> {
    let raw = header(req, "Cookie")?;
    raw.split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// A `Set-Cookie` value. `max_age` of 0 deletes the cookie.
pub fn set_cookie(name: &str, value: &str, max_age_secs: u64, secure: bool) -> String {
    let mut c = format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}");
    if secure {
        c.push_str("; Secure");
    }
    c
}

pub fn is_safe_method(m: &Method) -> bool {
    matches!(m, Method::Get | Method::Head | Method::Options)
}

/// For cookie-authenticated requests that change state: the request must come
/// from the app itself. Non-browser clients use bearer tokens and skip this.
pub fn check_same_origin(req: &Request, origin: &str, dev: bool) -> AppResult<()> {
    if let Some(o) = header(req, "Origin") {
        if o == origin || (dev && (o.starts_with("http://localhost:") || o.starts_with("http://127.0.0.1:"))) {
            return Ok(());
        }
        return Err(AppError(ApiError::new(
            ErrorCode::Forbidden,
            "Cross-site request refused",
        )));
    }
    match header(req, "Sec-Fetch-Site").as_deref() {
        Some("same-origin") | Some("none") => Ok(()),
        _ => Err(AppError(ApiError::new(
            ErrorCode::Forbidden,
            "Cross-site request refused",
        ))),
    }
}

/// The client's IP as Cloudflare saw it.
pub fn client_ip(req: &Request) -> Option<String> {
    header(req, "CF-Connecting-IP")
}

/// "Stockholm, SE" from Cloudflare's request metadata.
pub fn client_location(req: &Request) -> Option<String> {
    let cf = req.cf()?;
    let city = cf.city();
    let country = cf.country();
    match (city, country) {
        (Some(c), Some(k)) => Some(format!("{c}, {k}")),
        (None, Some(k)) => Some(k),
        (Some(c), None) => Some(c),
        _ => None,
    }
}

pub fn is_websocket_upgrade(req: &Request) -> bool {
    header(req, "Upgrade").is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
}
