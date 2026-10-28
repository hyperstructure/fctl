//! SessionObject: one Durable Object per session.
//!
//! It stores the event log (SQLite), serves viewer WebSockets (browsers and
//! the CLI), holds the host's uplink, delivers messages to the agent, and
//! writes summary changes through to D1 and the dashboard feed.
//!
//! Sockets use the hibernation API, so the object can leave memory while
//! connections stay open; everything that must survive lives in SQLite.

use crate::ctx::now_ms;
use crate::internal::{
    self, CloseUplink, ControlState, Deliver, Delivered, DrainStop, DrainStopped, ForgetSession,
    Indexed, InitSession, PutDocs, ViewerIdentity,
};
use fctl_domain::events::{self as ev, ClaudeKind};
use fctl_domain::files;
use fctl_domain::follow_ups;
use fctl_domain::journal::{self, Take};
use fctl_domain::kept_images;
use fctl_domain::notify;
use fctl_domain::search;
use fctl_domain::tickets::Progress;
use fctl_domain::usage as plan;
use fctl_proto::api::{EventPage, PlanLimit};
use fctl_proto::event::{
    EventBody, FollowUp, FollowUpEvent, GitState, KeptImage, StatusChange, UserMessage,
};
use fctl_proto::session::{PullRequestRef, RepoRef, Session, SessionRole, SessionStatus, UserRef};
use fctl_proto::uplink::{
    AgentUnreachable, Delivery, DeliveryKind, FEATURE_ATTACHMENTS, SessionState, UplinkDown,
    UplinkUp,
};
use fctl_proto::viewer::{Presence, ViewerClient, ViewerServer};
use fctl_proto::{ApiError, AttachmentId, ErrorCode};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use worker::{
    DurableObject, Env, Headers, Request, Response, Result, SqlStorage, SqlStorageValue, State,
    WebSocket, WebSocketIncomingMessage, WebSocketPair, durable_object,
};

const TAG_VIEWER: &str = "viewer";
const TAG_UPLINK: &str = "uplink";
/// Events in a snapshot without `?after=`.
const SNAPSHOT_EVENTS: i64 = 400;
/// Byte budget of the events in one snapshot or page.
const SNAPSHOT_BYTES: usize = 3_000_000;
/// An uplink that sends nothing for this long counts as away until it does:
/// hosts ping every 20 seconds, so this one's host is frozen or cut off (a
/// laptop that went to sleep), and its socket may stay open for a long time.
const QUIET_MS: i64 = 60_000;
/// How often an uplink's socket notes when it was last heard from.
const HEARD_EVERY_MS: i64 = 5_000;
/// The longest reason a host gives for not reaching its agent that is kept.
const REASON_MAX_CHARS: usize = 300;
/// How long a session on a draining runner waits for a message before it
/// stops, so that quick replies still reach the running agent.
const DRAIN_IDLE_MS: i64 = 60_000;
/// How often an open viewer's access is checked again, on its pings: once
/// its person left the workspace, lost their role or share, or signed out,
/// the socket closes within this long. Messages and interrupts check it
/// every time.
const VIEWER_CHECK_MS: i64 = 60_000;
/// A viewer whose access hasn't been checked for this long (it stopped
/// pinging, which the apps do every 25 seconds) is closed at the next
/// broadcast, and reconnects through the Worker, which checks it: twice
/// [`VIEWER_CHECK_MS`], so that a check D1 didn't answer leaves the apps
/// another, and a client that stops pinging once its person lost access
/// hears little more.
const VIEWER_STALE_MS: i64 = 2 * VIEWER_CHECK_MS;
/// Close codes for viewers: their person can't see the session any more
/// (the apps stop reconnecting), and their access changed or wasn't
/// checked for a while (they reconnect).
const CLOSE_NO_ACCESS: u16 = 4403;
const CLOSE_RECHECK: u16 = 4000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SocketInfo {
    viewer: Option<ViewerIdentity>,
    runner_id: Option<String>,
    since: i64,
    #[serde(default)]
    typing_at: Option<i64>,
    /// An uplink's host features, once it said hello.
    #[serde(default)]
    features: Option<Vec<String>>,
    /// A viewer whose tab is hidden: its person is not watching.
    #[serde(default)]
    hidden: bool,
    /// An uplink: when its host last sent something (to within a few seconds).
    #[serde(default)]
    heard_at: Option<i64>,
    /// An uplink that went quiet, which viewers were told is away.
    #[serde(default)]
    quiet: bool,
    /// An uplink: its host can't reach the agent, as it last said.
    #[serde(default)]
    agent_unreachable: Option<AgentUnreachable>,
    /// A viewer: when its access was last checked.
    #[serde(default)]
    checked_at: Option<i64>,
}

impl SocketInfo {
    fn heard(&self) -> i64 {
        self.heard_at.unwrap_or(self.since)
    }

    fn checked(&self) -> i64 {
        self.checked_at.unwrap_or(self.since)
    }
}

/// What viewers see of the session's runner (`runner_connected`).
struct RunnerView {
    connected: bool,
    last_seen_at: Option<i64>,
    agent_unreachable: Option<AgentUnreachable>,
}

#[derive(Default)]
struct Mem {
    summary: Option<Session>,
    /// The API calls whose tokens the summary counts (`ev::Counted`).
    counted: Option<ev::Counted>,
    /// The live messages of the response being written.
    live: ev::Live,
    last_pulse: i64,
    pulse_weight: u32,
    last_d1_write: i64,
    d1_dirty: bool,
    /// A message or a reply came that the search index doesn't have yet.
    search_due: bool,
}

#[durable_object]
pub struct SessionObject {
    state: State,
    env: Env,
    sql: SqlStorage,
    mem: RefCell<Mem>,
}

#[derive(Deserialize)]
struct EventRow {
    seq: f64,
    ts: f64,
    kind: String,
    data: String,
}

#[derive(Deserialize)]
struct MetaRow {
    value: String,
}

#[derive(Deserialize)]
struct OutboxRow {
    data: String,
}

#[derive(Deserialize)]
struct SeqRow {
    seq: f64,
}

fn sv(s: &str) -> SqlStorageValue {
    SqlStorageValue::from(s)
}

fn iv(n: i64) -> SqlStorageValue {
    SqlStorageValue::from(n)
}

fn event_json(seq: i64, ts: i64, kind: &str, data: &str) -> String {
    format!(r#"{{"seq":{seq},"ts":{ts},"kind":"{kind}","data":{data}}}"#)
}

/// Why a review agent's session takes no message or interrupt.
const REVIEWS_TAKE_NO_MESSAGES: &str = "Review agents take no messages: this one reviews its pull request's commit, posts what it found on GitHub, and ends. A new commit starts a new review.";
/// The meta key that counts the reviews whose findings the agent got in a
/// row, which a person's message ends (`fctl_domain::reviews::MAX_PASSED_ON`).
const REVIEWS_PASSED_ON: &str = "reviews_passed_on";
/// The meta key that counts the failed checks the agent heard of in a row,
/// which a person's message, or their comment on GitHub, ends
/// (`fctl_domain::follow_ups::MAX_CHECKS_IN_A_ROW`).
const CHECKS_IN_A_ROW: &str = "checks_in_a_row";
/// The meta key that counts the conflicts the agent heard of in a row, as
/// [`CHECKS_IN_A_ROW`] does failed checks
/// (`fctl_domain::follow_ups::MAX_CONFLICTS_IN_A_ROW`).
const CONFLICTS_IN_A_ROW: &str = "conflicts_in_a_row";

/// The meta key that counts what of `kind` the agent heard of in a row.
fn in_a_row_key(kind: follow_ups::Repeats) -> &'static str {
    match kind {
        follow_ups::Repeats::Checks => CHECKS_IN_A_ROW,
        follow_ups::Repeats::Conflicts => CONFLICTS_IN_A_ROW,
    }
}

/// How soon after a message or a reply came the alarm puts it in the
/// workspace's search index, with whatever else came meanwhile.
const SEARCH_DELAY_MS: i64 = 5_000;
/// Events the search index takes at a time.
const SEARCH_BATCH: i64 = 500;
/// Batches the hourly job's request indexes at once; the alarm does the
/// rest of a long transcript.
const SEARCH_BACKFILL_ROUNDS: u32 = 10;
/// Times in a row the alarm tries again when the search index didn't take
/// what's new, waiting longer each time; after that the hourly job does
/// (`housekeeping::index_sessions`).
const SEARCH_RETRIES: u32 = 5;

/// The most of the agent's last message of a turn that SessionDO keeps,
/// for the ticket's thread its session answers in, which shows a shorter
/// cut of it (`fctl_domain::tickets::MAX_REPLY`).
const LAST_RESULT_CHARS: usize = 12_000;

fn api_err(code: ErrorCode, message: &str) -> ApiError {
    ApiError::new(code, message)
}

/// How far a session's work on its ticket has come, as its changes and
/// pull requests show (`fctl_domain::tickets::progress`).
fn progress(s: &Session) -> Option<Progress> {
    fctl_domain::tickets::progress(&s.pull_requests, s.stats.files_changed > 0)
}

/// The pull requests among `prs` that merged.
fn merged(prs: &[PullRequestRef]) -> Vec<PullRequestRef> {
    prs.iter()
        .filter(|p| p.state.as_deref() == Some("merged"))
        .cloned()
        .collect()
}

fn has_files(d: &Delivery) -> bool {
    matches!(&d.kind, DeliveryKind::Message { attachments, .. } if !attachments.is_empty())
}

