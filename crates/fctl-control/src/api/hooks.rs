//! Webhooks (docs/architecture.md#webhooks, ADR 0084):
//! `/api/v1/workspaces/{id}/webhooks` (a workspace's admins: its webhooks
//! and its projects'), `/api/v1/projects/{id}/webhooks` (a project's owners
//! and maintainers: its own, and the workspace's that send its events),
//! `/api/v1/webhooks/{id}…` (one webhook: changing it, its secret, tests,
//! its deliveries and resending them) and `/api/v1/webhook-events` (what
//! webhooks get). The rows are in D1 (`crate::hooks`); what went out to
//! each, in the workspace's HooksObject.

use crate::access;
use crate::ctx::Ctx;
use crate::db::{b, n, os, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, not_found, validation};
use crate::hooks::{self as rows, HookRow};
use crate::http;
use crate::internal::{
    self, HookDeliveries, HookDelivery, HookResendFailed, HookTest, HooksHealth, HooksHealthAnswer,
};
use crate::workspaces as ws;
use fctl_domain::hooks as rules;
use fctl_domain::verify::Sensitive;
use fctl_proto::hooks::{
    CreateWebhookRequest, CreatedWebhook, ResendFailedRequest, ResentFailed, TestWebhookRequest,
    UpdateWebhookRequest, Webhook, WebhookDelivery, WebhookDeliveryList, WebhookList,
    WebhookSecret,
};
use fctl_proto::session::UserRef;
use fctl_proto::{ProjectId, WebhookId};
use serde_json::json;
use std::collections::BTreeMap;
use worker::{Request, Response};

/// `GET /api/v1/webhook-events`: every event webhooks get, in its groups,
/// and where the instance lets them post.
pub async fn events(ctx: &Ctx) -> AppResult<Response> {
    ctx.user()?;
    let mut catalog = rules::catalog();
    catalog.reach = ctx.cfg.webhook_reach.shown();
    http::ok(&catalog)
}

/// A webhook's address, checked against where the instance lets webhooks
/// post; one it refuses names the entries of its settings it couldn't
/// read, which may be why.
fn checked_url(ctx: &Ctx, url: &str) -> AppResult<String> {
    rules::check_url(url, &ctx.cfg.webhook_reach).map_err(|mut e| {
        if !ctx.cfg.webhook_reach_unread.is_empty() {
            e.message = format!(
                "{}. FCTL_WEBHOOK_PRIVATE_HOSTS lists what fctl can't read, which it leaves out: {}",
                e.message.trim_end_matches('.'),
                ctx.cfg.webhook_reach_unread.join(", ")
            );
        }
        AppError(e)
    })
}

/// How some of a workspace's webhooks' deliveries went lately; nothing for
/// those its HooksObject couldn't say, which the list shows without.
async fn health(
    ctx: &Ctx,
    workspace_id: &str,
    ids: Vec<String>,
) -> BTreeMap<String, fctl_proto::hooks::WebhookHealth> {
    if ids.is_empty() {
        return BTreeMap::new();
    }
    let asked: AppResult<HooksHealthAnswer> = async {
        let stub = internal::hooks_stub(&ctx.env, workspace_id)?;
        internal::post(
            &stub,
            "/health",
            &HooksHealth {
                workspace_id: workspace_id.to_owned(),
                webhooks: ids,
            },
        )
        .await
    }
    .await;
    match asked {
        Ok(a) => a.health,
        Err(e) => {
            worker::console_error!("webhooks' health in {workspace_id}: {}", e.0.message);
            BTreeMap::new()
        }
    }
}

