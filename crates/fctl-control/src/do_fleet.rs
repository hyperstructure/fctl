//! FleetObject: one Durable Object per instance.
//!
//! It holds every host's control connection, assigns queued sessions to hosts
//! with free capacity, answers credential requests (serializing OAuth
//! refreshes), runs Claude sign-in jobs on hosts, and fans out live updates to
//! dashboard feeds.

use crate::crypto;
use crate::ctx::now_ms;
use crate::internal::{
    self, Activity, ClaudeUsage, ControlState, Drain, DrainStop, DrainStopped, FeedIdentity,
    LiveRunners, LoginCode, LoginStart, NotifySession, RunnerChanged, Schedule, Stop,
    WorkspacesChanged,
};
use crate::spec;
use fctl_domain::image::HostImages;
use fctl_domain::reconcile::{self, Known, Unassigned};
use fctl_domain::schedule::{Home, Slot, Wait};
use fctl_proto::api::{ClaudeLoginJob, ClaudeLoginState, FeedServer};
use fctl_proto::event::{Notice, NoticeLevel};
use fctl_proto::runner::{
    ClaudeLoginUpdate, CredentialKind, FleetToHost, HostHello, HostSessionReport, HostToFleet,
    SandboxLimits, StopReason,
};
use fctl_proto::session::{RunnerRef, SessionStatus};
use fctl_proto::{ApiError, ErrorCode, LoginJobId, RunnerId, SessionId};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashSet;
use worker::{
    DurableObject, Env, Request, Response, Result, SqlStorage, SqlStorageValue, State, WebSocket,
    WebSocketIncomingMessage, WebSocketPair, durable_object,
};

const TAG_FEED: &str = "feed";
/// How often an open feed's access is checked again, on its pings: once
/// its person signed out, was disabled, or lost a workspace, project or
/// instance role, it shows what they see now within this long.
const FEED_CHECK_MS: i64 = 60_000;
/// A feed whose access wasn't checked for this long (it stopped pinging,
/// which the app does every 25 seconds) closes at the next message to it,
/// and reconnects through the Worker, which checks it.
const FEED_STALE_MS: i64 = 5 * 60_000;
/// Close codes for feeds: their token no longer signs anyone in, and their
/// access wasn't checked for a while.
const CLOSE_NO_ACCESS: u16 = 4403;
const CLOSE_RECHECK: u16 = 4000;

/// The answer to a host's credential request.
struct Credentials {
    claude: Option<fctl_proto::runner::ClaudeCredential>,
    github: Option<fctl_proto::runner::GithubCredential>,
    github_read: Option<fctl_proto::runner::GithubCredential>,
}
/// Starting a session may be attempted this many times before it fails.
const MAX_ATTEMPTS: i64 = 5;
/// A runner that stays away this long loses its sessions.
const RUNNER_GRACE_MS: i64 = 10 * 60 * 1000;
/// How long a resumed session waits for the runner that has its sandbox,
/// while that runner is busy, before it starts on another one in a new
/// sandbox (without its conversation).
const HOME_WAIT_MS: i64 = 10 * 60 * 1000;
/// Which runner has a session's sandbox is forgotten after this long without
/// a run: hosts remove stopped sandboxes long before (`keep_stopped_hours`).
const HOME_KEEP_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const TICK_MS: i64 = 30_000;

/// `ALTER TABLE … ADD COLUMN` only when the column is missing.
fn add_column(sql: &SqlStorage, table: &str, column: &str, ty: &str) {
    #[derive(Deserialize)]
    struct Col {
        name: String,
    }
    let cols: Vec<Col> = sql
        .exec(&format!("PRAGMA table_info({table})"), None)
        .and_then(|c| c.to_array::<Col>())
        .unwrap_or_default();
    if !cols.is_empty()
        && !cols.iter().any(|c| c.name == column)
        && let Err(e) = sql.exec(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {ty}"),
            None,
        )
    {
        worker::console_error!("migration {table}.{column}: {e}");
    }
}

fn runner_tag(id: &str) -> String {
    format!("runner:{id}")
}

/// The tag of a person's feed sockets.
fn feed_user_tag(user_id: &str) -> String {
    format!("feed:{user_id}")
}

/// What of runners a feed's person sees.
fn feed_view(ident: &FeedIdentity) -> crate::model::RunnerView<'_> {
    crate::model::RunnerView {
        user_id: Some(&ident.user_id),
        workspaces: (!ident.admin).then_some(ident.workspaces.as_slice()),
    }
}

async fn user_row(
    db: &worker::D1Database,
    id: &str,
) -> crate::error::AppResult<Option<crate::model::UserRow>> {
    crate::db::q(
        db,
        &format!(
            "SELECT {} FROM users u WHERE u.id = ?",
            crate::model::user_columns("u")
        ),
        &[crate::db::s(id)],
    )?
    .first()
    .await
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RunnerSocket {
    id: String,
    name: String,
    since: i64,
}

#[derive(Deserialize)]
struct QueueRow {
    session_id: String,
    attempts: f64,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    enqueued_at: f64,
}

#[derive(Deserialize)]
struct AssignRow {
    session_id: String,
    runner_id: String,
    accepted: f64,
}

#[derive(Deserialize)]
struct LiveRow {
    id: String,
    name: String,
    hello: Option<String>,
    connected: f64,
    last_seen: Option<f64>,
    #[serde(default)]
    memory_available_mb: Option<f64>,
    #[serde(default)]
    disk_available_mb: Option<f64>,
    #[serde(default)]
    draining_since: Option<f64>,
    #[serde(default)]
    drained_by: Option<String>,
}

/// A connected runner, as scheduling sees it.
struct Candidate {
    id: String,
    name: String,
    capacity: u32,
    active: u32,
    labels: Vec<String>,
    sandbox: Option<SandboxLimits>,
    draining: bool,
    /// Its free disk is below what its host keeps.
    low_disk: bool,
    image: Option<String>,
    allowed_images: Option<Vec<String>>,
    allowed_builds: Option<Vec<String>>,
    /// The workspaces whose sessions it runs.
    serves: Serves,
}

/// Whose sessions a runner runs, as D1 had it at the last dispatch.
#[derive(Clone, Debug, Default)]
struct Serves {
    /// Who registered it.
    owner: Option<String>,
    /// The workspaces it runs sessions of, each with whether it runs only
    /// its owner's there.
    workspaces: Vec<(String, bool)>,
}

impl Serves {
    /// Whether it takes a session that wants `want`: one of its workspace
    /// that runs as someone it runs sessions for there.
    fn takes(&self, want: &spec::SessionWants) -> bool {
        self.workspaces.iter().any(|(w, personal)| {
            *w == want.workspace_id
                && fctl_domain::schedule::runs_for(
                    self.owner.as_deref(),
                    *personal,
                    want.person.as_deref(),
                    want.own_runners,
                )
        })
    }

    /// Whether it runs any sessions of `workspace_id`.
    fn runs(&self, workspace_id: &str) -> bool {
        self.workspaces.iter().any(|(w, _)| w == workspace_id)
    }
}

impl Candidate {
    /// As the scheduler sees it, for a session that wants `want`.
    fn slot(&self, want: &spec::SessionWants) -> Slot<'_> {
        Slot {
            id: &self.id,
            online: true,
            enabled: true,
            serves: self.serves.takes(want),
            capacity: self.capacity,
            active: self.active,
            labels: &self.labels,
            sandbox: self.sandbox,
            draining: self.draining,
            low_disk: self.low_disk,
            images: HostImages {
                own: self.image.as_deref(),
                allowed: self.allowed_images.as_deref(),
                builds: self.allowed_builds.as_deref(),
            },
        }
    }
}

#[derive(Deserialize)]
struct LoginRow {
    runner_id: Option<String>,
    user_id: String,
}

fn sv(s: &str) -> SqlStorageValue {
    SqlStorageValue::from(s)
}

fn iv(n: i64) -> SqlStorageValue {
    SqlStorageValue::from(n)
}

#[durable_object]
pub struct FleetObject {
    state: State,
    env: Env,
    sql: SqlStorage,
    /// Credentials being refreshed right now (serializes OAuth refreshes).
    refreshing: RefCell<HashSet<String>>,
    dispatching: RefCell<bool>,
    /// runner id → the workspaces it serves, read from D1 before each
    /// dispatch.
    runner_workspaces: RefCell<std::collections::HashMap<String, Serves>>,
}

