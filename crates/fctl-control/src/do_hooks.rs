//! HooksObject: one Durable Object per workspace, which posts its webhooks'
//! deliveries (docs/architecture.md#webhooks, ADR 0083).
//!
//! What happens in the workspace comes to it as events (`/emit`), only
//! when one of its webhooks wants it (`crate::hooks::emit`). It keeps each
//! event's body once, and a delivery of it for each webhook that wants it,
//! and posts them from its alarm: signed, with a timeout and no redirects,
//! at most `PER_MINUTE` a minute to a webhook, trying a failed one again
//! after each of `RETRIES_MS`. A webhook all of whose attempts failed for
//! three days turns itself off. People read the deliveries, resend them and
//! send tests through the API (`api::hooks`), which asks it here. It forgets
//! deliveries after 30 days, and all but each webhook's newest 1,000, so
//! that a busy webhook's log stays within bounds; nothing here is in D1.

use crate::config::Config;
use crate::ctx::now_ms;
use crate::error::AppResult;
use crate::hooks::{self as hook_rows, HookRow};
use crate::internal::{
    self, HookDeliveries, HookDelivery, HookEmit, HookResendFailed, HookTest, HooksChanged,
    HooksHealth, HooksHealthAnswer,
};
use fctl_domain::hooks::{self as rules, By, Headers, Kind};
use fctl_proto::hooks::{
    HookPlace, ResentFailed, WebhookAttempt, WebhookDelivery, WebhookDeliveryList,
    WebhookDeliveryState, WebhookHeader, WebhookHealth, WebhookRequest, WebhookResponse,
};
use fctl_proto::session::UserRef;
use fctl_proto::{
    ApiError, ErrorCode, ProjectId, SessionId, WebhookDeliveryId, WebhookEventId, WebhookId,
};
use futures::StreamExt;
use futures::future::join_all;
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use worker::{
    DurableObject, Env, Fetch, Method, Request, RequestInit, RequestRedirect, Response, Result,
    SqlStorage, SqlStorageValue, State, durable_object,
};

/// How long the object goes by the webhooks and the names it read from D1,
/// besides being told they changed.
const READ_FOR_MS: i64 = 60_000;
/// Deliveries one run of the alarm posts, at most (a run's subrequests are
/// limited), and to one webhook, each after the last.
const PER_RUN: usize = 40;
const PER_RUN_EACH: usize = 10;
/// How often the log is trimmed.
const TRIM_EVERY_MS: i64 = 3_600_000;
/// How long the alarm waits to try again when D1 couldn't be read.
const D1_AGAIN_MS: i64 = 30_000;
/// How long a webhook waits between two "resend the failed ones".
const RESEND_FAILED_EVERY_MS: i64 = 10 * 60_000;
/// The longest an event's body may be: what fctl's own events carry stays
/// far below it.
const BODY_MAX: usize = 256 * 1024;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

fn sv(s: &str) -> SqlStorageValue {
    SqlStorageValue::from(s)
}

fn iv(n: i64) -> SqlStorageValue {
    SqlStorageValue::from(n)
}

fn ov(s: Option<&str>) -> SqlStorageValue {
    s.map_or(SqlStorageValue::Null, sv)
}

fn onv(n: Option<i64>) -> SqlStorageValue {
    n.map_or(SqlStorageValue::Null, iv)
}

fn create_schema(sql: &SqlStorage) {
    for stmt in [
        // Each event once: its body, as every delivery of it posts it.
        "CREATE TABLE IF NOT EXISTS events (id TEXT PRIMARY KEY, kind TEXT NOT NULL, project_id TEXT, \
         session_id TEXT, gist TEXT NOT NULL, body TEXT NOT NULL, at INTEGER NOT NULL)",
        // Each event on its way to each webhook: pending, delivered or
        // failed, its attempts (the newest with the start of its answer),
        // and the headers and address it was last posted with.
        "CREATE TABLE IF NOT EXISTS deliveries (id TEXT PRIMARY KEY, event_id TEXT NOT NULL, \
         webhook_id TEXT NOT NULL, state TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, \
         next_at INTEGER, created_at INTEGER NOT NULL, done_at INTEGER, test INTEGER NOT NULL DEFAULT 0, \
         resends INTEGER NOT NULL DEFAULT 0, reason TEXT, last TEXT, history TEXT, headers TEXT, url TEXT)",
        "CREATE INDEX IF NOT EXISTS deliveries_due ON deliveries (next_at) WHERE state = 'pending'",
        "CREATE INDEX IF NOT EXISTS deliveries_webhook ON deliveries (webhook_id, id)",
        "CREATE INDEX IF NOT EXISTS deliveries_event ON deliveries (event_id)",
        // Each attempt in the last hour, for the limits.
        "CREATE TABLE IF NOT EXISTS sends (webhook_id TEXT NOT NULL, at INTEGER NOT NULL, manual INTEGER NOT NULL DEFAULT 0)",
        "CREATE INDEX IF NOT EXISTS sends_webhook ON sends (webhook_id, at)",
        // How each webhook stands: since when its attempts fail, the events
        // dropped for a full queue, and how its newest attempt went (not its
        // answer, nor who made it).
        "CREATE TABLE IF NOT EXISTS health (webhook_id TEXT PRIMARY KEY, failing_since INTEGER, \
         dropped INTEGER NOT NULL DEFAULT 0, last TEXT)",
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    ] {
        if let Err(e) = sql.exec(stmt, None) {
            worker::console_error!("hooks schema: {e}");
        }
    }
}

/// A workspace and its projects, as posts name them, with each project's
/// primary repository (for tests' samples).
struct Places {
    workspace: HookPlace,
    projects: Vec<(HookPlace, Option<String>)>,
}

impl Places {
    fn project(&self, id: &str) -> Option<&(HookPlace, Option<String>)> {
        self.projects.iter().find(|(p, _)| p.id == id)
    }
}

#[derive(Default)]
struct Mem {
    hooks: Option<(i64, Rc<Vec<HookRow>>)>,
    places: Option<(i64, Rc<Places>)>,
}

#[durable_object]
pub struct HooksObject {
    state: State,
    env: Env,
    sql: SqlStorage,
    mem: RefCell<Mem>,
}

