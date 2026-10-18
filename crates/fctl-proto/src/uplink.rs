//! SessionDO ⇄ host, one WebSocket per active session, over
//! `GET /api/v1/runner/sessions/{id}/uplink` with the runner token.
//!
//! Reliability: agent events carry `agent_seq`, assigned and journaled by
//! fctl-agent. SessionDO stores each event once (ignoring `agent_seq` it has
//! already seen) and acknowledges. On (re)connect SessionDO sends `Welcome`
//! with the last acknowledged `agent_seq` and every unconfirmed delivery; the
//! host replays the journal from there and re-sends deliveries to the agent,
//! which ignores ids it has already handled.

use crate::event::{Attachment, EventBody, LogLine};
use crate::session::{RemoteControlState, SessionStats, SessionStatus, UserRef};
use crate::{DeliveryId, Millis, RunnerId};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// A host or fctl-agent that passes attachments to Claude (`UplinkUp::Hello`,
/// `AgentHello`).
pub const FEATURE_ATTACHMENTS: &str = "attachments";

/// An event produced in the sandbox.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AgentEvent {
    pub agent_seq: u64,
    pub ts: Millis,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub body: EventBody,
}

/// Something for the agent: a message, an interrupt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Delivery {
    pub id: DeliveryId,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub kind: DeliveryKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeliveryKind {
    /// A user turn. `author` is absent for system messages.
    Message {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        author: Option<UserRef>,
        /// Files sent with the message. The host fetches their bytes
        /// (`GET /api/v1/runner/sessions/{id}/attachments/{attachment}`) and
        /// hands them to fctl-agent before the message.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(feature = "ts", ts(as = "Option<Vec<Attachment>>", optional))]
        attachments: Vec<Attachment>,
    },
    /// Abort the current turn.
    Interrupt {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        by: Option<UserRef>,
    },
}

/// Session facts the host reports as state (idempotent, latest wins).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SessionState {
    pub status: SessionStatus,
    pub reason: Option<String>,
    pub resume_at: Option<Millis>,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub claude_session_id: Option<String>,
    pub remote_control: Option<RemoteControlState>,
    pub stats: Option<SessionStats>,
    /// The title the agent gave the session (`fctl edit` in the sandbox).
    /// SessionDO applies it when it differs from the title the agent
    /// reported before, so a person's rename stands until the agent renames
    /// the session again.
    pub title: Option<String>,
    /// What the agent says the session is about, applied the same way; an
    /// empty string clears it.
    pub description: Option<String>,
}

/// The host has no working link to fctl-agent in the session's sandbox: the
/// last one broke, and attaching again hasn't worked yet. The agent may
/// well be working; its events wait in its journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AgentUnreachable {
    /// When the host last heard from fctl-agent, or began attaching to it.
    pub since: Millis,
    /// What went wrong last, as the host saw it.
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UplinkUp {
    Hello {
        runner_id: RunnerId,
        protocol: u32,
        /// What this host can do beyond the protocol version, for example
        /// [`FEATURE_ATTACHMENTS`]. Older hosts send none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
        features: Vec<String>,
    },
    /// Events in `agent_seq` order. SessionDO ignores ones it already has.
    Events {
        events: Vec<AgentEvent>,
    },
    /// An ephemeral Claude `stream_event`.
    Stream {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        event: serde_json::Value,
    },
    State {
        state: SessionState,
    },
    /// A best-effort log line from the host itself (before the agent runs).
    Log {
        line: LogLine,
    },
    /// The agent handled a delivery.
    Delivered {
        delivery_id: DeliveryId,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        error: Option<String>,
    },
    /// Whether the host reaches fctl-agent: sent when that changes, and
    /// again after every reconnect. SessionDO forgets it when the uplink
    /// closes. Older hosts never send it.
    AgentLink {
        /// Absent once the host reaches the agent again.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        unreachable: Option<AgentUnreachable>,
    },
    Ping,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UplinkDown {
    Welcome {
        /// The last `agent_seq` SessionDO has stored (0 if none).
        acked_agent_seq: u64,
        /// Deliveries the agent has not confirmed yet, oldest first.
        pending: Vec<Delivery>,
    },
    Ack {
        agent_seq: u64,
    },
    Deliver {
        delivery: Delivery,
    },
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_report_whether_they_reach_the_agent() -> serde_json::Result<()> {
        let down = UplinkUp::AgentLink {
            unreachable: Some(AgentUnreachable {
                since: 1_000,
                reason: "nothing came from fctl-agent for a minute".into(),
            }),
        };
        let json = serde_json::to_value(&down)?;
        assert_eq!(json["type"], "agent_link");
        assert_eq!(json["unreachable"]["since"], 1_000);
        assert_eq!(serde_json::from_value::<UplinkUp>(json)?, down);

        // Reachable again: nothing but the tag.
        let up = UplinkUp::AgentLink { unreachable: None };
        assert_eq!(
            serde_json::to_string(&up)?,
            r#"{"type":"agent_link"}"#.to_owned()
        );
        assert_eq!(
            serde_json::from_str::<UplinkUp>(r#"{"type":"agent_link"}"#)?,
            up
        );
        Ok(())
    }
}
