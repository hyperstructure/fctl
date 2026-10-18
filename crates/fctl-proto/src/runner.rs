//! FleetDO ⇄ host: the control connection, over
//! `GET /api/v1/runner/connect` with the runner token (`fctl_rt_…`).
//!
//! The host says `Hello` (capabilities and the sessions it still runs),
//! FleetDO answers `Welcome`, and from then on assigns sessions with
//! `StartSession`. Session events do not use this connection; each session has
//! its own uplink (see [`crate::uplink`]).

use crate::session::{ProjectSettings, RepoRef, RunnerRef, SessionStatus, UserRef};
use crate::{CredentialId, LoginJobId, Millis, ProjectId, SessionId};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::BTreeMap;

/// A sandbox driver the host knows about.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct DriverInfo {
    /// `apple`, `lima`, `docker` or `process`.
    pub name: String,
    pub available: bool,
    /// `vm`, `container` or `none`.
    pub isolation: String,
    pub version: Option<String>,
    /// Why it is unavailable, or other details.
    pub detail: Option<String>,
}

/// A sandbox's CPUs and memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SandboxSize {
    pub cpus: u32,
    /// MiB.
    pub memory_mb: u32,
}

/// The sizes a host gives sandboxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SandboxLimits {
    /// For sessions whose project sets no size.
    pub default: SandboxSize,
    /// The most one sandbox may have: what the host was given, within what
    /// the machine (or, with Docker on macOS, the Docker VM) has.
    pub max: SandboxSize,
}

/// A session the host has a sandbox for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HostSessionReport {
    pub session_id: SessionId,
    pub status: SessionStatus,
    /// The last event sequence number the agent produced.
    pub agent_seq: u64,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HostHello {
    pub protocol: u32,
    /// fctl version of the host.
    pub version: String,
    pub hostname: String,
    /// `macos` or `linux`.
    pub os: String,
    /// `aarch64` or `x86_64`.
    pub arch: String,
    pub cpus: u32,
    pub memory_mb: u64,
    pub drivers: Vec<DriverInfo>,
    /// The driver this host uses for new sessions.
    pub driver: String,
    /// Maximum concurrent sessions.
    pub capacity: u32,
    pub labels: Vec<String>,
    pub sessions: Vec<HostSessionReport>,
    /// The default sandbox image the host uses.
    pub image: Option<String>,
    /// What the host gives each sandbox. Older hosts send none and get
    /// sessions of any size.
    pub sandbox: Option<SandboxLimits>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HostHeartbeat {
    pub sessions: Vec<HostSessionReport>,
    pub load1: Option<f32>,
    pub memory_available_mb: Option<u64>,
    pub disk_available_mb: Option<u64>,
}

/// Which credential a session needs refreshed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// A fresh Claude access token (the old one expired or was rejected).
    Claude,
    /// A fresh GitHub installation token for the session's repository.
    Github,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ClaudeCredentialKind {
    /// From `claude setup-token`: inference only, valid for a year, no refresh.
    SetupToken,
    /// From `claude auth login`: full scopes (Remote Control), refreshed by fctl.
    Oauth,
}

impl ClaudeCredentialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SetupToken => "setup_token",
            Self::Oauth => "oauth",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "setup_token" => Some(Self::SetupToken),
            "oauth" => Some(Self::Oauth),
            _ => None,
        }
    }
}

/// The Claude account a credential belongs to, as `~/.claude.json` records it.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ClaudeAccount {
    pub account_uuid: Option<String>,
    pub email: Option<String>,
    pub organization_uuid: Option<String>,
    pub organization_name: Option<String>,
    pub display_name: Option<String>,
    /// `oauthAccount` from `~/.claude.json`, verbatim, so sandboxes can be
    /// seeded with it.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub raw: Option<serde_json::Value>,
}

/// What a sandbox needs to run Claude on someone's subscription.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ClaudeCredential {
    pub id: CredentialId,
    pub kind: ClaudeCredentialKind,
    /// Goes into `CLAUDE_CODE_OAUTH_TOKEN`.
    pub access_token: String,
    /// Goes into `CLAUDE_CODE_OAUTH_SCOPES`.
    pub scopes: Vec<String>,
    pub expires_at: Option<Millis>,
    /// `pro`, `max`, `team` or `enterprise`.
    pub subscription_type: Option<String>,
    pub account: Option<ClaudeAccount>,
}