#[derive(Deserialize)]
struct DeliveryRow {
    id: String,
    event_id: String,
    webhook_id: String,
    state: String,
    attempts: f64,
    next_at: Option<f64>,
    created_at: f64,
    test: f64,
    resends: f64,
    reason: Option<String>,
    last: Option<String>,
    history: Option<String>,
    headers: Option<String>,
    url: Option<String>,
    kind: String,
    gist: String,
    project_id: Option<String>,
    session_id: Option<String>,
}

const DELIVERY_SELECT: &str = "SELECT d.id, d.event_id, d.webhook_id, d.state, d.attempts, d.next_at, \
    d.created_at, d.test, d.resends, d.reason, d.last, d.history, d.headers, d.url, e.kind, e.gist, \
    e.project_id, e.session_id FROM deliveries d JOIN events e ON e.id = d.event_id";

fn int(f: f64) -> i64 {
    f as i64
}

fn count(f: f64) -> u32 {
    u32::try_from(int(f).max(0)).unwrap_or(u32::MAX)
}

impl DeliveryRow {
    fn state(&self) -> WebhookDeliveryState {
        WebhookDeliveryState::parse(&self.state)
    }

    fn last(&self) -> Option<WebhookAttempt> {
        self.last
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
    }

    fn history(&self) -> Vec<WebhookAttempt> {
        self.history
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default()
    }

    /// The API's delivery: a list's, or, with `body`, one's in full.
    fn to_delivery(&self, body: Option<String>) -> WebhookDelivery {
        let mut last = self.last();
        // A delivery given up without an attempt says why in its place.
        if last.is_none()
            && let Some(reason) = &self.reason
        {
            last = Some(WebhookAttempt {
                at: int(self.created_at),
                error: Some(reason.clone()),
                ..Default::default()
            });
        }
        let full = body.is_some();
        WebhookDelivery {
            id: WebhookDeliveryId(self.id.clone()),
            webhook_id: WebhookId(self.webhook_id.clone()),
            event_id: WebhookEventId(self.event_id.clone()),
            kind: self.kind.clone(),
            gist: self.gist.clone(),
            project_id: self.project_id.clone().map(ProjectId),
            session_id: self.session_id.clone().map(SessionId),
            created_at: int(self.created_at),
            state: self.state(),
            attempts: count(self.attempts),
            next_at: self.next_at.map(int),
            last: if full {
                last
            } else {
                last.map(|a| WebhookAttempt {
                    response: None,
                    ..a
                })
            },
            test: self.test > 0.0,
            resends: count(self.resends),
            request: body.map(|body| WebhookRequest {
                url: self.url.clone().unwrap_or_default(),
                headers: self
                    .headers
                    .as_deref()
                    .and_then(|j| serde_json::from_str(j).ok())
                    .unwrap_or_default(),
                body,
            }),
            history: if full { self.history() } else { Vec::new() },
        }
    }
}

#[derive(Deserialize)]
struct BodyRow {
    body: String,
}

#[derive(Deserialize)]
struct MetaRow {
    value: String,
}

#[derive(Deserialize)]
struct N {
    n: Option<f64>,
}

#[derive(Deserialize)]
struct HealthRow {
    failing_since: Option<f64>,
    dropped: f64,
    last: Option<String>,
}

#[derive(Deserialize)]
struct Counts {
    delivered: Option<f64>,
    failed: Option<f64>,
    pending: Option<f64>,
}

#[derive(Deserialize)]
struct DueRow {
    id: String,
    webhook_id: String,
    next_at: f64,
}

/// How an attempt went.
struct Posted {
    status: Option<u16>,
    error: Option<String>,
    response: Option<WebhookResponse>,
    ms: u32,
    retry_after: Option<i64>,
}

fn err(code: ErrorCode, message: impl Into<String>) -> ApiError {
    ApiError::new(code, message.into())
}

impl DurableObject for HooksObject {
    fn new(state: State, env: Env) -> Self {
        let sql = state.storage().sql();
        create_schema(&sql);
        Self {
            state,
            env,
            sql,
            mem: RefCell::new(Mem::default()),
        }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        if req.method() != Method::Post {
            return internal::do_error(&ApiError::not_found("Hooks endpoint"));
        }
        macro_rules! body {
            ($t:ty) => {
                match req.json::<$t>().await {
                    Ok(b) => b,
                    Err(e) => return internal::do_error(&err(ErrorCode::BadRequest, e.to_string())),
                }
            };
        }
        let result: std::result::Result<serde_json::Value, ApiError> = match path.as_str() {
            "/emit" => {
                let b = body!(HookEmit);
                self.emit(b).await
            }
            "/changed" => {
                let b = body!(HooksChanged);
                self.changed(&b.workspace_id);
                if let Some(id) = b.fresh {
                    self.exec(
                        "UPDATE health SET failing_since = NULL, dropped = 0 WHERE webhook_id = ?",
                        vec![sv(&id)],
                    );
                }
                Ok(serde_json::json!({ "ok": true }))
            }
            "/forget" => {
                let b = body!(HooksChanged);
                self.changed(&b.workspace_id);
                if let Some(id) = b.webhook_id {
                    self.forget(&id);
                }
                Ok(serde_json::json!({ "ok": true }))
            }
            "/forget-project" => {
                let b = body!(HooksChanged);
                self.changed(&b.workspace_id);
                for id in &b.webhooks {
                    self.forget(id);
                }
                if let Some(project) = b.project_id {
                    self.forget_project(&project);
                }
                Ok(serde_json::json!({ "ok": true }))
            }
            "/health" => {
                let b = body!(HooksHealth);
                Ok(serde_json::json!(self.health(&b.webhooks)))
            }
            "/deliveries" => {
                let b = body!(HookDeliveries);
                Ok(serde_json::json!(self.deliveries(&b)))
            }
            "/delivery" => {
                let b = body!(HookDelivery);
                self.delivery(&b.webhook_id, &b.delivery_id)
                    .map(|d| serde_json::json!(d))
            }
            "/resend" => {
                let b = body!(HookDelivery);
                self.resend(b).await.map(|d| serde_json::json!(d))
            }
            "/test" => {
                let b = body!(HookTest);
                self.test(b).await.map(|d| serde_json::json!(d))
            }
            "/resend-failed" => {
                let b = body!(HookResendFailed);
                self.resend_failed(&b).await.map(|r| serde_json::json!(r))
            }
            "/purge" => {
                let storage = self.state.storage();
                let _ = storage.delete_alarm().await;
                let _ = storage.delete_all().await;
                Ok(serde_json::json!({ "ok": true }))
            }
            _ => Err(ApiError::not_found("Hooks endpoint")),
        };
        match result {
            Ok(v) => internal::do_json(&v),
            Err(e) => internal::do_error(&e),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        let now = now_ms();
        self.trim(now);
        if let Some(workspace) = self.meta("workspace_id")
            && !self.post_due(&workspace, now).await
        {
            // D1 didn't say what the webhooks are: try again in a while,
            // rather than every second while it can't.
            let _ = self.state.storage().set_alarm(D1_AGAIN_MS).await;
            return Response::ok("later");
        }
        self.rearm(now_ms()).await;
        Response::ok("ok")
    }
}

impl HooksObject {
    fn meta(&self, key: &str) -> Option<String> {
        self.sql
            .exec("SELECT value FROM meta WHERE key = ?", Some(vec![sv(key)]))
            .and_then(|c| c.to_array::<MetaRow>())
            .ok()?
            .into_iter()
            .next()
            .map(|r| r.value)
    }

