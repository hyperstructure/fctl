//! Host ⇄ fctl-agent (the supervisor inside a sandbox).
//!
//! Framing: one JSON value per line (`\n`), UTF-8, compact. fctl-agent runs
//! detached in the sandbox and listens on a unix socket; the host reaches it
//! through `fctl-agent attach` over the driver's exec channel (or the socket
//! directly for the `process` driver). Several attaches over the lifetime of
//! a session are normal: after each one the host sends `Replay` with the last
//! `agent_seq` the control plane acknowledged.

use crate::DeliveryId;
use crate::event::LogLine;
use crate::runner::{
    ClaudeCredential, CredentialKind, GithubCredential, SandboxSize, SessionSpec, StopReason,
};
use crate::session::SessionStatus;
use crate::uplink::{AgentEvent, Delivery, SessionState};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::BTreeMap;

/// Longest frame either side accepts.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Everything fctl-agent needs to run a session.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AgentConfig {
    pub spec: SessionSpec,
    /// Where the repository is cloned (the agent's working directory).
    pub workdir: String,
    /// HOME for Claude (its config and transcripts live here).
    pub home: Option<String>,
    /// The `claude` executable (default: `claude` on PATH).
    pub claude_bin: Option<String>,
    /// Extra arguments for `claude` (tests use them to pick a scenario).
    pub claude_args: Vec<String>,
    /// Extra environment for Claude (host configuration, not secrets).
    pub env: BTreeMap<String, String>,
    /// Push the branch after each turn that produced commits.
    pub push_on_commit: bool,
    /// Number events after this one: the control plane already has the
    /// session's earlier events, from a sandbox that no longer exists.
    #[serde(default)]
    pub first_agent_seq: Option<u64>,
    /// What the sandbox is limited to, when its driver limits it (not the
    /// process driver).
    #[serde(default)]
    pub sandbox: Option<SandboxSize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostToAgent {
    /// Start the session. Ignored (with a log line) when already configured.
    Configure {
        config: Box<AgentConfig>,
    },
    /// Send every journaled event with `agent_seq` greater than this.
    Replay {
        after_agent_seq: u64,
    },
    /// The control plane stored events up to this; the journal may drop them.
    Ack {
        agent_seq: u64,
    },
    Deliver {
        delivery: Delivery,
    },
    /// Fresh credentials, pushed or in answer to `CredentialRequest`.
    Credentials {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        claude: Option<ClaudeCredential>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        github: Option<GithubCredential>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        error: Option<String>,
    },
    /// Finish: interrupt Claude, wait up to `grace_ms`, then exit.
    Stop {
        reason: StopReason,
        grace_ms: u64,
    },
    Ping,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AgentHello {
    pub protocol: u32,
    pub version: String,
    pub configured: bool,
    pub status: SessionStatus,
    /// The highest `agent_seq` journaled so far.
    pub agent_seq: u64,
    pub claude_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentToHost {
    Hello(AgentHello),
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
    /// A log line that should not be journaled (progress noise).
    Log {
        line: LogLine,
    },
    Delivered {
        delivery_id: DeliveryId,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        error: Option<String>,
    },
    CredentialRequest {
        request_id: String,
        kind: CredentialKind,
    },
    /// The supervisor is exiting.
    Exited {
        status: SessionStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        reason: Option<String>,
    },
    Pong,
}
