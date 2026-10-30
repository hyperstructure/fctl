//! Request and response helpers: JSON bodies, cookies, security headers.

use crate::error::{AppError, AppResult};
use fctl_domain::cookies::Cookie;
use fctl_proto::{ApiError, ErrorCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use worker::{Headers, Method, Request, Response};

/// Largest JSON body accepted.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

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

pub fn error_response(e: &ApiError) -> Response {
    let status = e.code.http_status();
    let body = serde_json::to_string(e)
        .unwrap_or_else(|_| r#"{"code":"internal","message":"Something went wrong"}"#.to_owned());
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
    let too_large = || AppError(ApiError::bad_request("The request body is too large"));
    // Refused before it's read, where the client says how large it is.
    if length(req).is_some_and(|l| l > MAX_BODY_BYTES as u64) {
        return Err(too_large());
    }
    let bytes = read_upto(req, MAX_BODY_BYTES).await?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(too_large());
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| AppError(ApiError::bad_request(format!("Invalid request body: {e}"))))
}

/// The request's body, read as it comes, until it holds more than `limit`
/// bytes: then the rest stays unread, and what comes back is longer than
/// `limit`, for the caller's own check to refuse. A client that doesn't say
/// how large a body is (`Transfer-Encoding: chunked`, as the standalone
/// server's proxy passes bodies on) would otherwise have it kept in memory
/// whole, however large, before anything looked at its size. A failed
/// read is the runtime's error, for each caller to say as it does.
pub async fn read_upto(req: &mut Request, limit: usize) -> worker::Result<Vec<u8>> {
    use futures::StreamExt;
    let mut stream = match req.stream() {
        Ok(stream) => stream,
        // A request without a body.
        Err(worker::Error::RustError(_)) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk?);
        if bytes.len() > limit {
            break;
        }
    }
    Ok(bytes)
}

/// The size of the request's body, as its `Content-Length` says.
pub fn length(req: &Request) -> Option<u64> {
    header(req, "Content-Length").and_then(|l| l.trim().parse().ok())
}

/// A path segment as it was before percent-encoding (`ada%40example.com`
/// is `ada@example.com`). Bytes that aren't valid UTF-8 are replaced.
pub fn decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b, hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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
    req.headers()
        .get(name)
        .ok()
        .flatten()
        .filter(|v| !v.is_empty())
}

/// Reads one of our cookies, by its name where cookies are `secure` or not
/// (`Config::secure_cookies`).
pub fn cookie(req: &Request, cookie: Cookie, secure: bool) -> Option<String> {
    let name = cookie.name(secure);
    let raw = header(req, "Cookie")?;
    raw.split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// A `Set-Cookie` value. `max_age` of 0 deletes the cookie. Without `secure`
/// (plain http outside localhost) its name goes without the `__Host-` prefix,
/// which browsers take only on `Secure` cookies (`fctl_domain::cookies`).
pub fn set_cookie(cookie: Cookie, value: &str, max_age_secs: u64, secure: bool) -> String {
    let name = cookie.name(secure);
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
        if o == origin
            || (dev && (o.starts_with("http://localhost:") || o.starts_with("http://127.0.0.1:")))
        {
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