    fn set_meta(&self, key: &str, value: &str) {
        if let Err(e) = self.sql.exec(
            "INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            Some(vec![sv(key), sv(value)]),
        ) {
            worker::console_error!("hooks meta: {e}");
        }
    }

    fn exec(&self, sql: &str, params: Vec<SqlStorageValue>) {
        if let Err(e) = self.sql.exec(sql, Some(params)) {
            worker::console_error!("hooks storage: {e}");
        }
    }

    fn number(&self, sql: &str, params: Vec<SqlStorageValue>) -> i64 {
        self.sql
            .exec(sql, Some(params))
            .and_then(|c| c.to_array::<N>())
            .ok()
            .and_then(|r| r.into_iter().next())
            .and_then(|r| r.n)
            .map_or(0, int)
    }

    // ───────────────────────────── what it reads from D1 ─────────────────────────────

    /// The workspace's webhooks changed: read them again when next needed.
    fn changed(&self, workspace_id: &str) {
        self.set_meta("workspace_id", workspace_id);
        let mut mem = self.mem.borrow_mut();
        mem.hooks = None;
        mem.places = None;
    }

    /// The workspace's webhooks, as D1 has them.
    async fn hooks(&self, workspace_id: &str) -> AppResult<Rc<Vec<HookRow>>> {
        let now = now_ms();
        if let Some((at, hooks)) = &self.mem.borrow().hooks
            && now - at < READ_FOR_MS
        {
            return Ok(hooks.clone());
        }
        let db = self.env.d1("DB")?;
        let hooks = Rc::new(hook_rows::of_workspace(&db, workspace_id).await?);
        self.mem.borrow_mut().hooks = Some((now, hooks.clone()));
        Ok(hooks)
    }

    async fn hook(&self, workspace_id: &str, id: &str) -> std::result::Result<HookRow, ApiError> {
        self.hooks(workspace_id)
            .await
            .map_err(|e| e.0)?
            .iter()
            .find(|h| h.id == id)
            .cloned()
            .ok_or_else(|| ApiError::not_found("Webhook"))
    }

    /// The workspace's and its projects' names.
    async fn places(&self, workspace_id: &str) -> AppResult<Rc<Places>> {
        let now = now_ms();
        if let Some((at, places)) = &self.mem.borrow().places
            && now - at < READ_FOR_MS
        {
            return Ok(places.clone());
        }
        #[derive(Deserialize)]
        struct Place {
            id: String,
            slug: String,
            name: String,
            #[serde(default)]
            repo_owner: Option<String>,
            #[serde(default)]
            repo_name: Option<String>,
        }
        let db = self.env.d1("DB")?;
        let workspace: Option<Place> = crate::db::q(
            &db,
            "SELECT id, slug, name FROM workspaces WHERE id = ?",
            &[crate::db::s(workspace_id)],
        )?
        .first()
        .await?;
        let workspace = workspace.ok_or_else(|| crate::error::not_found("Workspace"))?;
        let projects: Vec<Place> = crate::db::q(
            &db,
            "SELECT id, slug, name, repo_owner, repo_name FROM projects WHERE workspace_id = ? ORDER BY created_at",
            &[crate::db::s(workspace_id)],
        )?
        .all()
        .await?;
        let places = Rc::new(Places {
            workspace: HookPlace {
                id: workspace.id,
                slug: workspace.slug,
                name: workspace.name,
            },
            projects: projects
                .into_iter()
                .map(|p| {
                    let repo = p
                        .repo_owner
                        .zip(p.repo_name)
                        .map(|(o, n)| format!("{o}/{n}"));
                    (
                        HookPlace {
                            id: p.id,
                            slug: p.slug,
                            name: p.name,
                        },
                        repo,
                    )
                })
                .collect(),
        });
        self.mem.borrow_mut().places = Some((now, places.clone()));
        Ok(places)
    }

    // ───────────────────────────── events in ─────────────────────────────

