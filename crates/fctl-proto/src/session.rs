//! Summaries of the main resources (users, projects, sessions, runners) as
//! they appear in API responses and live updates.

use crate::{Millis, ProjectId, RunnerId, SessionId, UserId};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// A user as other people see them (author of a message, starter of a session).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UserRef {
    pub id: UserId,
    pub name: String,
    pub avatar_url: Option<String>,
}

/// Instance-wide role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum InstanceRole {
    /// Created the instance; can do everything, including managing admins.
    Owner,
    /// Manages access (allowlist, users), runners and instance settings.
    Admin,
    /// Uses projects they are a member of.
    Member,
}

impl InstanceRole {
    pub fn is_admin(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            _ => None,
        }
    }
}

/// Role within a project, from most to least privileged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ProjectRole {
    /// Full control, including deleting the project.
    Owner,
    /// Manages settings, secrets and members.
    Maintainer,
    /// Starts sessions and messages agents.
    Editor,
    /// Reads sessions.
    Viewer,
}

impl ProjectRole {
    /// Higher is more privileged.
    pub fn rank(self) -> u8 {
        match self {
            Self::Owner => 4,
            Self::Maintainer => 3,
            Self::Editor => 2,
            Self::Viewer => 1,
        }
    }

    pub fn at_least(self, other: Self) -> bool {
        self.rank() >= other.rank()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Maintainer => "maintainer",
            Self::Editor => "editor",
            Self::Viewer => "viewer",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "maintainer" => Some(Self::Maintainer),
            "editor" => Some(Self::Editor),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }
}

/// What a person may do with a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SessionRole {
    /// Read the transcript.
    Viewer,
    /// Also message the agent and interrupt it.
    Editor,
    /// Also stop, resume, archive and share it.
    Owner,
}

impl SessionRole {
    pub fn can_message(self) -> bool {
        matches!(self, Self::Editor | Self::Owner)
    }

    pub fn can_manage(self) -> bool {
        matches!(self, Self::Owner)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Editor => "editor",
            Self::Owner => "owner",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "viewer" => Some(Self::Viewer),
            "editor" => Some(Self::Editor),
            "owner" => Some(Self::Owner),
            _ => None,
        }
    }
}

/// Lifecycle of a session.
///
/// ```text
/// queued → provisioning → preparing → running ⇄ idle → stopped
///                                        └→ paused (usage limit) → running
/// any → failed
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Waiting for a runner with free capacity.
    Queued,
    /// A runner is creating the sandbox.
    Provisioning,
    /// Cloning, creating the branch, running the setup script.
    Preparing,
    /// The agent is working on a turn.
    Running,
    /// The agent finished its turn and waits for a message.
    Idle,
    /// Waiting for a Claude usage limit to reset; resumes on its own.
    Paused,
    /// Shutting down.
    Stopping,
    /// The sandbox is stopped. The session can be resumed.
    Stopped,
    /// Something went wrong; `status_reason` says what.
    Failed,
}

impl SessionStatus {
    /// Whether a sandbox is (or should be) running for this session.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Provisioning
                | Self::Preparing
                | Self::Running
                | Self::Idle
                | Self::Paused
                | Self::Stopping
        )
    }

    /// Whether the session has ended (it may still be resumed).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }

    /// Whether the agent can take a message right now (it is queued if busy).
    pub fn accepts_messages(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Provisioning
                | Self::Preparing
                | Self::Running
                | Self::Idle
                | Self::Paused
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Provisioning => "provisioning",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Paused => "paused",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Self::Queued,
            "provisioning" => Self::Provisioning,
            "preparing" => Self::Preparing,
            "running" => Self::Running,
            "idle" => Self::Idle,
            "paused" => Self::Paused,
            "stopping" => Self::Stopping,
            "stopped" => Self::Stopped,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

/// A GitHub repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// Parses `owner/name`.
    pub fn parse(s: &str) -> Option<Self> {
        let (owner, name) = s.trim().split_once('/')?;
        let ok = |p: &str| {
            !p.is_empty()
                && p.len() <= 100
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                && p != "."
                && p != ".."
        };
        (ok(owner) && ok(name)).then(|| Self {
            owner: owner.to_owned(),
            name: name.trim_end_matches(".git").to_owned(),
        })
    }

    pub fn html_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }

    pub fn clone_url(&self) -> String {
        format!("https://github.com/{}/{}.git", self.owner, self.name)
    }

    /// Whether both name the same repository (GitHub ignores case).
    pub fn same(&self, other: &RepoRef) -> bool {
        self.owner.eq_ignore_ascii_case(&other.owner) && self.name.eq_ignore_ascii_case(&other.name)
    }
}

