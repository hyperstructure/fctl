//! SessionDO ⇄ host, one WebSocket per active session, over
//! `GET /api/v1/runner/sessions/{id}/uplink` with the runner token.
//!
//! Reliability: agent events carry `agent_seq`, assigned and journaled by
//! fctl-agent. SessionDO stores each event once (ignoring `agent_seq` it has
//! already seen) and acknowledges. On (re)connect SessionDO sends `Welcome`
//! with the last acknowledged `agent_seq` and every unconfirmed delivery; the
//! host replays the journal from there and re-sends deliveries to the agent,
//! which ignores ids it has already handled.

use crate::event::{EventBody, LogLine};
use crate::session::{RemoteControlState, SessionStats, SessionStatus, UserRef};
use crate::{DeliveryId, Millis, RunnerId};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UplinkUp {
    Hello {
        runner_id: RunnerId,
        protocol: u32,
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