    /// Keeps an event, and a delivery of it for each webhook that wants it,
    /// for the alarm to post.
    async fn emit(&self, e: HookEmit) -> std::result::Result<serde_json::Value, ApiError> {
        self.set_meta("workspace_id", &e.workspace_id);
        let Some(kind) = Kind::parse(&e.kind) else {
            return Ok(serde_json::json!({ "deliveries": 0 }));
        };
        let hooks = self.hooks(&e.workspace_id).await.map_err(|x| x.0)?;
        let wanted: Vec<&HookRow> = hooks
            .iter()
            .filter(|h| rules::wants(&h.hook(), kind, e.project_id.as_deref(), e.origin))
            .collect();
        if wanted.is_empty() {
            return Ok(serde_json::json!({ "deliveries": 0 }));
        }
        let places = self.places(&e.workspace_id).await.map_err(|x| x.0)?;
        let project = e
            .project_id
            .as_deref()
            .and_then(|p| places.project(p))
            .map(|(p, _)| p.clone());
        let envelope = rules::envelope(
            &WebhookEventId(e.event_id.clone()),
            &e.kind,
            e.at,
            places.workspace.clone(),
            project,
            e.data,
            false,
        );
        let body = serde_json::to_string(&envelope)
            .map_err(|x| err(ErrorCode::Internal, x.to_string()))?;
        if body.len() > BODY_MAX {
            worker::console_error!(
                "webhooks: {} is {} bytes, past the most fctl posts",
                e.kind,
                body.len()
            );
            return Ok(serde_json::json!({ "deliveries": 0 }));
        }
        let now = now_ms();
        self.exec(
            "INSERT OR IGNORE INTO events (id, kind, project_id, session_id, gist, body, at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                sv(&e.event_id),
                sv(&e.kind),
                ov(e.project_id.as_deref()),
                ov(e.session_id.as_deref()),
                sv(&e.gist),
                sv(&body),
                iv(e.at),
            ],
        );
        let mut queued = 0;
        for h in wanted {
            let waiting = self.number(
                "SELECT COUNT(*) AS n FROM deliveries WHERE webhook_id = ? AND state = 'pending'",
                vec![sv(&h.id)],
            );
            if waiting >= i64::from(rules::WAITING_MAX) {
                self.exec(
                    "INSERT INTO health (webhook_id, dropped) VALUES (?, 1) \
                     ON CONFLICT (webhook_id) DO UPDATE SET dropped = dropped + 1",
                    vec![sv(&h.id)],
                );
                continue;
            }
            self.exec(
                "INSERT INTO deliveries (id, event_id, webhook_id, state, next_at, created_at) VALUES (?, ?, ?, 'pending', ?, ?)",
                vec![
                    sv(&fctl_domain::ids::new_id(WebhookDeliveryId::PREFIX, now)),
                    sv(&e.event_id),
                    sv(&h.id),
                    iv(now),
                    iv(now),
                ],
            );
            queued += 1;
        }
        if queued > 0 {
            self.arm(now).await;
        }
        Ok(serde_json::json!({ "deliveries": queued }))
    }

    /// Has the alarm go off by `at`.
    async fn arm(&self, at: i64) {
        let storage = self.state.storage();
        match storage.get_alarm().await {
            Ok(Some(set)) if set <= at => {}
            _ => {
                let _ = storage.set_alarm((at - now_ms()).max(0)).await;
            }
        }
    }

    /// The alarm's next time: the earliest pending delivery's, a second on
    /// when some are due still (the run's limits held them back), or an
    /// hour on to trim the log while it holds anything.
    async fn rearm(&self, now: i64) {
        let next = self.number(
            "SELECT MIN(next_at) AS n FROM deliveries WHERE state = 'pending' AND next_at IS NOT NULL",
            vec![],
        );
        let storage = self.state.storage();
        let at = if next > 0 {
            next.max(now + 1000)
        } else if self.number("SELECT COUNT(*) AS n FROM deliveries", vec![]) > 0 {
            now + TRIM_EVERY_MS
        } else {
            let _ = storage.delete_alarm().await;
            return;
        };
        let _ = storage.set_alarm((at - now).max(0)).await;
    }

    // ───────────────────────────── posting ─────────────────────────────

    /// Posts the deliveries that are due, those each webhook has room for
    /// this minute, the oldest first: each webhook's one after another, the
    /// webhooks at once. False when D1 couldn't say what the webhooks are.
    async fn post_due(&self, workspace_id: &str, now: i64) -> bool {
        let hooks = match self.hooks(workspace_id).await {
            Ok(h) => h,
            Err(e) => {
                worker::console_error!("webhooks of {workspace_id}: {}", e.0.message);
                return false;
            }
        };
        let due: Vec<DueRow> = self
            .sql
            .exec(
                "SELECT id, webhook_id, next_at FROM deliveries WHERE state = 'pending' AND next_at <= ? \
                 ORDER BY next_at, id LIMIT 500",
                Some(vec![iv(now)]),
            )
            .and_then(|c| c.to_array())
            .unwrap_or_default();
        let mut by_hook: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut oldest: Vec<(String, i64)> = Vec::new();
        for d in due {
            let Some(hook) = hooks.iter().find(|h| h.id == d.webhook_id) else {
                self.give_up(&d.id, "Its webhook was deleted.", now);
                continue;
            };
            if !hook.enabled {
                self.give_up(&d.id, "Its webhook is off.", now);
                continue;
            }
            let ids = by_hook.entry(d.webhook_id.clone()).or_insert_with(|| {
                oldest.push((d.webhook_id.clone(), int(d.next_at)));
                Vec::new()
            });
            ids.push(d.id);
        }
        rules::order_due(&mut oldest);
        let mut jobs = Vec::new();
        let mut left = PER_RUN;
        for (hook_id, _) in oldest {
            let Some(hook) = hooks.iter().find(|h| h.id == hook_id) else {
                continue;
            };
            let sent = self.number(
                "SELECT COUNT(*) AS n FROM sends WHERE webhook_id = ? AND at > ?",
                vec![sv(&hook_id), iv(now - 60_000)],
            );
            let room = usize::try_from(rules::room(u32::try_from(sent.max(0)).unwrap_or(u32::MAX)))
                .unwrap_or(0)
                .min(PER_RUN_EACH)
                .min(left);
            let ids: Vec<String> = by_hook
                .remove(&hook_id)
                .unwrap_or_default()
                .into_iter()
                .take(room)
                .collect();
            if ids.is_empty() {
                continue;
            }
            left -= ids.len();
            let hook = hook.clone();
            jobs.push(async move {
                for id in ids {
                    self.attempt(&hook, &id, None, By::Fctl).await;
                }
            });
            if left == 0 {
                break;
            }
        }
        join_all(jobs).await;
        true
    }

    /// A delivery that won't be posted: why, in its place in the log.
    fn give_up(&self, id: &str, why: &str, now: i64) {
        self.exec(
            "UPDATE deliveries SET state = 'failed', next_at = NULL, done_at = ?, reason = ? WHERE id = ? AND state = 'pending'",
            vec![iv(now), sv(why), sv(id)],
        );
    }

    fn delivery_row(&self, id: &str) -> Option<DeliveryRow> {
        self.sql
            .exec(
                &format!("{DELIVERY_SELECT} WHERE d.id = ?"),
                Some(vec![sv(id)]),
            )
            .and_then(|c| c.to_array::<DeliveryRow>())
            .ok()?
            .into_iter()
            .next()
    }

    fn body(&self, event_id: &str) -> Option<String> {
        self.sql
            .exec(
                "SELECT body FROM events WHERE id = ?",
                Some(vec![sv(event_id)]),
            )
            .and_then(|c| c.to_array::<BodyRow>())
            .ok()?
            .into_iter()
            .next()
            .map(|r| r.body)
    }