/// What agents may do in a repository (`Read` < `Write`).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RepoAccess {
    /// Clone and fetch it, and read its issues, pull requests and CI results.
    #[default]
    Read,
    /// Also push branches, open pull requests and comment on issues.
    Write,
}

impl RepoAccess {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            _ => None,
        }
    }
}

/// A repository a project's agents may use besides its primary one, or an
/// owner-wide grant: every repository of an account the GitHub App can reach.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct RepoGrant {
    pub owner: String,
    /// Absent for an owner-wide grant.
    pub name: Option<String>,
    pub access: RepoAccess,
}

impl RepoGrant {
    /// `owner/name`, or `owner/*` for an owner-wide grant.
    pub fn label(&self) -> String {
        format!("{}/{}", self.owner, self.name.as_deref().unwrap_or("*"))
    }

    /// The repository, unless this is an owner-wide grant.
    pub fn repo(&self) -> Option<RepoRef> {
        self.name.as_ref().map(|name| RepoRef {
            owner: self.owner.clone(),
            name: name.clone(),
        })
    }
}

/// Settings agents use for a project. Every field has a sensible default, so
/// clients send only what they change.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct ProjectSettings {
    /// OCI image for sandboxes. Defaults to the host's configured agent image.
    pub image: Option<String>,
    /// Shell script run in the repository after cloning (for example `npm ci`).
    pub setup_script: Option<String>,
    /// Claude model alias or name (`opus`, `sonnet`, …). Defaults to Claude's own default.
    pub model: Option<String>,
    /// Claude effort level (`low` … `max`).
    pub effort: Option<String>,
    /// Text appended to Claude's system prompt for every session.
    pub instructions: Option<String>,
    /// CPUs per sandbox.
    pub cpus: Option<u32>,
    /// Memory per sandbox, in MiB.
    pub memory_mb: Option<u32>,
    /// Minutes an idle session keeps its sandbox before it stops.
    pub idle_timeout_minutes: Option<u32>,
    /// Ask agents to open a pull request when they finish.
    pub auto_pull_request: bool,
    /// Enable Remote Control (follow in the Claude apps) when the credential allows it.
    pub remote_control: bool,
    /// Let agents use the claude.ai connectors of the person who starts a
    /// session. Only full Claude logins have them.
    pub claude_ai_connectors: bool,
    /// Credit the person who started a session in its commits, with a
    /// `Co-authored-by` trailer for the commit email they chose.
    pub credit_people: bool,
    /// Credit Claude in commits, as Claude Code does by default: its
    /// `Co-authored-by` trailer, and in Remote Control sessions the
    /// claude.ai session link. Older agents always do.
    pub credit_claude: bool,
    /// Let agents publish screenshots on the control plane (`fctl publish`),
    /// at links anyone who has one can open, to show them in pull requests:
    /// GitHub takes no uploads from a session's GitHub App token. Off by
    /// default, and only where the instance's admins allow it
    /// ([`crate::api::InstanceSettings`]). Off, the project's published
    /// screenshots are offline too.
    pub publish_screenshots: bool,
    /// Days a published screenshot's link works, 1 to 60. Absent: 30.
    pub screenshot_days: Option<u32>,
    /// Non-secret environment variables for sandboxes.
    pub env: std::collections::BTreeMap<String, String>,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            image: None,
            setup_script: None,
            model: None,
            effort: None,
            instructions: None,
            cpus: None,
            memory_mb: None,
            idle_timeout_minutes: None,
            auto_pull_request: true,
            remote_control: true,
            claude_ai_connectors: true,
            credit_people: true,
            credit_claude: true,
            publish_screenshots: false,
            screenshot_days: None,
            env: Default::default(),
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Project {
    pub id: ProjectId,
    /// URL-safe, unique per instance.
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub repo: Option<RepoRef>,
    pub default_branch: String,
    pub settings: ProjectSettings,
    /// The caller's role (absent for instance admins without membership).
    pub role: Option<ProjectRole>,
    pub created_by: UserRef,
    pub created_at: Millis,
    pub updated_at: Millis,
    pub archived_at: Option<Millis>,
    /// Sessions currently provisioning, preparing, running, idle or paused.
    pub active_sessions: u32,
    pub total_sessions: u32,
    pub last_activity_at: Option<Millis>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PullRequestRef {
    pub number: u64,
    pub url: String,
    pub title: Option<String>,
    /// `open`, `closed`, `merged` or `draft`.
    pub state: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct RunnerRef {
    pub id: RunnerId,
    pub name: String,
}

/// Remote Control state of a session (following it in the Claude apps).
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct RemoteControlState {
    pub enabled: bool,
    /// claude.ai/code URL of the session.
    pub session_url: Option<String>,
    /// Why Remote Control is off, when it was requested.
    pub unavailable_reason: Option<String>,
}

/// Counters shown on session lists.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct SessionStats {
    pub turns: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub tool_calls: u32,
    pub files_changed: u32,
    pub additions: u32,
    pub deletions: u32,
    /// Milliseconds Claude spent working (sum of turn durations).
    pub busy_ms: u64,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Session {
    pub id: SessionId,
    pub project_id: ProjectId,
    pub project_slug: String,
    /// Sequential per project (`#12`).
    pub number: u32,
    /// Starts as the first line of the task; the agent renames it to what it
    /// works on (`fctl edit`), and so can its owner.
    pub title: String,
    /// What the session is about, in a sentence or two (absent until the
    /// agent or its owner writes one).
    pub description: Option<String>,
    pub status: SessionStatus,
    pub status_reason: Option<String>,
    /// When a paused session resumes on its own.
    pub resume_at: Option<Millis>,
    pub created_by: UserRef,
    pub created_at: Millis,
    pub updated_at: Millis,
    pub started_at: Option<Millis>,
    pub ended_at: Option<Millis>,
    pub last_activity_at: Option<Millis>,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub pull_request: Option<PullRequestRef>,
    pub runner: Option<RunnerRef>,
    pub model: Option<String>,
    pub remote_control: Option<RemoteControlState>,
    pub stats: SessionStats,
    /// The caller's role.
    pub role: Option<SessionRole>,
    pub archived: bool,
    /// Whether the caller follows it: its followers are notified when it
    /// waits for a reply, pauses on a usage limit or fails. Absent from live
    /// updates, which go to everyone watching.
    pub following: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_stored_before_a_field_get_its_default() -> Result<(), serde_json::Error> {
        // A project saved before `claude_ai_connectors` existed keeps its
        // connectors, and one from before the credit settings credits
        // people and Claude.
        let old: ProjectSettings =
            serde_json::from_str(r#"{"auto_pull_request":false,"remote_control":true,"env":{}}"#)?;
        assert!(old.claude_ai_connectors);
        assert!(old.credit_people && old.credit_claude);
        assert!(!old.auto_pull_request);
        // Nothing published screenshots before the setting existed.
        assert!(!old.publish_screenshots);
        assert_eq!(old.screenshot_days, None);
        let on: ProjectSettings =
            serde_json::from_str(r#"{"publish_screenshots":true,"screenshot_days":7}"#)?;
        assert!(on.publish_screenshots);
        assert_eq!(on.screenshot_days, Some(7));
        assert!(serde_json::to_value(&old)?.get("screenshot_days").is_none());
        let off: ProjectSettings =
            serde_json::from_str(r#"{"credit_people":false,"credit_claude":false}"#)?;
        assert!(!off.credit_people && !off.credit_claude);
        assert!(off.claude_ai_connectors);
        let off: ProjectSettings = serde_json::from_str(r#"{"claude_ai_connectors":false}"#)?;
        assert_eq!(
            off,
            ProjectSettings {
                claude_ai_connectors: false,
                ..Default::default()
            }
        );
        Ok(())
    }
}
