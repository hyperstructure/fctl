//! Transactional email through Resend (optional; see config.rs).

use crate::config::EmailConfig;
use crate::ctx::Ctx;
use crate::error::{AppResult, unavailable};
use crate::model::UserRow;
use worker::{Fetch, Headers, Method, Request, RequestInit};

pub async fn send(cfg: &EmailConfig, to: &str, subject: &str, text: &str) -> AppResult<()> {
    let body =
        serde_json::json!({ "from": cfg.from, "to": [to], "subject": subject, "text": text });
    post(cfg, "https://api.resend.com/emails", &body).await
}

/// The most emails Resend takes in one batch.
const BATCH: usize = 100;

/// One email to each of `messages` (`to`, `subject`, `text`), in batches of
/// [`BATCH`]: a request for each hundred people, rather than one each,
/// which a change that concerns everyone at a domain would run out of.
pub async fn send_each(cfg: &EmailConfig, messages: &[(String, String, String)]) -> AppResult<()> {
    for chunk in messages.chunks(BATCH) {
        let body: Vec<serde_json::Value> = chunk
            .iter()
            .map(|(to, subject, text)| {
                serde_json::json!({ "from": cfg.from, "to": [to], "subject": subject, "text": text })
            })
            .collect();
        post(
            cfg,
            "https://api.resend.com/emails/batch",
            &serde_json::Value::Array(body),
        )
        .await?;
    }
    Ok(())
}

async fn post(cfg: &EmailConfig, url: &str, body: &serde_json::Value) -> AppResult<()> {
    let headers = Headers::new();
    headers.set("Authorization", &format!("Bearer {}", cfg.resend_api_key))?;
    headers.set("Content-Type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(body.to_string().into()));
    let req = Request::new_with_init(url, &init)?;
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
