//! FleetDO ⇄ host: the control connection, over
//! `GET /api/v1/runner/connect` with the runner token (`fctl_rt_…`).
//!
//! The host says `Hello` (capabilities and the sessions it still runs),
//! FleetDO answers `Welcome`, and from then on assigns sessions with
//! `StartSession`. Session events do not use this connection; each session has
//! its own uplink (see [`crate::uplink`]).

use crate::event::Attachment;
use crate::session::{
    GithubAccess, Memory, ProjectSettings, PullRequestRef, RepoAccess, RepoGrant, RepoRef,
    RunnerRef, SessionStatus, UserRef,
};
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
    /// The images it runs for projects that choose their own, besides
    /// `image`: references, or prefixes ending in `/*`. Absent, as from
    /// older hosts: any image.
    pub allowed_images: Option<Vec<String>>,
    /// The repositories (`owner/name`, `owner/*`) whose files it builds
    /// images from, for projects that build theirs. Absent, as from older
    /// hosts and hosts whose driver uses no images: none.
    pub allowed_builds: Option<Vec<String>>,
    /// The free disk (MiB) it keeps where its sandboxes are. While its
    /// heartbeats' `disk_available_mb` is below it, it removes stopped
    /// sandboxes, those stopped longest first, and takes no new sessions,
    /// only resumes of the ones whose sandboxes it has. Absent when it
    /// keeps none, and from older hosts.
    pub min_free_disk_mb: Option<u64>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HostHeartbeat {
    pub sessions: Vec<HostSessionReport>,
    pub load1: Option<f32>,
    pub memory_available_mb: Option<u64>,
    /// Free disk (MiB) where its sandboxes are (see
    /// [`HostHello::min_free_disk_mb`]). Older hosts measure their data
    /// directory.
    pub disk_available_mb: Option<u64>,
}

/// Which credential a session needs refreshed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// A fresh Claude access token (the old one expired or was rejected).
    Claude,
    /// Fresh GitHub installation tokens for the session's repositories.
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

/// A GitHub installation token, scoped to some of the session's
/// repositories and valid for an hour.
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
    /// For the primary repository and the project's other writable ones.
    pub github: Option<GithubCredential>,
    /// Read-only, for the project's read-only repositories (every repository
    /// of the account, for an owner-wide grant).
    #[serde(default)]
    pub github_read: Option<GithubCredential>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ProjectSpec {
    pub id: ProjectId,
    pub slug: String,
    pub name: String,
    /// The primary repository, cloned into the agent's working directory.
    pub repo: Option<RepoRef>,
    pub default_branch: String,
    pub settings: ProjectSettings,
    /// Other repositories the agent may use, cloned when it needs them.
    /// Absent when there are none. Older agents ignore it and work in the
    /// primary repository only.
    #[serde(default)]
    pub repos: Option<Vec<SessionRepo>>,
}

/// A repository a session's agent may use besides the primary one, or every
/// repository of an account (an owner-wide grant).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SessionRepo {
    pub owner: String,
    /// Absent for an owner-wide grant.
    pub name: Option<String>,
    pub access: RepoAccess,
    /// Why the session has no token for it (it left the GitHub App's
    /// installation). The agent says so and works without it.
    pub unavailable: Option<String>,
}

impl SessionRepo {
    pub fn grant(&self) -> RepoGrant {
        RepoGrant {
            owner: self.owner.clone(),
            name: self.name.clone(),
            access: self.access,
        }
    }
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
    /// Files attached to the task. The host fetches them like a message's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<Attachment>>", optional))]
    pub task_attachments: Vec<Attachment>,
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
    /// Not used, and empty. Control planes from before `co_authored_by`
    /// sent the starter's sign-in email here, which no agent may put in a
    /// commit; older agents still expect the field.
    pub co_authors: Vec<GitIdentity>,
    /// Credited in commits with `Co-authored-by` trailers: the person who
    /// started the session, with the commit email they chose. Absent when
    /// nobody chose one; older agents add no trailers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<GitIdentity>>", optional))]
    pub co_authored_by: Vec<GitIdentity>,
    /// The web URL of the session, for links in pull requests.
    pub web_url: String,
    pub idle_timeout_secs: u64,
    /// Identifies this run of the session; echoed in `SessionEnded` so a late
    /// report from an earlier run cannot end a newer one.
    pub run_id: Option<String>,
    /// The pull requests the session lists. Its agent follows them with the
    /// ones it finds, so their state stays current in a new sandbox too.
    /// Older agents ignore them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<PullRequestRef>>", optional))]
    pub pull_requests: Vec<PullRequestRef>,
    /// The project's memory as the run starts, when the project keeps one
    /// (`ProjectSettings::project_memory`): its agent starts with these
    /// notes and keeps the memory with `fctl memory`. Absent otherwise, and
    /// from older control planes; older hosts leave it out of the agent's
    /// configuration, so their agents never offer `fctl memory`.
    pub memory: Option<MemorySpec>,
    /// The project asks its agents for retrospectives
    /// (`ProjectSettings::retrospectives`), which they record with `fctl
    /// retro`. False from older control planes, and through older hosts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub retrospectives: bool,
    /// What the agent may do on GitHub (`credentials` carry it), when it
    /// may do less than push: its agent then pushes nothing, and tells
    /// Claude what it may do. Absent: push, as from older control planes.
    /// Older hosts leave it out of the agent's configuration, and their
    /// agents' pushes fail.
    pub github_access: Option<GithubAccess>,
}