fn create_schema(sql: &SqlStorage) {
    for stmt in [
        "CREATE TABLE IF NOT EXISTS events (seq INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, kind TEXT NOT NULL, data TEXT NOT NULL, agent_seq INTEGER)",
        "CREATE INDEX IF NOT EXISTS events_agent ON events (agent_seq) WHERE agent_seq IS NOT NULL",
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS outbox (id TEXT PRIMARY KEY, n INTEGER NOT NULL, data TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS client_keys (key TEXT PRIMARY KEY, delivery_id TEXT NOT NULL, ts INTEGER NOT NULL)",
        // What happened to pull requests its agent opened, gathered before
        // the agent hears of it (`fctl_domain::follow_ups`): in the order
        // it came, until `due`.
        "CREATE TABLE IF NOT EXISTS follow_ups (key TEXT PRIMARY KEY, n INTEGER NOT NULL, due INTEGER NOT NULL, data TEXT NOT NULL)",
        // Pull requests its agent opened that GitHub didn't know yet
        // whether they conflict with their base branch: asked `asked` times,
        // asked again at `due` (`fctl_domain::follow_ups::Ask`).
        "CREATE TABLE IF NOT EXISTS asks (url TEXT PRIMARY KEY, repo TEXT NOT NULL, number INTEGER NOT NULL, asked INTEGER NOT NULL, due INTEGER NOT NULL)",
        // Images its agent read, kept for its viewers until `expires_at`
        // (`fctl_domain::kept_images`), in chunks (SQLite rows hold at most
        // 2 MB).
        "CREATE TABLE IF NOT EXISTS kept_images (id TEXT PRIMARY KEY, media_type TEXT NOT NULL, size INTEGER NOT NULL, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL)",
        "CREATE INDEX IF NOT EXISTS kept_images_expiry ON kept_images (expires_at)",
        "CREATE TABLE IF NOT EXISTS kept_image_chunks (image_id TEXT NOT NULL, n INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (image_id, n))",
    ] {
        if let Err(e) = sql.exec(stmt, None) {
            worker::console_error!("session schema: {e}");
        }
    }
}

impl DurableObject for SessionObject {
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
        let result = match (req.method(), path.as_str()) {
            (worker::Method::Post, "/init") => match req.json::<InitSession>().await {
                Ok(b) => self.init(b).await,
                Err(e) => Err(api_err(ErrorCode::BadRequest, &e.to_string())),
            },
            (worker::Method::Post, "/deliver") => match req.json::<Deliver>().await {
                Ok(b) => self.deliver(b).await.map(|d| serde_json::json!(d)),
                Err(e) => Err(api_err(ErrorCode::BadRequest, &e.to_string())),
            },
            (worker::Method::Post, "/control") => match req.json::<ControlState>().await {
                Ok(b) => self.control(b).await,
                Err(e) => Err(api_err(ErrorCode::BadRequest, &e.to_string())),
            },
            (worker::Method::Post, "/drain-stop") => match req.json::<DrainStop>().await {
                Ok(b) => self.drain_stop(b).await.map(|d| serde_json::json!(d)),
                Err(e) => Err(api_err(ErrorCode::BadRequest, &e.to_string())),
            },
            (worker::Method::Post, "/pull-requests") => {
                match req.json::<internal::PullRequestsSeen>().await {
                    Ok(b) => self.pull_requests_seen(b).await,
                    Err(e) => Err(api_err(ErrorCode::BadRequest, &e.to_string())),
                }
            }
            (worker::Method::Post, "/follow-up") => match req.json::<internal::FollowUps>().await {
                Ok(b) => self
                    .take_follow_ups(b.items)
                    .await
                    .map(|t| serde_json::json!(t)),
                Err(e) => Err(api_err(ErrorCode::BadRequest, &e.to_string())),
            },
            (worker::Method::Post, "/review-findings") => {
                match req.json::<internal::ReviewFindings>().await {
                    Ok(b) => self.review_findings(b).await.map(|t| serde_json::json!(t)),
                    Err(e) => Err(api_err(ErrorCode::BadRequest, &e.to_string())),
                }
            }
            (worker::Method::Post, "/notice") => {
                match req.json::<fctl_proto::event::Notice>().await {
                    Ok(n) => Ok(self.control_notice(n)),
                    Err(e) => Err(api_err(ErrorCode::BadRequest, &e.to_string())),
                }
            }
            (worker::Method::Get, "/summary") => match self.summary() {
                Some(s) => Ok(serde_json::json!(s)),
                None => Err(api_err(ErrorCode::NotFound, "Session not initialized")),
            },
            (worker::Method::Get, "/events") => return self.events_page(&req),
            (worker::Method::Get, "/image") => return self.kept_image(&req),
            (worker::Method::Get, "/viewer") => return self.accept_viewer(&req),
            (worker::Method::Get, "/uplink") => return self.accept_uplink(&req),
            (worker::Method::Post, "/purge") => self.purge().await,
            (worker::Method::Post, "/index") => {
                self.backfill_search().await.map(|d| serde_json::json!(d))
            }
            (worker::Method::Post, "/close-uplink") => match req.json::<CloseUplink>().await {
                Ok(b) => Ok(self.close_uplink(&b.runner_id)),
                Err(e) => Err(api_err(ErrorCode::BadRequest, &e.to_string())),
            },
            _ => Err(api_err(ErrorCode::NotFound, "Unknown session endpoint")),
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
        let tags = self.state.get_tags(&ws);
        if tags.iter().any(|t| t == TAG_UPLINK) {
            match serde_json::from_str::<UplinkUp>(&text) {
                Ok(msg) => self.on_uplink(&ws, msg).await,
                Err(e) => worker::console_error!("bad uplink message: {e}"),
            }
        } else if tags.iter().any(|t| t == TAG_VIEWER) {
            match serde_json::from_str::<ViewerClient>(&text) {
                Ok(msg) => self.on_viewer(&ws, msg).await,
                Err(_) => {
                    let _ = ws.send_with_str(
                        r#"{"type":"error","error":{"code":"bad_request","message":"Unknown message"}}"#,
                    );
                }
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
        self.on_close(&ws);
        Ok(())
    }

    async fn websocket_error(&self, ws: WebSocket, _error: worker::Error) -> Result<()> {
        self.on_close(&ws);
        Ok(())
    }

    async fn alarm(&self) -> Result<Response> {
        // Flush summary changes that were throttled.
        if self.mem.borrow().d1_dirty {
            self.write_through().await;
        }
        self.flush_follow_ups(now_ms()).await;
        self.ask_again(now_ms()).await;
        self.index_now(now_ms()).await;
        self.expire_images(now_ms()).await;
        self.see_if_done(now_ms()).await;
        Response::ok("ok")
    }
}

impl SessionObject {
    /// Deletes everything the session stored and closes its sockets. Its
    /// project was deleted, so nothing routes here afterwards
    /// (`housekeeping::purge_sessions`).
    async fn purge(&self) -> std::result::Result<serde_json::Value, ApiError> {
        // Its documents leave the workspace's search index first: once its
        // storage is gone, nothing says which workspace has them.
        if let Some(s) = self.summary().filter(|s| s.review.is_none())
            && let Some(workspace) = self.workspace_id(&s).await
        {
            let stub = internal::search_stub(&self.env, &workspace).map_err(|e| e.0)?;
            let _: serde_json::Value = internal::post(
                &stub,
                "/forget",
                &ForgetSession {
                    session_id: s.id.clone(),
                },
            )
            .await
            .map_err(|e| e.0)?;
        }
        for ws in self.state.get_websockets() {
            let _ = ws.close(Some(4404), Some("the session was deleted"));
        }
        let failed = |e: worker::Error| api_err(ErrorCode::Internal, &format!("purge: {e}"));
        let storage = self.state.storage();
        // deleteAll leaves the alarm in place.
        storage.delete_alarm().await.map_err(failed)?;
        storage.delete_all().await.map_err(failed)?;
        // Empty tables, as a new object would have, for anything still in flight.
        create_schema(&self.sql);
        *self.mem.borrow_mut() = Mem::default();
        Ok(serde_json::json!({ "ok": true }))
    }

    fn on_close(&self, ws: &WebSocket) {
        let tags = self.state.get_tags(ws);
        if tags.iter().any(|t| t == TAG_UPLINK) {
            // Another uplink may already have replaced this one.
            let others = self
                .state
                .get_websockets_with_tag(TAG_UPLINK)
                .into_iter()
                .filter(|w| w != ws)
                .count();
            if others == 0 {
                // For the people watching: when the runner was last heard from.
                let heard = ws
                    .deserialize_attachment::<SocketInfo>()
                    .ok()
                    .flatten()
                    .map_or_else(now_ms, |i| i.heard());
                self.set_meta("runner_seen_at", &heard.to_string());
                self.broadcast_runner(Some(ws));
            }
        } else {
            self.broadcast_presence(Some(ws));
        }
    }

    // ───────────────────────────── storage ─────────────────────────────

    fn meta(&self, key: &str) -> Option<String> {
        self.sql
            .exec("SELECT value FROM meta WHERE key = ?", Some(vec![sv(key)]))
            .ok()?
            .to_array::<MetaRow>()
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
            worker::console_error!("meta write: {e}");
        }
    }

    fn delete_meta(&self, key: &str) {
        let _ = self
            .sql
            .exec("DELETE FROM meta WHERE key = ?", Some(vec![sv(key)]));
    }

    fn summary(&self) -> Option<Session> {
        if let Some(s) = self.mem.borrow().summary.clone() {
            return Some(s);
        }
        let mut s: Session = serde_json::from_str(&self.meta("summary")?).ok()?;
        fctl_domain::pulls::backfill(&mut s);
        self.mem.borrow_mut().summary = Some(s.clone());
        Some(s)
    }

    fn save_summary(&self, s: &Session) {
        if let Ok(text) = serde_json::to_string(s) {
            self.set_meta("summary", &text);
        }
        self.mem.borrow_mut().summary = Some(s.clone());
    }

    /// On whose Claude subscription the session runs: who started it, until
    /// someone else resumes it (`ControlState::claude_user`). The Remote
    /// Control state its agent reports is theirs.
    fn claude_user(&self, s: &Session) -> UserRef {
        self.meta("claude_user")
            .and_then(|u| serde_json::from_str(&u).ok())
            .unwrap_or_else(|| s.created_by.clone())
    }

    fn counted(&self) -> ev::Counted {
        if let Some(c) = self.mem.borrow().counted.clone() {
            return c;
        }
        let c: ev::Counted = self
            .meta("counted")
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        self.mem.borrow_mut().counted = Some(c.clone());
        c
    }

    fn save_counted(&self, c: ev::Counted) {
        if let Ok(text) = serde_json::to_string(&c) {
            self.set_meta("counted", &text);
        }
        self.mem.borrow_mut().counted = Some(c);
    }

    fn acked_agent_seq(&self) -> u64 {
        self.meta("acked_agent_seq")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    /// Stores an event and returns its JSON (for broadcasting).
    fn append(&self, ts: i64, body: &EventBody, agent_seq: Option<u64>) -> Option<String> {
        if search::searchable(body) {
            self.mem.borrow_mut().search_due = true;
        }
        let kind = body.kind();
        let data = match serde_json::to_value(body) {
            Ok(v) => v
                .get("data")
                .map(|d| d.to_string())
                .unwrap_or_else(|| "null".to_owned()),
            Err(_) => return None,
        };
        self.append_raw(ts, kind, &data, agent_seq)
    }

    fn append_raw(
        &self,
        ts: i64,
        kind: &str,
        data: &str,
        agent_seq: Option<u64>,
    ) -> Option<String> {
        let seq_param = agent_seq.map_or(SqlStorageValue::Null, |s| iv(s as i64));
        let rows = self
            .sql
            .exec(
                "INSERT INTO events (ts, kind, data, agent_seq) VALUES (?, ?, ?, ?) RETURNING seq",
                Some(vec![iv(ts), sv(kind), sv(data), seq_param]),
            )
            .and_then(|c| c.to_array::<SeqRow>());
        match rows {
            Ok(r) => r
                .first()
                .map(|row| event_json(row.seq as i64, ts, kind, data)),
            Err(e) => {
                worker::console_error!("event insert failed: {e}");
                None
            }
        }
    }

    fn next_seq(&self) -> i64 {
        #[derive(Deserialize)]
        struct M {
            m: Option<f64>,
        }
        self.sql
            .exec("SELECT MAX(seq) AS m FROM events", None)
            .and_then(|c| c.to_array::<M>())
            .ok()
            .and_then(|r| r.first().and_then(|m| m.m))
            .map_or(1, |m| m as i64 + 1)
    }

    fn load_events(&self, sql: &str, params: Vec<SqlStorageValue>) -> Vec<EventRow> {
        self.sql
            .exec(sql, Some(params))
            .and_then(|c| c.to_array::<EventRow>())
            .unwrap_or_default()
    }

    /// Events as JSON strings within a byte budget, oldest first.
    fn render(rows: Vec<EventRow>, budget: usize, newest_first: bool) -> (Vec<String>, bool) {
        let mut out = Vec::new();
        let mut bytes = 0usize;
        let mut truncated = false;
        for r in rows {
            let json = event_json(r.seq as i64, r.ts as i64, &r.kind, &r.data);
            bytes += json.len();
            if bytes > budget && !out.is_empty() {
                truncated = true;
                break;
            }
            out.push(json);
        }
        if newest_first {
            out.reverse();
        }
        (out, truncated)
    }

    // ───────────────────────────── broadcasting ─────────────────────────────

    /// The viewers' sockets with what they hold. Those whose access wasn't
    /// checked for [`VIEWER_STALE_MS`] close instead: their person may have
    /// lost it meanwhile.
    fn viewers(&self) -> Vec<(WebSocket, SocketInfo)> {
        let now = now_ms();
        let mut out = Vec::new();
        for ws in self.state.get_websockets_with_tag(TAG_VIEWER) {
            let Ok(Some(info)) = ws.deserialize_attachment::<SocketInfo>() else {
                continue;
            };
            if now - info.checked() > VIEWER_STALE_MS {
                let _ = ws.close(Some(CLOSE_RECHECK), Some("checking your access again"));
                continue;
            }
            out.push((ws, info));
        }
        out
    }

    fn broadcast(&self, text: &str) {
        for (ws, _) in self.viewers() {
            let _ = ws.send_with_str(text);
        }
    }

    fn broadcast_events(&self, events: &[String]) {
        if events.is_empty() {
            return;
        }
        self.broadcast(&format!(
            r#"{{"type":"events","events":[{}]}}"#,
            events.join(",")
        ));
    }

    fn uplink(&self) -> Option<WebSocket> {
        self.state
            .get_websockets_with_tag(TAG_UPLINK)
            .into_iter()
            .last()
    }

    fn send_uplink(&self, msg: &UplinkDown) {
        if let (Some(ws), Ok(text)) = (self.uplink(), serde_json::to_string(msg)) {
            let _ = ws.send_with_str(&text);
        }
    }

    fn presence(&self, exclude: Option<&WebSocket>) -> Vec<Presence> {
        let mut by_user: Vec<Presence> = Vec::new();
        for ws in self.state.get_websockets_with_tag(TAG_VIEWER) {
            if exclude.is_some_and(|x| x == &ws) {
                continue;
            }
            let Ok(Some(info)) = ws.deserialize_attachment::<SocketInfo>() else {
                continue;
            };
            let Some(viewer) = info.viewer else { continue };
            match by_user.iter_mut().find(|p| p.user.id == viewer.user.id) {
                Some(p) => {
                    p.connections += 1;
                    p.since = p.since.min(info.since);
                    p.typing_at = p.typing_at.max(info.typing_at);
                }
                None => by_user.push(Presence {
                    user: viewer.user,
                    connections: 1,
                    since: info.since,
                    typing_at: info.typing_at,
                }),
            }
        }
        by_user.sort_by_key(|p| p.since);
        by_user
    }

    /// People who can see the session right now: a viewer of theirs is open
    /// and its tab is not hidden.
    fn watching(&self) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        for ws in self.state.get_websockets_with_tag(TAG_VIEWER) {
            let Ok(Some(info)) = ws.deserialize_attachment::<SocketInfo>() else {
                continue;
            };
            if let Some(viewer) = info.viewer
                && !info.hidden
                && !ids.contains(&viewer.user.id.0)
            {
                ids.push(viewer.user.id.0);
            }
        }
        ids
    }

    fn broadcast_presence(&self, exclude: Option<&WebSocket>) {
        let viewers = self.presence(exclude);
        if let Ok(text) =
            serde_json::to_string(&fctl_proto::viewer::ViewerServer::Presence { viewers })
        {
            for (ws, _) in self.viewers() {
                if exclude.is_some_and(|x| x == &ws) {
                    continue;
                }
                let _ = ws.send_with_str(&text);
            }
        }
    }

    fn broadcast_summary(&self, s: &Session) {
        // Each viewer gets the summary with their own role.
        for (ws, info) in self.viewers() {
            Self::send_summary(&ws, s, info.viewer.map(|v| v.role));
        }
    }

    fn send_summary(ws: &WebSocket, s: &Session, role: Option<SessionRole>) {
        let mut copy = s.clone();
        copy.role = role;
        if let Ok(text) = serde_json::to_string(&fctl_proto::viewer::ViewerServer::Session {
            session: Box::new(copy),
        }) {
            let _ = ws.send_with_str(&text);
        }
    }

    // ───────────────────────────── control-plane requests ─────────────────────────────

    async fn init(&self, b: InitSession) -> std::result::Result<serde_json::Value, ApiError> {
        if self.summary().is_some() {
            return Ok(serde_json::json!({ "ok": true, "existing": true }));
        }
        let mut s = b.session;
        s.role = None;
        s.following = None;
        self.save_summary(&s);
        if !b.prompt.attachments.is_empty() {
            self.set_meta("task_files", "1");
        }
        let now = now_ms();
        self.append(now, &EventBody::UserMessage(b.prompt), None);
        self.append(
            now,
            &EventBody::Status(StatusChange {
                status: SessionStatus::Queued,
                reason: None,
                resume_at: None,
            }),
            None,
        );
        // Its title too.
        self.mem.borrow_mut().search_due = true;
        self.search_soon(now).await;
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn deliver(&self, b: Deliver) -> std::result::Result<Delivered, ApiError> {
        let now = now_ms();
        let message = matches!(b.delivery.kind, DeliveryKind::Message { .. });
        if self.summary().is_some_and(|s| s.review.is_some()) {
            return Err(api_err(ErrorCode::Conflict, REVIEWS_TAKE_NO_MESSAGES));
        }
        if let Some(key) = b.client_key.as_deref() {
            if let Some(delivery_id) = self.keyed(key) {
                // Sent again after an error: it may not have resumed the session.
                if message {
                    self.wake(now).await?;
                }
                return Ok(Delivered {
                    delivery_id: fctl_proto::DeliveryId(delivery_id),
                });
            }
            self.key(key, b.delivery.id.as_str(), now);
        }
        // A run that stops takes nothing more: the next one gets messages
        // from the outbox. Nothing is left to interrupt, and an interrupt
        // kept for the next run would drop the messages that start it.
        let stopping = self
            .summary()
            .is_some_and(|s| s.status.resumes_for_messages());
        if stopping && !message {
            return Ok(Delivered {
                delivery_id: b.delivery.id,
            });
        }
        let data = serde_json::to_string(&b.delivery)
            .map_err(|e| api_err(ErrorCode::Internal, &e.to_string()))?;
        #[derive(Deserialize)]
        struct N {
            n: Option<f64>,
        }
        let next = self
            .sql
            .exec("SELECT MAX(n) AS n FROM outbox", None)
            .and_then(|c| c.to_array::<N>())
            .ok()
            .and_then(|r| r.first().and_then(|x| x.n))
            .map_or(1, |n| n as i64 + 1);
        let _ = self.sql.exec(
            "INSERT OR REPLACE INTO outbox (id, n, data) VALUES (?, ?, ?)",
            Some(vec![sv(b.delivery.id.as_str()), iv(next), sv(&data)]),
        );
        if let DeliveryKind::Interrupt { by: Some(by) } = &b.delivery.kind {
            self.set_meta("interrupted_by", by.id.as_str());
        }
        // A person's message: the agent gets reviews' findings, and hears
        // of failed checks and conflicts, again.
        if let DeliveryKind::Message {
            author: Some(_), ..
        } = &b.delivery.kind
        {
            self.delete_meta(REVIEWS_PASSED_ON);
            self.delete_meta(CHECKS_IN_A_ROW);
            self.delete_meta(CONFLICTS_IN_A_ROW);
        }
        if let Some(json) = self.append(now, &b.event, None) {
            self.broadcast_events(&[json]);
        }
        self.touch_activity(now);
        self.search_soon(now).await;
        if !stopping {
            self.send_uplink(&UplinkDown::Deliver {
                delivery: b.delivery.clone(),
            });
            if has_files(&b.delivery) && self.uplink_lacks_files() {
                self.old_host_notice(now);
            }
        }
        if message {
            self.wake(now).await?;
        }
        Ok(Delivered {
            delivery_id: b.delivery.id,
        })
    }

    /// The delivery an idempotency key went with, when it came before.
    fn keyed(&self, key: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct K {
            delivery_id: String,
        }
        self.sql
            .exec(
                "SELECT delivery_id FROM client_keys WHERE key = ?",
                Some(vec![sv(key)]),
            )
            .and_then(|c| c.to_array::<K>())
            .unwrap_or_default()
            .into_iter()
            .next()
            .map(|k| k.delivery_id)
    }

    /// Keeps an idempotency key for a day, with what it went with.
    fn key(&self, key: &str, delivery_id: &str, now: i64) {
        let _ = self.sql.exec(
            "INSERT INTO client_keys (key, delivery_id, ts) VALUES (?, ?, ?)",
            Some(vec![sv(key), sv(delivery_id), iv(now)]),
        );
        let _ = self.sql.exec(
            "DELETE FROM client_keys WHERE ts < ?",
            Some(vec![iv(now - 24 * 3600 * 1000)]),
        );
    }

    /// fctl's review agent posted findings on a pull request this
    /// session's agent opened (`crate::reviews`): a message from fctl hands
    /// them to the agent, and resumes the session when it stopped, as a
    /// person's message would. After the findings of
    /// [`fctl_domain::reviews::MAX_PASSED_ON`] reviews in a row while
    /// nobody messaged the agent, the transcript says what was found
    /// instead, so that the agent and the review agent don't go back and
    /// forth for good. A review's findings come once; archived and failed
    /// sessions, and review agents', get none.
    async fn review_findings(
        &self,
        b: internal::ReviewFindings,
    ) -> std::result::Result<internal::FindingsTold, ApiError> {
        let Some(s) = self.summary() else {
            return Err(api_err(ErrorCode::NotFound, "Session not initialized"));
        };
        let not_sent = |why: String| internal::FindingsTold {
            delivery_id: None,
            not_sent: Some(why),
        };
        if s.review.is_some() {
            return Ok(not_sent("it is a review agent's session".into()));
        }
        if s.archived {
            return Ok(not_sent("it is archived".into()));
        }
        if !s.status.accepts_messages() && !s.status.resumes_for_messages() {
            return Ok(not_sent("it failed".into()));
        }
        let held = |n: u32| {
            not_sent(format!(
                "its agent got the findings of {n} reviews in a row, and nobody has messaged it since"
            ))
        };
        let key = format!("review:{}", b.review);
        let in_a_row = self
            .meta(REVIEWS_PASSED_ON)
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        match self.keyed(&key) {
            Some(id) if id.is_empty() => return Ok(held(in_a_row)),
            Some(id) => {
                return Ok(internal::FindingsTold {
                    delivery_id: Some(fctl_proto::DeliveryId(id)),
                    not_sent: None,
                });
            }
            None => {}
        }
        let now = now_ms();
        if in_a_row >= fctl_domain::reviews::MAX_PASSED_ON {
            self.key(&key, "", now);
            let notice = fctl_proto::event::Notice {
                ticket: None,
                level: fctl_proto::event::NoticeLevel::Info,
                code: "review_held".into(),
                message: b.held,
                until: None,
                published: None,
                memory: None,
                retrospective: None,
                review: None,
                follow_up: None,
            };
            if let Some(json) = self.append(now, &EventBody::Notice(notice), None) {
                self.broadcast_events(&[json]);
            }
            return Ok(held(in_a_row));
        }
        // Counted before the message waits for a resume, so that another
        // review's findings, which may come meanwhile, count it. A resume
        // that fails leaves the message in the outbox, for the next run.
        self.set_meta(REVIEWS_PASSED_ON, &(in_a_row + 1).to_string());
        let id = fctl_domain::ids::delivery_id(now);
        let delivered = self
            .deliver(Deliver {
                delivery: Delivery {
                    id: id.clone(),
                    kind: DeliveryKind::Message {
                        text: b.text.clone(),
                        author: None,
                        author_called: None,
                        attachments: Vec::new(),
                    },
                },
                client_key: Some(key),
                event: EventBody::UserMessage(UserMessage {
                    ticket: None,
                    id,
                    author: None,
                    text: b.text,
                    source: fctl_proto::event::MessageSource::System,
                    attachments: Vec::new(),
                    follow_up: None,
                    review: b.shown,
                }),
            })
            .await?;
        Ok(internal::FindingsTold {
            delivery_id: Some(delivered.delivery_id),
            not_sent: None,
        })
    }

    /// Why the session's agent hears of nothing that happens to its pull
    /// requests: review agents' sessions, archived and failed ones.
    fn no_follow_ups(s: &Session) -> Option<&'static str> {
        if s.review.is_some() {
            Some("it is a review agent's session")
        } else if s.archived {
            Some("it is archived")
        } else if !s.status.accepts_messages() && !s.status.resumes_for_messages() {
            Some("it failed")
        } else {
            None
        }
    }

    /// What happened to pull requests its agent opened (`crate::follow_ups`):
    /// kept for [`follow_ups::GATHER_MS`], so that what comes together (a
    /// review and its comments) reaches the agent in one message
    /// ([`Self::flush_follow_ups`]). What it passed on before stays out;
    /// what comes again while it waits takes the place of what came first,
    /// as the checks that failed by then.
    async fn take_follow_ups(
        &self,
        items: Vec<follow_ups::Item>,
    ) -> std::result::Result<internal::FollowUpsTaken, ApiError> {
        let Some(s) = self.summary() else {
            return Err(api_err(ErrorCode::NotFound, "Session not initialized"));
        };
        if let Some(why) = Self::no_follow_ups(&s) {
            return Ok(internal::FollowUpsTaken {
                taken: 0,
                not_sent: Some(why.to_owned()),
            });
        }
        let now = now_ms();
        let mut taken = 0;
        for item in items {
            if self.keyed(&item.key).is_some() {
                continue;
            }
            let Ok(data) = serde_json::to_string(&item) else {
                continue;
            };
            // Again while it waits (GitHub sent it twice): taken already.
            let again = self.follow_ups_waiting(Some(&item.key));
            let stored = self.sql.exec(
                "INSERT INTO follow_ups (key, n, due, data) VALUES (?, (SELECT COALESCE(MAX(n), 0) + 1 FROM follow_ups), ?, ?) \
                 ON CONFLICT (key) DO UPDATE SET data = excluded.data",
                Some(vec![sv(&item.key), iv(now + follow_ups::GATHER_MS), sv(&data)]),
            );
            match stored {
                Ok(_) if !again => taken += 1,
                Ok(_) => {}
                Err(e) => worker::console_error!("keeping a follow-up: {e}"),
            }
        }
        if taken > 0 {
            self.arm(now + follow_ups::GATHER_MS).await;
        }
        Ok(internal::FollowUpsTaken {
            taken,
            not_sent: None,
        })
    }

    /// Passes on what happened to the agent's pull requests once the first
    /// of it has waited [`follow_ups::GATHER_MS`]: one message from fctl for
    /// each pull request, delivered as a person's message is, which resumes
    /// the session if it stopped. Failed checks and conflicts wait while a
    /// turn runs, since the agent may be at them, and then wait a little
    /// longer for what it pushed as its turn ended; they are dropped once
    /// they are old news: the pull request merged or closed, its checks
    /// passed after a re-run, or a newer commit came. After
    /// [`follow_ups::MAX_CHECKS_IN_A_ROW`] failed checks in a row while no
    /// person messaged the agent, nor wrote to it on GitHub, the transcript
    /// says what failed instead, and likewise for conflicts.
    async fn flush_follow_ups(&self, now: i64) {
        #[derive(Deserialize)]
        struct Waiting {
            key: String,
            due: f64,
            data: String,
        }
        let waiting: Vec<Waiting> = self
            .sql
            .exec("SELECT key, due, data FROM follow_ups ORDER BY n", None)
            .and_then(|c| c.to_array())
            .unwrap_or_default();
        if waiting.is_empty() {
            return;
        }
        let Some(s) = self.summary() else { return };
        if let Some(why) = Self::no_follow_ups(&s) {
            worker::console_log!(
                "dropping {} follow-ups of session {}: {why}",
                waiting.len(),
                s.id
            );
            let _ = self.sql.exec("DELETE FROM follow_ups", None);
            return;
        }
        // Failed checks and conflicts wait for the turn to end, which brings
        // them back (`apply_state`); the rest go once the first has waited
        // its time.
        let running = s.status == SessionStatus::Running;
        let held = |w: &Waiting| {
            running
                && follow_ups::Repeats::ALL
                    .iter()
                    .any(|kind| w.key.starts_with(kind.key()))
        };
        let Some(first) = waiting
            .iter()
            .filter(|w| !held(w))
            .map(|w| w.due as i64)
            .min()
        else {
            return;
        };
        if first > now {
            self.arm(first).await;
            return;
        }
        let done = |key: &str| {
            let _ = self
                .sql
                .exec("DELETE FROM follow_ups WHERE key = ?", Some(vec![sv(key)]));
        };
        let mut send = Vec::new();
        for w in waiting {
            let Ok(item) = serde_json::from_str::<follow_ups::Item>(&w.data) else {
                done(&w.key);
                continue;
            };
            let still = match &item.event {
                FollowUpEvent::ChecksFailed { sha, .. } => {
                    Some(Self::still_failing(&s, &item, sha))
                }
                FollowUpEvent::Conflicts { sha, .. } => Some(follow_ups::conflicts_still(
                    s.pull_requests
                        .iter()
                        .find(|p| p.url.eq_ignore_ascii_case(&item.pull.url)),
                    sha,
                    item.seen,
                )),
                _ => None,
            };
            if let Some(still) = still {
                if !still {
                    done(&w.key);
                    continue;
                }
                // The agent may be at them: they wait for its turn to end.
                if running {
                    continue;
                }
            }
            done(&w.key);
            send.push(item);
        }
        for (f, keys) in follow_ups::group(&send) {
            self.pass_on(f, keys, now).await;
        }
    }

    /// Whether failed checks of `sha` are still news: their pull request,
    /// as GitHub last said of it, is neither merged nor closed, and its
    /// checks are still that commit's and still failed. A newer commit, the
    /// agent's push or anyone's, makes GitHub say so within seconds.
    fn still_failing(s: &Session, item: &follow_ups::Item, sha: &str) -> bool {
        let Some(pr) = s
            .pull_requests
            .iter()
            .find(|p| p.url.eq_ignore_ascii_case(&item.pull.url))
        else {
            return true;
        };
        if !follow_ups::is_open(pr.state.as_deref()) {
            return false;
        }
        pr.checks
            .as_ref()
            .is_none_or(|c| c.sha.as_deref().is_none_or(|c_sha| c_sha == sha) && c.failed > 0)
    }

    /// Hands what happened to one pull request (`keys`: its items) to the
    /// agent in a message from fctl, or, for failed checks or conflicts the
    /// agent heard of too often in a row, says in the transcript what
    /// happened.
    async fn pass_on(&self, mut f: FollowUp, keys: Vec<String>, now: i64) {
        // People wrote to the agent: as a person's message, that ends the
        // automatic ones in a row.
        if follow_ups::from_people(&f) {
            self.delete_meta(REVIEWS_PASSED_ON);
        }
        for kind in follow_ups::Repeats::ALL {
            let before = self
                .meta(in_a_row_key(kind))
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0);
            let (in_a_row, held) = follow_ups::in_a_row(&f, kind, before);
            if held {
                if let Some(message) = follow_ups::held_back(&f, kind) {
                    let notice = fctl_proto::event::Notice {
                        ticket: None,
                        level: fctl_proto::event::NoticeLevel::Info,
                        code: "follow_up_held".into(),
                        message,
                        until: None,
                        published: None,
                        memory: None,
                        retrospective: None,
                        review: None,
                        follow_up: Some(FollowUp {
                            events: f.events.iter().filter(|e| kind.of(e)).cloned().collect(),
                            ..f.clone()
                        }),
                    };
                    if let Some(json) = self.append(now, &EventBody::Notice(notice), None) {
                        self.broadcast_events(&[json]);
                    }
                }
                for key in keys.iter().filter(|k| k.starts_with(kind.key())) {
                    self.key(key, "", now);
                }
                f.events.retain(|e| !kind.of(e));
            }
            if in_a_row != before {
                self.set_meta(in_a_row_key(kind), &in_a_row.to_string());
            }
        }
        if f.events.is_empty() {
            return;
        }
        let text = follow_ups::message(&f);
        let id = fctl_domain::ids::delivery_id(now);
        for key in &keys {
            if self.keyed(key).is_none() {
                self.key(key, id.as_str(), now);
            }
        }
        let delivered = self
            .deliver(Deliver {
                delivery: Delivery {
                    id: id.clone(),
                    kind: DeliveryKind::Message {
                        text: text.clone(),
                        author: None,
                        author_called: None,
                        attachments: Vec::new(),
                    },
                },
                client_key: None,
                event: EventBody::UserMessage(UserMessage {
                    ticket: None,
                    id,
                    author: None,
                    text,
                    source: fctl_proto::event::MessageSource::System,
                    attachments: Vec::new(),
                    follow_up: Some(f),
                    review: None,
                }),
            })
            .await;
        // A resume that fails leaves the message in the outbox, for the next run.
        if let Err(e) = delivered {
            worker::console_error!("passing a follow-up on: {}", e.message);
        }
    }

    /// A message resumes a stopped session, on the Claude subscription it
    /// ran on (a resume runs it on the resumer's), and the message waits in
    /// the outbox for the new run. One that is still stopping is queued now:
    /// FleetDO starts it once the last run has stopped.
    async fn wake(&self, now: i64) -> std::result::Result<(), ApiError> {
        let Some(stopped) = self.summary().filter(|s| s.status.resumes_for_messages()) else {
            return Ok(());
        };
        let mut s = stopped.clone();
        self.set_status(
            &mut s,
            SessionStatus::Queued,
            Some("Resuming".into()),
            None,
            now,
        );
        s.runner = None;
        self.publish(&mut s, now).await;
        let scheduled: std::result::Result<serde_json::Value, ApiError> =
            match internal::fleet_stub(&self.env) {
                Ok(fleet) => internal::post(
                    &fleet,
                    "/schedule",
                    &internal::Schedule {
                        session_id: s.id.clone(),
                    },
                )
                .await
                .map_err(|e| e.0),
                Err(e) => Err(e.0),
            };
        if let Err(e) = scheduled {
            // Stopped as before: sending the message again resumes it.
            worker::console_error!("resuming for a message failed: {}", e.message);
            let mut s = stopped;
            let (status, reason) = (s.status, s.status_reason.clone());
            self.set_status(&mut s, status, reason, None, now);
            self.publish(&mut s, now).await;
            return Err(e);
        }
        Ok(())
    }

    /// Its runner drains: the session stops now if it has waited for a
    /// message for a while and none is on its way to the agent. Checking and
    /// marking it stopping happen together, so a message that comes after
    /// waits for the next run (`deliver`), which starts on the same runner.
    async fn drain_stop(&self, b: DrainStop) -> std::result::Result<DrainStopped, ApiError> {
        let now = now_ms();
        let meta_ms = |key: &str| self.meta(key).and_then(|v| v.parse::<i64>().ok());
        let Some(mut s) = self.summary() else {
            return Ok(DrainStopped { stopping: false });
        };
        let idle_since = meta_ms("idle_since");
        let waited = s.status == SessionStatus::Idle
            && idle_since.is_some_and(|t| now - t >= DRAIN_IDLE_MS)
            // A message the agent took after it went idle starts a turn.
            && meta_ms("last_delivered").is_none_or(|d| idle_since.is_some_and(|t| d < t))
            && !self.messages_waiting();
        if !waited {
            return Ok(DrainStopped { stopping: false });
        }
        self.set_status(
            &mut s,
            SessionStatus::Stopping,
            Some(format!("Stopping: {} is draining", b.runner_name)),
            None,
            now,
        );
        self.publish(&mut s, now).await;
        Ok(DrainStopped { stopping: true })
    }

    /// Stores a changed summary and tells viewers, D1 and the dashboards.
    async fn publish(&self, s: &mut Session, now: i64) {
        s.updated_at = now;
        self.save_summary(s);
        self.broadcast_summary(s);
        self.write_through().await;
        self.notify_fleet(s);
    }

    /// Whether follow-ups wait to reach the agent: the one by `key`, or any.
    fn follow_ups_waiting(&self, key: Option<&str>) -> bool {
        #[derive(Deserialize)]
        struct N {
            n: f64,
        }
        let counted = match key {
            Some(k) => self.sql.exec(
                "SELECT COUNT(*) AS n FROM follow_ups WHERE key = ?",
                Some(vec![sv(k)]),
            ),
            None => self.sql.exec("SELECT COUNT(*) AS n FROM follow_ups", None),
        };
        counted
            .and_then(|c| c.to_array::<N>())
            .ok()
            .and_then(|r| r.first().map(|x| x.n > 0.0))
            .unwrap_or(false)
    }

    /// Whether messages wait in the outbox for the agent to take them.
    fn messages_waiting(&self) -> bool {
        self.sql
            .exec("SELECT data FROM outbox", None)
            .and_then(|c| c.to_array::<OutboxRow>())
            .unwrap_or_default()
            .iter()
            .filter_map(|r| serde_json::from_str::<Delivery>(&r.data).ok())
            .any(|d| matches!(d.kind, DeliveryKind::Message { .. }))
    }

    /// Whether the connected host said hello without the attachments feature.
    fn uplink_lacks_files(&self) -> bool {
        self.uplink()
            .and_then(|ws| ws.deserialize_attachment::<SocketInfo>().ok().flatten())
            .and_then(|info| info.features)
            .is_some_and(|f| !f.iter().any(|x| x == FEATURE_ATTACHMENTS))
    }

    /// Whether files wait for the host: in unconfirmed messages, or with the
    /// task of a session that has not started.
    fn files_waiting(&self) -> bool {
        let task = self.meta("task_files").is_some()
            && self.summary().is_some_and(|s| s.started_at.is_none());
        task || self
            .sql
            .exec("SELECT data FROM outbox", None)
            .and_then(|c| c.to_array::<OutboxRow>())
            .unwrap_or_default()
            .iter()
            .filter_map(|r| serde_json::from_str::<Delivery>(&r.data).ok())
            .any(|d| has_files(&d))
    }

    fn old_host_notice(&self, now: i64) {
        let runner = self.summary().and_then(|s| s.runner).map_or_else(
            || "This session's runner".to_owned(),
            |r| format!("The runner {}", r.name),
        );
        let notice = fctl_proto::event::Notice {
            ticket: None,
            level: fctl_proto::event::NoticeLevel::Warn,
            code: "attachments_unsupported".into(),
            message: format!(
                "{runner} runs a version of fctl that cannot pass files to agents, so the agent gets messages without their attachments. Run `fctl self update` on it, then send the files again."
            ),
            until: None,
            published: None,
            memory: None,
            retrospective: None,
            review: None,
            follow_up: None,
        };
        if let Some(json) = self.append(now, &EventBody::Notice(notice), None) {
            self.broadcast_events(&[json]);
        }
    }

    async fn control(&self, b: ControlState) -> std::result::Result<serde_json::Value, ApiError> {
        let Some(mut s) = self.summary() else {
            return Err(api_err(ErrorCode::NotFound, "Session not initialized"));
        };
        let now = now_ms();
        let before = s.status;
        let mut described = false;
        if let Some(t) = b.title {
            described |= s.title != t;
            s.title = t;
        }
        if let Some(d) = b.description {
            let d = (!d.is_empty()).then_some(d);
            described |= s.description != d;
            s.description = d;
        }
        if let Some(a) = b.archived {
            s.archived = a;
        }
        if let Some(u) = b.claude_user
            && let Ok(json) = serde_json::to_string(&u)
        {
            self.set_meta("claude_user", &json);
        }
        let new_runner = b.runner.is_some() || b.clear_runner;
        if let Some(r) = b.runner {
            s.runner = Some(r);
        }
        if b.clear_runner {
            s.runner = None;
        }
        // When the last run's runner was seen is no news for this one.
        if new_runner {
            self.delete_meta("runner_seen_at");
        }
        let mut status_changed = false;
        if let Some(st) = b.status
            && (st != s.status || b.reason != s.status_reason)
        {
            status_changed = true;
            self.set_status(&mut s, st, b.reason.clone(), None, now);
        }
        if let Some(notice) = b.notice
            && let Some(json) = self.append(now, &EventBody::Notice(notice), None)
        {
            self.broadcast_events(&[json]);
        }
        s.updated_at = now;
        self.save_summary(&s);
        self.broadcast_summary(&s);
        // After the summary: viewers of a session that the fleet stopped
        // because its runner stayed away don't take this for the runner's
        // return.
        if new_runner {
            self.broadcast_runner(None);
        }
        // The API wrote the title to D1 already; writing it again now keeps
        // a write-through that raced it from leaving the old one there.
        if status_changed || described {
            self.write_through().await;
        }
        if described {
            self.mem.borrow_mut().search_due = true;
            self.search_soon(now).await;
        }
        self.notify_fleet(&s);
        self.notify_followers(&s, before, now);
        Ok(serde_json::json!({ "ok": true }))
    }

    fn set_status(
        &self,
        s: &mut Session,
        status: SessionStatus,
        reason: Option<String>,
        resume_at: Option<i64>,
        now: i64,
    ) {
        if status != SessionStatus::Idle {
            self.delete_meta("idle_since");
        } else if s.status != SessionStatus::Idle {
            self.set_meta("idle_since", &now.to_string());
        }
        s.status = status;
        s.status_reason = reason.clone();
        s.resume_at = resume_at;
        ev::apply_status(&mut s.stats, status == SessionStatus::Running, now);
        if matches!(status, SessionStatus::Running | SessionStatus::Idle) && s.started_at.is_none()
        {
            s.started_at = Some(now);
        }
        if status.is_terminal() {
            s.ended_at = Some(now);
        } else {
            s.ended_at = None;
        }
        if let Some(json) = self.append(
            now,
            &EventBody::Status(StatusChange {
                status,
                reason,
                resume_at,
            }),
            None,
        ) {
            self.broadcast_events(&[json]);
        }
    }

    fn events_page(&self, req: &Request) -> Result<Response> {
        let param = |k: &str| crate::http::query(req, k).and_then(|v| v.parse::<i64>().ok());
        let limit = param("limit").unwrap_or(200).clamp(1, 1000);
        let (rows, newest_first) = if let Some(before) = param("before") {
            (
                self.load_events(
                    "SELECT seq, ts, kind, data FROM events WHERE seq < ? ORDER BY seq DESC LIMIT ?",
                    vec![iv(before), iv(limit + 1)],
                ),
                true,
            )
        } else {
            let after = param("after").unwrap_or(0);
            (
                self.load_events(
                    "SELECT seq, ts, kind, data FROM events WHERE seq > ? ORDER BY seq ASC LIMIT ?",
                    vec![iv(after), iv(limit + 1)],
                ),
                false,
            )
        };
        let more_rows = rows.len() as i64 > limit;
        let rows: Vec<EventRow> = rows.into_iter().take(limit as usize).collect();
        let (events, truncated) = Self::render(rows, SNAPSHOT_BYTES, newest_first);
        let has_more = more_rows || truncated;
        let body = format!(
            r#"{{"events":[{}],"has_more":{has_more}}}"#,
            events.join(",")
        );
        let _ = EventPage {
            events: vec![],
            has_more,
        };
        let h = worker::Headers::new();
        h.set("Content-Type", "application/json")?;
        Ok(Response::ok(body)?.with_headers(h))
    }

    // ───────────────────────────── viewers ─────────────────────────────

    fn accept_viewer(&self, req: &Request) -> Result<Response> {
        let ident: ViewerIdentity = match crate::http::header(req, internal::HDR_VIEWER)
            .and_then(|h| serde_json::from_str(&h).ok())
        {
            Some(i) => i,
            None => {
                return internal::do_error(&api_err(ErrorCode::Unauthorized, "No viewer identity"));
            }
        };
        let Some(summary) = self.summary() else {
            return internal::do_error(&api_err(ErrorCode::NotFound, "Session not initialized"));
        };
        let after = crate::http::query(req, "after").and_then(|v| v.parse::<i64>().ok());
        let pair = WebSocketPair::new()?;
        let now = now_ms();
        self.check_quiet(now);
        let role = ident.role;
        // The Worker checked its access just now.
        let info = SocketInfo {
            viewer: Some(ident),
            since: now,
            checked_at: Some(now),
            ..Default::default()
        };
        self.state
            .accept_websocket_with_tags(&pair.server, &[TAG_VIEWER]);
        pair.server.serialize_attachment(&info)?;
        let snapshot = self.snapshot(summary, role, after);
        pair.server.send_with_str(&snapshot)?;
        self.broadcast_presence(None);
        Response::from_websocket(pair.client)
    }

    fn snapshot(&self, mut summary: Session, role: SessionRole, after: Option<i64>) -> String {
        summary.role = Some(role);
        let (events, has_more_before) = match after {
            Some(a) => {
                let rows = self.load_events(
                    "SELECT seq, ts, kind, data FROM events WHERE seq > ? ORDER BY seq ASC LIMIT 5000",
                    vec![iv(a)],
                );
                let (ev, _) = Self::render(rows, SNAPSHOT_BYTES * 3, false);
                (ev, a > 0)
            }
            None => {
                let rows = self.load_events(
                    "SELECT seq, ts, kind, data FROM events ORDER BY seq DESC LIMIT ?",
                    vec![iv(SNAPSHOT_EVENTS + 1)],
                );
                let more = rows.len() as i64 > SNAPSHOT_EVENTS;
                let rows: Vec<EventRow> = rows.into_iter().take(SNAPSHOT_EVENTS as usize).collect();
                let (ev, truncated) = Self::render(rows, SNAPSHOT_BYTES, true);
                (ev, more || truncated)
            }
        };
        let live = self.mem.borrow().live.to_json_elements();
        let viewers = serde_json::to_string(&self.presence(None)).unwrap_or_else(|_| "[]".into());
        let session = serde_json::to_string(&summary).unwrap_or_else(|_| "null".into());
        let role = serde_json::to_string(&role).unwrap_or_else(|_| "\"viewer\"".into());
        let runner = self.runner_view(None);
        let mut runner_fields = String::new();
        if let Some(t) = runner.last_seen_at {
            runner_fields.push_str(&format!(r#","runner_last_seen_at":{t}"#));
        }
        if let Some(a) = runner
            .agent_unreachable
            .and_then(|a| serde_json::to_string(&a).ok())
        {
            runner_fields.push_str(&format!(r#","agent_unreachable":{a}"#));
        }
        format!(
            r#"{{"type":"snapshot","session":{session},"role":{role},"events":[{}],"has_more_before":{has_more_before},"next_seq":{},"live":[{live}],"viewers":{viewers},"runner_connected":{}{runner_fields}}}"#,
            events.join(","),
            self.next_seq(),
            runner.connected
        )
    }

    async fn on_viewer(&self, ws: &WebSocket, msg: ViewerClient) {
        let Ok(Some(mut info)) = ws.deserialize_attachment::<SocketInfo>() else {
            return;
        };
        if info.viewer.is_none() {
            return;
        }
        let reply = |client_id: &str, result: std::result::Result<(), ApiError>| {
            let msg = match result {
                Ok(()) => fctl_proto::viewer::ViewerServer::Result {
                    client_id: client_id.to_owned(),
                    ok: true,
                    error: None,
                },
                Err(e) => fctl_proto::viewer::ViewerServer::Result {
                    client_id: client_id.to_owned(),
                    ok: false,
                    error: Some(e),
                },
            };
            if let Ok(t) = serde_json::to_string(&msg) {
                let _ = ws.send_with_str(&t);
            }
        };
        match msg {
            ViewerClient::Ping => {
                let _ = ws.send_with_str(r#"{"type":"pong"}"#);
                let now = now_ms();
                self.check_quiet(now);
                if now - info.checked() >= VIEWER_CHECK_MS {
                    // A failed check leaves it for the next ping.
                    let _ = self.recheck_viewer(ws, &mut info, now).await;
                }
            }
            ViewerClient::Typing => {
                info.typing_at = Some(now_ms());
                let _ = ws.serialize_attachment(&info);
                self.broadcast_presence(None);
            }
            ViewerClient::Visibility { visible } => {
                info.hidden = !visible;
                let _ = ws.serialize_attachment(&info);
            }
            ViewerClient::Send {
                client_id,
                text,
                attachments,
            } => {
                // What a message does is checked as the Worker would now.
                let r = match self.recheck_viewer(ws, &mut info, now_ms()).await {
                    Ok(Some(viewer)) => {
                        self.viewer_send(&viewer, &client_id, text, attachments)
                            .await
                    }
                    Ok(None) => return,
                    Err(e) => Err(e),
                };
                if r.is_ok() {
                    info.typing_at = None;
                    let _ = ws.serialize_attachment(&info);
                }
                reply(&client_id, r);
            }
            ViewerClient::Interrupt { client_id } => {
                let r = match self.recheck_viewer(ws, &mut info, now_ms()).await {
                    Ok(Some(viewer)) => self.viewer_interrupt(&viewer).await,
                    Ok(None) => return,
                    Err(e) => Err(e),
                };
                reply(&client_id, r);
            }
        }
    }

    /// Checks again what an open viewer may do, as the Worker checked it
    /// when it opened: the token it opened with still signs its person in,
    /// and they still see the session. A viewer whose person lost access
    /// (they left the workspace or were removed, their role or share went,
    /// they signed out or the account was disabled) closes, and `None` says
    /// so; one whose role changed gets the new one. `Err` when D1 didn't
    /// answer, which changes nothing.
    async fn recheck_viewer(
        &self,
        ws: &WebSocket,
        info: &mut SocketInfo,
        now: i64,
    ) -> std::result::Result<Option<ViewerIdentity>, ApiError> {
        let Some(viewer) = info.viewer.clone() else {
            return Ok(None);
        };
        let Some(summary) = self.summary() else {
            return Err(api_err(ErrorCode::NotFound, "Session not initialized"));
        };
        // Opened through an older Worker: through this one, it says.
        let Some(token_id) = viewer.token_id.as_deref() else {
            let _ = ws.close(Some(CLOSE_RECHECK), Some("checking your access again"));
            return Ok(None);
        };
        let db = self
            .env
            .d1("DB")
            .map_err(|e| api_err(ErrorCode::Internal, &e.to_string()))?;
        // What agents call them goes with their messages as it is now.
        let (role, called) = match crate::auth::by_token_id(&db, token_id, now)
            .await
            .map_err(|e| e.0)?
        {
            Some(h) if h.user.id == viewer.user.id.0 => (
                crate::access::session_role_now(
                    &db,
                    &h.user,
                    &h.settings,
                    h.limits.as_ref(),
                    summary.id.as_str(),
                )
                .await
                .map_err(|e| e.0)?,
                h.user.called(),
            ),
            _ => (None, None),
        };
        let Some(role) = role else {
            let _ = ws.close(
                Some(CLOSE_NO_ACCESS),
                Some("You no longer have access to this session"),
            );
            self.broadcast_presence(Some(ws));
            return Ok(None);
        };
        let changed = role != viewer.role;
        let viewer = ViewerIdentity {
            role,
            called,
            ..viewer
        };
        info.viewer = Some(viewer.clone());
        info.checked_at = Some(now);
        let _ = ws.serialize_attachment(&*info);
        if changed {
            Self::send_summary(ws, &summary, Some(role));
        }
        Ok(Some(viewer))
    }

    async fn viewer_send(
        &self,
        viewer: &ViewerIdentity,
        client_id: &str,
        text: String,
        attachments: Vec<AttachmentId>,
    ) -> std::result::Result<(), ApiError> {
        if !viewer.role.can_message() {
            return Err(api_err(
                ErrorCode::Forbidden,
                "You can view this session but not message the agent",
            ));
        }
        if self.summary().is_some_and(|s| s.review.is_some()) {
            return Err(api_err(ErrorCode::Conflict, REVIEWS_TAKE_NO_MESSAGES));
        }
        let files = fctl_domain::validate::attachment_ids(&attachments)?;
        let text = fctl_domain::validate::message_with_files("text", &text, files.len())?;
        let Some(summary) = self.summary() else {
            return Err(api_err(ErrorCode::NotFound, "Session not initialized"));
        };
        if !summary.status.accepts_messages() && !summary.status.resumes_for_messages() {
            return Err(api_err(
                ErrorCode::Conflict,
                "The session failed. Resume it to continue.",
            ));
        }
        let author = viewer.user.clone();
        let attachments = internal::claim_files(
            &self.env,
            summary.project_id.as_str(),
            &summary.id,
            author.id.as_str(),
            &files,
        )
        .await
        .map_err(|e| e.0)?;
        let now = now_ms();
        let id = fctl_domain::ids::delivery_id(now);
        self.deliver(Deliver {
            delivery: Delivery {
                id: id.clone(),
                kind: DeliveryKind::Message {
                    text: text.clone(),
                    author: Some(author.clone()),
                    author_called: viewer.called.clone(),
                    attachments: attachments.clone(),
                },
            },
            client_key: Some(format!("{}:{client_id}", author.id)),
            event: EventBody::UserMessage(UserMessage {
                ticket: None,
                id,
                author: Some(author),
                text,
                source: fctl_proto::event::MessageSource::Web,
                attachments,
                follow_up: None,
                review: None,
            }),
        })
        .await
        .map(|_| ())
    }

    async fn viewer_interrupt(&self, viewer: &ViewerIdentity) -> std::result::Result<(), ApiError> {
        if !viewer.role.can_message() {
            return Err(api_err(
                ErrorCode::Forbidden,
                "You can view this session but not interrupt the agent",
            ));
        }
        if self.summary().is_some_and(|s| s.review.is_some()) {
            return Err(api_err(ErrorCode::Conflict, REVIEWS_TAKE_NO_MESSAGES));
        }
        let now = now_ms();
        let id = fctl_domain::ids::delivery_id(now);
        let by: UserRef = viewer.user.clone();
        self.deliver(Deliver {
            delivery: Delivery {
                id: id.clone(),
                kind: DeliveryKind::Interrupt {
                    by: Some(by.clone()),
                },
            },
            client_key: None,
            event: EventBody::Interrupt(fctl_proto::event::Interrupt { id, by: Some(by) }),
        })
        .await
        .map(|_| ())
    }

    // ───────────────────────────── uplink ─────────────────────────────

    /// Closes `runner_id`'s uplinks (`internal::CloseUplink`).
    fn close_uplink(&self, runner_id: &str) -> serde_json::Value {
        for ws in self.state.get_websockets_with_tag(TAG_UPLINK) {
            let theirs = ws
                .deserialize_attachment::<SocketInfo>()
                .ok()
                .flatten()
                .is_some_and(|i| i.runner_id.as_deref() == Some(runner_id));
            if theirs {
                let _ = ws.close(Some(4003), Some("this runner no longer runs this session"));
            }
        }
        serde_json::json!({ "ok": true })
    }

    fn accept_uplink(&self, req: &Request) -> Result<Response> {
        let Some(runner_id) = crate::http::header(req, internal::HDR_RUNNER) else {
            return internal::do_error(&api_err(ErrorCode::Unauthorized, "No runner identity"));
        };
        for old in self.state.get_websockets_with_tag(TAG_UPLINK) {
            let _ = old.close(Some(4001), Some("replaced by a newer connection"));
        }
        let pair = WebSocketPair::new()?;
        self.state
            .accept_websocket_with_tags(&pair.server, &[TAG_UPLINK]);
        pair.server.serialize_attachment(&SocketInfo {
            runner_id: Some(runner_id),
            since: now_ms(),
            ..Default::default()
        })?;
        let pending: Vec<Delivery> = self
            .sql
            .exec("SELECT data FROM outbox ORDER BY n ASC", None)
            .and_then(|c| c.to_array::<OutboxRow>())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| serde_json::from_str(&r.data).ok())
            .collect();
        let welcome = UplinkDown::Welcome {
            acked_agent_seq: self.acked_agent_seq(),
            pending,
        };
        if let Ok(text) = serde_json::to_string(&welcome) {
            pair.server.send_with_str(&text)?;
        }
        self.delete_meta("runner_seen_at");
        self.broadcast_runner(None);
        Response::from_websocket(pair.client)
    }

    async fn on_uplink(&self, ws: &WebSocket, msg: UplinkUp) {
        let now = now_ms();
        // Token deltas come by the dozen a second; pings every 20 s suffice.
        if !matches!(msg, UplinkUp::Stream { .. }) {
            self.heard(ws, now);
        }
        match msg {
            UplinkUp::Hello { features, .. } => {
                let supported = features.iter().any(|f| f == FEATURE_ATTACHMENTS);
                if let Ok(Some(mut info)) = ws.deserialize_attachment::<SocketInfo>() {
                    info.features = Some(features);
                    let _ = ws.serialize_attachment(&info);
                }
                if !supported && self.files_waiting() {
                    self.old_host_notice(now);
                }
            }
            UplinkUp::Ping => {
                let _ = ws.send_with_str(r#"{"type":"pong"}"#);
            }
            UplinkUp::Events { events } => {
                let mut acked = self.acked_agent_seq();
                let acked_before = acked;
                let mut out = Vec::new();
                let mut summary = self.summary();
                let merged_before = summary
                    .as_ref()
                    .map_or(0, |s| merged(&s.pull_requests).len());
                let progress_before = summary.as_ref().and_then(progress);
                let mut summary_changed = false;
                let mut lists_changed = false;
                // Pull requests the session lists from now on.
                let mut found = Vec::new();
                let mut turn_ended = false;
                let mut usage = Vec::new();
                // Images from the results of the agent's tools were kept.
                let mut kept_some = false;
                let counted_before = self.counted();
                let mut counted = counted_before.clone();
                // Newer events that came before older ones wait for them
                // (`fctl_domain::journal`).
                let waiting_since = self.meta("waiting_since").and_then(|v| v.parse().ok());
                let mut waits = false;
                for e in events {
                    match journal::take(acked, e.agent_seq, waiting_since, now) {
                        Take::Seen => continue,
                        Take::Wait => {
                            if waiting_since.is_none() {
                                worker::console_log!(
                                    "agent_seq {} came before {}: waiting for the host's replay",
                                    e.agent_seq,
                                    acked + 1
                                );
                                self.set_meta("waiting_since", &now.to_string());
                            }
                            waits = true;
                            break;
                        }
                        Take::Store if e.agent_seq > acked + 1 => worker::console_error!(
                            "agent_seq {} to {} never came; going on without them",
                            acked + 1,
                            e.agent_seq - 1
                        ),
                        Take::Store => {}
                    }
                    acked = e.agent_seq;
                    let mut body = e.body;
                    // People's messages, interrupts and status changes are
                    // the control plane's to record, never an agent's: from
                    // an uplink (a host, or what runs in its sandbox posing
                    // as the agent) they are acknowledged and dropped, so
                    // that nobody's words can be put in the transcript.
                    if matches!(
                        body,
                        EventBody::UserMessage(_) | EventBody::Status(_) | EventBody::Interrupt(_)
                    ) {
                        worker::console_error!("dropping a {} event from an uplink", body.kind());
                        continue;
                    }
                    // Claude's thinking progress is only live, but older
                    // agents store it, about once a second: acknowledged
                    // and dropped.
                    if let EventBody::Claude(v) = &body
                        && ev::is_ephemeral(ev::classify(v))
                    {
                        continue;
                    }
                    if let (EventBody::Claude(v), Some(s)) = (&mut body, summary.as_mut()) {
                        summary_changed |= Self::apply_claude(s, &mut counted, v, e.ts, &self.mem);
                        turn_ended |= self.remember_reply(v, e.agent_seq);
                        self.mirror_to_linear(s, v, now);
                        plan::merge(&mut usage, plan::readings(v, now));
                        kept_some |= self.keep_images(v, now);
                        ev::shape_for_storage(v);
                    }
                    // What runs in the sandbox may pose as the agent, and
                    // the apps show these as the session's own links:
                    // pull requests only as GitHub's own URLs name them,
                    // without checks, which are GitHub's to say.
                    if let EventBody::Git(g) = &mut body {
                        fctl_domain::pulls::from_agent_git(g, now);
                    }
                    // A published screenshot's link only to this instance's.
                    if let EventBody::Notice(n) = &mut body
                        && n.published.as_ref().is_some_and(|p| {
                            !fctl_domain::published::is_link(
                                &p.url,
                                &crate::config::Config::from_env(&self.env).public_url,
                            )
                        })
                    {
                        n.published = None;
                    }
                    // What became of pull requests, with people's comments
                    // on them, is the control plane's to tell, never an
                    // uplink's; and a posted review's link goes only to
                    // GitHub's pull request.
                    if let EventBody::Notice(n) = &mut body {
                        n.follow_up = None;
                        n.ticket = None;
                        if let Some(r) = n.review.as_mut() {
                            r.url = r.url.as_deref().and_then(fctl_domain::reviews::posted_link);
                            r.sent_to = r
                                .sent_to
                                .take()
                                .filter(|s| fctl_proto::SessionId::is_valid(s.as_str()));
                        }
                    }
                    if let (EventBody::Git(g), Some(s)) = (&body, summary.as_mut()) {
                        let (changed, new) = Self::apply_git(s, g, now);
                        lists_changed |= changed;
                        found.extend(new);
                        summary_changed = true;
                    }
                    if let (EventBody::RemoteControl(rc), Some(s)) = (&mut body, summary.as_mut()) {
                        // A link the apps open: only claude.ai's.
                        rc.session_url = rc
                            .session_url
                            .take()
                            .filter(|u| fctl_domain::remote_control::is_session_url(u));
                        rc.user = Some(self.claude_user(s));
                        s.remote_control = Some(rc.clone());
                        summary_changed = true;
                    }
                    if let Some(json) = self.append(e.ts, &body, Some(e.agent_seq)) {
                        out.push(json);
                    }
                    self.pulse(&body, now);
                }
                // The events it waited for came, stored or dropped.
                if waiting_since.is_some() && !waits && acked > acked_before {
                    self.delete_meta("waiting_since");
                }
                self.set_meta("acked_agent_seq", &acked.to_string());
                if counted != counted_before {
                    self.save_counted(counted);
                }
                self.broadcast_events(&out);
                self.send_uplink(&UplinkDown::Ack { agent_seq: acked });
                self.record_usage(usage);
                if kept_some {
                    self.arm(kept_images::expires_at(now)).await;
                }
                if let Some(mut s) = summary {
                    s.last_activity_at = Some(now);
                    s.updated_at = now;
                    self.save_summary(&s);
                    if summary_changed {
                        self.broadcast_summary(&s);
                    }
                    // The dashboards hear of each status change, and of the
                    // changes and pull requests the agent reports, during
                    // turns too (at most every ten seconds).
                    if lists_changed {
                        self.notify_fleet(&s);
                    }
                    if !found.is_empty() {
                        self.track_pull_requests(&s, found);
                    }
                    let merged_more = merged(&s.pull_requests).len() > merged_before;
                    self.want_done(&s, progress_before, merged_more).await;
                    self.move_on_reported(&s, progress_before, turn_ended);
                    // A host that reconnects reports its state before it
                    // replays the events it kept: a turn that ended meanwhile
                    // shows as a result while the session is already idle.
                    if turn_ended && s.status == SessionStatus::Idle {
                        self.notify(&s, notify::Kind::Waiting, now);
                    }
                    self.mark_dirty(now).await;
                }
                self.search_soon(now).await;
            }
            UplinkUp::Stream { event } => {
                let text = event.to_string();
                self.mem.borrow_mut().live.push(&event, &text);
                self.broadcast(&format!(r#"{{"type":"stream","event":{text}}}"#));
                self.pulse_kind("delta", 1, now);
                if ev::reports_usage(&event) {
                    self.count_stream(&event, now).await;
                }
            }
            UplinkUp::State { state } => self.apply_state(state, now).await,
            UplinkUp::Log { line } => {
                if let Some(json) = self.append(now, &EventBody::Log(line), None) {
                    self.broadcast_events(&[json]);
                }
            }
            UplinkUp::Delivered {
                delivery_id,
                ok,
                error,
            } => {
                let _ = self.sql.exec(
                    "DELETE FROM outbox WHERE id = ?",
                    Some(vec![sv(delivery_id.as_str())]),
                );
                // The agent took it: a turn starts, whatever the status says yet.
                self.set_meta("last_delivered", &now.to_string());
                if !ok {
                    let notice = fctl_proto::event::Notice {
                        ticket: None,
                        level: fctl_proto::event::NoticeLevel::Warn,
                        code: "delivery_failed".into(),
                        message: error
                            .unwrap_or_else(|| "The agent could not take a message".into()),
                        until: None,
                        published: None,
                        memory: None,
                        retrospective: None,
                        review: None,
                        follow_up: None,
                    };
                    if let Some(json) = self.append(now, &EventBody::Notice(notice), None) {
                        self.broadcast_events(&[json]);
                    }
                }
            }
            UplinkUp::AgentLink { unreachable } => {
                let unreachable = unreachable.map(|mut a| {
                    if let Some((cut, _)) = a.reason.char_indices().nth(REASON_MAX_CHARS) {
                        a.reason.truncate(cut);
                    }
                    a
                });
                if let Ok(Some(mut info)) = ws.deserialize_attachment::<SocketInfo>()
                    && info.agent_unreachable != unreachable
                {
                    info.agent_unreachable = unreachable;
                    let _ = ws.serialize_attachment(&info);
                    self.broadcast_runner(None);
                }
            }
        }
    }

    /// The uplink sent something: notes when, now and then, and tells viewers
    /// that a runner that had gone quiet is back.
    fn heard(&self, ws: &WebSocket, now: i64) {
        let Ok(Some(mut info)) = ws.deserialize_attachment::<SocketInfo>() else {
            return;
        };
        let was_quiet = info.quiet;
        if was_quiet || info.heard_at.is_none_or(|t| now - t >= HEARD_EVERY_MS) {
            info.quiet = false;
            info.heard_at = Some(now);
            let _ = ws.serialize_attachment(&info);
        }
        if was_quiet {
            self.broadcast_runner(None);
        }
    }

    /// Tells viewers when the uplink has sent nothing for [`QUIET_MS`]. The
    /// object has no timer of its own while sockets hibernate, so viewers'
    /// pings (every 25 s) and new viewers check: it is noticed while someone
    /// watches.
    fn check_quiet(&self, now: i64) {
        let Some(ws) = self.uplink() else { return };
        let Ok(Some(mut info)) = ws.deserialize_attachment::<SocketInfo>() else {
            return;
        };
        if info.quiet || now - info.heard() < QUIET_MS {
            return;
        }
        info.quiet = true;
        let _ = ws.serialize_attachment(&info);
        self.broadcast_runner(None);
    }

    /// The runner as viewers see it, leaving out an uplink that is closing.
    fn runner_view(&self, closing: Option<&WebSocket>) -> RunnerView {
        let uplink = self
            .state
            .get_websockets_with_tag(TAG_UPLINK)
            .into_iter()
            .rfind(|w| closing.is_none_or(|c| c != w));
        match uplink.map(|ws| ws.deserialize_attachment::<SocketInfo>().ok().flatten()) {
            Some(Some(info)) if info.quiet => RunnerView {
                connected: false,
                last_seen_at: Some(info.heard()),
                agent_unreachable: None,
            },
            Some(info) => RunnerView {
                connected: true,
                last_seen_at: None,
                agent_unreachable: info.and_then(|i| i.agent_unreachable),
            },
            None => RunnerView {
                connected: false,
                last_seen_at: self.meta("runner_seen_at").and_then(|v| v.parse().ok()),
                agent_unreachable: None,
            },
        }
    }

    fn broadcast_runner(&self, closing: Option<&WebSocket>) {
        let view = self.runner_view(closing);
        if let Ok(text) = serde_json::to_string(&ViewerServer::RunnerConnected {
            connected: view.connected,
            last_seen_at: view.last_seen_at,
            agent_unreachable: view.agent_unreachable,
        }) {
            self.broadcast(&text);
        }
    }

    /// Updates the summary from a Claude message sent at `ts`; returns
    /// whether it changed visibly.
    fn apply_claude(
        s: &mut Session,
        counted: &mut ev::Counted,
        v: &serde_json::Value,
        ts: i64,
        mem: &RefCell<Mem>,
    ) -> bool {
        let kind = ev::classify(v);
        let before = s.stats.clone();
        let running = s.status == SessionStatus::Running;
        ev::apply_stats(&mut s.stats, counted, v, ts, running);
        let mut changed = before != s.stats;
        match kind {
            ClaudeKind::Init => {
                if let Some(m) = v.get("model").and_then(serde_json::Value::as_str)
                    && s.model.as_deref() != Some(m)
                {
                    s.model = Some(m.to_owned());
                    changed = true;
                }
            }
            ClaudeKind::Assistant => {
                // A complete block arrived: the live buffer is now stale.
                mem.borrow_mut().live.clear();
            }
            _ => {}
        }
        changed
    }

    /// Counts an API call's tokens from the stream events that start and end
    /// its response, which come only live: the turn's result settles what
    /// goes missing.
    async fn count_stream(&self, event: &serde_json::Value, now: i64) {
        let Some(mut s) = self.summary() else { return };
        let before = self.counted();
        let mut counted = before.clone();
        let changed = ev::apply_stream_stats(&mut s.stats, &mut counted, event);
        if counted != before {
            self.save_counted(counted);
        }
        if changed {
            self.save_summary(&s);
            self.broadcast_summary(&s);
            self.mark_dirty(now).await;
        }
    }

    /// Counts changes in every repository, and keeps every pull request the
    /// agent found (`fctl_domain::pulls`). The session's pull request, for
    /// older clients, is the primary repository's, or another repository's
    /// when it has none. Returns whether what session lists show changed
    /// (the counts or the pull requests), and the pull requests it lists
    /// from now on.
    fn apply_git(s: &mut Session, g: &GitState, now: i64) -> (bool, Vec<PullRequestRef>) {
        let listed = |s: &Session| {
            (
                s.stats.files_changed,
                s.stats.additions,
                s.stats.deletions,
                s.pull_request.as_ref().map(|p| p.url.clone()),
            )
        };
        let before = listed(s);
        if let Some(b) = &g.branch {
            s.branch = Some(b.clone());
        }
        s.stats.files_changed = g.all_files().count() as u32;
        s.stats.additions = g.all_files().filter_map(|f| f.additions).sum();
        s.stats.deletions = g.all_files().filter_map(|f| f.deletions).sum();
        let found = fctl_domain::pulls::found(g);
        let known = s.pull_requests.len();
        let merged = fctl_domain::pulls::merge(&mut s.pull_requests, &found, now);
        if let Some(pr) = found.first() {
            // As the list has it now, with what GitHub said of it.
            let listed = s
                .pull_requests
                .iter()
                .find(|p| p.url.eq_ignore_ascii_case(&pr.url));
            s.pull_request = Some(listed.unwrap_or(pr).clone());
        }
        let new = s.pull_requests.get(known..).unwrap_or_default().to_vec();
        (merged || listed(s) != before, new)
    }

    /// Records the pull requests the session lists from now on, so that
    /// GitHub's webhooks about them reach it, and looks them up where the
    /// instance takes the webhook (`crate::pull_requests`).
    fn track_pull_requests(&self, s: &Session, prs: Vec<PullRequestRef>) {
        let env = self.env.clone();
        let id = s.id.as_str().to_owned();
        let linear = Self::answers_linear(s);
        self.state.wait_until(async move {
            if linear && let Err(e) = crate::delegations::pull_requests(&env, &id, &prs).await {
                worker::console_error!(
                    "telling Linear of the pull requests of {id}: {}",
                    e.0.message
                );
            }
            let Ok(db) = env.d1("DB") else { return };
            let tracked = match crate::pull_requests::track(&db, &id, &prs).await {
                Ok(t) => t,
                Err(e) => {
                    worker::console_error!("tracking the pull requests of {id}: {}", e.0.message);
                    return;
                }
            };
            let cfg = crate::config::Config::from_env(&env);
            if crate::pull_requests::enabled(&cfg)
                && let Err(e) =
                    crate::pull_requests::refresh(&env, &cfg, &db, &tracked, now_ms(), true).await
            {
                worker::console_error!("looking up the pull requests of {id}: {}", e.0.message);
            }
        });
    }

    /// GitHub says what became of pull requests the session lists: their
    /// state, and their checks (`crate::pull_requests`).
    async fn pull_requests_seen(
        &self,
        b: internal::PullRequestsSeen,
    ) -> std::result::Result<serde_json::Value, ApiError> {
        let Some(mut s) = self.summary() else {
            return Err(api_err(ErrorCode::NotFound, "Session not initialized"));
        };
        let now = now_ms();
        let merged_before = merged(&s.pull_requests).len();
        let progress_before = progress(&s);
        let changed = fctl_domain::pulls::observe(&mut s.pull_requests, &b.prs, now);
        if changed {
            let merged_more = merged(&s.pull_requests).len() > merged_before;
            self.want_done(&s, progress_before, merged_more).await;
            self.pull_requests_changed(s, now).await;
        }
        // After the pull requests, which tell whether failed checks are news.
        if !b.follow_ups.is_empty() {
            let taken = self.take_follow_ups(b.follow_ups).await?;
            if let Some(why) = taken.not_sent {
                worker::console_log!("no follow-ups for this session: {why}");
            }
        }
        if !b.asks.is_empty() {
            self.take_asks(b.asks, now).await;
        }
        Ok(serde_json::json!({ "ok": true, "changed": changed }))
    }

    /// Pull requests its agent opened that GitHub didn't know yet whether
    /// they conflict with their base branch: the session asks again when
    /// [`follow_ups::ask_again_in`] says, sooner for one asked about fewer
    /// times, and its alarm does ([`Self::ask_again`]). None for sessions
    /// that hear of nothing.
    async fn take_asks(&self, asks: Vec<follow_ups::Ask>, now: i64) {
        let Some(s) = self.summary() else { return };
        if Self::no_follow_ups(&s).is_some() {
            return;
        }
        let mut first: Option<i64> = None;
        for a in asks {
            let Some(after) = follow_ups::ask_again_in(a.asked) else {
                continue;
            };
            let due = now + after;
            let stored = self.sql.exec(
                "INSERT INTO asks (url, repo, number, asked, due) VALUES (?, ?, ?, ?, ?) \
                 ON CONFLICT (url) DO UPDATE SET asked = MIN(asked, excluded.asked), due = MIN(due, excluded.due)",
                Some(vec![
                    sv(&a.url.to_ascii_lowercase()),
                    sv(&a.repo.full_name()),
                    iv(i64::try_from(a.number).unwrap_or(i64::MAX)),
                    iv(i64::from(a.asked)),
                    iv(due),
                ]),
            );
            match stored {
                Ok(_) => first = Some(first.map_or(due, |f| f.min(due))),
                Err(e) => worker::console_error!("keeping an ask: {e}"),
            }
        }
        if let Some(due) = first {
            self.arm(due).await;
        }
    }

    /// Asks GitHub again about the pull requests whose time came
    /// ([`Self::take_asks`]), through the control plane, which tells the
    /// sessions that list them what it says, this one's agent of their
    /// conflicts, and this one again of those GitHub still doesn't know
    /// about (`crate::pull_requests::ask_again`). Sets the alarm for the
    /// next.
    async fn ask_again(&self, now: i64) {
        #[derive(Deserialize)]
        struct Asked {
            url: String,
            repo: String,
            number: f64,
            asked: f64,
            due: f64,
        }
        let asked: Vec<Asked> = self
            .sql
            .exec("SELECT url, repo, number, asked, due FROM asks", None)
            .and_then(|c| c.to_array())
            .unwrap_or_default();
        if asked.is_empty() {
            return;
        }
        let (due, later): (Vec<Asked>, Vec<Asked>) =
            asked.into_iter().partition(|a| a.due as i64 <= now);
        if let Some(next) = later.iter().map(|a| a.due as i64).min() {
            self.arm(next).await;
        }
        if due.is_empty() {
            return;
        }
        for a in &due {
            let _ = self
                .sql
                .exec("DELETE FROM asks WHERE url = ?", Some(vec![sv(&a.url)]));
        }
        let asks: Vec<follow_ups::Ask> = due
            .into_iter()
            .filter_map(|a| {
                Some(follow_ups::Ask {
                    repo: RepoRef::parse(&a.repo)?,
                    number: a.number as u64,
                    url: a.url,
                    asked: a.asked as u32,
                })
            })
            .collect();
        let env = self.env.clone();
        self.state.wait_until(async move {
            let Ok(db) = env.d1("DB") else { return };
            let cfg = crate::config::Config::from_env(&env);
            if let Err(e) = crate::pull_requests::ask_again(&env, &cfg, &db, &asks, now_ms()).await
            {
                worker::console_error!("asking whether pull requests conflict: {}", e.0.message);
            }
        });
    }

    /// Keeps the session's pull requests as GitHub said they are now, and
    /// tells viewers, the dashboards and D1.
    async fn pull_requests_changed(&self, mut s: Session, now: i64) {
        if let Some(pr) = &s.pull_request
            && let Some(listed) = s
                .pull_requests
                .iter()
                .find(|p| p.url.eq_ignore_ascii_case(&pr.url))
        {
            s.pull_request = Some(listed.clone());
        }
        self.save_summary(&s);
        self.broadcast_summary(&s);
        self.notify_fleet(&s);
        self.mark_dirty(now).await;
    }

    async fn apply_state(&self, state: SessionState, now: i64) {
        let Some(mut s) = self.summary() else { return };
        // Queued again (a message or a resume) while its last run stops: that
        // run's uplink still says so. FleetDO starts the next run after it.
        if s.status == SessionStatus::Queued
            && (state.status.is_terminal() || state.status == SessionStatus::Stopping)
        {
            return;
        }
        let before = s.status;
        let mut status_changed = false;
        if state.status != s.status
            || state.reason != s.status_reason
            || state.resume_at != s.resume_at
        {
            status_changed = state.status != s.status;
            self.set_status(
                &mut s,
                state.status,
                state.reason.clone(),
                state.resume_at,
                now,
            );
        }
        let described = self.apply_agent_description(&mut s, &state);
        if let Some(b) = state.branch {
            s.branch = Some(b);
        }
        if let Some(m) = state.model {
            s.model = Some(m);
        }
        // Empty when Claude's model takes no effort level.
        if let Some(e) = state.effort {
            s.effort = fctl_domain::validate::effort(&e).ok();
        }
        if let Some(mut rc) = state.remote_control {
            rc.session_url = rc
                .session_url
                .take()
                .filter(|u| fctl_domain::remote_control::is_session_url(u));
            rc.user = Some(self.claude_user(&s));
            s.remote_control = Some(rc);
        }
        if let Some(st) = state.stats {
            s.stats = st;
        }
        if let Some(c) = state.context {
            s.stats.context_window = Some(c.tokens);
            s.stats.compact_at = c.compact_at;
        }
        if matches!(
            state.status,
            SessionStatus::Idle | SessionStatus::Stopped | SessionStatus::Failed
        ) {
            self.mem.borrow_mut().live.clear();
        }
        s.updated_at = now;
        self.save_summary(&s);
        self.broadcast_summary(&s);
        if status_changed || described {
            self.write_through().await;
        } else {
            self.mark_dirty(now).await;
        }
        if described {
            self.mem.borrow_mut().search_due = true;
            self.search_soon(now).await;
        }
        self.notify_fleet(&s);
        self.notify_followers(&s, before, now);
        // Failed checks and conflicts waited for the turn to end: they
        // reach the agent once what it pushed as the turn ended had time to
        // say so.
        if before == SessionStatus::Running
            && s.status != SessionStatus::Running
            && self.follow_ups_waiting(None)
        {
            let due = now + follow_ups::GATHER_MS;
            for kind in follow_ups::Repeats::ALL {
                let _ = self.sql.exec(
                    "UPDATE follow_ups SET due = MAX(due, ?) WHERE key LIKE ? || '%'",
                    Some(vec![iv(due), sv(kind.key())]),
                );
            }
            self.arm(due).await;
        }
        // The agent stopped on its own (its idle timeout) as a message came,
        // so it didn't take the message: the session resumes for it.
        if !before.resumes_for_messages()
            && s.status.resumes_for_messages()
            && self.messages_waiting()
            && let Err(e) = self.wake(now).await
        {
            worker::console_error!("resuming for a waiting message failed: {}", e.message);
        }
    }

    /// Keeps the plan usage the agent reported with the session's Claude
    /// credential, for its owner (`crate::usage`).
    fn record_usage(&self, readings: Vec<PlanLimit>) {
        if readings.is_empty() {
            return;
        }
        let Some(s) = self.summary() else { return };
        let env = self.env.clone();
        self.state.wait_until(async move {
            if let Err(e) = crate::usage::record(&env, s.id.as_str(), &readings).await {
                worker::console_error!("recording plan usage failed: {}", e.0.message);
            }
        });
    }

    /// The end of a turn: what the agent said last, for the notification that
    /// the session waits for a reply. Returns whether `v` ended a turn.
    fn remember_reply(&self, v: &serde_json::Value, agent_seq: u64) -> bool {
        if ev::classify(v) != ClaudeKind::Result {
            return false;
        }
        let text = v.get("result").and_then(serde_json::Value::as_str);
        match text.and_then(|text| notify::excerpt(text, notify::EXCERPT_CHARS)) {
            Some(reply) => self.set_meta("last_reply", &reply),
            None => self.delete_meta("last_reply"),
        }
        // All of it, for the ticket's thread its session answers in.
        match text.filter(|t| !t.trim().is_empty()) {
            Some(t) => {
                let kept: String = t.chars().take(LAST_RESULT_CHARS).collect();
                self.set_meta("last_result", &kept);
                self.set_meta("last_result_seq", &agent_seq.to_string());
            }
            None => self.delete_meta("last_result"),
        }
        true
    }

    /// Whether the session started from a ticket in Linear, whose agent
    /// session it answers in (`crate::delegations`).
    fn answers_linear(s: &Session) -> bool {
        s.ticket
            .as_ref()
            .is_some_and(|t| t.tracker == fctl_proto::session::Tracker::Linear)
    }

    /// Has the alarm see whether the session's ticket in Linear is done
    /// ([`Self::see_if_done`]) once its pull requests merged, as what fctl
    /// knows of them went from `before` (`merged_more`: one more merged;
    /// `fctl_domain::tickets::done_due`).
    async fn want_done(&self, s: &Session, before: Option<Progress>, merged_more: bool) {
        if !Self::answers_linear(s)
            || !fctl_domain::tickets::done_due(before, progress(s), merged_more)
        {
            return;
        }
        let now = now_ms();
        self.set_meta("linear_done_due", &now.to_string());
        self.set_meta("linear_done_tries", "0");
        self.arm(now).await;
    }

    /// Moves the session's ticket to Done once its time came
    /// ([`Self::want_done`]), while its pull requests are still merged
    /// (`crate::delegations::move_on`). When GitHub or Linear couldn't say,
    /// it sees again later, a few times
    /// (`fctl_domain::tickets::DONE_AGAIN_IN_MS`): nothing else would, since
    /// the session is usually done by then.
    async fn see_if_done(&self, now: i64) {
        let Some(due) = self
            .meta("linear_done_due")
            .and_then(|d| d.parse::<i64>().ok())
        else {
            return;
        };
        if due > now {
            self.arm(due).await;
            return;
        }
        let tries: usize = self
            .meta("linear_done_tries")
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);
        let done = |this: &Self| {
            this.delete_meta("linear_done_due");
            this.delete_meta("linear_done_tries");
        };
        let Some(s) = self
            .summary()
            .filter(|s| Self::answers_linear(s) && progress(s) == Some(Progress::Merged))
        else {
            done(self);
            return;
        };
        let id = s.id.0.clone();
        let settled = crate::delegations::move_on(
            &self.env,
            &id,
            Progress::Merged,
            &merged(&s.pull_requests),
        )
        .await
        .inspect_err(|e| worker::console_error!("moving the ticket of {id} on: {}", e.0.message))
        .unwrap_or(false);
        // Asked again meanwhile: that one goes on.
        if self.meta("linear_done_due").as_deref() != Some(due.to_string().as_str()) {
            return;
        }
        match fctl_domain::tickets::DONE_AGAIN_IN_MS.get(tries) {
            Some(after) if !settled => {
                let next = now_ms() + after;
                self.set_meta("linear_done_due", &next.to_string());
                self.set_meta("linear_done_tries", &(tries + 1).to_string());
                self.arm(next).await;
            }
            _ => done(self),
        }
    }

    /// Moves the session's ticket in Linear on as far as what its agent
    /// reported takes it from `before` (`fctl_domain::tickets::reported`):
    /// the agent's last report of a turn can come after the turn ended,
    /// which moved the ticket as far as the session had come then
    /// ([`Self::report_to_linear`]). A batch that ends the turn leaves it to
    /// the turn's end, which sees all of it; merges move it on their own
    /// ([`Self::want_done`]).
    fn move_on_reported(&self, s: &Session, before: Option<Progress>, turn_ended: bool) {
        if turn_ended || !Self::answers_linear(s) {
            return;
        }
        let turn_over = s.status != SessionStatus::Running;
        let Some(to) = fctl_domain::tickets::reported(before, progress(s), turn_over) else {
            return;
        };
        let env = self.env.clone();
        let id = s.id.0.clone();
        self.state.wait_until(async move {
            if let Err(e) = crate::delegations::move_on(&env, &id, to, &[]).await {
                worker::console_error!("moving the ticket of {id} on: {}", e.0.message);
            }
        });
    }

    /// What the agent does, for Linear's session: a step now and then (one
    /// of its tool calls), and its plan (Claude Code's todo list).
    fn mirror_to_linear(&self, s: &Session, v: &serde_json::Value, now: i64) {
        if !Self::answers_linear(s) || ev::classify(v) != ClaudeKind::Assistant {
            return;
        }
        let blocks = v
            .pointer("/message/content")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for b in blocks
            .iter()
            .filter(|b| b.get("type").and_then(serde_json::Value::as_str) == Some("tool_use"))
        {
            let name = b
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let input = b.get("input").cloned().unwrap_or_default();
            if name == "TodoWrite" {
                let Some(plan) = input.get("todos").and_then(fctl_domain::linear::plan) else {
                    continue;
                };
                let text = plan.to_string();
                if self.meta("linear_plan").as_deref() == Some(text.as_str()) {
                    continue;
                }
                self.set_meta("linear_plan", &text);
                let env = self.env.clone();
                let id = s.id.0.clone();
                self.state.wait_until(async move {
                    if let Err(e) = crate::delegations::plan(&env, &id, plan).await {
                        worker::console_error!("the plan of {id} for Linear: {}", e.0.message);
                    }
                });
                continue;
            }
            let Some((action, parameter)) = fctl_domain::linear::step(name, &input) else {
                continue;
            };
            let last = self
                .meta("linear_step_at")
                .and_then(|t| t.parse::<i64>().ok());
            if last.is_some_and(|at| now - at < fctl_domain::linear::STEP_EVERY_MS) {
                continue;
            }
            self.set_meta("linear_step_at", &now.to_string());
            let env = self.env.clone();
            let id = s.id.0.clone();
            self.state.wait_until(async move {
                if let Err(e) = crate::delegations::step(&env, &id, &action, &parameter).await {
                    worker::console_error!("a step of {id} for Linear: {}", e.0.message);
                }
            });
        }
    }

    /// Tells the session's agent session in Linear that a turn ended (with
    /// the agent's last message), or that the session paused or failed.
    fn report_to_linear(&self, s: &Session, kind: notify::Kind) {
        if !Self::answers_linear(s) {
            return;
        }
        let env = self.env.clone();
        let id = s.id.0.clone();
        match kind {
            notify::Kind::Waiting => {
                let (Some(text), Some(seq)) =
                    (self.meta("last_result"), self.meta("last_result_seq"))
                else {
                    return;
                };
                // Once for each turn, whichever report of its end comes first.
                if self.meta("linear_replied_seq").as_deref() == Some(seq.as_str()) {
                    return;
                }
                self.set_meta("linear_replied_seq", &seq);
                // How far it came, which moves its ticket on; merges move it
                // to Done by themselves (`want_done`).
                let progress = progress(s).filter(|p| *p != Progress::Merged);
                self.state.wait_until(async move {
                    if let Err(e) = crate::delegations::turn_ended(&env, &id, &text).await {
                        worker::console_error!("replying in Linear for {id}: {}", e.0.message);
                    }
                    if let Some(p) = progress
                        && let Err(e) = crate::delegations::move_on(&env, &id, p, &[]).await
                    {
                        worker::console_error!("moving the ticket of {id} on: {}", e.0.message);
                    }
                });
            }
            notify::Kind::Paused | notify::Kind::Failed => {
                let status = s.status;
                let reason = s.status_reason.clone();
                let until = s.resume_at;
                self.state.wait_until(async move {
                    if let Err(e) =
                        crate::delegations::halted(&env, &id, status, reason.as_deref(), until)
                            .await
                    {
                        worker::console_error!("telling Linear about {id}: {}", e.0.message);
                    }
                });
            }
        }
    }

    /// A notice the control plane adds to the transcript: what it did in
    /// the session's ticket tracker for it.
    fn control_notice(&self, n: fctl_proto::event::Notice) -> serde_json::Value {
        if let Some(json) = self.append(now_ms(), &EventBody::Notice(n), None) {
            self.broadcast_events(&[json]);
        }
        serde_json::json!({ "ok": true })
    }

    /// Tells the people who follow the session when a status change means it
    /// finished a turn, paused or failed.
    fn notify_followers(&self, s: &Session, before: SessionStatus, now: i64) {
        // A new turn: whoever interrupted the last one is not part of it.
        if s.status == SessionStatus::Running && before != SessionStatus::Running {
            self.delete_meta("interrupted_by");
        }
        if s.review.is_some() {
            self.review_moved(s, before);
        }
        // A review agent waits for nobody: its turn's end is its review's.
        let kind = notify::Kind::of(before, s.status)
            .filter(|k| s.review.is_none() || *k != notify::Kind::Waiting);
        if let Some(kind) = kind {
            self.notify(s, kind, now);
        }
    }

    /// A review session's runner took it, or it ended: its review's check
    /// on GitHub says so (`crate::reviews`).
    fn review_moved(&self, s: &Session, before: SessionStatus) {
        let env = self.env.clone();
        let id = s.id.0.clone();
        if s.status.is_terminal() && !before.is_terminal() {
            let failed = s.status == SessionStatus::Failed;
            let reason = s.status_reason.clone();
            self.state.wait_until(async move {
                if let Err(e) = crate::reviews::ended(&env, &id, failed, reason.as_deref()).await {
                    worker::console_error!("ending review {id}: {}", e.0.message);
                }
            });
        } else if matches!(s.status, SessionStatus::Preparing | SessionStatus::Running)
            && matches!(before, SessionStatus::Queued | SessionStatus::Provisioning)
            && self.meta("review_started").is_none()
        {
            self.set_meta("review_started", "1");
            self.state.wait_until(async move {
                if let Err(e) = crate::reviews::started(&env, &id).await {
                    worker::console_error!("starting review {id}'s check: {}", e.0.message);
                }
            });
        }
    }

    /// Hands `kind` to the people who follow the session (`crate::notify`),
    /// except those who can see it now.
    fn notify(&self, s: &Session, kind: notify::Kind, now: i64) {
        self.report_to_linear(s, kind);
        let mut except = self.watching();
        let detail = match kind {
            notify::Kind::Waiting => {
                // Whoever interrupted the turn knows that it ended.
                except.extend(self.meta("interrupted_by"));
                self.delete_meta("interrupted_by");
                self.meta("last_reply")
            }
            notify::Kind::Paused => None,
            notify::Kind::Failed => s.status_reason.clone(),
        };
        let event = notify::Event {
            session_id: s.id.clone(),
            reference: format!("{}#{}", s.project_slug, s.number),
            title: s.title.clone(),
            kind,
            detail,
            at: now,
        };
        let env = self.env.clone();
        self.state.wait_until(async move {
            if let Err(e) = crate::notify::session_changed(&env, event, except).await {
                worker::console_error!("notifying followers failed: {}", e.0.message);
            }
        });
    }

    /// The agent named or described the session (`fctl edit` in its
    /// sandbox). Its state repeats both with every report, so only a value
    /// that differs from its previous report counts: a person's rename
    /// stands until the agent renames the session again. Values come from
    /// the sandbox, so they are validated again here.
    fn apply_agent_description(&self, s: &mut Session, state: &SessionState) -> bool {
        let mut changed = false;
        if let Some(t) = state.title.as_deref()
            && self.meta("agent_title").as_deref() != Some(t)
        {
            self.set_meta("agent_title", t);
            match fctl_domain::validate::session_title(t) {
                Ok(t) => {
                    changed |= s.title != t;
                    s.title = t;
                }
                Err(e) => worker::console_error!("ignoring the agent's title: {}", e.message),
            }
        }
        if let Some(d) = state.description.as_deref()
            && self.meta("agent_description").as_deref() != Some(d)
        {
            self.set_meta("agent_description", d);
            match fctl_domain::validate::session_description(d) {
                Ok(d) => {
                    changed |= s.description != d;
                    s.description = d;
                }
                Err(e) => {
                    worker::console_error!("ignoring the agent's description: {}", e.message)
                }
            }
        }
        changed
    }

    // ───────────────────────────── kept images ─────────────────────────────

    /// Keeps the images in the results of the agent's tools (`v`, a Claude
    /// message) for the session's viewers until they expire, and makes room
    /// for them by deleting its oldest (`fctl_domain::kept_images`). Returns
    /// whether it kept any.
    fn keep_images(&self, v: &mut serde_json::Value, now: i64) -> bool {
        let kept = kept_images::keep_images(v, |bytes, info| {
            let image = KeptImage {
                id: fctl_domain::ids::kept_image_id(now),
                media_type: info.media_type.to_owned(),
                size: bytes.len() as u64,
                width: info.width,
                height: info.height,
                expires_at: kept_images::expires_at(now),
            };
            match self.store_image(&image, bytes, now) {
                Ok(()) => Some(image),
                Err(e) => {
                    worker::console_error!("keeping an image: {e}");
                    self.delete_image(image.id.as_str());
                    None
                }
            }
        });
        if kept > 0 {
            self.trim_images();
        }
        kept > 0
    }

    fn store_image(&self, image: &KeptImage, bytes: &[u8], now: i64) -> Result<()> {
        self.sql.exec(
            "INSERT INTO kept_images (id, media_type, size, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
            Some(vec![
                sv(image.id.as_str()),
                sv(&image.media_type),
                iv(image.size as i64),
                iv(now),
                iv(image.expires_at),
            ]),
        )?;
        for (n, chunk) in bytes.chunks(files::CHUNK_BYTES).enumerate() {
            self.sql.exec(
                "INSERT INTO kept_image_chunks (image_id, n, data) VALUES (?, ?, ?)",
                Some(vec![
                    sv(image.id.as_str()),
                    iv(n as i64),
                    SqlStorageValue::Blob(chunk.to_vec()),
                ]),
            )?;
        }
        Ok(())
    }

    fn delete_image(&self, id: &str) {
        for stmt in [
            "DELETE FROM kept_image_chunks WHERE image_id = ?",
            "DELETE FROM kept_images WHERE id = ?",
        ] {
            if let Err(e) = self.sql.exec(stmt, Some(vec![sv(id)])) {
                worker::console_error!("deleting image {id}: {e}");
            }
        }
    }

    /// Deletes the oldest images while the session keeps more of them, or
    /// more bytes, than `fctl_domain::kept_images` allows.
    fn trim_images(&self) {
        #[derive(Deserialize)]
        struct Kept {
            n: f64,
            total: Option<f64>,
        }
        #[derive(Deserialize)]
        struct Id {
            id: String,
        }
        let (most, most_bytes) = (
            kept_images::MAX_SESSION_IMAGES as i64,
            kept_images::MAX_SESSION_BYTES as i64,
        );
        let kept = self
            .sql
            .exec(
                "SELECT COUNT(*) AS n, SUM(size) AS total FROM kept_images",
                None,
            )
            .and_then(|c| c.to_array::<Kept>())
            .ok()
            .and_then(|r| r.into_iter().next());
        let Some(kept) = kept else { return };
        if kept.n as i64 <= most && (kept.total.unwrap_or(0.0) as i64) <= most_bytes {
            return;
        }
        // Newest first: those past the newest that fit.
        let over: Vec<Id> = self
            .sql
            .exec(
                "SELECT id FROM (SELECT id, ROW_NUMBER() OVER newest AS k, SUM(size) OVER newest AS total FROM kept_images \
                 WINDOW newest AS (ORDER BY created_at DESC, id DESC)) WHERE k > ? OR total > ?",
                Some(vec![iv(most), iv(most_bytes)]),
            )
            .and_then(|c| c.to_array())
            .unwrap_or_else(|e| {
                worker::console_error!("finding the oldest images: {e}");
                Vec::new()
            });
        for o in over {
            self.delete_image(&o.id);
        }
    }

    /// An image the agent read (`?id=`), until it expires. The Worker
    /// checks that the caller can see the session.
    fn kept_image(&self, req: &Request) -> Result<Response> {
        #[derive(Deserialize)]
        struct Row {
            media_type: String,
            size: f64,
            expires_at: f64,
        }
        let id = crate::http::query(req, "id").unwrap_or_default();
        let now = now_ms();
        let row = self
            .sql
            .exec(
                "SELECT media_type, size, expires_at FROM kept_images WHERE id = ? AND expires_at > ?",
                Some(vec![sv(&id), iv(now)]),
            )
            .and_then(|c| c.to_array::<Row>())
            .ok()
            .and_then(|r| r.into_iter().next());
        let Some(row) = row else {
            return internal::do_error(&ApiError::not_found("Image"));
        };
        let mut bytes = Vec::with_capacity(row.size as usize);
        let cursor = self.sql.exec(
            "SELECT data FROM kept_image_chunks WHERE image_id = ? ORDER BY n",
            Some(vec![sv(&id)]),
        )?;
        for r in cursor.raw() {
            if let Some(SqlStorageValue::Blob(chunk)) = r?.into_iter().next() {
                bytes.extend_from_slice(&chunk);
            }
        }
        if bytes.len() as u64 != row.size as u64 {
            worker::console_error!("image {id} has {} of {} bytes", bytes.len(), row.size);
            return internal::do_error(&api_err(ErrorCode::Internal, "The image is incomplete"));
        }
        let h = Headers::new();
        h.set("Content-Type", files::served_type(&row.media_type))?;
        h.set("Content-Length", &bytes.len().to_string())?;
        h.set("Content-Disposition", "inline")?;
        // It never changes, and only people who can see the session get
        // it, until it expires.
        let left = ((row.expires_at as i64 - now) / 1000).max(0);
        h.set(
            "Cache-Control",
            &format!("private, max-age={left}, immutable"),
        )?;
        h.set("X-Content-Type-Options", "nosniff")?;
        h.set(
            "Content-Security-Policy",
            "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; sandbox",
        )?;
        h.set("Cross-Origin-Resource-Policy", "same-origin")?;
        h.set("Referrer-Policy", "same-origin")?;
        Ok(Response::from_bytes(bytes)?.with_headers(h))
    }

    /// Deletes the images that expired, and has the alarm go off when the
    /// next one does.
    async fn expire_images(&self, now: i64) {
        for stmt in [
            "DELETE FROM kept_image_chunks WHERE image_id IN (SELECT id FROM kept_images WHERE expires_at <= ?)",
            "DELETE FROM kept_images WHERE expires_at <= ?",
        ] {
            if let Err(e) = self.sql.exec(stmt, Some(vec![iv(now)])) {
                worker::console_error!("expiring images: {e}");
            }
        }
        #[derive(Deserialize)]
        struct Next {
            at: Option<f64>,
        }
        let next = self
            .sql
            .exec("SELECT MIN(expires_at) AS at FROM kept_images", None)
            .and_then(|c| c.to_array::<Next>())
            .ok()
            .and_then(|r| r.into_iter().next())
            .and_then(|n| n.at);
        if let Some(at) = next {
            self.arm(at as i64).await;
        }
    }

    // ───────────────────────────── search ─────────────────────────────

    /// The workspace of the session's project, whose search index has it:
    /// as its summary says, or D1 for a summary from before workspaces.
    async fn workspace_id(&self, s: &Session) -> Option<String> {
        if let Some(w) = &s.workspace {
            return Some(w.id.0.clone());
        }
        if let Some(w) = self.meta("workspace_id") {
            return Some(w);
        }
        #[derive(Deserialize)]
        struct W {
            workspace_id: Option<String>,
        }
        let db = self.env.d1("DB").ok()?;
        let row: Option<W> = crate::db::q(
            &db,
            "SELECT workspace_id FROM projects WHERE id = ?",
            &[crate::db::s(s.project_id.as_str())],
        )
        .ok()?
        .first()
        .await
        .ok()?;
        let w = row?.workspace_id?;
        self.set_meta("workspace_id", &w);
        Some(w)
    }

    /// Has the alarm put what's new in the search index soon, once a
    /// message or a reply came, or the title or description changed.
    async fn search_soon(&self, now: i64) {
        let due = std::mem::take(&mut self.mem.borrow_mut().search_due);
        if due {
            self.arm(now + SEARCH_DELAY_MS).await;
        }
    }

    /// Puts what the workspace's search index doesn't have yet in it: the
    /// title and description when they changed, and the messages and
    /// replies of the events after the last it took, `SEARCH_BATCH` events
    /// at most. `Ok(true)` once it has them all. A review agent's session
    /// isn't searched: its project's reviews list it.
    async fn index_search(&self) -> std::result::Result<bool, ApiError> {
        let Some(s) = self.summary().filter(|s| s.review.is_none()) else {
            return Ok(true);
        };
        let Some(workspace) = self.workspace_id(&s).await else {
            return Err(api_err(
                ErrorCode::Internal,
                "the session's workspace is unknown",
            ));
        };
        let after: i64 = self
            .meta("search_seq")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let last = self.next_seq() - 1;
        let about = search::about(&s.title, s.description.as_deref(), s.created_at);
        let about_key = about
            .iter()
            .map(|d| d.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let about_changed = self.meta("search_about").as_deref() != Some(about_key.as_str());
        if !about_changed && after >= last {
            return Ok(true);
        }
        // Only events that may hold a message or a reply are read; tool
        // results are most of a transcript.
        let rows = self.load_events(
            "SELECT seq, ts, kind, data FROM events WHERE seq > ? AND seq <= ? \
             AND (kind = 'user_message' OR (kind = 'claude' AND json_extract(data, '$.type') = 'assistant')) \
             ORDER BY seq LIMIT ?",
            vec![iv(after), iv(last), iv(SEARCH_BATCH)],
        );
        let done = (rows.len() as i64) < SEARCH_BATCH;
        let through = match rows.last() {
            Some(r) if !done => r.seq as i64,
            _ => last,
        };
        let docs: Vec<search::Doc> = rows
            .iter()
            .filter_map(|r| {
                let data = serde_json::from_str::<serde_json::Value>(&r.data).ok()?;
                Some(search::docs_of(r.seq as i64, r.ts as i64, &r.kind, &data))
            })
            .flatten()
            .collect();
        if about_changed || !docs.is_empty() {
            let stub = internal::search_stub(&self.env, &workspace).map_err(|e| e.0)?;
            let _: serde_json::Value = internal::post(
                &stub,
                "/put",
                &PutDocs {
                    session_id: s.id.clone(),
                    project_id: s.project_id.clone(),
                    about: about_changed.then_some(about),
                    docs,
                },
            )
            .await
            .map_err(|e| e.0)?;
        }
        if about_changed {
            self.set_meta("search_about", &about_key);
        }
        // Another indexing may have gone further meanwhile.
        let now_at: i64 = self
            .meta("search_seq")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        self.set_meta("search_seq", &through.max(now_at).to_string());
        Ok(done)
    }

    /// Records in D1 whether the search index has all the session's events
    /// (`sessions.search_indexed`), when that changed: the hourly job
    /// indexes those that it doesn't.
    async fn search_state(&self, indexed: bool) {
        let value = if indexed { "1" } else { "0" };
        if self.meta("search_indexed").as_deref() == Some(value) {
            return;
        }
        let Some(s) = self.summary() else { return };
        let Ok(db) = self.env.d1("DB") else { return };
        let res = match crate::db::q(
            &db,
            "UPDATE sessions SET search_indexed = ? WHERE id = ?",
            &[crate::db::b(indexed), crate::db::s(s.id.as_str())],
        ) {
            Ok(q) => q.run().await,
            Err(e) => Err(e),
        };
        match res {
            Ok(_) => self.set_meta("search_indexed", value),
            Err(e) => worker::console_error!("recording the search index: {}", e.0.message),
        }
    }

    /// The alarm's part in search: what's new goes in the index, and what's
    /// left after that comes next; one that the index didn't take is tried
    /// again a few times, a little later each time.
    async fn index_now(&self, now: i64) {
        match self.index_search().await {
            Ok(true) => {
                self.delete_meta("search_failures");
                self.search_state(true).await;
            }
            Ok(false) => self.arm(now + 1_000).await,
            Err(e) => {
                let failures = self
                    .meta("search_failures")
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(0)
                    + 1;
                worker::console_error!("indexing the session for search: {}", e.message);
                self.set_meta("search_failures", &failures.to_string());
                self.search_state(false).await;
                if failures <= SEARCH_RETRIES {
                    self.arm(now + 30_000 * (1 << failures.min(SEARCH_RETRIES)))
                        .await;
                }
            }
        }
    }

    /// The hourly job's request (`/index`), for a session whose index may
    /// lack something (one from before search, one whose index fell behind,
    /// or each when its index is to be built again): puts all of it in the
    /// index again, from its first event, a few batches at once, and has the
    /// alarm do the rest of a long transcript, which says so in D1 when done.
    async fn backfill_search(&self) -> std::result::Result<Indexed, ApiError> {
        self.delete_meta("search_seq");
        self.delete_meta("search_about");
        self.delete_meta("search_indexed");
        for _ in 0..SEARCH_BACKFILL_ROUNDS {
            if self.index_search().await? {
                self.delete_meta("search_failures");
                self.search_state(true).await;
                return Ok(Indexed { done: true });
            }
        }
        self.arm(now_ms() + 1_000).await;
        Ok(Indexed { done: false })
    }

    // ───────────────────────────── write-through ─────────────────────────────

    fn touch_activity(&self, now: i64) {
        if let Some(mut s) = self.summary() {
            s.last_activity_at = Some(now);
            self.save_summary(&s);
            self.mem.borrow_mut().d1_dirty = true;
        }
    }

    /// Schedules a D1 write within 5 s (throttled).
    async fn mark_dirty(&self, now: i64) {
        let due = {
            let mut mem = self.mem.borrow_mut();
            mem.d1_dirty = true;
            now - mem.last_d1_write > 5_000
        };
        if due {
            self.write_through().await;
        } else {
            self.arm(now + 5_000).await;
        }
    }

    /// Has the object's alarm go off by `at`: it flushes summary changes
    /// and passes follow-ups on, whichever is due.
    async fn arm(&self, at: i64) {
        let storage = self.state.storage();
        match storage.get_alarm().await {
            Ok(Some(set)) if set <= at => {}
            _ => {
                let _ = storage.set_alarm((at - now_ms()).max(0)).await;
            }
        }
    }

    async fn write_through(&self) {
        let Some(s) = self.summary() else { return };
        {
            let mut mem = self.mem.borrow_mut();
            mem.d1_dirty = false;
            mem.last_d1_write = now_ms();
        }
        let Ok(db) = self.env.d1("DB") else { return };
        let stats = serde_json::to_string(&s.stats).unwrap_or_else(|_| "{}".into());
        let rc = s
            .remote_control
            .as_ref()
            .and_then(|r| serde_json::to_string(r).ok());
        let pr = s.pull_request.clone();
        let prs = (!s.pull_requests.is_empty())
            .then(|| serde_json::to_string(&s.pull_requests).ok())
            .flatten();
        let res = crate::db::q(
            &db,
            "UPDATE sessions SET title = ?, description = ?, status = ?, status_reason = ?, resume_at = ?, model = COALESCE(?, model), \
             claude_effort = ?, branch = COALESCE(?, branch), remote_control = ?, stats = ?, pr_number = ?, pr_url = ?, pr_title = ?, pr_state = ?, \
             pull_requests = ?, updated_at = ?, last_activity_at = ?, started_at = COALESCE(started_at, ?), ended_at = ? WHERE id = ?",
            &[
                crate::db::s(&s.title),
                crate::db::os(s.description.as_deref()),
                crate::db::s(s.status.as_str()),
                crate::db::os(s.status_reason.as_deref()),
                crate::db::on(s.resume_at),
                crate::db::os(s.model.as_deref()),
                crate::db::os(s.effort.as_deref()),
                crate::db::os(s.branch.as_deref()),
                crate::db::os(rc.as_deref()),
                crate::db::s(&stats),
                crate::db::on(pr.as_ref().map(|p| p.number as i64)),
                crate::db::os(pr.as_ref().map(|p| p.url.as_str())),
                crate::db::os(pr.as_ref().and_then(|p| p.title.as_deref())),
                crate::db::os(pr.as_ref().and_then(|p| p.state.as_deref())),
                crate::db::os(prs.as_deref()),
                crate::db::n(s.updated_at),
                crate::db::on(s.last_activity_at),
                crate::db::on(s.started_at),
                crate::db::on(s.ended_at),
                crate::db::s(s.id.as_str()),
            ],
        );
        match res {
            Ok(query) => {
                if let Err(e) = query.run().await {
                    worker::console_error!("session write-through failed: {}", e.0.message);
                }
            }
            Err(e) => worker::console_error!("session write-through failed: {}", e.0.message),
        }
    }

    fn notify_fleet(&self, s: &Session) {
        let env = self.env.clone();
        let session = s.clone();
        self.state.wait_until(async move {
            if let Ok(stub) = internal::fleet_stub(&env) {
                let _: std::result::Result<serde_json::Value, _> = internal::post(
                    &stub,
                    "/notify-session",
                    &internal::NotifySession { session },
                )
                .await;
            }
        });
    }

    fn pulse(&self, body: &EventBody, now: i64) {
        let (kind, weight) = match body {
            EventBody::Claude(v) => match ev::activity(v) {
                Some(kw) => kw,
                None => return,
            },
            EventBody::UserMessage(_) => ("user_message", 50),
            _ => return,
        };
        self.pulse_kind(kind, weight, now);
    }

    /// Activity pulses for the dashboard, at most four per second.
    fn pulse_kind(&self, kind: &str, weight: u32, now: i64) {
        let send = {
            let mut mem = self.mem.borrow_mut();
            mem.pulse_weight = mem.pulse_weight.saturating_add(weight);
            if now - mem.last_pulse < 250 {
                None
            } else {
                mem.last_pulse = now;
                let w = mem.pulse_weight;
                mem.pulse_weight = 0;
                Some(w)
            }
        };
        let Some(weight) = send else { return };
        let Some(s) = self.summary() else { return };
        let env = self.env.clone();
        let activity = internal::Activity {
            session_id: s.id,
            project_id: s.project_id,
            workspace_id: s.workspace.map(|w| w.id.0),
            kind: kind.to_owned(),
            weight,
        };
        self.state.wait_until(async move {
            if let Ok(stub) = internal::fleet_stub(&env) {
                let _: std::result::Result<serde_json::Value, _> =
                    internal::post(&stub, "/activity", &activity).await;
            }
        });
    }
}