impl DurableObject for FleetObject {
    fn new(state: State, env: Env) -> Self {
        let sql = state.storage().sql();
        for stmt in [
            "CREATE TABLE IF NOT EXISTS queue (session_id TEXT PRIMARY KEY, enqueued_at INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, not_before INTEGER NOT NULL DEFAULT 0)",
            "CREATE TABLE IF NOT EXISTS assignments (session_id TEXT PRIMARY KEY, runner_id TEXT NOT NULL, assigned_at INTEGER NOT NULL, accepted INTEGER NOT NULL DEFAULT 0, run_id TEXT)",
            "CREATE TABLE IF NOT EXISTS runners (id TEXT PRIMARY KEY, name TEXT NOT NULL, hello TEXT, connected INTEGER NOT NULL DEFAULT 0, last_seen INTEGER, sessions TEXT)",
            "CREATE TABLE IF NOT EXISTS logins (job_id TEXT PRIMARY KEY, runner_id TEXT, user_id TEXT NOT NULL, created_at INTEGER NOT NULL)",
            // The runner that last took each session: it keeps the session's
            // sandbox, so a resume goes back to it.
            "CREATE TABLE IF NOT EXISTS sandboxes (session_id TEXT PRIMARY KEY, runner_id TEXT NOT NULL, since INTEGER NOT NULL)",
            // Sessions stopped because their runner stayed away: they carry
            // on if it comes back with them.
            "CREATE TABLE IF NOT EXISTS stopped_away (session_id TEXT PRIMARY KEY, runner_id TEXT NOT NULL, since INTEGER NOT NULL)",
        ] {
            if let Err(e) = sql.exec(stmt, None) {
                worker::console_error!("fleet schema: {e}");
            }
        }
        // Columns added after the first release. A failing statement throws past
        // Rust (workers-rs does not catch every SQL error), so check first.
        add_column(&sql, "assignments", "run_id", "TEXT");
        // Why a queued session waits, as it last said.
        add_column(&sql, "queue", "reason", "TEXT");
        add_column(&sql, "runners", "memory_available_mb", "INTEGER");
        add_column(&sql, "runners", "disk_available_mb", "INTEGER");
        // Set while the runner drains: since when, and the admin who drained
        // it (JSON), or none when its host did.
        add_column(&sql, "runners", "draining_since", "INTEGER");
        add_column(&sql, "runners", "drained_by", "TEXT");
        Self {
            state,
            env,
            sql,
            refreshing: RefCell::new(HashSet::new()),
            dispatching: RefCell::new(false),
            runner_workspaces: RefCell::new(std::collections::HashMap::new()),
        }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        let method = req.method();
        let result: std::result::Result<serde_json::Value, ApiError> = match (method, path.as_str())
        {
            (worker::Method::Get, "/runner") => return self.accept_runner(&req),
            (worker::Method::Get, "/feed") => return self.accept_feed(&req).await,
            (worker::Method::Get, "/runners") => Ok(serde_json::json!(self.live_runners())),
            (worker::Method::Post, "/schedule") => match req.json::<Schedule>().await {
                Ok(b) => self.schedule(b).await,
                Err(e) => Err(ApiError::bad_request(e.to_string())),
            },
            (worker::Method::Post, "/stop") => match req.json::<Stop>().await {
                Ok(b) => self.stop(b).await,
                Err(e) => Err(ApiError::bad_request(e.to_string())),
            },
            (worker::Method::Post, "/workspaces-changed") => {
                match req.json::<WorkspacesChanged>().await {
                    Ok(b) => {
                        self.workspaces_changed(&b.user_ids).await;
                        Ok(serde_json::json!({ "ok": true }))
                    }
                    Err(e) => Err(ApiError::bad_request(e.to_string())),
                }
            }
            (worker::Method::Post, "/runner-changed") => match req.json::<RunnerChanged>().await {
                Ok(b) => {
                    self.stop_unserved(&b.runner_id).await;
                    self.feed_runner(&b.runner_id).await;
                    self.dispatch().await;
                    Ok(serde_json::json!({ "ok": true }))
                }
                Err(e) => Err(ApiError::bad_request(e.to_string())),
            },
            (worker::Method::Post, "/notify-session") => match req.json::<NotifySession>().await {
                Ok(b) => {
                    self.feed_session(&b.session);
                    Ok(serde_json::json!({ "ok": true }))
                }
                Err(e) => Err(ApiError::bad_request(e.to_string())),
            },
            (worker::Method::Post, "/activity") => match req.json::<Activity>().await {
                Ok(b) => {
                    self.feed_activity(b);
                    Ok(serde_json::json!({ "ok": true }))
                }
                Err(e) => Err(ApiError::bad_request(e.to_string())),
            },
            (worker::Method::Post, "/usage") => match req.json::<ClaudeUsage>().await {
                Ok(b) => {
                    self.feed_usage(b);
                    Ok(serde_json::json!({ "ok": true }))
                }
                Err(e) => Err(ApiError::bad_request(e.to_string())),
            },
            (worker::Method::Post, "/drain") => match req.json::<Drain>().await {
                Ok(b) => self.drain(b).await,
                Err(e) => Err(ApiError::bad_request(e.to_string())),
            },
            (worker::Method::Post, "/runner-disabled") => {
                match req.json::<serde_json::Value>().await {
                    Ok(b) => {
                        let id = b
                            .get("runner_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        for ws in self.state.get_websockets_with_tag(&runner_tag(&id)) {
                            let _ = ws.close(Some(4003), Some("runner disabled or token rotated"));
                        }
                        let _ = self.sql.exec(
                            "UPDATE runners SET connected = 0 WHERE id = ?",
                            Some(vec![sv(&id)]),
                        );
                        // Its sessions' uplinks too: they connect again with
                        // a token that works, or not at all.
                        for a in self.assigned_to_runner(&id) {
                            self.close_uplink(&a.session_id, &id).await;
                        }
                        Ok(serde_json::json!({ "ok": true }))
                    }
                    Err(e) => Err(ApiError::bad_request(e.to_string())),
                }
            }
            (worker::Method::Post, "/login/start") => match req.json::<LoginStart>().await {
                Ok(b) => self.login_start(b),
                Err(e) => Err(ApiError::bad_request(e.to_string())),
            },
            (worker::Method::Post, "/login/code") => match req.json::<LoginCode>().await {
                Ok(b) => self.login_code(b),
                Err(e) => Err(ApiError::bad_request(e.to_string())),
            },
            _ => Err(ApiError::not_found("Fleet endpoint")),
        };
        match result {
            Ok(v) => internal::do_json(&v),
            Err(e) => internal::do_error(&e),
        }
    }

    async fn websocket_message(
        &self,
        ws: WebSocket,
        message: WebSocketIncomingMessage,
    ) -> Result<()> {
        let WebSocketIncomingMessage::String(text) = message else {
            return Ok(());
        };
        if self.state.get_tags(&ws).iter().any(|t| t == TAG_FEED) {
            if text.contains("\"ping\"") {
                let _ = ws.send_with_str(r#"{"type":"pong"}"#);
                if let Ok(Some(ident)) = ws.deserialize_attachment::<FeedIdentity>() {
                    let now = now_ms();
                    if now - ident.checked_at.unwrap_or_default() >= FEED_CHECK_MS {
                        self.recheck_feed(&ws, ident, now).await;
                    }
                }
            }
        } else if let Ok(Some(rs)) = ws.deserialize_attachment::<RunnerSocket>() {
            match serde_json::from_str::<HostToFleet>(&text) {
                Ok(msg) => self.on_runner(&ws, &rs, msg).await,
                Err(e) => worker::console_error!("bad runner message: {e}"),
            }
        }
        Ok(())
    }

    async fn websocket_close(
        &self,
        ws: WebSocket,
        _code: usize,
        _reason: String,
        _clean: bool,
    ) -> Result<()> {
        self.on_close(&ws).await;
        Ok(())
    }

    async fn websocket_error(&self, ws: WebSocket, _error: worker::Error) -> Result<()> {
        self.on_close(&ws).await;
        Ok(())
    }

    async fn alarm(&self) -> Result<Response> {
        let now = now_ms();
        self.expire_offline_runners(now).await;
        self.dispatch().await;
        // Keep ticking while there is something to wait for.
        if self.has_pending_work() {
            let _ = self.state.storage().set_alarm(TICK_MS).await;
        }
        Response::ok("ok")
    }
}

impl FleetObject {
    async fn on_close(&self, ws: &WebSocket) {
        if let Ok(Some(rs)) = ws.deserialize_attachment::<RunnerSocket>() {
            let still = self
                .state
                .get_websockets_with_tag(&runner_tag(&rs.id))
                .into_iter()
                .any(|w| &w != ws);
            if !still {
                let now = now_ms();
                let _ = self.sql.exec(
                    "UPDATE runners SET connected = 0, last_seen = ? WHERE id = ?",
                    Some(vec![iv(now), sv(&rs.id)]),
                );
                self.feed_runner(&rs.id).await;
                self.ensure_alarm().await;
            }
        }
    }

    // ───────────────────────────── connections ─────────────────────────────

    fn accept_runner(&self, req: &Request) -> Result<Response> {
        #[derive(Deserialize)]
        struct Ident {
            id: String,
            name: String,
        }
        let Some(ident) = crate::http::header(req, internal::HDR_RUNNER)
            .and_then(|h| serde_json::from_str::<Ident>(&h).ok())
        else {
            return internal::do_error(&ApiError::new(
                ErrorCode::Unauthorized,
                "No runner identity",
            ));
        };
        for old in self.state.get_websockets_with_tag(&runner_tag(&ident.id)) {
            let _ = old.close(Some(4000), Some("replaced by a newer connection"));
        }
        let pair = WebSocketPair::new()?;
        let tag = runner_tag(&ident.id);
        self.state
            .accept_websocket_with_tags(&pair.server, &[tag.as_str()]);
        pair.server.serialize_attachment(&RunnerSocket {
            id: ident.id.clone(),
            name: ident.name.clone(),
            since: now_ms(),
        })?;
        let _ = self.sql.exec(
            "INSERT INTO runners (id, name, connected, last_seen) VALUES (?, ?, 1, ?) \
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, connected = 1, last_seen = excluded.last_seen",
            Some(vec![sv(&ident.id), sv(&ident.name), iv(now_ms())]),
        );
        Response::from_websocket(pair.client)
    }

    async fn accept_feed(&self, req: &Request) -> Result<Response> {
        let Some(ident) = crate::http::header(req, internal::HDR_VIEWER)
            .and_then(|h| serde_json::from_str::<FeedIdentity>(&h).ok())
        else {
            return internal::do_error(&ApiError::new(
                ErrorCode::Unauthorized,
                "No viewer identity",
            ));
        };
        let pair = WebSocketPair::new()?;
        let user_tag = feed_user_tag(&ident.user_id);
        self.state
            .accept_websocket_with_tags(&pair.server, &[TAG_FEED, user_tag.as_str()]);
        pair.server.serialize_attachment(&ident)?;
        let runners = self.runners_for(&ident).await.unwrap_or_default();
        let hello = FeedServer::Hello {
            runners,
            server_time: now_ms(),
        };
        if let Ok(text) = serde_json::to_string(&hello) {
            pair.server.send_with_str(&text)?;
        }
        Response::from_websocket(pair.client)
    }

    fn runner_socket(&self, runner_id: &str) -> Option<WebSocket> {
        self.state
            .get_websockets_with_tag(&runner_tag(runner_id))
            .into_iter()
            .last()
    }

    fn send_runner(&self, runner_id: &str, msg: &FleetToHost) -> bool {
        match (self.runner_socket(runner_id), serde_json::to_string(msg)) {
            (Some(ws), Ok(text)) => ws.send_with_str(&text).is_ok(),
            _ => false,
        }
    }

    fn live_rows(&self) -> Vec<LiveRow> {
        self.sql
            .exec(
                "SELECT id, name, hello, connected, last_seen, memory_available_mb, disk_available_mb, draining_since, drained_by FROM runners",
                None,
            )
            .and_then(|c| c.to_array::<LiveRow>())
            .unwrap_or_default()
    }

    fn live_runners(&self) -> LiveRunners {
        let mut out = LiveRunners::default();
        for r in self.live_rows() {
            if r.connected > 0.0 && self.runner_socket(&r.id).is_some() {
                out.online.push(r.id.clone());
                out.free.insert(
                    r.id.clone(),
                    internal::Free {
                        memory_mb: r.memory_available_mb.map(|m| m as u64),
                        disk_mb: r.disk_available_mb.map(|d| d as u64),
                    },
                );
            }
            if let Some(since) = r.draining_since {
                out.drains.insert(
                    r.id.clone(),
                    fctl_proto::api::RunnerDrain {
                        since: since as i64,
                        by: r
                            .drained_by
                            .as_deref()
                            .and_then(|b| serde_json::from_str(b).ok()),
                    },
                );
            }
            if let Some(t) = r.last_seen {
                out.last_seen.insert(r.id, t as i64);
            }
        }
        out
    }