    /// Posts delivery `id` to `hook` now and keeps how it went: fctl's own
    /// attempt, which tries again later when it fails, or a person's.
    async fn attempt(
        &self,
        hook: &HookRow,
        id: &str,
        person: Option<UserRef>,
        by: By,
    ) -> Option<DeliveryRow> {
        let d = self.delivery_row(id)?;
        let now = now_ms();
        // fctl's own attempt posts only one that still waits and is due.
        if !rules::posts(by, d.state(), d.next_at.map(int), now) {
            return None;
        }
        let body = self.body(&d.event_id)?;
        let number = count(d.attempts) + 1;
        let test = d.test > 0.0;
        let keys = crate::crypto::keys(&Config::from_env(&self.env))
            .and_then(|k| hook.signing_keys(&k, now));
        let (posted, headers) = match keys {
            Ok(keys) if !keys.is_empty() => {
                let timestamp = now / 1000;
                let signature = rules::signatures(&keys, &d.event_id, timestamp, body.as_bytes());
                let headers = Headers {
                    event_id: &d.event_id,
                    kind: &d.kind,
                    delivery_id: &d.id,
                    webhook_id: &hook.id,
                    workspace_id: &hook.workspace_id,
                    project_id: d.project_id.as_deref(),
                    timestamp,
                    signature: &signature,
                    attempt: number,
                    version: env!("CARGO_PKG_VERSION"),
                    test,
                }
                .to_vec();
                (post(&hook.url, &headers, &body).await, headers)
            }
            _ => (
                Posted {
                    status: None,
                    error: Some("fctl couldn't open the webhook's secret to sign it.".into()),
                    response: None,
                    ms: 0,
                    retry_after: None,
                },
                Vec::new(),
            ),
        };
        let ok = posted.status.is_some_and(rules::delivered);
        let attempt = WebhookAttempt {
            at: now,
            ms: posted.ms,
            status: posted.status,
            error: posted.error.clone(),
            by: person,
            response: posted.response,
        };
        // Read again: while this attempt waited for its answer, a resend may
        // have made another, turning the webhook off may have failed it, and
        // deleting the webhook may have forgotten it, which leaves nothing
        // to write.
        let d = self.delivery_row(id)?;
        let number = count(d.attempts) + 1;
        let mut history = d.history();
        history.insert(
            0,
            WebhookAttempt {
                response: None,
                ..attempt.clone()
            },
        );
        history.truncate(rules::HISTORY_KEPT);
        let settled = rules::settle(
            by,
            d.state(),
            d.next_at.map(int),
            ok,
            rules::own_attempts(&history),
            now,
            posted.retry_after,
        );
        let headers: Vec<WebhookHeader> = headers
            .into_iter()
            .map(|(name, value)| WebhookHeader { name, value })
            .collect();
        self.exec(
            "UPDATE deliveries SET state = ?, attempts = ?, next_at = ?, done_at = COALESCE(?, done_at), last = ?, \
             history = ?, headers = ?, url = ?, reason = CASE WHEN ? THEN reason END WHERE id = ?",
            vec![
                sv(settled.state.as_str()),
                iv(i64::from(number)),
                onv(settled.next_at),
                onv(settled.done_at),
                sv(&serde_json::to_string(&attempt).unwrap_or_default()),
                sv(&serde_json::to_string(&history).unwrap_or_default()),
                sv(&serde_json::to_string(&headers).unwrap_or_default()),
                sv(&hook.url),
                iv(i64::from(settled.kept)),
                sv(id),
            ],
        );
        // A person's test or resend counted itself as it was asked for
        // (`reserve`).
        if by == By::Fctl {
            self.exec(
                "INSERT INTO sends (webhook_id, at, manual) VALUES (?, ?, 0)",
                vec![sv(&hook.id), iv(now)],
            );
        }
        // The webhook keeps how its newest attempt went for as long as it
        // lasts, so without the answer or who made it: the log has those,
        // for 30 days at most.
        let last = serde_json::to_string(&WebhookAttempt {
            response: None,
            by: None,
            ..attempt
        })
        .unwrap_or_default();
        // How the webhook stands goes by fctl's own attempts: any that went
        // through says it answers, but a person's test or resend that failed
        // starts no failing, which would turn it off three days on.
        if ok {
            self.exec(
                "INSERT INTO health (webhook_id, failing_since, dropped, last) VALUES (?, NULL, 0, ?) \
                 ON CONFLICT (webhook_id) DO UPDATE SET failing_since = NULL, dropped = 0, last = excluded.last",
                vec![sv(&hook.id), sv(&last)],
            );
        } else if by == By::Fctl {
            self.exec(
                "INSERT INTO health (webhook_id, failing_since, last) VALUES (?, ?, ?) \
                 ON CONFLICT (webhook_id) DO UPDATE SET failing_since = COALESCE(failing_since, excluded.failing_since), \
                 last = excluded.last",
                vec![sv(&hook.id), iv(now), sv(&last)],
            );
            let since = self.number(
                "SELECT failing_since AS n FROM health WHERE webhook_id = ?",
                vec![sv(&hook.id)],
            );
            if rules::turns_off(Some(since).filter(|s| *s > 0), now) {
                let why = posted.error.as_deref().unwrap_or("no answer");
                self.turn_off(hook, &rules::off_reason(why), now).await;
            }
        }
        self.delivery_row(id)
    }

