//! SessionDO ⇄ viewers (browsers and the CLI), over
//! `GET /api/v1/sessions/{id}/ws`.
//!
//! The first server frame is always [`ViewerServer::Snapshot`]. Clients that
//! reconnect pass `?after=<seq>` and get only newer events in the snapshot.

use crate::error::ApiError;
use crate::event::SessionEvent;
use crate::session::{Session, SessionRole, UserRef};
use crate::uplink::AgentUnreachable;
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
    /// The live messages of the response in progress (`Stream`), so a viewer
    /// that joins mid-turn sees the text being written: of its thinking
    /// progress, only the newest, last.
    #[cfg_attr(
        feature = "ts",
        ts(type = "Array<import('../lib/claude-types').ClaudeStreamEvent>")
    )]
    pub live: Vec<serde_json::Value>,
    pub viewers: Vec<Presence>,
    /// Whether a runner is connected for this session right now: its uplink
    /// is open, and SessionDO heard from it within the last minute.
    pub runner_connected: bool,
    /// When SessionDO last heard from the session's runner, while it isn't
    /// connected. Absent when no runner has been connected for this run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub runner_last_seen_at: Option<Millis>,
    /// The connected runner can't reach the agent in the sandbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub agent_unreachable: Option<AgentUnreachable>,
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
    /// A Claude message that is shown live and not stored: a
    /// `stream_event` (token deltas), a tool heartbeat (`tool_progress`) or
    /// how far its thinking got (`system/thinking_tokens`).
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
    /// The session's runner connected, went away or went quiet, or what it
    /// says about reaching the agent changed. The fields are as in the
    /// snapshot.
    RunnerConnected {
        connected: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        last_seen_at: Option<Millis>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        agent_unreachable: Option<AgentUnreachable>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_messages_from_older_control_planes_still_read() -> serde_json::Result<()> {
        let old: ViewerServer =
            serde_json::from_str(r#"{"type":"runner_connected","connected":false}"#)?;
        assert_eq!(
            old,
            ViewerServer::RunnerConnected {
                connected: false,
                last_seen_at: None,
                agent_unreachable: None,
            }
        );
        // Nothing to say leaves the new fields out, for older clients.
        let json = serde_json::to_string(&old)?;
        assert_eq!(json, r#"{"type":"runner_connected","connected":false}"#);
        Ok(())
    }

    #[test]
    fn runner_messages_say_when_it_was_last_heard_from_and_whether_it_reaches_the_agent()
    -> serde_json::Result<()> {
        let msg: ViewerServer = serde_json::from_value(serde_json::json!({
            "type": "runner_connected",
            "connected": true,
            "agent_unreachable": { "since": 5, "reason": "attaching timed out" }
        }))?;
        let ViewerServer::RunnerConnected {
            connected,
            last_seen_at,
            agent_unreachable,
        } = msg
        else {
            panic!("expected runner_connected");
        };
        assert!(connected);
        assert_eq!(last_seen_at, None);
        assert_eq!(agent_unreachable.map(|a| a.since), Some(5));
        Ok(())
    }
}