async fn listed(ctx: &Ctx, workspace_id: &str, hooks: Vec<(HookRow, bool)>) -> AppResult<Response> {
    let ids = hooks.iter().map(|(h, _)| h.id.clone()).collect();
    let mut health = health(ctx, workspace_id, ids).await;
    let count = rows::count(&ctx.db, workspace_id).await?;
    http::ok(&WebhookList {
        webhooks: hooks
            .into_iter()
            .map(|(h, manage)| {
                let health = health.remove(&h.id);
                h.to_webhook(health, manage)
            })
            .collect(),
        count,
        limit: rules::MAX_WEBHOOKS,
    })
}

/// `GET /api/v1/workspaces/{id}/webhooks`: the workspace's webhooks and
/// its projects', for its admins.
pub async fn workspace_list(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let hooks = rows::of_workspace(&ctx.db, a.id())
        .await?
        .into_iter()
        .map(|h| (h, true))
        .collect();
    listed(ctx, a.id(), hooks).await
}

/// `GET /api/v1/projects/{id}/webhooks`: the project's own webhooks, and
/// the workspace's that send its events, which only its admins manage.
pub async fn project_list(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    p.require_maintainer()?;
    let admin = ws::load(ctx, p.workspace_id())
        .await
        .is_ok_and(|a| a.require_admin().is_ok());
    let hooks = rows::of_workspace(&ctx.db, p.workspace_id())
        .await?
        .into_iter()
        .filter_map(|h| match &h.project_id {
            Some(own) if *own == p.row.id => Some((h, true)),
            Some(_) => None,
            None => {
                let projects = h.projects();
                (projects.is_empty() || projects.contains(&p.row.id)).then_some((h, admin))
            }
        })
        .collect();
    listed(ctx, p.workspace_id(), hooks).await
}

/// What a request to add or change a webhook asks for, checked.
struct Checked {
    url: String,
    name: Option<String>,
    events: Vec<String>,
    projects: Vec<String>,
    started_by: Vec<fctl_proto::hooks::WebhookOrigin>,
}

/// Checks `projects` are the workspace's.
async fn check_projects(
    ctx: &Ctx,
    workspace_id: &str,
    projects: &[ProjectId],
) -> AppResult<Vec<String>> {
    if projects.is_empty() {
        return Ok(Vec::new());
    }
    #[derive(serde::Deserialize)]
    struct Row {
        id: String,
    }
    let theirs: Vec<Row> = q(
        &ctx.db,
        "SELECT id FROM projects WHERE workspace_id = ? ORDER BY created_at",
        &[s(workspace_id)],
    )?
    .all()
    .await?;
    let mut out = Vec::new();
    for p in projects {
        if !theirs.iter().any(|t| t.id == p.0) {
            return Err(validation(
                "projects",
                &format!("{} isn't one of the workspace's projects", p.0),
            ));
        }
        if !out.contains(&p.0) {
            out.push(p.0.clone());
        }
    }
    Ok(out)
}

