//! Host ⇄ fctl-agent (the supervisor inside a sandbox), and `fctl` in the
//! sandbox ⇄ fctl-agent.
//!
//! Framing: one JSON value per line (`\n`), UTF-8, compact. fctl-agent runs
//! detached in the sandbox and listens on a unix socket; the host reaches it
//! through `fctl-agent attach` over the driver's exec channel (or the socket
//! directly for the `process` driver). Several attaches over the lifetime of
//! a session are normal: after each one the host sends `Replay` with the last
//! `agent_seq` the control plane acknowledged.
//!
//! A second socket, the session socket ([`SESSION_SOCKET_ENV`]), serves
//! programs inside the sandbox: `fctl edit`, `fctl publish`, `fctl memory`
//! and `fctl retro` send one [`SessionRequest`] and read one
//! [`SessionReply`]. That is how an agent names and describes its own
//! session, publishes screenshots, keeps its project's memory and records
//! retrospectives, without credentials for the control plane.

use crate::api::{AgentAnswer, AgentCall};
use crate::event::{Attachment, LogLine, PublishedFile};
use crate::runner::{
    ClaudeCredential, CredentialKind, GithubCredential, SandboxSize, SessionSpec, StopReason,
};
use crate::session::SessionStatus;
use crate::uplink::{AgentEvent, Delivery, SessionState};
use crate::{ApiError, DeliveryId, SessionId};
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
    /// Where the primary repository is cloned (the agent's working
    /// directory). Other repositories go under `~/repos/<owner>/<name>`.
    pub workdir: String,
    /// HOME for Claude (its config and transcripts live here).
    pub home: Option<String>,
    /// The `claude` executable (default: `claude` on PATH).
    pub claude_bin: Option<String>,
    /// Extra arguments for `claude` (tests use them to pick a scenario).
    pub claude_args: Vec<String>,
    /// Extra environment for Claude (host configuration, not secrets).
    pub env: BTreeMap<String, String>,
    /// Push the session branch after each turn that produced commits.
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
    /// An attachment's bytes (base64), sent before the `Configure` or
    /// `Deliver` that refers to it. Only to agents that announce
    /// [`crate::uplink::FEATURE_ATTACHMENTS`].
    File {
        attachment: Attachment,
        data: String,
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
        /// See [`crate::runner::SessionCredentials::github_read`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        github_read: Option<GithubCredential>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        error: Option<String>,
    },
    /// Finish: interrupt Claude, wait up to `grace_ms`, then exit.
    Stop {
        reason: StopReason,
        grace_ms: u64,
    },
    /// The answer to [`AgentToHost::Publish`]: the published file, or why
    /// it wasn't published.
    Published {
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        file: Option<PublishedFile>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        error: Option<ApiError>,
    },
    /// The control plane's answer to [`AgentToHost::Call`], or why there is
    /// none.
    Answer {
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        answer: Option<AgentAnswer>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        error: Option<ApiError>,
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
    /// What this agent can do beyond the protocol version (older agents
    /// send none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub features: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentToHost {
    Hello(AgentHello),
    Events {
        events: Vec<AgentEvent>,
    },
    /// A Claude message that goes to viewers live and isn't stored: a
    /// `stream_event`, a tool heartbeat (`tool_progress`) or how far its
    /// thinking got (`system/thinking_tokens`).
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
    /// Publish a screenshot (`fctl publish`): the host uploads it to the
    /// control plane with its runner token
    /// (`POST /api/v1/runner/sessions/{id}/published`) and answers with
    /// [`HostToAgent::Published`]. Agents send it only when the session's
    /// project settings allow publishing, which reach them only through
    /// hosts that know the setting, and so this frame.
    Publish {
        request_id: String,
        /// The file's name, without directories.
        name: String,
        /// The bytes, base64.
        data: String,
    },
    /// Ask the control plane something about the session's project (`fctl
    /// memory`, `fctl retro`): the host sends it with its runner token
    /// (`POST /api/v1/runner/sessions/{id}/calls`) and answers with
    /// [`HostToAgent::Answer`]. Agents send it only for what the session's
    /// spec turns on, which reaches them only through hosts that know it,
    /// and so this frame.
    Call {
        request_id: String,
        call: AgentCall,
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

/// The environment variable that holds the session socket's path, set for
/// Claude and everything it runs.
pub const SESSION_SOCKET_ENV: &str = "FCTL_SESSION_SOCKET";

/// Longest request the session socket accepts: a screenshot to publish, in
/// base64.
pub const MAX_SESSION_REQUEST_BYTES: usize = 8 * 1024 * 1024;

/// From `fctl` in the sandbox to fctl-agent: one per connection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionRequest {
    /// Rename the session or say what it is about. Absent fields stay as
    /// they are; an empty description clears it.
    Edit {
        /// The session the caller means (from `FCTL_SESSION_ID`); fctl-agent
        /// refuses a request for another one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// Publish a screenshot at a link anyone who has it can open, until it
    /// expires, for pull requests. Only in projects that publish
    /// screenshots.
    Publish {
        /// The session the caller means, as for `Edit`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        /// The file's name, without directories.
        name: String,
        /// The bytes, base64.
        data: String,
    },
    /// Read or keep the project's memory (`fctl memory`), or record a
    /// retrospective (`fctl retro`): fctl-agent asks the control plane
    /// through its host. Only for what the project turned on.
    Call {
        /// The session the caller means, as for `Edit`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
        call: AgentCall,
    },
}

/// fctl-agent's answer on the session socket.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionReply {
    /// The session as the agent now describes it. fctl-agent passes it on
    /// to the control plane as soon as its host is connected.
    Edited {
        id: SessionId,
        project_slug: String,
        number: u32,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// The screenshot is published.
    Published {
        file: PublishedFile,
    },
    /// The control plane's answer to a `Call`.
    Answer {
        answer: AgentAnswer,
    },
    Error {
        error: ApiError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uplink::SessionState;

    #[test]
    fn session_socket_wire_format() -> Result<(), serde_json::Error> {
        let req: SessionRequest =
            serde_json::from_str(r#"{"type":"edit","title":"Fix the flaky login test"}"#)?;
        assert_eq!(
            req,
            SessionRequest::Edit {
                session_id: None,
                title: Some("Fix the flaky login test".into()),
                description: None,
            }
        );
        let reply = serde_json::to_value(SessionReply::Edited {
            id: SessionId("ses_1".into()),
            project_slug: "widgets".into(),
            number: 7,
            title: "Fix the flaky login test".into(),
            description: None,
        })?;
        assert_eq!(reply["type"], "edited");
        assert_eq!(reply["id"], "ses_1");
        assert!(reply.get("description").is_none());
        Ok(())
    }

    #[test]
    fn publishing_wire_format() -> Result<(), serde_json::Error> {
        let req: SessionRequest = serde_json::from_str(
            r#"{"type":"publish","session_id":"ses_1","name":"after.png","data":"iVBORw0KGgo="}"#,
        )?;
        assert!(matches!(req, SessionRequest::Publish { ref name, .. } if name == "after.png"));
        let up = serde_json::to_value(AgentToHost::Publish {
            request_id: "pub-1".into(),
            name: "after.png".into(),
            data: "iVBORw0KGgo=".into(),
        })?;
        assert_eq!(up["type"], "publish");
        let refused: HostToAgent = serde_json::from_value(serde_json::json!({
            "type": "published", "request_id": "pub-1",
            "error": { "code": "forbidden", "message": "Publishing screenshots is off" }
        }))?;
        let HostToAgent::Published { file, error, .. } = refused else {
            panic!("expected published");
        };
        assert_eq!(file, None);
        assert_eq!(error.map(|e| e.code), Some(crate::ErrorCode::Forbidden));
        Ok(())
    }

    #[test]
    fn calls_wire_format() -> Result<(), serde_json::Error> {
        let req: SessionRequest = serde_json::from_str(
            r#"{"type":"call","session_id":"ses_1","call":{"type":"remember","text":"Run npm ci in web/ first"}}"#,
        )?;
        let SessionRequest::Call { call, .. } = req else {
            panic!("expected a call");
        };
        assert_eq!(
            call,
            AgentCall::Remember {
                text: "Run npm ci in web/ first".into(),
                replaces: None,
                also_replaces: vec![],
            }
        );
        // `stale` may be left out.
        let list: AgentCall = serde_json::from_str(r#"{"type":"memories"}"#)?;
        assert_eq!(list, AgentCall::Memories { stale: false });
        let up = serde_json::to_value(AgentToHost::Call {
            request_id: "call-1".into(),
            call: AgentCall::Retrospective {
                outcome: crate::session::RetrospectiveOutcome::Completed,
                worked_on: "The login redirect.".into(),
                approach: None,
                challenges: None,
                insights: Some("The e2e tests need Chromium.".into()),
            },
        })?;
        assert_eq!(up["type"], "call");
        assert_eq!(up["call"]["type"], "retrospective");
        assert_eq!(up["call"]["outcome"], "completed");
        assert!(up["call"].get("approach").is_none());
        let refused: HostToAgent = serde_json::from_value(serde_json::json!({
            "type": "answer", "request_id": "call-1",
            "error": { "code": "forbidden", "message": "This project keeps no memory" }
        }))?;
        let HostToAgent::Answer { answer, error, .. } = refused else {
            panic!("expected an answer");
        };
        assert_eq!(answer, None);
        assert_eq!(error.map(|e| e.code), Some(crate::ErrorCode::Forbidden));
        Ok(())
    }

    #[test]
    fn duplicates_and_notes_that_replace_several_on_the_wire() -> Result<(), serde_json::Error> {
        use crate::MemoryId;
        // What fctl from before them sends still reads.
        let stale: AgentCall =
            serde_json::from_str(r#"{"type":"mark_stale","id":"mem_1","reason":"Moved"}"#)?;
        assert!(matches!(
            stale,
            AgentCall::MarkStale {
                duplicate_of: None,
                ..
            }
        ));
        // A duplicate keeps its reason for hosts from before duplicates.
        let duplicate = serde_json::to_value(AgentCall::MarkStale {
            id: MemoryId("mem_2".into()),
            reason: "Duplicate of mem_1".into(),
            duplicate_of: Some(MemoryId("mem_1".into())),
        })?;
        assert_eq!(duplicate["duplicate_of"], "mem_1");
        assert_eq!(duplicate["reason"], "Duplicate of mem_1");
        assert!(
            serde_json::to_value(AgentCall::MarkStale {
                id: MemoryId("mem_2".into()),
                reason: "Moved".into(),
                duplicate_of: None,
            })?
            .get("duplicate_of")
            .is_none()
        );
        // The first note it replaces goes as `replaces`, which hosts from
        // before `also_replaces` take alone.
        let several: AgentCall = serde_json::from_value(serde_json::json!({
            "type": "remember", "text": "Run npm ci first",
            "replaces": "mem_1", "also_replaces": ["mem_2", "mem_3"]
        }))?;
        let AgentCall::Remember {
            replaces,
            also_replaces,
            ..
        } = several
        else {
            panic!("expected remember");
        };
        assert_eq!(replaces, Some(MemoryId("mem_1".into())));
        assert_eq!(also_replaces.len(), 2);
        let one = serde_json::to_value(AgentCall::Remember {
            text: "Run npm ci first".into(),
            replaces: Some(MemoryId("mem_1".into())),
            also_replaces: vec![],
        })?;
        assert!(one.get("also_replaces").is_none());
        // Control planes from before `also_replaced` answer without it.
        let answer: crate::api::AgentAnswer = serde_json::from_value(serde_json::json!({
            "type": "memory",
            "memory": { "id": "mem_4", "project_id": "prj_1", "text": "Run npm ci first", "created_at": 5 }
        }))?;
        assert!(matches!(
            answer,
            crate::api::AgentAnswer::Memory { replaced: None, ref also_replaced, .. } if also_replaced.is_empty()
        ));
        assert!(
            serde_json::to_value(&answer)?
                .get("also_replaced")
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn updating_and_listing_retrospectives_on_the_wire() -> Result<(), serde_json::Error> {
        use crate::RetrospectiveId;
        use crate::api::AgentAnswer;
        use crate::session::RetrospectiveOutcome;
        // Only what changes goes: the parts left out stay as they were.
        let update = serde_json::to_value(AgentCall::UpdateRetrospective {
            id: RetrospectiveId("rtr_1".into()),
            outcome: Some(RetrospectiveOutcome::Completed),
            worked_on: None,
            approach: None,
            challenges: Some(String::new()),
            insights: None,
        })?;
        assert_eq!(
            update,
            serde_json::json!({
                "type": "update_retrospective", "id": "rtr_1",
                "outcome": "completed", "challenges": ""
            })
        );
        let read: AgentCall = serde_json::from_value(update)?;
        assert!(matches!(
            read,
            AgentCall::UpdateRetrospective { worked_on: None, ref challenges, .. }
                if challenges.as_deref() == Some("")
        ));
        let list = serde_json::to_value(AgentCall::Retrospectives)?;
        assert_eq!(list, serde_json::json!({ "type": "retrospectives" }));
        assert_eq!(
            serde_json::from_value::<AgentCall>(list)?,
            AgentCall::Retrospectives
        );
        let listed: AgentAnswer = serde_json::from_value(serde_json::json!({
            "type": "retrospectives",
            "retrospectives": [{
                "id": "rtr_1", "project_id": "prj_1",
                "session": { "id": "ses_1", "number": 3, "title": "Fix it" },
                "created_at": 5, "updated_at": 8, "outcome": "partial",
                "worked_on": "The login redirect."
            }]
        }))?;
        let AgentAnswer::Retrospectives { retrospectives } = listed else {
            panic!("expected retrospectives");
        };
        assert_eq!(retrospectives[0].updated_at, Some(8));
        Ok(())
    }

    #[test]
    fn state_from_older_agents_has_no_title() -> Result<(), serde_json::Error> {
        let st: SessionState = serde_json::from_str(r#"{"status":"running"}"#)?;
        assert_eq!(st.title, None);
        assert_eq!(st.description, None);
        let json = serde_json::to_value(&st)?;
        assert!(json.get("title").is_none());
        Ok(())
    }
}
