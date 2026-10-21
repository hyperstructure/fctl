//! The GitHub App's webhook, `POST /api/v1/github/webhook`: GitHub says
//! that pull requests or their checks changed, and the sessions that list
//! them hear what (`crate::pull_requests`). GitHub signs each delivery with
//! the webhook's secret (`GITHUB_WEBHOOK_SECRET`); without the secret the
//! instance takes none.

use crate::ctx::Ctx;
use crate::error::{AppError, AppResult, not_configured};
use crate::{http, pull_requests};
use fctl_domain::webhooks::{self, Delivery};
use fctl_proto::{ApiError, ErrorCode};
use serde_json::{Value, json};
use worker::{Request, Response};

/// The largest delivery read. GitHub caps them at 25 MB; those about pull
/// requests and checks are a few dozen KB.
const MAX_DELIVERY_BYTES: usize = 5 * 1024 * 1024;

pub async fn receive(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let Some(secret) = ctx
        .cfg
        .github
        .as_ref()
        .and_then(|g| g.webhook_secret.clone())
    else {
        return Err(not_configured(
            "This instance takes no GitHub webhooks: set GITHUB_WEBHOOK_SECRET",
        ));
    };
    let body = req.bytes().await?;
    if body.len() > MAX_DELIVERY_BYTES {
        return Err(AppError(ApiError::bad_request("The delivery is too large")));
    }
    let signature = http::header(req, "X-Hub-Signature-256").unwrap_or_default();
    if !webhooks::signed(&secret, &body, &signature) {
        return Err(AppError(ApiError::new(
            ErrorCode::Unauthorized,
            "The delivery isn't signed with the webhook's secret",
        )));
    }
    let event = http::header(req, "X-GitHub-Event").unwrap_or_default();
    let delivery = http::header(req, "X-GitHub-Delivery").unwrap_or_default();
    let Some(payload) = payload(&body) else {
        return Err(AppError(ApiError::bad_request("The delivery is not JSON")));
    };
    let told = match webhooks::delivery(&event, &payload) {
        Delivery::Ping => return http::ok(&json!({ "ok": true })),
        Delivery::Other => 0,
        Delivery::Hook(hook) => {
            let told = pull_requests::hook(&ctx.env, &ctx.cfg, &ctx.db, &hook, ctx.now).await?;
            if told > 0 {
                worker::console_log!(
                    "webhook {delivery} ({event}, {}): told {told} sessions",
                    hook.repo.full_name()
                );
            }
            told
        }
    };
    http::ok(&json!({ "ok": true, "sessions": told }))
}

/// The delivery's JSON: its body, or the `payload` of a form, as GitHub
/// sends it to webhooks whose content type is
/// `application/x-www-form-urlencoded`.
fn payload(body: &[u8]) -> Option<Value> {
    if let Ok(v) = serde_json::from_slice(body) {
        return Some(v);
    }
    let (_, json) = url::form_urlencoded::parse(body).find(|(k, _)| k == "payload")?;
    serde_json::from_str(&json).ok()
}