    async fn runner_rows(&self) -> std::result::Result<Vec<crate::model::RunnerRow>, ApiError> {
        let db = self
            .env
            .d1("DB")
            .map_err(|e| ApiError::internal(e.to_string()))?;
        crate::db::q(
            &db,
            &format!("{} ORDER BY r.created_at", crate::model::RUNNER_SELECT),
            &[],
        )
        .map_err(|e| e.0)?
        .all()
        .await
        .map_err(|e| e.0)
    }

    /// The runners a feed's person sees: those of their workspaces and
    /// their own, or all of them for instance admins.
    async fn runners_for(
        &self,
        ident: &FeedIdentity,
    ) -> std::result::Result<Vec<fctl_proto::api::Runner>, ApiError> {
        let rows = self.runner_rows().await?;
        let live = self.live_runners();
        let view = feed_view(ident);
        Ok(rows
            .iter()
            .filter(|r| r.visible_to(&view))
            .map(|r| r.to_runner(&live, &view))
            .collect())
    }

    async fn ensure_alarm(&self) {
        if let Ok(None) = self.state.storage().get_alarm().await {
            let _ = self.state.storage().set_alarm(TICK_MS).await;
        }
    }

    fn has_pending_work(&self) -> bool {
        #[derive(Deserialize)]
        struct C {
            n: f64,
        }
        let count = |sql: &str| {
            self.sql
                .exec(sql, None)
                .and_then(|c| c.to_array::<C>())
                .ok()
                .and_then(|r| r.first().map(|c| c.n))
                .unwrap_or(0.0)
        };
        count("SELECT COUNT(*) AS n FROM queue") > 0.0
            || count(
                "SELECT COUNT(*) AS n FROM assignments a JOIN runners r ON r.id = a.runner_id WHERE r.connected = 0",
            ) > 0.0
    }

    /// Whether the session waits in the queue (a message or a resume queued
    /// it again while its last run was still assigned).
    fn queued(&self, session_id: &str) -> bool {
        self.sql
            .exec(
                "SELECT session_id, attempts FROM queue WHERE session_id = ?",
                Some(vec![sv(session_id)]),
            )
            .and_then(|c| c.to_array::<QueueRow>())
            .map(|r| !r.is_empty())
            .unwrap_or(false)
    }

    // ───────────────────────────── runner messages ─────────────────────────────

    async fn on_runner(&self, ws: &WebSocket, rs: &RunnerSocket, msg: HostToFleet) {
        let now = now_ms();
        let _ = self.sql.exec(
            "UPDATE runners SET last_seen = ?, connected = 1 WHERE id = ?",
            Some(vec![iv(now), sv(&rs.id)]),
        );
        match msg {
            HostToFleet::Hello(hello) => self.on_hello(ws, rs, *hello, now).await,
            HostToFleet::Heartbeat(hb) => {
                let (was_low, min_free) = self.low_on_disk(&rs.id);
                let sessions = serde_json::to_string(&hb.sessions).unwrap_or_else(|_| "[]".into());
                let mib =
                    |v: Option<u64>| SqlStorageValue::from(v.and_then(|v| i64::try_from(v).ok()));
                let _ = self.sql.exec(
                    "UPDATE runners SET sessions = ?, memory_available_mb = ?, disk_available_mb = ? WHERE id = ?",
                    Some(vec![
                        sv(&sessions),
                        mib(hb.memory_available_mb),
                        mib(hb.disk_available_mb),
                        sv(&rs.id),
                    ]),
                );
                // Persist liveness in D1 now and then (every heartbeat is fine: one write per 15 s).
                if let Ok(db) = self.env.d1("DB")
                    && let Ok(q) = crate::db::q(
                        &db,
                        "UPDATE runners SET last_seen_at = ? WHERE id = ?",
                        &[crate::db::n(now), crate::db::s(&rs.id)],
                    )
                {
                    let _ = q.run().await;
                }
                self.stop_idle_if_draining(&rs.id, &hb.sessions).await;
                // The runners list says when a runner takes no new sessions
                // for its disk, and when it takes them again.
                if fctl_domain::disk::low(hb.disk_available_mb, min_free) != was_low {
                    self.feed_runner(&rs.id).await;
                }
                self.dispatch().await;
            }
            HostToFleet::StartResult {
                session_id,
                accepted,
                error,
            } => {
                self.on_start_result(&rs.id, &session_id, accepted, error, now)
                    .await;
            }
            HostToFleet::SessionEnded {
                session_id,
                status,
                reason,
                run_id,
            } => {
                // A report from an earlier run must not end a newer one.
                #[derive(Deserialize)]
                struct R {
                    runner_id: String,
                    run_id: Option<String>,
                }
                let current: Option<R> = self
                    .sql
                    .exec(
                        "SELECT runner_id, run_id FROM assignments WHERE session_id = ?",
                        Some(vec![sv(session_id.as_str())]),
                    )
                    .and_then(|c| c.to_array::<R>())
                    .ok()
                    .and_then(|r| r.into_iter().next());
                let stale = reconcile::ended_earlier(
                    &rs.id,
                    run_id.as_deref(),
                    current.as_ref().map(|c| reconcile::Current {
                        runner_id: &c.runner_id,
                        run_id: c.run_id.as_deref(),
                    }),
                );
                let queued_again = self.queued(session_id.as_str());
                // Unassigned, it ends only a run of this runner's: another
                // runner's report about it changes nothing.
                let theirs = current.is_some()
                    || match self.env.d1("DB") {
                        Ok(db) => spec::session_row(&db, session_id.as_str())
                            .await
                            .ok()
                            .flatten()
                            .is_some_and(|row| row.runner_id.as_deref() == Some(rs.id.as_str())),
                        Err(_) => false,
                    };
                if stale || !theirs {
                    worker::console_log!("ignoring the end of an earlier run of {session_id}");
                } else {
                    let _ = self.sql.exec(
                        "DELETE FROM assignments WHERE session_id = ? AND runner_id = ?",
                        Some(vec![sv(session_id.as_str()), sv(&rs.id)]),
                    );
                    // Re-queued (resumed) meanwhile: free the slot, keep the new status.
                    if !queued_again {
                        self.session_control(&session_id, Some(status), reason, false)
                            .await;
                    }
                }
                self.dispatch().await;
            }
            HostToFleet::CredentialRequest {
                request_id,
                session_id,
                kind,
            } => {
                let reply = self.credential(&rs.id, &session_id, kind, now).await;
                let msg = match reply {
                    Ok(c) => FleetToHost::Credential {
                        request_id,
                        claude: c.claude,
                        github: c.github,
                        github_read: c.github_read,
                        error: None,
                    },
                    Err(e) => FleetToHost::Credential {
                        request_id,
                        claude: None,
                        github: None,
                        github_read: None,
                        error: Some(e.message),
                    },
                };
                self.send_runner(&rs.id, &msg);
            }
            HostToFleet::ClaudeLogin { job_id, update } => {
                self.on_login_update(&rs.id, &job_id, update, now).await
            }
            HostToFleet::Pong => {}
        }
    }

    async fn on_hello(&self, ws: &WebSocket, rs: &RunnerSocket, hello: HostHello, now: i64) {
        let info = serde_json::to_string(&hello).unwrap_or_default();
        let sessions = serde_json::to_string(&hello.sessions).unwrap_or_else(|_| "[]".into());
        let _ = self.sql.exec(
            "UPDATE runners SET hello = ?, sessions = ?, connected = 1, last_seen = ? WHERE id = ?",
            Some(vec![sv(&info), sv(&sessions), iv(now), sv(&rs.id)]),
        );
        if let Ok(db) = self.env.d1("DB")
            && let Ok(q) = crate::db::q(
                &db,
                "UPDATE runners SET info = ?, last_seen_at = ? WHERE id = ?",
                &[crate::db::s(&info), crate::db::n(now), crate::db::s(&rs.id)],
            )
        {
            let _ = q.run().await;
        }
        let welcome = FleetToHost::Welcome {
            runner: RunnerRef {
                id: RunnerId(rs.id.clone()),
                name: rs.name.clone(),
            },
            heartbeat_secs: 15,
            server_time: now,
        };
        if let Ok(text) = serde_json::to_string(&welcome) {
            let _ = ws.send_with_str(&text);
        }
        self.reconcile(rs, &hello.sessions, now).await;
        self.stop_idle_if_draining(&rs.id, &hello.sessions).await;
        self.feed_runner(&rs.id).await;
        self.dispatch().await;
    }

    /// Matches what the host reports with what we assigned to it.
    async fn reconcile(&self, rs: &RunnerSocket, reported: &[HostSessionReport], now: i64) {
        let runner_id = rs.id.as_str();
        let assigned: Vec<AssignRow> = self
            .sql
            .exec(
                "SELECT session_id, runner_id, accepted FROM assignments WHERE runner_id = ?",
                Some(vec![sv(runner_id)]),
            )
            .and_then(|c| c.to_array::<AssignRow>())
            .unwrap_or_default();
        let reported_ids: HashSet<&str> = reported.iter().map(|r| r.session_id.as_str()).collect();
        // Assigned and accepted, but the host no longer has it: it was lost.
        // One queued again meanwhile keeps its new status.
        for a in &assigned {
            if a.accepted > 0.0 && !reported_ids.contains(a.session_id.as_str()) {
                let _ = self.sql.exec(
                    "DELETE FROM assignments WHERE session_id = ?",
                    Some(vec![sv(&a.session_id)]),
                );
                if self.queued(&a.session_id) {
                    continue;
                }
                self.session_control(
                    &SessionId(a.session_id.clone()),
                    Some(SessionStatus::Stopped),
                    Some("The runner restarted without this session's sandbox. Resume it to continue.".into()),
                    true,
                )
                .await;
            }
        }
        // Reported, but not assigned to it: the runner was away, or the
        // session was deleted, stopped or resumed elsewhere meanwhile.
        let assigned_ids: HashSet<&str> = assigned.iter().map(|a| a.session_id.as_str()).collect();
        let db = self.env.d1("DB");
        for r in reported {
            let id = r.session_id.as_str();
            if assigned_ids.contains(id) || !r.status.is_active() {
                continue;
            }
            // Without the session, the run is left alone until the next hello.
            let row = match &db {
                Ok(db) => spec::session_row(db, id).await.map_err(|e| e.0.message),
                Err(e) => Err(e.to_string()),
            };
            let row = match row {
                Ok(row) => row,
                Err(e) => {
                    worker::console_error!("reconciling {id} on {}: {e}", rs.name);
                    continue;
                }
            };
            let known = row.as_ref().map(|row| Known {
                runner_id: row.runner_id.as_deref(),
                status: row.status(),
                archived: row.archived,
            });
            let mut verdict =
                reconcile::unassigned(runner_id, known, self.stopped_away(id, runner_id));
            // A runner that stopped serving the session's workspace while it
            // was away doesn't carry on with it there.
            if matches!(verdict, Unassigned::Keep | Unassigned::CarryOn)
                && let Ok(db) = &db
                && matches!(spec::runner_serves(db, runner_id, id).await, Ok(false))
            {
                self.unserved(runner_id, id).await;
                verdict = Unassigned::Stop(StopReason::Shutdown);
            }
            match verdict {
                Unassigned::Keep => {
                    let _ = self.sql.exec(
                        "INSERT OR REPLACE INTO assignments (session_id, runner_id, assigned_at, accepted) VALUES (?, ?, ?, 1)",
                        Some(vec![sv(id), sv(runner_id), iv(now)]),
                    );
                    self.set_home(id, runner_id, now);
                }
                Unassigned::CarryOn => self.carry_on(rs, r, now).await,
                Unassigned::Stop(reason) => {
                    let _ = self.sql.exec(
                        "DELETE FROM stopped_away WHERE session_id = ?",
                        Some(vec![sv(id)]),
                    );
                    self.send_runner(
                        runner_id,
                        &FleetToHost::StopSession {
                            session_id: r.session_id.clone(),
                            reason,
                            destroy: false,
                        },
                    );
                }
            }
        }
    }