    /// Turns `hook` off: every attempt failed for three days. Its waiting
    /// deliveries go nowhere, and the workspace's audit log says why.
    async fn turn_off(&self, hook: &HookRow, why: &str, now: i64) {
        let turned: AppResult<usize> = async {
            let db = self.env.d1("DB")?;
            let changed = crate::db::q(
                &db,
                "UPDATE webhooks SET enabled = 0, disabled_reason = ?, disabled_at = ?, updated_at = ? WHERE id = ? AND enabled = 1",
                &[
                    crate::db::s(why),
                    crate::db::n(now),
                    crate::db::n(now),
                    crate::db::s(&hook.id),
                ],
            )?
            .run()
            .await?;
            if changed > 0 {
                crate::db::q(
                    &db,
                    "INSERT INTO audit_log (id, ts, actor_id, action, target, data, ip, workspace_id) VALUES (?, ?, NULL, 'webhook.turned_off', ?, ?, NULL, ?)",
                    &[
                        crate::db::s(&fctl_domain::ids::new_id("aud", now)),
                        crate::db::n(now),
                        crate::db::s(&hook.id),
                        crate::db::s(
                            &serde_json::json!({ "url": hook.url, "reason": why }).to_string(),
                        ),
                        crate::db::s(&hook.workspace_id),
                    ],
                )?
                .run()
                .await?;
            }
            Ok(changed)
        }
        .await;
        match turned {
            Ok(_) => {
                self.exec(
                    "UPDATE deliveries SET state = 'failed', next_at = NULL, done_at = ?, reason = ? \
                     WHERE webhook_id = ? AND state = 'pending'",
                    vec![iv(now), sv("Its webhook was turned off."), sv(&hook.id)],
                );
                self.changed(&hook.workspace_id);
                hook_rows::forget_cached(&hook.workspace_id);
            }
            Err(e) => worker::console_error!("turning webhook {} off: {}", hook.id, e.0.message),
        }
    }

    // ───────────────────────────── people's ─────────────────────────────

    /// Refuses another test or resend once a webhook had its hour's.
    fn manual_room(&self, webhook_id: &str, now: i64) -> std::result::Result<(), ApiError> {
        let sent = self.number(
            "SELECT COUNT(*) AS n FROM sends WHERE webhook_id = ? AND manual = 1 AND at > ?",
            vec![sv(webhook_id), iv(now - HOUR)],
        );
        if sent < i64::from(rules::MANUAL_PER_HOUR) {
            return Ok(());
        }
        let oldest = self.number(
            "SELECT MIN(at) AS n FROM (SELECT at FROM sends WHERE webhook_id = ? AND manual = 1 AND at > ? ORDER BY at DESC LIMIT ?)",
            vec![sv(webhook_id), iv(now - HOUR), iv(i64::from(rules::MANUAL_PER_HOUR))],
        );
        let wait_s = ((oldest + HOUR - now) / 1000).max(1);
        Err(err(
            ErrorCode::RateLimited,
            format!(
                "This webhook took {} tests and resends in the last hour, the most it takes. Try again in {} minutes.",
                rules::MANUAL_PER_HOUR,
                (wait_s + 59) / 60
            ),
        )
        .with_retry_after(u32::try_from(wait_s).unwrap_or(60)))
    }

    /// Counts a person's test or resend against the webhook's hour as it's
    /// asked for, before it waits on anything, so that those asked for at
    /// once all count (`manual_room`).
    fn reserve(&self, webhook_id: &str, now: i64) {
        self.exec(
            "INSERT INTO sends (webhook_id, at, manual) VALUES (?, ?, 1)",
            vec![sv(webhook_id), iv(now)],
        );
    }

    async fn resend(&self, b: HookDelivery) -> std::result::Result<WebhookDelivery, ApiError> {
        let now = now_ms();
        let d = self
            .delivery_row(&b.delivery_id)
            .filter(|d| d.webhook_id == b.webhook_id)
            .ok_or_else(|| ApiError::not_found("Delivery"))?;
        if d.test > 0.0 {
            return Err(err(
                ErrorCode::Conflict,
                "A test isn't resent: send another one.",
            ));
        }
        if count(d.resends) >= rules::RESENDS {
            return Err(err(
                ErrorCode::Conflict,
                format!(
                    "It was resent {} times, the most a delivery is.",
                    rules::RESENDS
                ),
            ));
        }
        self.manual_room(&b.webhook_id, now)?;
        // Counted now, before anything waits: another resend that comes
        // meanwhile counts this one.
        self.reserve(&b.webhook_id, now);
        self.exec(
            "UPDATE deliveries SET resends = resends + 1 WHERE id = ?",
            vec![sv(&d.id)],
        );
        let hook = self.hook(&b.workspace_id, &b.webhook_id).await?;
        let d = self
            .attempt(&hook, &d.id, b.by, By::Resend)
            .await
            .ok_or_else(|| ApiError::not_found("Delivery"))?;
        self.rearm(now_ms()).await;
        Ok(d.to_delivery(self.body(&d.event_id)))
    }

    /// Posts a sample of an event to a webhook now, marked a test.
    async fn test(&self, b: HookTest) -> std::result::Result<WebhookDelivery, ApiError> {
        let now = now_ms();
        self.set_meta("workspace_id", &b.workspace_id);
        let kind = Kind::parse(&b.kind).ok_or_else(|| {
            ApiError::validation("event", format!("fctl has no event called {}", b.kind))
        })?;
        self.manual_room(&b.webhook_id, now)?;
        self.reserve(&b.webhook_id, now);
        let hook = self.hook(&b.workspace_id, &b.webhook_id).await?;
        let places = self.places(&b.workspace_id).await.map_err(|e| e.0)?;
        // The webhook's own project, else the first it hears of, else the
        // workspace's first: the sample happens there.
        let projects = hook.projects();
        let chosen = hook
            .project_id
            .as_deref()
            .or_else(|| projects.first().map(String::as_str))
            .and_then(|p| places.project(p))
            .or_else(|| places.projects.first());
        let (project, repo) = chosen.cloned().unwrap_or_else(|| {
            (
                HookPlace {
                    id: "prj_01KSAMPLEPROJECT0000000000".into(),
                    slug: "web".into(),
                    name: "Web".into(),
                },
                None,
            )
        });
        let repo = repo.unwrap_or_else(|| "acme/web".into());
        let cfg = Config::from_env(&self.env);
        let event = rules::sample(kind, &project, &repo, &cfg.public_url, now);
        let event_id = fctl_domain::ids::new_id(WebhookEventId::PREFIX, now);
        let envelope = rules::envelope(
            &WebhookEventId(event_id.clone()),
            kind.as_str(),
            now,
            places.workspace.clone(),
            Some(project.clone()),
            event.data,
            true,
        );
        let body = serde_json::to_string(&envelope)
            .map_err(|x| err(ErrorCode::Internal, x.to_string()))?;
        self.exec(
            "INSERT INTO events (id, kind, project_id, session_id, gist, body, at) VALUES (?, ?, ?, NULL, ?, ?, ?)",
            vec![
                sv(&event_id),
                sv(kind.as_str()),
                sv(&project.id),
                sv(&format!("Test: {}", event.gist)),
                sv(&body),
                iv(now),
            ],
        );
        let id = fctl_domain::ids::new_id(WebhookDeliveryId::PREFIX, now);
        self.exec(
            "INSERT INTO deliveries (id, event_id, webhook_id, state, next_at, created_at, test) VALUES (?, ?, ?, 'pending', NULL, ?, 1)",
            vec![sv(&id), sv(&event_id), sv(&hook.id), iv(now)],
        );
        let d = self
            .attempt(&hook, &id, Some(b.by), By::Test)
            .await
            .ok_or_else(|| ApiError::not_found("Delivery"))?;
        self.rearm(now_ms()).await;
        Ok(d.to_delivery(self.body(&d.event_id)))
    }