/// A project's memory, as a session's agent starts with it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemorySpec {
    /// Its current notes, newest first: as many as fit in what the agent
    /// reads as it starts (`fctl_domain::memory::for_prompt`).
    pub notes: Vec<Memory>,
    /// How many more current notes it has, which `fctl memory` lists.
    #[serde(default)]
    pub more: u32,
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
        /// See [`SessionCredentials::github_read`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        github_read: Option<GithubCredential>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_json() -> serde_json::Value {
        serde_json::json!({
            "session_id": "ses_1",
            "number": 1,
            "title": "T",
            "project": {
                "id": "prj_1", "slug": "app", "name": "App",
                "repo": { "owner": "acme", "name": "app" },
                "default_branch": "main", "settings": {}
            },
            "branch": "fctl/1", "base_branch": "main",
            "author": { "id": "usr_1", "name": "Ada" },
            "claude_session_id": "00000000-0000-0000-0000-000000000000",
            "resume": false, "secrets": {},
            "credentials": {
                "claude": { "id": "crd_1", "kind": "oauth", "access_token": "a", "scopes": [] },
                "github": { "token": "ghs_w", "expires_at": 1 }
            },
            "git_author": { "name": "bot", "email": "bot@example.com" },
            "co_authors": [], "web_url": "https://fctl.test/s/ses_1", "idle_timeout_secs": 60
        })
    }

    #[test]
    fn specs_without_other_repositories_read_and_write_as_before() -> serde_json::Result<()> {
        let spec: SessionSpec = serde_json::from_value(spec_json())?;
        assert_eq!(spec.project.repos, None);
        assert_eq!(spec.credentials.github_read, None);
        let json = serde_json::to_value(&spec)?;
        assert!(json["project"].get("repos").is_none());
        assert!(json["credentials"].get("github_read").is_none());
        Ok(())
    }

    #[test]
    fn specs_carry_other_repositories_and_a_read_token() -> serde_json::Result<()> {
        let mut json = spec_json();
        json["project"]["repos"] = serde_json::json!([
            { "owner": "acme", "name": "lib", "access": "write" },
            { "owner": "acme", "access": "read" }
        ]);
        json["credentials"]["github_read"] =
            serde_json::json!({ "token": "ghs_r", "expires_at": 2 });
        let spec: SessionSpec = serde_json::from_value(json)?;
        let repos = spec.project.repos.unwrap_or_default();
        assert_eq!(repos[0].grant().label(), "acme/lib");
        assert_eq!(repos[1].grant().label(), "acme/*");
        assert_eq!(repos[1].access, RepoAccess::Read);
        assert_eq!(
            spec.credentials.github_read.map(|c| c.token),
            Some("ghs_r".into())
        );
        Ok(())
    }

    #[test]
    fn only_co_authored_by_credits_people() -> serde_json::Result<()> {
        // An older control plane's spec: the starter's sign-in email in
        // `co_authors`, which is not consent to put it in commits.
        let mut json = spec_json();
        json["co_authors"] = serde_json::json!([{ "name": "Ada", "email": "ada@example.com" }]);
        let spec: SessionSpec = serde_json::from_value(json)?;
        assert!(spec.co_authored_by.is_empty());

        let mut json = spec_json();
        json["co_authored_by"] =
            serde_json::json!([{ "name": "Ada", "email": "1+ada@users.noreply.github.com" }]);
        let spec: SessionSpec = serde_json::from_value(json)?;
        assert_eq!(
            spec.co_authored_by[0].email,
            "1+ada@users.noreply.github.com"
        );

        // Older agents need `co_authors`; nobody to credit leaves the new field out.
        let spec: SessionSpec = serde_json::from_value(spec_json())?;
        let json = serde_json::to_value(&spec)?;
        assert_eq!(json["co_authors"], serde_json::json!([]));
        assert!(json.get("co_authored_by").is_none());
        Ok(())
    }

    #[test]
    fn credentials_messages_add_the_read_token_as_an_option() -> serde_json::Result<()> {
        // From a control plane that sends one token.
        let msg: FleetToHost = serde_json::from_value(serde_json::json!({
            "type": "credential", "request_id": "r1",
            "github": { "token": "ghs_w", "expires_at": 1 }
        }))?;
        let FleetToHost::Credential { github_read, .. } = &msg else {
            panic!("expected a credential");
        };
        assert_eq!(github_read, &None);
        let json = serde_json::to_value(&msg)?;
        assert!(json.get("github_read").is_none());
        Ok(())
    }
}
