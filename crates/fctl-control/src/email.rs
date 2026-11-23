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
/// The pause between two batches: Resend takes two requests a second by
/// default.
const BATCH_GAP_MS: u64 = 600;

/// One email to each of `messages` (`to`, `subject`, `text`), in batches of
/// [`BATCH`]: a request for each hundred people, rather than one each,
/// which a change that concerns everyone at a domain would run out of.
/// A batch that fails doesn't stop the others: it is logged, and one that
/// Resend turned away for its rate limit goes again once. Says how many
/// weren't sent, if any.
pub async fn send_each(cfg: &EmailConfig, messages: &[(String, String, String)]) -> AppResult<()> {
    let mut unsent = 0;
    for (i, chunk) in messages.chunks(BATCH).enumerate() {
        if i > 0 {
            worker::Delay::from(std::time::Duration::from_millis(BATCH_GAP_MS)).await;
        }
        let body = serde_json::Value::Array(
            chunk
                .iter()
                .map(|(to, subject, text)| {
                    serde_json::json!({ "from": cfg.from, "to": [to], "subject": subject, "text": text })
                })
                .collect(),
        );
        let url = "https://api.resend.com/emails/batch";
        let mut status = request(cfg, url, &body).await;
        if matches!(status, Ok(429)) {
            worker::Delay::from(std::time::Duration::from_millis(BATCH_GAP_MS * 2)).await;
            status = request(cfg, url, &body).await;
        }
        let failed = match status {
            Ok(code) if code < 300 => None,
            Ok(code) => Some(format!("HTTP {code}")),
            Err(e) => Some(e.0.message),
        };
        if let Some(why) = failed {
            worker::console_error!(
                "email batch {} of {} people failed: {why}",
                i + 1,
                chunk.len()
            );
            unsent += chunk.len();
        }
    }
    if unsent > 0 {
        return Err(unavailable(&format!(
            "{unsent} of {} emails could not be sent",
            messages.len()
        )));
    }
    Ok(())
}

/// Posts `body` to Resend at `url`, and answers its status.
async fn request(cfg: &EmailConfig, url: &str, body: &serde_json::Value) -> AppResult<u16> {
    let headers = Headers::new();
    headers.set("Authorization", &format!("Bearer {}", cfg.resend_api_key))?;
    headers.set("Content-Type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(body.to_string().into()));
    let req = Request::new_with_init(url, &init)?;
    Ok(Fetch::Request(req).send().await?.status_code())
}

async fn post(cfg: &EmailConfig, url: &str, body: &serde_json::Value) -> AppResult<()> {
    let status = request(cfg, url, body).await?;
    if status >= 300 {
        worker::console_error!("email send failed: HTTP {status}");
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