    /// Whether the fleet stopped the session because `runner_id` stayed away.
    fn stopped_away(&self, session_id: &str, runner_id: &str) -> bool {
        #[derive(Deserialize)]
        struct R {
            runner_id: String,
        }
        self.sql
            .exec(
                "SELECT runner_id FROM stopped_away WHERE session_id = ?",
                Some(vec![sv(session_id)]),
            )
            .and_then(|c| c.to_array::<R>())
            .ok()
            .and_then(|r| r.into_iter().next())
            .is_some_and(|r| r.runner_id == runner_id)
    }

    /// The fleet stopped the session because its runner stayed away, and the
    /// runner is back with the agent still running: the session carries on
    /// there. The host's uplink, which may connect again now, brings what the
    /// agent did meanwhile, and the messages sent since.
    async fn carry_on(&self, rs: &RunnerSocket, r: &HostSessionReport, now: i64) {
        let id = r.session_id.as_str();
        // Resumed meanwhile, but no runner took it: this run gets the messages.
        let _ = self
            .sql
            .exec("DELETE FROM queue WHERE session_id = ?", Some(vec![sv(id)]));
        let _ = self.sql.exec(
            "DELETE FROM stopped_away WHERE session_id = ?",
            Some(vec![sv(id)]),
        );
        let _ = self.sql.exec(
            "INSERT OR REPLACE INTO assignments (session_id, runner_id, assigned_at, accepted) VALUES (?, ?, ?, 1)",
            Some(vec![sv(id), sv(&rs.id), iv(now)]),
        );
        self.set_home(id, &rs.id, now);
        // Uplinks connect only to the sessions of their runner.
        if let Ok(db) = self.env.d1("DB")
            && let Ok(q) = crate::db::q(
                &db,
                "UPDATE sessions SET runner_id = ?, updated_at = ? WHERE id = ?",
                &[crate::db::s(&rs.id), crate::db::n(now), crate::db::s(id)],
            )
            && let Err(e) = q.run().await
        {
            worker::console_error!("carrying {id} on: {}", e.0.message);
        }
        let notice = Notice {
            level: NoticeLevel::Info,
            code: "runner_back".into(),
            message: format!(
                "Runner {} is back with the agent still running, so the session carries on there.",
                rs.name
            ),
            until: None,
            published: None,
            memory: None,
            retrospective: None,
        };
        if let Ok(stub) = internal::session_stub(&self.env, id) {
            let done: std::result::Result<serde_json::Value, _> = internal::post(
                &stub,
                "/control",
                &ControlState {
                    status: Some(r.status),
                    runner: Some(RunnerRef {
                        id: RunnerId(rs.id.clone()),
                        name: rs.name.clone(),
                    }),
                    notice: Some(notice),
                    ..Default::default()
                },
            )
            .await;
            if let Err(e) = done {
                worker::console_error!("carrying {id} on: {}", e.0.message);
            }
        }
        worker::console_log!("{id} carries on: {} is back", rs.name);
    }

    fn assigned_to_runner(&self, runner_id: &str) -> Vec<AssignRow> {
        self.sql
            .exec(
                "SELECT session_id, runner_id, accepted FROM assignments WHERE runner_id = ?",
                Some(vec![sv(runner_id)]),
            )
            .and_then(|c| c.to_array::<AssignRow>())
            .unwrap_or_default()
    }

    /// Stops the sessions `runner_id` runs in workspaces it no longer serves
    /// (it was taken out of one, its owner left it or was disabled), so that
    /// they resume on a runner of their workspace.
    async fn stop_unserved(&self, runner_id: &str) {
        let Ok(db) = self.env.d1("DB") else { return };
        for a in self.assigned_to_runner(runner_id) {
            match spec::runner_serves(&db, runner_id, &a.session_id).await {
                Ok(false) => {}
                Ok(true) => continue,
                Err(e) => {
                    worker::console_error!(
                        "checking {} on {runner_id}: {}",
                        a.session_id,
                        e.0.message
                    );
                    continue;
                }
            }
            self.unserved(runner_id, &a.session_id).await;
            self.send_runner(
                runner_id,
                &FleetToHost::StopSession {
                    session_id: SessionId(a.session_id.clone()),
                    reason: StopReason::Shutdown,
                    destroy: false,
                },
            );
        }
    }

    /// Takes a session from a runner that no longer serves its workspace:
    /// it stops now, whatever the host does, and resumes elsewhere; the
    /// runner's uplink to it closes. Its host is asked to stop the run.
    async fn unserved(&self, runner_id: &str, session_id: &str) {
        let _ = self.sql.exec(
            "DELETE FROM assignments WHERE session_id = ? AND runner_id = ?",
            Some(vec![sv(session_id), sv(runner_id)]),
        );
        let _ = self.sql.exec(
            "DELETE FROM sandboxes WHERE session_id = ? AND runner_id = ?",
            Some(vec![sv(session_id), sv(runner_id)]),
        );
        let _ = self.sql.exec(
            "DELETE FROM stopped_away WHERE session_id = ?",
            Some(vec![sv(session_id)]),
        );
        let id = SessionId(session_id.to_owned());
        if !self.queued(session_id) {
            self.session_control(
                &id,
                Some(SessionStatus::Stopped),
                Some(
                    "Its runner no longer runs this workspace's sessions. A message or a resume starts it on another one."
                        .into(),
                ),
                true,
            )
            .await;
        }
        self.close_uplink(session_id, runner_id).await;
    }

    /// Closes `runner_id`'s uplink to a session.
    async fn close_uplink(&self, session_id: &str, runner_id: &str) {
        if let Ok(stub) = internal::session_stub(&self.env, session_id) {
            let r: std::result::Result<serde_json::Value, _> = internal::post(
                &stub,
                "/close-uplink",
                &internal::CloseUplink {
                    runner_id: runner_id.to_owned(),
                },
            )
            .await;
            if let Err(e) = r {
                worker::console_error!("closing the uplink of {session_id}: {}", e.0.message);
            }
        }
    }

    async fn on_start_result(
        &self,
        runner_id: &str,
        session_id: &SessionId,
        accepted: bool,
        error: Option<String>,
        now: i64,
    ) {
        // Only the runner the session was given answers for it: another
        // one's answer, late or made up, leaves it alone.
        if !self.assigned_to(session_id.as_str(), runner_id) {
            worker::console_log!("ignoring an answer for {session_id} from a runner it isn't on");
            return;
        }
        if accepted {
            let _ = self.sql.exec(
                "UPDATE assignments SET accepted = 1 WHERE session_id = ? AND runner_id = ?",
                Some(vec![sv(session_id.as_str()), sv(runner_id)]),
            );
            self.set_home(session_id.as_str(), runner_id, now);
            // A new run: one left on a runner that was away stops if it comes back.
            let _ = self.sql.exec(
                "DELETE FROM stopped_away WHERE session_id = ?",
                Some(vec![sv(session_id.as_str())]),
            );
            return;
        }
        let _ = self.sql.exec(
            "DELETE FROM assignments WHERE session_id = ?",
            Some(vec![sv(session_id.as_str())]),
        );
        #[derive(Deserialize)]
        struct A {
            attempts: f64,
        }
        let attempts = self
            .sql
            .exec(
                "SELECT attempts FROM queue WHERE session_id = ?",
                Some(vec![sv(session_id.as_str())]),
            )
            .and_then(|c| c.to_array::<A>())
            .ok()
            .and_then(|r| r.first().map(|a| a.attempts as i64))
            .unwrap_or(0);
        let reason = error.unwrap_or_else(|| "The runner declined the session".into());
        if attempts + 1 >= MAX_ATTEMPTS {
            let _ = self.sql.exec(
                "DELETE FROM queue WHERE session_id = ?",
                Some(vec![sv(session_id.as_str())]),
            );
            self.session_control(session_id, Some(SessionStatus::Failed), Some(reason), true)
                .await;
        } else {
            let backoff = 5_000 * (1 << attempts.min(5));
            let _ = self.sql.exec(
                "INSERT INTO queue (session_id, enqueued_at, attempts, not_before) VALUES (?, ?, ?, ?) \
                 ON CONFLICT (session_id) DO UPDATE SET attempts = excluded.attempts, not_before = excluded.not_before",
                Some(vec![sv(session_id.as_str()), iv(now), iv(attempts + 1), iv(now + backoff)]),
            );
            self.session_control(
                session_id,
                Some(SessionStatus::Queued),
                Some(format!("Retrying: {reason}")),
                true,
            )
            .await;
            self.ensure_alarm().await;
        }
    }

