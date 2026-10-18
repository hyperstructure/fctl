//! `/api/v1/me/notifications`: what notifies you, and the browsers that show
//! your notifications (Web Push subscriptions). Following sessions is in
//! `sessions.rs`.

use crate::ctx::Ctx;
use crate::db::{Count, n, os, q, s};
use crate::error::{AppResult, conflict, not_found};
use crate::http;
use crate::{notify, push};
use fctl_domain::{validate, webpush};
use fctl_proto::PushDeviceId;
use fctl_proto::api::*;
use serde::Deserialize;
use worker::{Request, Response};

#[derive(Deserialize)]
struct DeviceRow {
    id: String,
    token_id: Option<String>,
    endpoint: String,
    name: Option<String>,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    last_sent_at: Option<i64>,
}

const DEVICE_SELECT: &str =
    "SELECT id, token_id, endpoint, name, created_at, last_sent_at FROM push_subscriptions";

impl DeviceRow {
    /// `token_id`: the sign-in of the request, to tell which browser it is.
    fn device(&self, token_id: Option<&str>) -> PushDevice {
        PushDevice {
            id: PushDeviceId(self.id.clone()),
            name: self.name.clone(),
            service: webpush::service(&self.endpoint),
            created_at: self.created_at,
            last_sent_at: self.last_sent_at,
            current: token_id.is_some() && self.token_id.as_deref() == token_id,
        }
    }
}

fn token_id(ctx: &Ctx) -> Option<&str> {
    ctx.auth.as_ref().map(|a| a.token_id.as_str())
}

async fn device_count(ctx: &Ctx, user_id: &str) -> AppResult<i64> {
    let c: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM push_subscriptions WHERE user_id = ?",
        &[s(user_id)],
    )?
    .first()
    .await?;
    Ok(c.map_or(0, |c| c.n))
}

async fn notifications(ctx: &Ctx) -> AppResult<Notifications> {
    let user = ctx.user()?;
    let rows: Vec<DeviceRow> = q(
        &ctx.db,
        &format!("{DEVICE_SELECT} WHERE user_id = ? ORDER BY created_at"),
        &[s(&user.id)],
    )?
    .all()
    .await?;
    Ok(Notifications {
        settings: notify::settings(&ctx.db, &user.id).await?,
        devices: rows.iter().map(|r| r.device(token_id(ctx))).collect(),
        public_key: push::public_key(&ctx.db, &ctx.cfg).await?,
    })
}

/// `GET /me/notifications`.
pub async fn get(ctx: &Ctx) -> AppResult<Response> {
    http::ok(&notifications(ctx).await?)
}

/// `PATCH /me/notifications`: changes the settings that are sent.
pub async fn update(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: UpdateNotificationsRequest = http::body(req).await?;
    let user_id = ctx.user()?.id.clone();
    let mut settings = notify::settings(&ctx.db, &user_id).await?;
    for (field, value) in [
        (&mut settings.waiting, body.waiting),
        (&mut settings.paused, body.paused),
        (&mut settings.failed, body.failed),
        (&mut settings.follow_started, body.follow_started),
    ] {
        if let Some(v) = value {
            *field = v;
        }
    }
    notify::save_settings(&ctx.db, &user_id, &settings, ctx.now).await?;
    get(ctx).await
}

/// `POST /me/notifications/devices`: turns notifications on in a browser
/// (its push subscription). The same endpoint again updates it, and moves
/// it to whoever is signed in there now.
pub async fn add_device(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: AddPushDeviceRequest = http::body(req).await?;
    let user_id = ctx.user()?.id.clone();
    let endpoint = validate::push_endpoint(&body.endpoint, ctx.cfg.dev)?;
    let (p256dh, auth) = validate::push_keys(&body.keys.p256dh, &body.keys.auth)?;
    let name = match body.name.as_deref() {
        Some(name) => validate::device_name(name)?,
        None => None,
    };
    let others: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM push_subscriptions WHERE user_id = ? AND endpoint != ?",
        &[s(&user_id), s(&endpoint)],
    )?
    .first()
    .await?;
    if others.map_or(0, |c| c.n) >= push::MAX_DEVICES {
        return Err(conflict(&format!(
            "Notifications are on in {} browsers already. Remove one in Settings first.",
            push::MAX_DEVICES
        )));
    }
    let id = fctl_domain::ids::push_device_id(ctx.now);
    q(
        &ctx.db,
        "INSERT INTO push_subscriptions (id, user_id, token_id, endpoint, p256dh, auth, name, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (endpoint) DO UPDATE SET user_id = excluded.user_id, \
         token_id = excluded.token_id, p256dh = excluded.p256dh, auth = excluded.auth, name = excluded.name",
        &[
            s(id.as_str()),
            s(&user_id),
            os(token_id(ctx)),
            s(&endpoint),
            s(&p256dh),
            s(&auth),
            os(name.as_deref()),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    let row: Option<DeviceRow> = q(
        &ctx.db,
        &format!("{DEVICE_SELECT} WHERE endpoint = ?"),
        &[s(&endpoint)],
    )?
    .first()
    .await?;
    let row = row.ok_or_else(|| not_found("Browser"))?;
    http::ok(&row.device(token_id(ctx)))
}

/// `DELETE /me/notifications/devices/{id}`: turns notifications off for a
/// browser.
pub async fn remove_device(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let user_id = ctx.user()?.id.clone();
    let changed = q(
        &ctx.db,
        "DELETE FROM push_subscriptions WHERE id = ? AND user_id = ?",
        &[s(id), s(&user_id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(not_found("Browser"));
    }
    get(ctx).await
}

/// `POST /me/notifications/test`: a notification to every browser of yours.
pub async fn test(ctx: &mut Ctx) -> AppResult<Response> {
    let user_id = ctx.user()?.id.clone();
    ctx.rate_limit(&format!("push-test:{user_id}")).await?;
    if device_count(ctx, &user_id).await? == 0 {
        return Err(conflict(
            "Notifications are not on in any browser. Turn them on in Settings → Notifications.",
        ));
    }
    let message = fctl_domain::notify::test_message(ctx.now);
    http::ok(&push::deliver(&ctx.cfg, &ctx.db, &user_id, &message).await?)
}
