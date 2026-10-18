//! SessionDO ⇄ viewers (browsers and the CLI), over
//! `GET /api/v1/sessions/{id}/ws`.
//!
//! The first server frame is always [`ViewerServer::Snapshot`]. Clients that
//! reconnect pass `?after=<seq>` and get only newer events in the snapshot.

use crate::error::ApiError;
use crate::event::SessionEvent;
use crate::session::{Session, SessionRole, UserRef};
use crate::{AttachmentId, Millis};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// Someone watching a session.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Presence {
    pub user: UserRef,
    /// Open connections (tabs, terminals).
    pub connections: u32,
    pub since: Millis,
    /// Last time this person typed in the composer.
    pub typing_at: Option<Millis>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Snapshot {
    pub session: Session,
    pub role: SessionRole,
    /// The latest events (or those after `?after=`), oldest first.
    pub events: Vec<SessionEvent>,
    /// Whether older events exist; page them with `GET …/events?before=`.
    pub has_more_before: bool,
    /// The `seq` the next event will get.
    pub next_seq: u64,
    /// `stream_event` messages of the turn in progress, so a viewer that joins
    /// mid-turn sees the text being written.
    #[cfg_attr(
        feature = "ts",
        ts(type = "Array<import('../lib/claude-types').ClaudeStreamEvent>")
    )]
    pub live: Vec<serde_json::Value>,
    pub viewers: Vec<Presence>,
    /// Whether a runner is connected for this session right now.
    pub runner_connected: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewerServer {
    Snapshot(Box<Snapshot>),
    /// New stored events, in order.
    Events {
        events: Vec<SessionEvent>,
    },
    /// An ephemeral Claude `stream_event` (token deltas). Not stored.
    Stream {
        #[cfg_attr(
            feature = "ts",
            ts(type = "import('../lib/claude-types').ClaudeStreamEvent")
        )]
        event: serde_json::Value,
    },
    /// The session summary changed (status, title, stats, pull request).
    Session {
        session: Box<Session>,
    },
    Presence {
        viewers: Vec<Presence>,
    },
    RunnerConnected {
        connected: bool,
    },
    /// The outcome of a client command, matched by `client_id`.
    Result {
        client_id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        error: Option<ApiError>,
    },
    Error {
        error: ApiError,
    },
    Pong,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewerClient {
    /// Message the agent. Queued if it is busy.
    Send {
        client_id: String,
        /// May be empty when `attachments` are given.
        text: String,
        /// Uploaded files to send with it (see `SendMessageRequest`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(feature = "ts", ts(as = "Option<Vec<AttachmentId>>", optional))]
        attachments: Vec<AttachmentId>,
    },
    /// Stop the agent's current turn.
    Interrupt {
        client_id: String,
    },
    /// The person is typing (sent at most every few seconds).
    Typing,
    /// Whether the person can see the session now (its browser tab is
    /// visible). Viewers are visible until they say otherwise; people who
    /// can see a session get no notifications about it.
    Visibility {
        visible: bool,
    },
    Ping,
}