    /// Runners offline for longer than the grace period lose their sessions,
    /// which carry on if the runner comes back with them before they run
    /// anywhere else.
    async fn expire_offline_runners(&self, now: i64) {
        let _ = self.sql.exec(
            "DELETE FROM stopped_away WHERE since < ?",
            Some(vec![iv(now - HOME_KEEP_MS)]),
        );
        for r in self.live_rows() {
            let offline_for = now - r.last_seen.map_or(now, |t| t as i64);
            if r.connected > 0.0 || offline_for < RUNNER_GRACE_MS {
                continue;
            }
            let assigned: Vec<AssignRow> = self
                .sql
                .exec(
                    "SELECT session_id, runner_id, accepted FROM assignments WHERE runner_id = ?",
                    Some(vec![sv(&r.id)]),
                )
                .and_then(|c| c.to_array::<AssignRow>())
                .unwrap_or_default();
            for a in assigned {
                let _ = self.sql.exec(
                    "DELETE FROM assignments WHERE session_id = ?",
                    Some(vec![sv(&a.session_id)]),
                );
                // Queued again meanwhile: it starts elsewhere.
                if self.queued(&a.session_id) {
                    continue;
                }
                let _ = self.sql.exec(
                    "INSERT OR REPLACE INTO stopped_away (session_id, runner_id, since) VALUES (?, ?, ?)",
                    Some(vec![sv(&a.session_id), sv(&r.id), iv(now)]),
                );
                self.session_control(
                    &SessionId(a.session_id),
                    Some(SessionStatus::Stopped),
                    Some(format!(
                        "Runner {} has been offline for 10 minutes. If it comes back with the agent still running, the session carries on there; a message resumes it on another runner.",
                        r.name
                    )),
                    true,
                )
                .await;
            }
        }
    }

    // ───────────────────────────── draining ─────────────────────────────

    /// Drains a runner (no new session starts on it; the sessions it has
    /// carry on there), or stops draining it.
    async fn drain(&self, b: Drain) -> std::result::Result<serde_json::Value, ApiError> {
        let now = now_ms();
        if b.draining {
            let by = b.by.as_ref().and_then(|u| serde_json::to_string(u).ok());
            // Draining again keeps when it started, and who started it.
            let _ = self.sql.exec(
                "INSERT INTO runners (id, name, connected, draining_since, drained_by) VALUES (?, ?, 0, ?, ?) \
                 ON CONFLICT (id) DO UPDATE SET \
                 drained_by = CASE WHEN runners.draining_since IS NULL THEN excluded.drained_by ELSE runners.drained_by END, \
                 draining_since = COALESCE(runners.draining_since, excluded.draining_since)",
                Some(vec![
                    sv(&b.runner_id),
                    sv(&b.runner_name),
                    iv(now),
                    SqlStorageValue::from(by),
                ]),
            );
            if b.stop_sessions {
                self.stop_all(&b.runner_id, &b.runner_name).await;
            } else {
                let reported = self.reported_sessions(&b.runner_id);
                self.stop_idle_if_draining(&b.runner_id, &reported).await;
            }
        } else {
            let _ = self.sql.exec(
                "UPDATE runners SET draining_since = NULL, drained_by = NULL WHERE id = ?",
                Some(vec![sv(&b.runner_id)]),
            );
        }
        self.feed_runner(&b.runner_id).await;
        self.dispatch().await;
        Ok(serde_json::json!({ "ok": true }))
    }

    /// Its host is being uninstalled: every session on the runner stops
    /// (their agents push their work), and from now on the sessions whose
    /// sandboxes it has resume on other runners. One whose runner is
    /// offline stops here and now: nothing can reach its host.
    async fn stop_all(&self, runner_id: &str, runner_name: &str) {
        let _ = self.sql.exec(
            "DELETE FROM sandboxes WHERE runner_id = ?",
            Some(vec![sv(runner_id)]),
        );
        let _ = self.sql.exec(
            "DELETE FROM stopped_away WHERE runner_id = ?",
            Some(vec![sv(runner_id)]),
        );
        let assigned: Vec<AssignRow> = self
            .sql
            .exec(
                "SELECT session_id, runner_id, accepted FROM assignments WHERE runner_id = ?",
                Some(vec![sv(runner_id)]),
            )
            .and_then(|c| c.to_array::<AssignRow>())
            .unwrap_or_default();
        let online = self.runner_socket(runner_id).is_some();
        for a in assigned {
            let session_id = SessionId(a.session_id);
            if online {
                // Stopping first, so that messages wait for the next run.
                self.session_control(
                    &session_id,
                    Some(SessionStatus::Stopping),
                    Some(format!("Stopping: {runner_name} is being uninstalled")),
                    false,
                )
                .await;
                self.send_runner(
                    runner_id,
                    &FleetToHost::StopSession {
                        session_id,
                        reason: StopReason::Shutdown,
                        destroy: false,
                    },
                );
            } else {
                let _ = self.sql.exec(
                    "DELETE FROM assignments WHERE session_id = ?",
                    Some(vec![sv(session_id.as_str())]),
                );
                if !self.queued(session_id.as_str()) {
                    self.session_control(
                        &session_id,
                        Some(SessionStatus::Stopped),
                        Some(format!(
                            "Stopped: {runner_name} was uninstalled. A message resumes the session on another runner."
                        )),
                        true,
                    )
                    .await;
                }
            }
        }
    }

    /// Whether a runner's free disk was below what its host keeps at its
    /// last heartbeat, and what its host keeps (MiB).
    fn low_on_disk(&self, runner_id: &str) -> (bool, Option<u64>) {
        #[derive(Deserialize)]
        struct R {
            hello: Option<String>,
            #[serde(default)]
            disk_available_mb: Option<f64>,
        }
        #[derive(Deserialize)]
        struct Floor {
            #[serde(default)]
            min_free_disk_mb: Option<u64>,
        }
        let Some(r) = self
            .sql
            .exec(
                "SELECT hello, disk_available_mb FROM runners WHERE id = ?",
                Some(vec![sv(runner_id)]),
            )
            .and_then(|c| c.to_array::<R>())
            .ok()
            .and_then(|r| r.into_iter().next())
        else {
            return (false, None);
        };
        let min_free = r
            .hello
            .as_deref()
            .and_then(|h| serde_json::from_str::<Floor>(h).ok())
            .and_then(|f| f.min_free_disk_mb);
        let free = r.disk_available_mb.map(|d| d as u64);
        (fctl_domain::disk::low(free, min_free), min_free)
    }

    /// The sessions a runner reported last.
    fn reported_sessions(&self, runner_id: &str) -> Vec<HostSessionReport> {
        #[derive(Deserialize)]
        struct R {
            sessions: Option<String>,
        }
        self.sql
            .exec(
                "SELECT sessions FROM runners WHERE id = ?",
                Some(vec![sv(runner_id)]),
            )
            .and_then(|c| c.to_array::<R>())
            .ok()
            .and_then(|r| r.into_iter().next())
            .and_then(|r| r.sessions)
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default()
    }

    /// On a draining runner, the sessions that wait for a message stop once
    /// they have waited a minute (SessionObject decides, so that none stops
    /// with a message on its way), rather than after their idle timeout. A
    /// message resumes them there.
    async fn stop_idle_if_draining(&self, runner_id: &str, reported: &[HostSessionReport]) {
        #[derive(Deserialize)]
        struct R {
            name: String,
        }
        let Some(name) = self
            .sql
            .exec(
                "SELECT name FROM runners WHERE id = ? AND draining_since IS NOT NULL",
                Some(vec![sv(runner_id)]),
            )
            .and_then(|c| c.to_array::<R>())
            .ok()
            .and_then(|r| r.into_iter().next())
            .map(|r| r.name)
        else {
            return;
        };
        let assigned: HashSet<String> = self
            .sql
            .exec(
                "SELECT session_id, runner_id, accepted FROM assignments WHERE runner_id = ?",
                Some(vec![sv(runner_id)]),
            )
            .and_then(|c| c.to_array::<AssignRow>())
            .unwrap_or_default()
            .into_iter()
            .map(|a| a.session_id)
            .collect();
        for r in reported {
            if r.status != SessionStatus::Idle || !assigned.contains(r.session_id.as_str()) {
                continue;
            }
            let Ok(stub) = internal::session_stub(&self.env, r.session_id.as_str()) else {
                continue;
            };
            let asked: std::result::Result<DrainStopped, _> = internal::post(
                &stub,
                "/drain-stop",
                &DrainStop {
                    runner_name: name.clone(),
                },
            )
            .await;
            if asked.is_ok_and(|a| a.stopping) {
                self.send_runner(
                    runner_id,
                    &FleetToHost::StopSession {
                        session_id: r.session_id.clone(),
                        reason: StopReason::Idle,
                        destroy: false,
                    },
                );
            }
        }
    }

    // ───────────────────────────── scheduling ─────────────────────────────

    async fn schedule(&self, b: Schedule) -> std::result::Result<serde_json::Value, ApiError> {
        let now = now_ms();
        let _ = self.sql.exec(
            "INSERT OR REPLACE INTO queue (session_id, enqueued_at, attempts, not_before) VALUES (?, ?, 0, 0)",
            Some(vec![sv(b.session_id.as_str()), iv(now)]),
        );
        self.dispatch().await;
        let queued = self
            .sql
            .exec(
                "SELECT session_id, attempts, reason FROM queue WHERE session_id = ?",
                Some(vec![sv(b.session_id.as_str())]),
            )
            .and_then(|c| c.to_array::<QueueRow>())
            .ok()
            .and_then(|r| r.into_iter().next());
        let still_queued = queued.is_some();
        if let Some(item) = queued {
            // Dispatch says why it waits; one already running skipped it.
            if item.reason.is_none() {
                self.note_waiting(&item, Wait::NoRunner.reason()).await;
            }
            self.ensure_alarm().await;
        }
        Ok(serde_json::json!({ "ok": true, "queued": still_queued }))
    }

    /// Shows why a queued session waits, when that changed.
    async fn note_waiting(&self, item: &QueueRow, reason: String) {
        if item.reason.as_deref() == Some(reason.as_str()) {
            return;
        }
        let _ = self.sql.exec(
            "UPDATE queue SET reason = ? WHERE session_id = ?",
            Some(vec![sv(&reason), sv(&item.session_id)]),
        );
        self.session_control(
            &SessionId(item.session_id.clone()),
            Some(SessionStatus::Queued),
            Some(reason),
            false,
        )
        .await;
    }

    async fn dispatch(&self) {
        if *self.dispatching.borrow() {
            return;
        }
        *self.dispatching.borrow_mut() = true;
        self.dispatch_inner().await;
        *self.dispatching.borrow_mut() = false;
    }