    /// The deliveries of a webhook that failed since `since` wait for an
    /// attempt again, in their turn, the oldest first.
    async fn resend_failed(
        &self,
        b: &HookResendFailed,
    ) -> std::result::Result<ResentFailed, ApiError> {
        let now = now_ms();
        let key = format!("resent_failed:{}", b.webhook_id);
        if let Some(at) = self.meta(&key).and_then(|v| v.parse::<i64>().ok())
            && now - at < RESEND_FAILED_EVERY_MS
        {
            let wait_s = (at + RESEND_FAILED_EVERY_MS - now) / 1000;
            let ago = match (now - at) / 60_000 {
                0 => "just now".to_owned(),
                1 => "a minute ago".to_owned(),
                m => format!("{m} minutes ago"),
            };
            return Err(err(
                ErrorCode::RateLimited,
                format!(
                    "Its failed deliveries were resent {ago}: try again in {} minutes.",
                    (wait_s + 59) / 60
                ),
            )
            .with_retry_after(u32::try_from(wait_s.max(1)).unwrap_or(60)));
        }
        let hook = self.hook(&b.workspace_id, &b.webhook_id).await?;
        if !hook.enabled {
            return Err(err(
                ErrorCode::Conflict,
                "The webhook is off: turn it on first, so that they go somewhere.",
            ));
        }
        // Counted before they change: SQLite's rows written count each
        // index's writes too.
        let which = "SELECT id FROM deliveries WHERE webhook_id = ?1 AND state = 'failed' AND test = 0 \
                     AND created_at >= ?2 ORDER BY id LIMIT ?3";
        let params = || {
            vec![
                sv(&b.webhook_id),
                iv(b.since),
                iv(i64::from(rules::RESEND_FAILED_MAX)),
            ]
        };
        let queued = self.number(&format!("SELECT COUNT(*) AS n FROM ({which})"), params());
        let mut update = params();
        update.push(iv(now));
        self.exec(
            &format!(
                "UPDATE deliveries SET state = 'pending', next_at = ?4, done_at = NULL, reason = NULL \
                 WHERE id IN ({which})"
            ),
            update,
        );
        self.set_meta(&key, &now.to_string());
        if queued > 0 {
            self.arm(now).await;
        }
        Ok(ResentFailed {
            queued: u32::try_from(queued.max(0)).unwrap_or(u32::MAX),
        })
    }

    // ───────────────────────────── reading the log ─────────────────────────────

    fn health(&self, webhooks: &[String]) -> HooksHealthAnswer {
        let now = now_ms();
        let mut health = BTreeMap::new();
        for id in webhooks {
            let counts: Option<Counts> = self
                .sql
                .exec(
                    "SELECT SUM(CASE WHEN state = 'delivered' AND done_at > ?2 THEN 1 ELSE 0 END) AS delivered, \
                     SUM(CASE WHEN state = 'failed' AND done_at > ?2 THEN 1 ELSE 0 END) AS failed, \
                     SUM(CASE WHEN state = 'pending' THEN 1 ELSE 0 END) AS pending \
                     FROM deliveries WHERE webhook_id = ?1 AND test = 0",
                    Some(vec![sv(id), iv(now - DAY)]),
                )
                .and_then(|c| c.to_array::<Counts>())
                .ok()
                .and_then(|r| r.into_iter().next());
            let row: Option<HealthRow> = self
                .sql
                .exec(
                    "SELECT failing_since, dropped, last FROM health WHERE webhook_id = ?",
                    Some(vec![sv(id)]),
                )
                .and_then(|c| c.to_array::<HealthRow>())
                .ok()
                .and_then(|r| r.into_iter().next());
            let c = counts.unwrap_or(Counts {
                delivered: None,
                failed: None,
                pending: None,
            });
            health.insert(
                id.clone(),
                WebhookHealth {
                    last: row
                        .as_ref()
                        .and_then(|r| r.last.as_deref())
                        .and_then(|j| serde_json::from_str(j).ok()),
                    delivered: count(c.delivered.unwrap_or(0.0)),
                    failed: count(c.failed.unwrap_or(0.0)),
                    pending: count(c.pending.unwrap_or(0.0)),
                    failing_since: row.as_ref().and_then(|r| r.failing_since).map(int),
                    dropped: row.as_ref().map_or(0, |r| count(r.dropped)),
                },
            );
        }
        HooksHealthAnswer { health }
    }

    fn deliveries(&self, b: &HookDeliveries) -> WebhookDeliveryList {
        let mut sql = format!("{DELIVERY_SELECT} WHERE d.webhook_id = ?");
        let mut params = vec![sv(&b.webhook_id)];
        if let Some(state) = b.state.as_deref() {
            sql.push_str(" AND d.state = ?");
            params.push(sv(state));
        }
        if let Some(kind) = b.kind.as_deref() {
            sql.push_str(" AND e.kind = ?");
            params.push(sv(kind));
        }
        if let Some(before) = b.before.as_deref() {
            sql.push_str(" AND d.id < ?");
            params.push(sv(before));
        }
        let limit = b.limit.clamp(1, 100);
        sql.push_str(" ORDER BY d.id DESC LIMIT ?");
        params.push(iv(i64::from(limit) + 1));
        let mut rows: Vec<DeliveryRow> = self
            .sql
            .exec(&sql, Some(params))
            .and_then(|c| c.to_array())
            .unwrap_or_default();
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        WebhookDeliveryList {
            next: if more {
                rows.last().map(|r| r.id.clone())
            } else {
                None
            },
            deliveries: rows.iter().map(|r| r.to_delivery(None)).collect(),
        }
    }