/// Adds a webhook to `workspace_id`, of project `project_id` or the
/// workspace's own, and answers it with its secret, once.
async fn create(
    ctx: &Ctx,
    req: &mut Request,
    workspace_id: &str,
    project_id: Option<&str>,
) -> AppResult<Response> {
    let body: CreateWebhookRequest = http::body(req).await?;
    let c = Checked {
        url: checked_url(ctx, &body.url)?,
        name: rules::check_name(body.name.as_deref()).map_err(AppError)?,
        events: rules::check_events(&body.events).map_err(AppError)?,
        projects: match project_id {
            Some(_) if !body.projects.is_empty() => {
                return Err(validation(
                    "projects",
                    "A project's webhook gets its own project's events: leave projects out",
                ));
            }
            Some(_) => Vec::new(),
            None => check_projects(ctx, workspace_id, &body.projects).await?,
        },
        started_by: rules::check_started_by(&body.started_by).map_err(AppError)?,
    };
    if rows::count(&ctx.db, workspace_id).await? >= rules::MAX_WEBHOOKS {
        return Err(conflict(&format!(
            "The workspace has {} webhooks, the most it may have: delete one first",
            rules::MAX_WEBHOOKS
        )));
    }
    ctx.verified(Sensitive::AddWebhook)?;
    let keys = ctx.keys()?;
    let id = fctl_domain::ids::new_id(WebhookId::PREFIX, ctx.now);
    let secret = rules::secret(&fctl_domain::random_bytes::<32>());
    let sealed = rows::seal_secret(&keys, &id, &secret)?;
    // The address takes fctl's requests before the webhook exists.
    rows::challenge(
        &ctx.env,
        workspace_id,
        project_id,
        &id,
        &c.url,
        Some(sealed.clone()),
    )
    .await?;
    let user = ctx.user()?.id.clone();
    let enabled = body.enabled.unwrap_or(true);
    q(
        &ctx.db,
        "INSERT INTO webhooks (id, workspace_id, project_id, name, url, events, projects, started_by, secret, \
         enabled, created_by, keeper_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            s(&id),
            s(workspace_id),
            os(project_id),
            os(c.name.as_deref()),
            s(&c.url),
            s(&json!(c.events).to_string()),
            s(&json!(c.projects).to_string()),
            s(&json!(c.started_by).to_string()),
            s(&sealed),
            b(enabled),
            s(&user),
            // It posts while they manage it (`crate::hooks::KEPT`).
            s(&user),
            n(ctx.now),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    rows::changed(&ctx.env, workspace_id, None, false).await;
    ctx.audit_in(
        Some(workspace_id),
        "webhook.created",
        Some(&id),
        Some(json!({
            "url": c.url,
            "project": project_id,
            "events": c.events,
            "projects": c.projects,
        })),
    );
    let row = rows::row(&ctx.db, &id)
        .await?
        .ok_or_else(|| crate::error::internal("the webhook vanished"))?;
    http::created(&CreatedWebhook {
        webhook: row.to_webhook(None, true),
        secret,
    })
}

/// `POST /api/v1/workspaces/{id}/webhooks`: one of the workspace's, for
/// all its projects or some (its admins).
pub async fn workspace_create(ctx: &Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    create(ctx, req, a.id(), None).await
}

/// `POST /api/v1/projects/{id}/webhooks`: one of the project's own (its
/// owners and maintainers).
pub async fn project_create(ctx: &Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    p.require_maintainer()?;
    let workspace = p.workspace_id().to_owned();
    create(ctx, req, &workspace, Some(&p.row.id)).await
}

/// Webhook `id`, where the caller manages it: a workspace's for its
/// admins, a project's for its owners and maintainers. Anyone else is told
/// it doesn't exist.
async fn managed(ctx: &Ctx, id: &str) -> AppResult<HookRow> {
    let row = rows::row(&ctx.db, id)
        .await?
        .ok_or_else(|| not_found("Webhook"))?;
    match &row.project_id {
        Some(project) => {
            let p = access::project(ctx, project)
                .await
                .map_err(|_| not_found("Webhook"))?;
            if p.require_maintainer().is_err() {
                return Err(not_found("Webhook"));
            }
        }
        None => {
            let a = ws::load(ctx, &row.workspace_id)
                .await
                .map_err(|_| not_found("Webhook"))?;
            if a.require_open().is_err() {
                return Err(not_found("Webhook"));
            }
            if a.require_admin().is_err() {
                return Err(forbidden("Only the workspace's admins manage its webhooks"));
            }
        }
    }
    Ok(row)
}

async fn shown(ctx: &Ctx, id: &str) -> AppResult<Webhook> {
    let row = rows::row(&ctx.db, id)
        .await?
        .ok_or_else(|| not_found("Webhook"))?;
    let mut health = health(ctx, &row.workspace_id, vec![row.id.clone()]).await;
    Ok(row.to_webhook(health.remove(&row.id), true))
}

/// `GET /api/v1/webhooks/{id}`.
pub async fn get(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let row = managed(ctx, id).await?;
    http::ok(&shown(ctx, &row.id).await?)
}

/// `PATCH /api/v1/webhooks/{id}`. Pointing it elsewhere takes confirming
/// it's you, as adding one does; turning it on again clears why fctl
/// turned it off.
pub async fn update(ctx: &Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let row = managed(ctx, id).await?;
    let body: UpdateWebhookRequest = http::body(req).await?;
    let url = match &body.url {
        Some(u) => Some(checked_url(ctx, u)?),
        None => None,
    };
    let moved = url.as_ref().is_some_and(|u| *u != row.url);
    let name = match &body.name {
        Some(nm) => Some(rules::check_name(Some(nm)).map_err(AppError)?),
        None => None,
    };
    let events = match &body.events {
        Some(e) => Some(rules::check_events(e).map_err(AppError)?),
        None => None,
    };
    let projects = match &body.projects {
        Some(_) if row.project_id.is_some() => {
            return Err(validation(
                "projects",
                "A project's webhook gets its own project's events",
            ));
        }
        Some(p) => Some(check_projects(ctx, &row.workspace_id, p).await?),
        None => None,
    };
    let started_by = match &body.started_by {
        Some(o) => Some(rules::check_started_by(o).map_err(AppError)?),
        None => None,
    };
    if let Some(u) = url.as_deref().filter(|_| moved) {
        ctx.verified(Sensitive::AddWebhook)?;
        // The new address takes fctl's requests before the webhook points
        // there; until then, it posts where it did.
        rows::challenge(
            &ctx.env,
            &row.workspace_id,
            row.project_id.as_deref(),
            &row.id,
            u,
            None,
        )
        .await?;
    }
    let enabled = body.enabled.unwrap_or(row.enabled);
    let turned_on = enabled && !row.enabled;
    // Whoever points it elsewhere, or turns it on again, chooses where it
    // posts: it posts while they manage it (`crate::hooks::KEPT`).
    let keeper = if moved || turned_on {
        Some(ctx.user()?.id.clone())
    } else {
        None
    };
    q(
        &ctx.db,
        "UPDATE webhooks SET url = ?, name = ?, events = ?, projects = ?, started_by = ?, enabled = ?, \
         disabled_reason = CASE WHEN ? THEN NULL ELSE disabled_reason END, \
         disabled_at = CASE WHEN ? THEN NULL ELSE disabled_at END, updated_at = ?, \
         keeper_id = COALESCE(?, keeper_id) WHERE id = ?",
        &[
            s(url.as_deref().unwrap_or(&row.url)),
            match &name {
                Some(nm) => os(nm.as_deref()),
                None => os(row.name.as_deref()),
            },
            s(&events.as_ref().map_or_else(|| row.events.clone(), |e| json!(e).to_string())),
            s(&projects.as_ref().map_or_else(|| row.projects.clone(), |p| json!(p).to_string())),
            s(&started_by
                .as_ref()
                .map_or_else(|| row.started_by.clone(), |o| json!(o).to_string())),
            b(enabled),
            b(enabled),
            b(enabled),
            n(ctx.now),
            os(keeper.as_deref()),
            s(&row.id),
        ],
    )?
    .run()
    .await?;
    // Turned on again, or pointed elsewhere: it fails afresh from now.
    if turned_on || moved {
        rows::afresh(&ctx.env, &row.workspace_id, &row.id).await;
    } else {
        rows::changed(&ctx.env, &row.workspace_id, None, false).await;
    }
    let mut changes = serde_json::Map::new();
    if moved {
        changes.insert("url".into(), json!(url));
    }
    if let Some(e) = &events {
        changes.insert("events".into(), json!(e));
    }
    if let Some(p) = &projects {
        changes.insert("projects".into(), json!(p));
    }
    if let Some(o) = &started_by {
        changes.insert("started_by".into(), json!(o));
    }
    if enabled != row.enabled {
        changes.insert("enabled".into(), json!(enabled));
    }
    if turned_on {
        changes.insert("was_off_because".into(), json!(row.disabled_reason));
    }
    ctx.audit_in(
        Some(&row.workspace_id),
        "webhook.updated",
        Some(&row.id),
        Some(serde_json::Value::Object(changes)),
    );
    http::ok(&shown(ctx, &row.id).await?)
}

/// `DELETE /api/v1/webhooks/{id}`: its deliveries go with it.
pub async fn delete(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let row = managed(ctx, id).await?;
    q(&ctx.db, "DELETE FROM webhooks WHERE id = ?", &[s(&row.id)])?
        .run()
        .await?;
    rows::changed(&ctx.env, &row.workspace_id, Some(&row.id), true).await;
    ctx.audit_in(
        Some(&row.workspace_id),
        "webhook.deleted",
        Some(&row.id),
        Some(json!({ "url": row.url, "project": row.project_id })),
    );
    http::ok(&json!({ "ok": true }))
}

/// `GET /api/v1/webhooks/{id}/secret`: its signing secret, which takes
/// confirming it's you.
pub async fn secret(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let row = managed(ctx, id).await?;
    ctx.verified(Sensitive::AddWebhook)?;
    let secret = row.secret(&ctx.keys()?)?;
    ctx.audit_in(
        Some(&row.workspace_id),
        "webhook.secret_read",
        Some(&row.id),
        None,
    );
    http::ok(&WebhookSecret {
        secret,
        previous_until: row
            .previous_until
            .filter(|u| row.previous_secret.is_some() && *u > ctx.now),
    })
}

/// `POST /api/v1/webhooks/{id}/secret`: a new secret, which signs from
/// now on; the one before signs too for a day, so that the receiver can
/// switch without missing a delivery. It answers with the secret, so it
/// takes confirming it's you, as reading one does.
pub async fn rotate(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let row = managed(ctx, id).await?;
    // Its answer is the new secret: it takes what reading one takes.
    ctx.verified(Sensitive::AddWebhook)?;
    let keys = ctx.keys()?;
    let secret = rules::secret(&fctl_domain::random_bytes::<32>());
    let until = ctx.now + rules::SECRET_OVERLAP_MS;
    q(
        &ctx.db,
        "UPDATE webhooks SET previous_secret = secret, previous_until = ?, secret = ?, updated_at = ? WHERE id = ?",
        &[
            n(until),
            s(&rows::seal_secret(&keys, &row.id, &secret)?),
            n(ctx.now),
            s(&row.id),
        ],
    )?
    .run()
    .await?;
    rows::changed(&ctx.env, &row.workspace_id, None, false).await;
    ctx.audit_in(
        Some(&row.workspace_id),
        "webhook.secret_rotated",
        Some(&row.id),
        None,
    );
    http::ok(&WebhookSecret {
        secret,
        previous_until: Some(until),
    })
}

fn caller(ctx: &Ctx) -> AppResult<UserRef> {
    let u = ctx.user()?;
    Ok(UserRef {
        id: fctl_proto::UserId(u.id.clone()),
        name: u.name.clone(),
        avatar_url: u.avatar_url.clone(),
    })
}

/// `POST /api/v1/webhooks/{id}/test`: posts a sample of an event now, and
/// answers how it went.
pub async fn test(ctx: &Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let row = managed(ctx, id).await?;
    let body: TestWebhookRequest = http::body(req).await?;
    if rules::Kind::parse(body.event.trim()).is_none() {
        return Err(validation(
            "event",
            &format!("fctl has no event called {}", body.event.trim()),
        ));
    }
    let stub = internal::hooks_stub(&ctx.env, &row.workspace_id)?;
    let delivery: WebhookDelivery = internal::post(
        &stub,
        "/test",
        &HookTest {
            workspace_id: row.workspace_id.clone(),
            webhook_id: row.id.clone(),
            kind: body.event.trim().to_owned(),
            by: caller(ctx)?,
        },
    )
    .await?;
    http::ok(&delivery)
}

/// `GET /api/v1/webhooks/{id}/deliveries`: newest first; `?state=`
/// (pending, delivered, failed), `?event=` and `?before=` narrow it down.
pub async fn deliveries(ctx: &Ctx, req: &Request, id: &str) -> AppResult<Response> {
    let row = managed(ctx, id).await?;
    let state = http::query(req, "state")
        .filter(|v| matches!(v.as_str(), "pending" | "delivered" | "failed"));
    let kind = http::query(req, "event").filter(|k| rules::Kind::parse(k).is_some());
    let limit = http::query(req, "limit")
        .and_then(|l| l.parse::<u32>().ok())
        .unwrap_or(50)
        .clamp(1, 100);
    let stub = internal::hooks_stub(&ctx.env, &row.workspace_id)?;
    let list: WebhookDeliveryList = internal::post(
        &stub,
        "/deliveries",
        &HookDeliveries {
            workspace_id: row.workspace_id.clone(),
            webhook_id: row.id.clone(),
            state,
            kind,
            before: http::query(req, "before"),
            limit,
        },
    )
    .await?;
    http::ok(&list)
}

/// `GET /api/v1/webhooks/{id}/deliveries/{delivery}`: the request as it was
/// posted, every attempt and the start of the last answer.
pub async fn delivery(ctx: &Ctx, id: &str, delivery: &str) -> AppResult<Response> {
    let row = managed(ctx, id).await?;
    let stub = internal::hooks_stub(&ctx.env, &row.workspace_id)?;
    let d: WebhookDelivery = internal::post(
        &stub,
        "/delivery",
        &HookDelivery {
            workspace_id: row.workspace_id.clone(),
            webhook_id: row.id.clone(),
            delivery_id: delivery.to_owned(),
            by: None,
        },
    )
    .await?;
    http::ok(&d)
}

/// `POST /api/v1/webhooks/{id}/deliveries/{delivery}/resend`: posts it now,
/// as it was, and answers how it went.
pub async fn resend(ctx: &Ctx, id: &str, delivery: &str) -> AppResult<Response> {
    let row = managed(ctx, id).await?;
    let stub = internal::hooks_stub(&ctx.env, &row.workspace_id)?;
    let d: WebhookDelivery = internal::post(
        &stub,
        "/resend",
        &HookDelivery {
            workspace_id: row.workspace_id.clone(),
            webhook_id: row.id.clone(),
            delivery_id: delivery.to_owned(),
            by: Some(caller(ctx)?),
        },
    )
    .await?;
    ctx.audit_in(
        Some(&row.workspace_id),
        "webhook.resent",
        Some(&row.id),
        Some(json!({ "delivery": delivery, "event": d.kind })),
    );
    http::ok(&d)
}

/// `POST /api/v1/webhooks/{id}/deliveries/resend-failed`: the deliveries
/// that failed since a time (the last day by default) wait for an attempt
/// again.
pub async fn resend_failed(ctx: &Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let row = managed(ctx, id).await?;
    let body: ResendFailedRequest = http::body(req).await.unwrap_or_default();
    let since = body
        .since
        .unwrap_or(ctx.now - 24 * 3_600_000)
        .max(ctx.now - rules::KEPT_MS);
    let stub = internal::hooks_stub(&ctx.env, &row.workspace_id)?;
    let r: ResentFailed = internal::post(
        &stub,
        "/resend-failed",
        &HookResendFailed {
            workspace_id: row.workspace_id.clone(),
            webhook_id: row.id.clone(),
            since,
        },
    )
    .await?;
    ctx.audit_in(
        Some(&row.workspace_id),
        "webhook.resent_failed",
        Some(&row.id),
        Some(json!({ "since": since, "queued": r.queued })),
    );
    http::ok(&r)
}