/// A GitHub installation token, scoped to the session's repository.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GithubCredential {
    pub token: String,
    pub expires_at: Millis,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SessionCredentials {
    pub claude: ClaudeCredential,
    pub github: Option<GithubCredential>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ProjectSpec {
    pub id: ProjectId,
    pub slug: String,
    pub name: String,
    pub repo: Option<RepoRef>,
    pub default_branch: String,
    pub settings: ProjectSettings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GitIdentity {
    pub name: String,
    pub email: String,
}

/// Everything a host needs to run a session. Built by FleetDO just in time,
/// so credentials are fresh.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SessionSpec {
    pub session_id: SessionId,
    pub number: u32,
    pub title: String,
    pub project: ProjectSpec,
    /// The branch the agent works on.
    pub branch: String,
    /// The branch it starts from.
    pub base_branch: String,
    /// The first message. Absent when resuming without a new message.
    pub prompt: Option<String>,
    /// The session's original task, also when resuming: an agent whose
    /// earlier conversation is gone (its sandbox expired) starts from it.
    #[serde(default)]
    pub task: Option<String>,
    pub author: UserRef,
    /// UUID passed to `claude --session-id` (or `--resume`).
    pub claude_session_id: String,
    /// Resume `claude_session_id` instead of starting a new conversation.
    pub resume: bool,
    /// Model override for this session.
    pub model: Option<String>,
    /// Effort override for this session.
    pub effort: Option<String>,
    /// Secret environment variables (project secrets).
    pub secrets: BTreeMap<String, String>,
    pub credentials: SessionCredentials,
    /// Commits are authored as this identity (the GitHub App's bot).
    pub git_author: GitIdentity,
    /// Added as `Co-authored-by` trailers (the person who started the session).
    pub co_authors: Vec<GitIdentity>,
    /// The web URL of the session, for links in pull requests.
    pub web_url: String,
    pub idle_timeout_secs: u64,
    /// Identifies this run of the session; echoed in `SessionEnded` so a late
    /// report from an earlier run cannot end a newer one.
    pub run_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Someone pressed stop.
    User,
    /// The idle timeout passed.
    Idle,
    /// The session or project was deleted.
    Deleted,
    /// The host is shutting down or the session moves elsewhere.
    Shutdown,
}

/// The complete result of `claude auth login`, sent once from the host to the
/// control plane, which encrypts and stores it. Never sent to sandboxes.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ClaudeOauthBundle {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: Option<Millis>,
    pub scopes: Vec<String>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub account: Option<ClaudeAccount>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClaudeLoginUpdate {
    Starting,
    /// Open `url`, approve, and paste the code shown.
    AwaitingCode {
        url: String,
    },
    Verifying,
    Succeeded {
        bundle: Box<ClaudeOauthBundle>,
    },
    Failed {
        error: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostToFleet {
    Hello(Box<HostHello>),
    Heartbeat(HostHeartbeat),
    /// Answer to `StartSession`.
    StartResult {
        session_id: SessionId,
        accepted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        error: Option<String>,
    },
    /// The sandbox of a session is gone (stopped, removed or crashed).
    SessionEnded {
        session_id: SessionId,
        status: SessionStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        run_id: Option<String>,
    },
    CredentialRequest {
        request_id: String,
        session_id: SessionId,
        kind: CredentialKind,
    },
    ClaudeLogin {
        job_id: LoginJobId,
        update: ClaudeLoginUpdate,
    },
    Pong,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FleetToHost {
    Welcome {
        runner: RunnerRef,
        heartbeat_secs: u32,
        server_time: Millis,
    },
    StartSession {
        spec: Box<SessionSpec>,
    },
    StopSession {
        session_id: SessionId,
        reason: StopReason,
        /// Also delete the sandbox (otherwise it is kept for resuming).
        destroy: bool,
    },
    Credential {
        request_id: String,
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
    /// Run `claude auth login` in a fresh sandbox for this person.
    ClaudeLoginStart {
        job_id: LoginJobId,
        user: UserRef,
    },
    /// The code the person pasted after approving.
    ClaudeLoginCode {
        job_id: LoginJobId,
        code: String,
    },
    ClaudeLoginCancel {
        job_id: LoginJobId,
    },
    Ping,
}