    fn delivery(
        &self,
        webhook_id: &str,
        id: &str,
    ) -> std::result::Result<WebhookDelivery, ApiError> {
        let d = self
            .delivery_row(id)
            .filter(|d| d.webhook_id == webhook_id)
            .ok_or_else(|| ApiError::not_found("Delivery"))?;
        let body = self.body(&d.event_id).unwrap_or_default();
        Ok(d.to_delivery(Some(body)))
    }

    /// Forgets a webhook that was deleted, and what went to it.
    fn forget(&self, webhook_id: &str) {
        for stmt in [
            "DELETE FROM deliveries WHERE webhook_id = ?",
            "DELETE FROM health WHERE webhook_id = ?",
            "DELETE FROM sends WHERE webhook_id = ?",
        ] {
            self.exec(stmt, vec![sv(webhook_id)]);
        }
        self.exec(
            "DELETE FROM events WHERE NOT EXISTS (SELECT 1 FROM deliveries d WHERE d.event_id = events.id)",
            vec![],
        );
    }

    /// Forgets a deleted project's events and their deliveries in every
    /// webhook's log (its own webhooks are forgotten whole, `forget`).
    fn forget_project(&self, project_id: &str) {
        self.exec(
            "DELETE FROM deliveries WHERE event_id IN (SELECT id FROM events WHERE project_id = ?)",
            vec![sv(project_id)],
        );
        self.exec(
            "DELETE FROM events WHERE project_id = ?",
            vec![sv(project_id)],
        );
    }

    /// Forgets deliveries past 30 days, and all but each webhook's newest
    /// 1,000, once an hour; never one that still waits.
    fn trim(&self, now: i64) {
        let last = self
            .meta("trimmed_at")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        if now - last < TRIM_EVERY_MS {
            return;
        }
        self.set_meta("trimmed_at", &now.to_string());
        // A person's attempt that never finished (the object went away under
        // it) waits for nothing: it failed.
        self.exec(
            "UPDATE deliveries SET state = 'failed', done_at = ?1, reason = 'Its attempt didn''t finish.' \
             WHERE state = 'pending' AND next_at IS NULL AND created_at < ?2",
            vec![iv(now), iv(now - HOUR)],
        );
        self.exec(
            "DELETE FROM deliveries WHERE state != 'pending' AND created_at < ?",
            vec![iv(now - rules::KEPT_MS)],
        );
        #[derive(Deserialize)]
        struct Hook {
            webhook_id: String,
        }
        let hooks: Vec<Hook> = self
            .sql
            .exec("SELECT DISTINCT webhook_id FROM deliveries", None)
            .and_then(|c| c.to_array())
            .unwrap_or_default();
        for h in hooks {
            self.exec(
                "DELETE FROM deliveries WHERE webhook_id = ?1 AND state != 'pending' AND id NOT IN \
                 (SELECT id FROM deliveries WHERE webhook_id = ?1 ORDER BY id DESC LIMIT ?2)",
                vec![sv(&h.webhook_id), iv(i64::from(rules::KEPT))],
            );
        }
        self.exec(
            "DELETE FROM events WHERE NOT EXISTS (SELECT 1 FROM deliveries d WHERE d.event_id = events.id)",
            vec![],
        );
        self.exec("DELETE FROM sends WHERE at < ?", vec![iv(now - HOUR)]);
    }
}

/// Posts `body` to `url` with `headers`: at most `TIMEOUT_MS` for the
/// answer and the start of its body, without following a redirect.
async fn post(url: &str, headers: &[(String, String)], body: &str) -> Posted {
    let started = now_ms();
    let done = |status: Option<u16>,
                error: Option<String>,
                response: Option<WebhookResponse>,
                retry_after: Option<i64>| Posted {
        status,
        error,
        response,
        ms: u32::try_from((now_ms() - started).max(0)).unwrap_or(u32::MAX),
        retry_after,
    };
    let h = worker::Headers::new();
    for (name, value) in headers {
        if let Err(e) = h.set(name, value) {
            return done(
                None,
                Some(format!("fctl couldn't set its {name} header: {e}")),
                None,
                None,
            );
        }
    }
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(h)
        .with_redirect(RequestRedirect::Manual)
        .with_body(Some(wasm_bindgen::JsValue::from_str(body)));
    let req = match Request::new_with_init(url, &init) {
        Ok(r) => r,
        Err(e) => {
            return done(
                None,
                Some(format!("fctl couldn't make the request: {e}")),
                None,
                None,
            );
        }
    };
    // The answer and its body within the timeout: AbortSignal.timeout
    // aborts both (workerd's setTimeout isn't what `worker::Delay` takes).
    let signal = worker::AbortSignal::from(web_sys::AbortSignal::timeout_with_u32(
        u32::try_from(rules::TIMEOUT_MS).unwrap_or(10_000),
    ));
    let exchange = async {
        let mut res = Fetch::Request(req).send_with_signal(&signal).await?;
        let status = res.status_code();
        let headers: Vec<WebhookHeader> = res
            .headers()
            .entries()
            .take(40)
            .map(|(name, value)| WebhookHeader {
                name,
                value: rules::clip(&value, 500),
            })
            .collect();
        let retry_after = res.headers().get("retry-after").ok().flatten();
        let mut body: Vec<u8> = Vec::new();
        let mut truncated = false;
        if let Ok(mut stream) = res.stream() {
            while let Some(chunk) = stream.next().await {
                body.extend_from_slice(&chunk?);
                if body.len() > rules::RESPONSE_KEPT {
                    body.truncate(rules::RESPONSE_KEPT);
                    truncated = true;
                    break;
                }
            }
        }
        Ok::<_, worker::Error>((status, headers, retry_after, body, truncated))
    };
    match exchange.await {
        Ok((status, headers, retry_after, body, truncated)) => {
            let text = String::from_utf8_lossy(&body).into_owned();
            let error = (!rules::delivered(status)).then(|| rules::status_error(status));
            done(
                Some(status),
                error,
                Some(WebhookResponse {
                    headers,
                    body: (!text.is_empty()).then_some(text),
                    truncated,
                }),
                retry_after.as_deref().and_then(rules::retry_after),
            )
        }
        Err(_) if signal.aborted() => done(None, Some(rules::timeout_error()), None, None),
        Err(e) => done(
            None,
            Some(format!(
                "fctl couldn't reach it: {}",
                rules::clip(&e.to_string(), 300)
            )),
            None,
            None,
        ),
    }
}