    async fn dispatch_inner(&self) {
        let now = now_ms();
        let queued: Vec<QueueRow> = self
            .sql
            .exec("SELECT session_id, attempts, reason, enqueued_at FROM queue WHERE not_before <= ? ORDER BY enqueued_at ASC LIMIT 50", Some(vec![iv(now)]))
            .and_then(|c| c.to_array::<QueueRow>())
            .unwrap_or_default();
        if queued.is_empty() {
            return;
        }
        let Ok(db) = self.env.d1("DB") else { return };
        let cfg = crate::config::Config::from_env(&self.env);
        let Ok(keys) = crypto::keys(&cfg) else {
            worker::console_error!("FCTL_MASTER_KEY is missing: sessions cannot start");
            return;
        };
        // What the sessions ask for, and the workspaces runners serve, read
        // when some runner could take one.
        let wants = if self.candidates().is_empty() {
            None
        } else {
            let ids: Vec<&str> = queued.iter().map(|q| q.session_id.as_str()).collect();
            match spec::session_wants(&db, &ids).await {
                Ok(w) => {
                    self.load_runner_workspaces(&db).await;
                    Some(w)
                }
                Err(e) => {
                    worker::console_error!("reading queued sessions' sizes: {}", e.0.message);
                    return;
                }
            }
        };
        for item in queued {
            // A session that cannot start now says why and lets the ones
            // behind it try: a smaller one may fit where it does not.
            let Some(wants) = &wants else {
                self.note_waiting(&item, Wait::NoRunner.reason()).await;
                continue;
            };
            // Two runs of a session never overlap: a resume while the last
            // run still stops starts once it has.
            if self.still_stopping(&item.session_id) {
                self.note_waiting(&item, "Starting again once the last run has stopped".into())
                    .await;
                continue;
            }
            let want = wants.get(&item.session_id).cloned().unwrap_or_default();
            // A resume goes back to the runner that has its sandbox, where
            // Claude continues the same conversation, and waits a while for
            // it when it is busy. Not after it declined the session.
            let home = if item.attempts > 0.0 {
                None
            } else {
                self.home_of(&item.session_id)
            };
            let wait = now - (item.enqueued_at as i64) < HOME_WAIT_MS;
            let picked = self.pick_runner(&want, home.as_deref(), wait);
            let (runner_id, runner_name) = match picked {
                Ok(r) => r,
                Err(Wait::Workspace) => {
                    self.note_waiting(&item, self.no_runner_takes(&want)).await;
                    continue;
                }
                Err(wait) => {
                    self.note_waiting(&item, wait.reason()).await;
                    continue;
                }
            };
            let session_id = SessionId(item.session_id.clone());
            let built = spec::build(&db, &cfg, &keys, &item.session_id, now).await;
            let spec = match built {
                Ok(mut s) => {
                    s.run_id = Some(fctl_domain::ids::new_id("run", now));
                    s
                }
                Err(e) => {
                    let _ = self.sql.exec(
                        "DELETE FROM queue WHERE session_id = ?",
                        Some(vec![sv(&item.session_id)]),
                    );
                    let retryable =
                        e.0.code == ErrorCode::Unavailable && (item.attempts as i64) < MAX_ATTEMPTS;
                    if retryable {
                        let _ = self.sql.exec(
                            "INSERT INTO queue (session_id, enqueued_at, attempts, not_before) VALUES (?, ?, ?, ?)",
                            Some(vec![sv(&item.session_id), iv(now), iv(item.attempts as i64 + 1), iv(now + 15_000)]),
                        );
                        self.session_control(
                            &session_id,
                            Some(SessionStatus::Queued),
                            Some(format!("Retrying: {}", e.0.message)),
                            false,
                        )
                        .await;
                    } else {
                        self.session_control(
                            &session_id,
                            Some(SessionStatus::Failed),
                            Some(e.0.message),
                            true,
                        )
                        .await;
                    }
                    continue;
                }
            };
            // Record the assignment before the host can open its uplink.
            if let Ok(q) = crate::db::q(
                &db,
                "UPDATE sessions SET runner_id = ?, updated_at = ? WHERE id = ?",
                &[
                    crate::db::s(&runner_id),
                    crate::db::n(now),
                    crate::db::s(&item.session_id),
                ],
            ) {
                let _ = q.run().await;
            }
            let _ = self.sql.exec(
                "DELETE FROM queue WHERE session_id = ?",
                Some(vec![sv(&item.session_id)]),
            );
            let run_id = spec.run_id.clone().unwrap_or_default();
            let _ = self.sql.exec(
                "INSERT OR REPLACE INTO assignments (session_id, runner_id, assigned_at, accepted, run_id) VALUES (?, ?, ?, 0, ?)",
                Some(vec![sv(&item.session_id), sv(&runner_id), iv(now), sv(&run_id)]),
            );
            let sent = self.send_runner(
                &runner_id,
                &FleetToHost::StartSession {
                    spec: Box::new(spec),
                },
            );
            if sent {
                let stub = internal::session_stub(&self.env, &item.session_id);
                if let Ok(stub) = stub {
                    let _: std::result::Result<serde_json::Value, _> = internal::post(
                        &stub,
                        "/control",
                        &ControlState {
                            status: Some(SessionStatus::Provisioning),
                            reason: Some(format!("Starting on {runner_name}")),
                            runner: Some(RunnerRef {
                                id: RunnerId(runner_id.clone()),
                                name: runner_name.clone(),
                            }),
                            ..Default::default()
                        },
                    )
                    .await;
                }
            } else {
                let _ = self.sql.exec(
                    "DELETE FROM assignments WHERE session_id = ?",
                    Some(vec![sv(&item.session_id)]),
                );
                let _ = self.sql.exec(
                    "INSERT OR REPLACE INTO queue (session_id, enqueued_at, attempts, not_before) VALUES (?, ?, ?, ?)",
                    Some(vec![sv(&item.session_id), iv(now), iv(item.attempts as i64), iv(now + 5_000)]),
                );
            }
        }
    }

    /// Connected runners that said hello, with their assigned sessions and
    /// the workspaces they serve (as D1 had them at the last dispatch).
    fn candidates(&self) -> Vec<Candidate> {
        #[derive(Deserialize)]
        struct C {
            runner_id: String,
            n: f64,
        }
        let active: Vec<C> = self
            .sql
            .exec(
                "SELECT runner_id, COUNT(*) AS n FROM assignments GROUP BY runner_id",
                None,
            )
            .and_then(|c| c.to_array::<C>())
            .unwrap_or_default();
        self.live_rows()
            .into_iter()
            .filter(|r| r.connected > 0.0 && self.runner_socket(&r.id).is_some())
            .filter_map(|r| {
                let hello: HostHello = serde_json::from_str(r.hello.as_deref()?).ok()?;
                let n = active
                    .iter()
                    .find(|c| c.runner_id == r.id)
                    .map_or(0, |c| c.n as u32);
                let serves = self
                    .runner_workspaces
                    .borrow()
                    .get(&r.id)
                    .cloned()
                    .unwrap_or_default();
                Some(Candidate {
                    id: r.id,
                    name: r.name,
                    capacity: hello.capacity,
                    active: n,
                    labels: hello.labels,
                    sandbox: hello.sandbox,
                    draining: r.draining_since.is_some(),
                    low_disk: fctl_domain::disk::low(
                        r.disk_available_mb.map(|d| d as u64),
                        hello.min_free_disk_mb,
                    ),
                    image: hello.image,
                    allowed_images: hello.allowed_images,
                    allowed_builds: hello.allowed_builds,
                    serves,
                })
            })
            .collect()
    }

    /// The runner that last took the session, which keeps its sandbox.
    fn home_of(&self, session_id: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct H {
            runner_id: String,
        }
        self.sql
            .exec(
                "SELECT runner_id FROM sandboxes WHERE session_id = ?",
                Some(vec![sv(session_id)]),
            )
            .and_then(|c| c.to_array::<H>())
            .ok()?
            .into_iter()
            .next()
            .map(|h| h.runner_id)
    }

    /// Whether the fleet gave the session to `runner_id`.
    fn assigned_to(&self, session_id: &str, runner_id: &str) -> bool {
        self.sql
            .exec(
                "SELECT session_id, runner_id, accepted FROM assignments WHERE session_id = ? AND runner_id = ?",
                Some(vec![sv(session_id), sv(runner_id)]),
            )
            .and_then(|c| c.to_array::<AssignRow>())
            .is_ok_and(|r| !r.is_empty())
    }

    /// Remembers that `runner_id` took the session, and forgets the runners
    /// of sessions that have not run for a long time.
    fn set_home(&self, session_id: &str, runner_id: &str, now: i64) {
        let _ = self.sql.exec(
            "INSERT INTO sandboxes (session_id, runner_id, since) VALUES (?, ?, ?) \
             ON CONFLICT (session_id) DO UPDATE SET runner_id = excluded.runner_id, since = excluded.since",
            Some(vec![sv(session_id), sv(runner_id), iv(now)]),
        );
        let _ = self.sql.exec(
            "DELETE FROM sandboxes WHERE since < ?",
            Some(vec![iv(now - HOME_KEEP_MS)]),
        );
    }

    /// Whether the session's last run still stops: the session is still
    /// assigned to a connected runner that reports it.
    fn still_stopping(&self, session_id: &str) -> bool {
        #[derive(Deserialize)]
        struct R {
            sessions: Option<String>,
        }
        self.sql
            .exec(
                "SELECT r.sessions AS sessions FROM assignments a JOIN runners r ON r.id = a.runner_id \
                 WHERE a.session_id = ? AND r.connected = 1",
                Some(vec![sv(session_id)]),
            )
            .and_then(|c| c.to_array::<R>())
            .unwrap_or_default()
            .iter()
            .filter_map(|r| r.sessions.as_deref())
            .filter_map(|j| serde_json::from_str::<Vec<HostSessionReport>>(j).ok())
            .any(|reports| reports.iter().any(|r| r.session_id.as_str() == session_id))
    }

