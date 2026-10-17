//! FleetObject: one Durable Object per instance.
//!
//! It holds every host's control connection, assigns queued sessions to hosts
//! with free capacity, answers credential requests (serializing OAuth
//! refreshes), runs Claude sign-in jobs on hosts, and fans out live updates to
//! dashboard feeds.

use crate::crypto;
use crate::ctx::now_ms;
use crate::internal::{
    self, Activity, ControlState, FeedIdentity, LiveRunners, LoginCode, LoginStart, NotifySession,
    Schedule, Stop,
};
use crate::spec;
use fctl_proto::api::{ClaudeLoginJob, ClaudeLoginState, FeedServer};
use fctl_proto::runner::{
    ClaudeLoginUpdate, CredentialKind, FleetToHost, HostHello, HostSessionReport, HostToFleet,
    StopReason,
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
/// Starting a session may be attempted this many times before it fails.
const MAX_ATTEMPTS: i64 = 5;
/// A runner that stays away this long loses its sessions.
const RUNNER_GRACE_MS: i64 = 10 * 60 * 1000;
const TICK_MS: i64 = 30_000;

fn runner_tag(id: &str) -> String {
    format!("runner:{id}")
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
}

impl DurableObject for FleetObject {
    fn new(state: State, env: Env) -> Self {
        let sql = state.storage().sql();
        for stmt in [
            "CREATE TABLE IF NOT EXISTS queue (session_id TEXT PRIMARY KEY, enqueued_at INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, not_before INTEGER NOT NULL DEFAULT 0)",
            "CREATE TABLE IF NOT EXISTS assignments (session_id TEXT PRIMARY KEY, runner_id TEXT NOT NULL, assigned_at INTEGER NOT NULL, accepted INTEGER NOT NULL DEFAULT 0, run_id TEXT)",
            "CREATE TABLE IF NOT EXISTS runners (id TEXT PRIMARY KEY, name TEXT NOT NULL, hello TEXT, connected INTEGER NOT NULL DEFAULT 0, last_seen INTEGER, sessions TEXT)",
            "CREATE TABLE IF NOT EXISTS logins (job_id TEXT PRIMARY KEY, runner_id TEXT, user_id TEXT NOT NULL, created_at INTEGER NOT NULL)",
        ] {
            if let Err(e) = sql.exec(stmt, None) {
                worker::console_error!("fleet schema: {e}");
            }
        }
        // Columns added after the first release (fails harmlessly when present).
        let _ = sql.exec("ALTER TABLE assignments ADD COLUMN run_id TEXT", None);
        Self {
            state,
            env,
            sql,
            refreshing: RefCell::new(HashSet::new()),
            dispatching: RefCell::new(false),
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
        if let Ok(Some(rs)) = ws.deserialize_attachment::<RunnerSocket>() {
            match serde_json::from_str::<HostToFleet>(&text) {
                Ok(msg) => self.on_runner(&ws, &rs, msg).await,
                Err(e) => worker::console_error!("bad runner message: {e}"),
            }
        } else if text.contains("\"ping\"") {
            let _ = ws.send_with_str(r#"{"type":"pong"}"#);
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
        self.state
            .accept_websocket_with_tags(&pair.server, &[TAG_FEED]);
        pair.server.serialize_attachment(&ident)?;
        let runners = self.runner_list().await.unwrap_or_default();
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
                "SELECT id, name, hello, connected, last_seen FROM runners",
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
            }
            if let Some(t) = r.last_seen {
                out.last_seen.insert(r.id, t as i64);
            }
        }
        out
    }

    async fn runner_list(&self) -> std::result::Result<Vec<fctl_proto::api::Runner>, ApiError> {
        let db = self
            .env
            .d1("DB")
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let rows: Vec<crate::model::RunnerRow> = crate::db::q(
            &db,
            &format!("{} ORDER BY r.created_at", crate::model::RUNNER_SELECT),
            &[],
        )
        .map_err(|e| e.0)?
        .all()
        .await
        .map_err(|e| e.0)?;
        let live = self.live_runners();
        Ok(rows
            .iter()
            .map(|r| r.to_runner(live.online.contains(&r.id)))
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
                let sessions = serde_json::to_string(&hb.sessions).unwrap_or_else(|_| "[]".into());
                let _ = self.sql.exec(
                    "UPDATE runners SET sessions = ? WHERE id = ?",
                    Some(vec![sv(&sessions), sv(&rs.id)]),
                );
                // Persist liveness in D1 now and then (every heartbeat is fine: one write per 15 s).
                if let Ok(db) = self.env.d1("DB")
                    && let Ok(q) = crate::db::q(
                        &db,
                        "UPDATE runners SET last_seen_at = ? WHERE id = ?",
                        &[crate::db::n(now), crate::db::s(&rs.id)],
                    ) {
                        let _ = q.run().await;
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
                    run_id: Option<String>,
                }
                let current: Option<Option<String>> = self
                    .sql
                    .exec(
                        "SELECT run_id FROM assignments WHERE session_id = ? AND runner_id = ?",
                        Some(vec![sv(session_id.as_str()), sv(&rs.id)]),
                    )
                    .and_then(|c| c.to_array::<R>())
                    .ok()
                    .and_then(|r| r.into_iter().next().map(|r| r.run_id));
                let stale =
                    matches!((&run_id, &current), (Some(ended), Some(Some(cur))) if ended != cur);
                let queued_again = self
                    .sql
                    .exec(
                        "SELECT session_id, attempts FROM queue WHERE session_id = ?",
                        Some(vec![sv(session_id.as_str())]),
                    )
                    .and_then(|c| c.to_array::<QueueRow>())
                    .map(|r| !r.is_empty())
                    .unwrap_or(false);
                if stale {
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
                let reply = self.credential(&session_id, kind, now).await;
                let msg = match reply {
                    Ok((claude, github)) => FleetToHost::Credential {
                        request_id,
                        claude,
                        github,
                        error: None,
                    },
                    Err(e) => FleetToHost::Credential {
                        request_id,
                        claude: None,
                        github: None,
                        error: Some(e.message),
                    },
                };
                self.send_runner(&rs.id, &msg);
            }
            HostToFleet::ClaudeLogin { job_id, update } => {
                self.on_login_update(&job_id, update, now).await
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
            ) {
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
        self.reconcile(&rs.id, &hello.sessions, now).await;
        self.feed_runner(&rs.id).await;
        self.dispatch().await;
    }

    /// Matches what the host reports with what we assigned to it.
    async fn reconcile(&self, runner_id: &str, reported: &[HostSessionReport], _now: i64) {
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
        for a in &assigned {
            if a.accepted > 0.0 && !reported_ids.contains(a.session_id.as_str()) {
                let _ = self.sql.exec(
                    "DELETE FROM assignments WHERE session_id = ?",
                    Some(vec![sv(&a.session_id)]),
                );
                self.session_control(
                    &SessionId(a.session_id.clone()),
                    Some(SessionStatus::Stopped),
                    Some("The runner restarted without this session's sandbox. Resume it to continue.".into()),
                    true,
                )
                .await;
            }
        }
        // Reported, but not ours (deleted, reassigned or already ended): stop it.
        let assigned_ids: HashSet<&str> = assigned.iter().map(|a| a.session_id.as_str()).collect();
        for r in reported {
            if !assigned_ids.contains(r.session_id.as_str()) && r.status.is_active() {
                let still_ours = match self.env.d1("DB") {
                    Ok(db) => spec::session_row(&db, r.session_id.as_str())
                        .await
                        .ok()
                        .flatten()
                        .filter(|row| {
                            row.runner_id.as_deref() == Some(runner_id)
                                && !row.status().is_terminal()
                        })
                        .is_some(),
                    Err(_) => false,
                };
                if still_ours {
                    let _ = self.sql.exec(
                        "INSERT OR REPLACE INTO assignments (session_id, runner_id, assigned_at, accepted) VALUES (?, ?, ?, 1)",
                        Some(vec![sv(r.session_id.as_str()), sv(runner_id), iv(now_ms())]),
                    );
                } else {
                    self.send_runner(
                        runner_id,
                        &FleetToHost::StopSession {
                            session_id: r.session_id.clone(),
                            reason: StopReason::Deleted,
                            destroy: false,
                        },
                    );
                }
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
        if accepted {
            let _ = self.sql.exec(
                "UPDATE assignments SET accepted = 1 WHERE session_id = ? AND runner_id = ?",
                Some(vec![sv(session_id.as_str()), sv(runner_id)]),
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

    /// Runners offline for longer than the grace period lose their sessions.
    async fn expire_offline_runners(&self, now: i64) {
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
                self.session_control(
                    &SessionId(a.session_id),
                    Some(SessionStatus::Stopped),
                    Some(format!("Runner {} has been offline for 10 minutes. Resume the session to continue elsewhere.", r.name)),
                    true,
                )
                .await;
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
        let still_queued = self
            .sql
            .exec(
                "SELECT session_id, attempts FROM queue WHERE session_id = ?",
                Some(vec![sv(b.session_id.as_str())]),
            )
            .and_then(|c| c.to_array::<QueueRow>())
            .map(|r| !r.is_empty())
            .unwrap_or(false);
        if still_queued {
            self.session_control(
                &b.session_id,
                Some(SessionStatus::Queued),
                Some("Waiting for a runner".into()),
                false,
            )
            .await;
            self.ensure_alarm().await;
        }
        Ok(serde_json::json!({ "ok": true, "queued": still_queued }))
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
            .exec("SELECT session_id, attempts FROM queue WHERE not_before <= ? ORDER BY enqueued_at ASC LIMIT 50", Some(vec![iv(now)]))
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
        for item in queued {
            let Some((runner_id, runner_name)) = self.pick_runner() else {
                break;
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

    /// The least loaded connected runner with a free slot.
    fn pick_runner(&self) -> Option<(String, String)> {
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
        let rows = self.live_rows();
        let candidates: Vec<(LiveRow, u32, u32, Vec<String>)> = rows
            .into_iter()
            .filter(|r| r.connected > 0.0 && self.runner_socket(&r.id).is_some())
            .filter_map(|r| {
                let hello: HostHello = serde_json::from_str(r.hello.as_deref()?).ok()?;
                let n = active
                    .iter()
                    .find(|c| c.runner_id == r.id)
                    .map_or(0, |c| c.n as u32);
                Some((r, hello.capacity, n, hello.labels))
            })
            .collect();
        let slots: Vec<fctl_domain::schedule::Slot<'_>> = candidates
            .iter()
            .map(|(r, cap, n, labels)| fctl_domain::schedule::Slot {
                id: r.id.as_str(),
                online: true,
                enabled: true,
                capacity: *cap,
                active: *n,
                labels: labels.as_slice(),
            })
            .collect();
        let chosen = fctl_domain::schedule::pick(&slots, &[])?;
        let name = candidates
            .iter()
            .find(|(r, ..)| r.id == chosen.id)
            .map(|(r, ..)| r.name.clone())?;
        Some((chosen.id.to_owned(), name))
    }

    async fn stop(&self, b: Stop) -> std::result::Result<serde_json::Value, ApiError> {
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
                self.send_runner(
                    &a.runner_id,
                    &FleetToHost::StopSession {
                        session_id: b.session_id.clone(),
                        reason: b.reason,
                        destroy: b.destroy,
                    },
                );
                self.session_control(
                    &b.session_id,
                    Some(SessionStatus::Stopping),
                    Some("Stopping".into()),
                    false,
                )
                .await;
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
                ) {
                    let _ = q.run().await;
                }
    }

    // ───────────────────────────── credentials ─────────────────────────────

    async fn credential(
        &self,
        session_id: &SessionId,
        kind: CredentialKind,
        now: i64,
    ) -> std::result::Result<
        (
            Option<fctl_proto::runner::ClaudeCredential>,
            Option<fctl_proto::runner::GithubCredential>,
        ),
        ApiError,
    > {
        let db = self
            .env
            .d1("DB")
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let cfg = crate::config::Config::from_env(&self.env);
        let row = spec::session_row(&db, session_id.as_str())
            .await
            .map_err(|e| e.0)?
            .ok_or_else(|| ApiError::not_found("Session"))?;
        match kind {
            CredentialKind::Github => {
                let project = spec::project_row(&db, &row.project_id)
                    .await
                    .map_err(|e| e.0)?
                    .ok_or_else(|| ApiError::not_found("Project"))?;
                let repo = project.repo().ok_or_else(|| {
                    ApiError::new(ErrorCode::NotConfigured, "The project has no repository")
                })?;
                let token = spec::github_token(&cfg, &repo, now)
                    .await
                    .map_err(|e| e.0)?;
                Ok((None, Some(token)))
            }
            CredentialKind::Claude => {
                let keys = crypto::keys(&cfg).map_err(|e| e.0)?;
                let cred_id = row.credential_id.clone().ok_or_else(|| {
                    ApiError::new(ErrorCode::NotConfigured, "No Claude credential")
                })?;
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
                Ok((Some(result.map_err(|e| e.0)?), None))
            }
        }
    }

    // ───────────────────────────── Claude sign-in ─────────────────────────────

    fn login_start(&self, b: LoginStart) -> std::result::Result<serde_json::Value, ApiError> {
        let online = self.live_runners().online;
        let Some(runner_id) = online.first().cloned() else {
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

    async fn on_login_update(&self, job_id: &LoginJobId, update: ClaudeLoginUpdate, now: i64) {
        let Some(row) = self.login_row(job_id.as_str()) else {
            return;
        };
        let Ok(db) = self.env.d1("DB") else { return };
        let state = match update {
            ClaudeLoginUpdate::Starting => ClaudeLoginState::Starting,
            ClaudeLoginUpdate::AwaitingCode { url } => ClaudeLoginState::AwaitingCode { url },
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

    fn feed_sockets(&self) -> Vec<(WebSocket, FeedIdentity)> {
        self.state
            .get_websockets_with_tag(TAG_FEED)
            .into_iter()
            .filter_map(|ws| {
                let ident = ws.deserialize_attachment::<FeedIdentity>().ok().flatten()?;
                Some((ws, ident))
            })
            .collect()
    }

    fn feed_session(&self, s: &fctl_proto::session::Session) {
        for (ws, ident) in self.feed_sockets() {
            if ident.admin
                || ident.projects.contains(&s.project_id.0)
                || s.created_by.id.0 == ident.user_id
            {
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
            if ident.admin || ident.projects.contains(&project) {
                let _ = ws.send_with_str(&text);
            }
        }
    }

    async fn feed_runner(&self, runner_id: &str) {
        let Ok(runners) = self.runner_list().await else {
            return;
        };
        let Some(runner) = runners.into_iter().find(|r| r.id.0 == runner_id) else {
            return;
        };
        let Ok(text) = serde_json::to_string(&FeedServer::Runner {
            runner: Box::new(runner),
        }) else {
            return;
        };
        for (ws, _) in self.feed_sockets() {
            let _ = ws.send_with_str(&text);
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
