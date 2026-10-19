//! Transactional email through Resend (optional; see config.rs).

use crate::config::EmailConfig;
use crate::ctx::Ctx;
use crate::error::{AppResult, unavailable};
use crate::model::UserRow;
use worker::{Fetch, Headers, Method, Request, RequestInit};

pub async fn send(cfg: &EmailConfig, to: &str, subject: &str, text: &str) -> AppResult<()> {
    let body =
        serde_json::json!({ "from": cfg.from, "to": [to], "subject": subject, "text": text });
    let headers = Headers::new();
    headers.set("Authorization", &format!("Bearer {}", cfg.resend_api_key))?;
    headers.set("Content-Type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(body.to_string().into()));
    let req = Request::new_with_init("https://api.resend.com/emails", &init)?;
    let res = Fetch::Request(req).send().await?;
    if res.status_code() >= 300 {
        worker::console_error!("email send failed: HTTP {}", res.status_code());
        return Err(unavailable("Email could not be sent right now"));
    }
    Ok(())
}

/// Emails `user` about a change to how they sign in, after the response,
/// when email is set up.
pub fn notice(ctx: &Ctx, user: &UserRow, subject: &str, text: String) {
    let Some(mail) = ctx.cfg.email.clone() else {
        return;
    };
    let to = user.email.clone();
    let subject = subject.to_owned();
    ctx.defer(async move { send(&mail, &to, &subject, &text).await });
}