    /// The least loaded connected runner with a free slot that takes the
    /// session, runs its image and gives its sandbox what it wants, or why
    /// there is none. `home`, the runner that has the session's sandbox,
    /// comes first, and with `wait` the session waits for it while it is
    /// busy.
    fn pick_runner(
        &self,
        want: &spec::SessionWants,
        home: Option<&str>,
        wait: bool,
    ) -> std::result::Result<(String, String), Wait> {
        let candidates = self.candidates();
        let slots: Vec<Slot<'_>> = candidates.iter().map(|c| c.slot(want)).collect();
        let home = home
            .and_then(|id| candidates.iter().find(|c| c.id == id))
            .map(|c| Home {
                id: &c.id,
                name: &c.name,
                wait,
            });
        let chosen = fctl_domain::schedule::pick(&slots, &[], want.size, &want.image, home)?;
        let name = candidates
            .iter()
            .find(|c| c.id == chosen.id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        Ok((chosen.id.to_owned(), name))
    }

    async fn stop(&self, b: Stop) -> std::result::Result<serde_json::Value, ApiError> {
        // Stopped by a person, or deleted: it doesn't carry on when a runner
        // that was away comes back with it.
        let _ = self.sql.exec(
            "DELETE FROM stopped_away WHERE session_id = ?",
            Some(vec![sv(b.session_id.as_str())]),
        );
        let removed_from_queue = self
            .sql
            .exec(
                "DELETE FROM queue WHERE session_id = ? RETURNING session_id",
                Some(vec![sv(b.session_id.as_str())]),
            )
            .and_then(|c| c.to_array::<serde_json::Value>())
            .map(|r| !r.is_empty())
            .unwrap_or(false);
        let assigned: Vec<AssignRow> = self
            .sql
            .exec(
                "SELECT session_id, runner_id, accepted FROM assignments WHERE session_id = ?",
                Some(vec![sv(b.session_id.as_str())]),
            )
            .and_then(|c| c.to_array::<AssignRow>())
            .unwrap_or_default();
        match assigned.first() {
            Some(a) if self.runner_socket(&a.runner_id).is_some() => {
                // Stopping first: from then on the session keeps messages for
                // its next run instead of handing them to an agent that stops.
                self.session_control(
                    &b.session_id,
                    Some(SessionStatus::Stopping),
                    Some("Stopping".into()),
                    false,
                )
                .await;
                self.send_runner(
                    &a.runner_id,
                    &FleetToHost::StopSession {
                        session_id: b.session_id.clone(),
                        reason: b.reason,
                        destroy: b.destroy,
                    },
                );
            }
            Some(_) => {
                // The runner is offline; it stops the sandbox when it reconnects.
                let _ = self.sql.exec(
                    "DELETE FROM assignments WHERE session_id = ?",
                    Some(vec![sv(b.session_id.as_str())]),
                );
                self.session_control(
                    &b.session_id,
                    Some(SessionStatus::Stopped),
                    Some("Stopped".into()),
                    true,
                )
                .await;
            }
            None => {
                let reason = if removed_from_queue {
                    "Stopped before it started"
                } else {
                    "Stopped"
                };
                self.session_control(
                    &b.session_id,
                    Some(SessionStatus::Stopped),
                    Some(reason.into()),
                    true,
                )
                .await;
            }
        }
        Ok(serde_json::json!({ "ok": true }))
    }

    /// Updates a session's status through its SessionObject (which writes D1).
    async fn session_control(
        &self,
        session_id: &SessionId,
        status: Option<SessionStatus>,
        reason: Option<String>,
        clear_runner: bool,
    ) {
        if let Ok(stub) = internal::session_stub(&self.env, session_id.as_str()) {
            let r: std::result::Result<serde_json::Value, _> = internal::post(
                &stub,
                "/control",
                &ControlState {
                    status,
                    reason,
                    clear_runner,
                    ..Default::default()
                },
            )
            .await;
            if let Err(e) = r {
                worker::console_error!("session control failed: {}", e.0.message);
            }
        }
        if clear_runner
            && let Ok(db) = self.env.d1("DB")
            && let Ok(q) = crate::db::q(
                &db,
                "UPDATE sessions SET runner_id = NULL WHERE id = ?",
                &[crate::db::s(session_id.as_str())],
            )
        {
            let _ = q.run().await;
        }
    }

    // ───────────────────────────── credentials ─────────────────────────────

    /// A session's credentials, for the runner it runs on (`runner_id`)
    /// only: as its uplink, another runner gets nothing, so that nobody's
    /// runner gets the GitHub tokens or Claude credential of sessions in
    /// workspaces it doesn't serve.
    async fn credential(
        &self,
        runner_id: &str,
        session_id: &SessionId,
        kind: CredentialKind,
        now: i64,
    ) -> std::result::Result<Credentials, ApiError> {
        let db = self
            .env
            .d1("DB")
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let cfg = crate::config::Config::from_env(&self.env);
        let row = spec::session_row(&db, session_id.as_str())
            .await
            .map_err(|e| e.0)?
            .ok_or_else(|| ApiError::not_found("Session"))?;
        if row.runner_id.as_deref() != Some(runner_id) {
            return Err(ApiError::forbidden(
                "This session is not assigned to this runner",
            ));
        }
        // Nor once the runner stopped serving its workspace, whatever its
        // host says about the session.
        if !spec::runner_serves(&db, runner_id, session_id.as_str())
            .await
            .map_err(|e| e.0)?
        {
            return Err(spec::not_served().0);
        }
        match kind {
            CredentialKind::Github => {
                let project = spec::project_row(&db, &row.project_id)
                    .await
                    .map_err(|e| e.0)?
                    .ok_or_else(|| ApiError::not_found("Project"))?;
                let grants = spec::grants(&db, &project.id).await.map_err(|e| e.0)?;
                let tokens = spec::github_tokens(
                    &cfg,
                    &db,
                    project.workspace_id(),
                    project.repo().as_ref(),
                    &grants,
                    now,
                )
                .await
                .map_err(|e| e.0)?;
                if tokens.write.is_none() && tokens.read.is_none() {
                    return Err(ApiError::new(
                        ErrorCode::NotConfigured,
                        "The project has no repositories",
                    ));
                }
                Ok(Credentials {
                    claude: None,
                    github: tokens.write,
                    github_read: tokens.read,
                })
            }
            CredentialKind::Claude => {
                let keys = crypto::keys(&cfg).map_err(|e| e.0)?;
                let cred_id = row.credential_id.clone().ok_or_else(|| {
                    ApiError::new(ErrorCode::NotConfigured, "No Claude credential")
                })?;
                // Nor once its owner left the session's workspace.
                if !spec::credential_serves(&db, &cred_id, session_id.as_str())
                    .await
                    .map_err(|e| e.0)?
                {
                    return Err(spec::credential_not_lent().0);
                }
                // One refresh per credential at a time; others wait for it.
                for _ in 0..80 {
                    if !self.refreshing.borrow().contains(&cred_id) {
                        break;
                    }
                    worker::Delay::from(std::time::Duration::from_millis(250)).await;
                }
                self.refreshing.borrow_mut().insert(cred_id.clone());
                let result = spec::claude_credential(&db, &keys, &cred_id, now).await;
                self.refreshing.borrow_mut().remove(&cred_id);
                Ok(Credentials {
                    claude: Some(result.map_err(|e| e.0)?),
                    github: None,
                    github_read: None,
                })
            }
        }
    }

    // ───────────────────────────── Claude sign-in ─────────────────────────────

    fn login_start(&self, b: LoginStart) -> std::result::Result<serde_json::Value, ApiError> {
        let online = self.live_runners().online;
        // The first online of the runners that may run it, in their order
        // (the person's own first): a login passes its tokens through the
        // host.
        let picked = match &b.runners {
            Some(allowed) => allowed.iter().find(|r| online.contains(r)),
            None => online.first(),
        };
        let Some(runner_id) = picked.cloned() else {
            return Ok(serde_json::json!({ "ok": false }));
        };
        let _ = self.sql.exec(
            "INSERT OR REPLACE INTO logins (job_id, runner_id, user_id, created_at) VALUES (?, ?, ?, ?)",
            Some(vec![sv(b.job_id.as_str()), sv(&runner_id), sv(b.user.id.as_str()), iv(now_ms())]),
        );
        let sent = self.send_runner(
            &runner_id,
            &FleetToHost::ClaudeLoginStart {
                job_id: b.job_id,
                user: b.user,
            },
        );
        Ok(serde_json::json!({ "ok": sent }))
    }

    fn login_row(&self, job_id: &str) -> Option<LoginRow> {
        self.sql
            .exec(
                "SELECT job_id, runner_id, user_id FROM logins WHERE job_id = ?",
                Some(vec![sv(job_id)]),
            )
            .and_then(|c| c.to_array::<LoginRow>())
            .ok()?
            .into_iter()
            .next()
    }

    fn login_code(&self, b: LoginCode) -> std::result::Result<serde_json::Value, ApiError> {
        let row = self
            .login_row(b.job_id.as_str())
            .ok_or_else(|| ApiError::not_found("Sign-in"))?;
        let runner = row.runner_id.ok_or_else(|| {
            ApiError::new(
                ErrorCode::Unavailable,
                "The sign-in lost its runner; start again",
            )
        })?;
        if !self.send_runner(
            &runner,
            &FleetToHost::ClaudeLoginCode {
                job_id: b.job_id,
                code: b.code,
            },
        ) {
            return Err(ApiError::new(
                ErrorCode::Unavailable,
                "The runner went offline; start the sign-in again",
            ));
        }
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn on_login_update(
        &self,
        runner_id: &str,
        job_id: &LoginJobId,
        update: ClaudeLoginUpdate,
        now: i64,
    ) {
        // Only from the runner the sign-in was sent to.
        let Some(row) = self
            .login_row(job_id.as_str())
            .filter(|r| r.runner_id.as_deref() == Some(runner_id))
        else {
            return;
        };
        let Ok(db) = self.env.d1("DB") else { return };
        let state = match update {
            ClaudeLoginUpdate::Starting => ClaudeLoginState::Starting,
            // The host may be another member's: only Claude's own page is
            // shown and opened as the place to approve the sign-in.
            ClaudeLoginUpdate::AwaitingCode { url }
                if fctl_domain::remote_control::is_login_url(&url) =>
            {
                ClaudeLoginState::AwaitingCode { url }
            }
            ClaudeLoginUpdate::AwaitingCode { .. } => ClaudeLoginState::Failed {
                error: "The runner sent a sign-in link that isn't on claude.ai. Start the sign-in again, or connect a long-lived token instead.".into(),
            },
            ClaudeLoginUpdate::Verifying => ClaudeLoginState::Verifying,
            ClaudeLoginUpdate::Failed { error } => ClaudeLoginState::Failed { error },
            ClaudeLoginUpdate::Succeeded { bundle } => {
                match self.store_login(&db, &row.user_id, *bundle, now).await {
                    Ok(credential_id) => ClaudeLoginState::Succeeded { credential_id },
                    Err(e) => ClaudeLoginState::Failed { error: e.0.message },
                }
            }
        };
        if matches!(
            state,
            ClaudeLoginState::Succeeded { .. } | ClaudeLoginState::Failed { .. }
        ) {
            let _ = self.sql.exec(
                "DELETE FROM logins WHERE job_id = ?",
                Some(vec![sv(job_id.as_str())]),
            );
        }
        let text = serde_json::to_string(&state).unwrap_or_default();
        if let Ok(q) = crate::db::q(
            &db,
            "UPDATE claude_login_jobs SET state = ?, runner_id = ?, updated_at = ? WHERE id = ?",
            &[
                crate::db::s(&text),
                crate::db::os(row.runner_id.as_deref()),
                crate::db::n(now),
                crate::db::s(job_id.as_str()),
            ],
        ) {
            let _ = q.run().await;
        }
        let job: Option<crate::api::claude::JobRow> = match crate::db::q(
            &db,
            "SELECT * FROM claude_login_jobs WHERE id = ?",
            &[crate::db::s(job_id.as_str())],
        ) {
            Ok(q) => q.first().await.ok().flatten(),
            Err(_) => None,
        };
        if let Some(job) = job {
            self.feed_login(&row.user_id, job.to_job());
        }
    }

    async fn store_login(
        &self,
        db: &worker::D1Database,
        user_id: &str,
        bundle: fctl_proto::runner::ClaudeOauthBundle,
        now: i64,
    ) -> crate::error::AppResult<fctl_proto::CredentialId> {
        let cfg = crate::config::Config::from_env(&self.env);
        let keys = crypto::keys(&cfg)?;
        let id = fctl_domain::ids::credential_id(now);
        let secret = crate::model::CredentialSecret {
            access_token: bundle.access_token,
            refresh_token: Some(bundle.refresh_token),
            expires_at: bundle.expires_at,
        };
        let sealed = keys.seal_json(
            &crate::model::CredentialRow::aad(id.as_str(), user_id),
            &secret,
        )?;
        let account = bundle
            .account
            .as_ref()
            .and_then(|a| serde_json::to_string(a).ok());
        let email = bundle.account.as_ref().and_then(|a| a.email.clone());
        crate::db::q(
            db,
            "INSERT INTO claude_credentials (id, user_id, kind, label, secret_enc, scopes, account_email, subscription_type, account, status, expires_at, created_at, updated_at) \
             VALUES (?, ?, 'oauth', ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?)",
            &[
                crate::db::s(id.as_str()),
                crate::db::s(user_id),
                crate::db::s("Claude login"),
                crate::db::s(&sealed),
                crate::db::s(&bundle.scopes.join(" ")),
                crate::db::os(email.as_deref()),
                crate::db::os(bundle.subscription_type.as_deref()),
                crate::db::os(account.as_deref()),
                crate::db::on(secret.expires_at),
                crate::db::n(now),
                crate::db::n(now),
            ],
        )?
        .run()
        .await?;
        crate::db::q(
            db,
            "UPDATE users SET default_credential_id = ? WHERE id = ?",
            &[crate::db::s(id.as_str()), crate::db::s(user_id)],
        )?
        .run()
        .await?;
        Ok(id)
    }

    // ───────────────────────────── feed ─────────────────────────────

    /// The feeds with who they are for. Those whose access wasn't checked
    /// for [`FEED_STALE_MS`] close instead: their person may have lost it
    /// meanwhile.
    fn feed_sockets(&self) -> Vec<(WebSocket, FeedIdentity)> {
        let now = now_ms();
        self.state
            .get_websockets_with_tag(TAG_FEED)
            .into_iter()
            .filter_map(|ws| {
                let ident = ws.deserialize_attachment::<FeedIdentity>().ok().flatten()?;
                if now - ident.checked_at.unwrap_or_default() > FEED_STALE_MS {
                    let _ = ws.close(Some(CLOSE_RECHECK), Some("checking your access again"));
                    return None;
                }
                Some((ws, ident))
            })
            .collect()
    }

    /// Checks again who an open feed is for, as the Worker did when it
    /// opened: its token still signs its person in, and it shows what they
    /// see now (`crate::workspaces::feed_identity`). A feed whose token no
    /// longer works closes; one whose view changed is told, as when their
    /// workspaces change. A check D1 doesn't answer is left for the next.
    async fn recheck_feed(&self, ws: &WebSocket, ident: FeedIdentity, now: i64) {
        // Opened through an older Worker: through this one, it says.
        let Some(token_id) = ident.token_id.clone() else {
            let _ = ws.close(Some(CLOSE_RECHECK), Some("checking your access again"));
            return;
        };
        let Ok(db) = self.env.d1("DB") else { return };
        let user = match crate::auth::by_token_id(&db, &token_id, now).await {
            Ok(Some((user, settings))) if user.id == ident.user_id => Some((user, settings)),
            Ok(_) => None,
            Err(e) => {
                worker::console_error!("checking a feed: {}", e.0.message);
                return;
            }
        };
        let Some((user, settings)) = user else {
            let _ = ws.close(Some(CLOSE_NO_ACCESS), Some("signed out"));
            return;
        };
        let mut fresh = match crate::workspaces::feed_identity(&db, &user, &settings).await {
            Ok(i) => i,
            Err(e) => {
                worker::console_error!("feed identity: {}", e.0.message);
                return;
            }
        };
        fresh.token_id = Some(token_id);
        fresh.checked_at = Some(now);
        let _ = ws.serialize_attachment(&fresh);
        if !fresh.same_view(&ident) {
            for text in self.view_changed(&fresh).await {
                let _ = ws.send_with_str(&text);
            }
        }
    }

    /// What tells a feed what its person now sees: the runners, and that
    /// their workspaces changed, after which the app fetches them again.
    async fn view_changed(&self, ident: &FeedIdentity) -> Vec<String> {
        let runners = self.runners_for(ident).await.unwrap_or_default();
        [
            serde_json::to_string(&FeedServer::Hello {
                runners,
                server_time: now_ms(),
            }),
            serde_json::to_string(&FeedServer::Workspaces),
        ]
        .into_iter()
        .filter_map(std::result::Result::ok)
        .collect()
    }

    fn feed_session(&self, s: &fctl_proto::session::Session) {
        let workspace = s.workspace.as_ref().map(|w| w.id.as_str());
        for (ws, ident) in self.feed_sockets() {
            if ident.sees(s.project_id.as_str(), workspace, s.created_by.id.as_str()) {
                let mut copy = s.clone();
                copy.role = None;
                if let Ok(text) = serde_json::to_string(&FeedServer::Session {
                    session: Box::new(copy),
                }) {
                    let _ = ws.send_with_str(&text);
                }
            }
        }
    }

    fn feed_activity(&self, a: Activity) {
        let workspace = a.workspace_id.clone();
        let msg = FeedServer::Activity {
            session_id: a.session_id,
            project_id: a.project_id,
            kind: a.kind,
            weight: a.weight,
        };
        let Ok(text) = serde_json::to_string(&msg) else {
            return;
        };
        let project = match &msg {
            FeedServer::Activity { project_id, .. } => project_id.0.clone(),
            _ => String::new(),
        };
        for (ws, ident) in self.feed_sockets() {
            if ident.sees(&project, workspace.as_deref(), "") {
                let _ = ws.send_with_str(&text);
            }
        }
    }

    async fn feed_runner(&self, runner_id: &str) {
        let Ok(rows) = self.runner_rows().await else {
            return;
        };
        let Some(row) = rows.into_iter().find(|r| r.id == runner_id) else {
            return;
        };
        let live = self.live_runners();
        for (ws, ident) in self.feed_sockets() {
            let view = feed_view(&ident);
            if !row.visible_to(&view) {
                continue;
            }
            if let Ok(text) = serde_json::to_string(&FeedServer::Runner {
                runner: Box::new(row.to_runner(&live, &view)),
            }) {
                let _ = ws.send_with_str(&text);
            }
        }
    }

    /// Why a session waits that no online runner takes
    /// ([`Wait::Workspace`]), by the runners, online or not, of its
    /// workspace (as of the last dispatch).
    fn no_runner_takes(&self, want: &spec::SessionWants) -> String {
        let runners = self.runner_workspaces.borrow();
        fctl_domain::schedule::no_runner_takes(
            want.own_runners.then_some(want.person_name.as_str()),
            runners.values().any(|s| s.takes(want)),
            runners.values().any(|s| s.runs(&want.workspace_id)),
        )
    }

    /// Reads which workspaces each runner serves, and for whom, for
    /// scheduling.
    async fn load_runner_workspaces(&self, db: &worker::D1Database) {
        #[derive(Deserialize)]
        struct R {
            runner_id: String,
            workspace_id: String,
            #[serde(deserialize_with = "crate::db::int_bool", default)]
            personal: bool,
            owner: Option<String>,
        }
        let rows: Vec<R> = match crate::db::q(
            db,
            concat!(
                "SELECT rw.runner_id, rw.workspace_id, rw.personal, r.created_by AS owner \
                 FROM runner_workspaces rw JOIN runners r ON r.id = rw.runner_id WHERE ",
                crate::serves!()
            ),
            &[],
        ) {
            Ok(q) => match q.all().await {
                Ok(r) => r,
                Err(e) => {
                    worker::console_error!("reading runners' workspaces: {}", e.0.message);
                    return;
                }
            },
            Err(_) => return,
        };
        let mut map: std::collections::HashMap<String, Serves> = std::collections::HashMap::new();
        for r in rows {
            let serves = map.entry(r.runner_id).or_default();
            serves.owner = r.owner;
            serves.workspaces.push((r.workspace_id, r.personal));
        }
        *self.runner_workspaces.borrow_mut() = map;
    }

    /// People's workspaces changed: their feeds show what they now may,
    /// with the runners of their workspaces, and they fetch their
    /// workspaces again.
    async fn workspaces_changed(&self, user_ids: &[String]) {
        let Ok(db) = self.env.d1("DB") else { return };
        let settings = match crate::instance::settings(&db).await {
            Ok(s) => s,
            Err(e) => {
                worker::console_error!("instance settings: {}", e.0.message);
                return;
            }
        };
        for user_id in user_ids {
            let sockets = self.state.get_websockets_with_tag(&feed_user_tag(user_id));
            if sockets.is_empty() {
                continue;
            }
            let Ok(Some(user)) = user_row(&db, user_id).await else {
                continue;
            };
            let ident = match crate::workspaces::feed_identity(&db, &user, &settings).await {
                Ok(i) => i,
                Err(e) => {
                    worker::console_error!("feed identity: {}", e.0.message);
                    continue;
                }
            };
            let messages = self.view_changed(&ident).await;
            for ws in sockets {
                // Each keeps the token it was opened with, checked as before.
                let old = ws.deserialize_attachment::<FeedIdentity>().ok().flatten();
                let mut ident = ident.clone();
                ident.token_id = old.as_ref().and_then(|o| o.token_id.clone());
                ident.checked_at = old.and_then(|o| o.checked_at);
                let _ = ws.serialize_attachment(&ident);
                for text in &messages {
                    let _ = ws.send_with_str(text);
                }
            }
        }
    }

    fn feed_usage(&self, u: ClaudeUsage) {
        let Ok(text) = serde_json::to_string(&FeedServer::ClaudeUsage {
            credential_id: u.credential_id,
            usage: u.usage,
        }) else {
            return;
        };
        for (ws, ident) in self.feed_sockets() {
            if ident.user_id == u.user_id {
                let _ = ws.send_with_str(&text);
            }
        }
    }

    fn feed_login(&self, user_id: &str, job: ClaudeLoginJob) {
        let Ok(text) = serde_json::to_string(&FeedServer::ClaudeLogin { job: Box::new(job) })
        else {
            return;
        };
        for (ws, ident) in self.feed_sockets() {
            if ident.user_id == user_id {
                let _ = ws.send_with_str(&text);
            }
        }
    }
}
