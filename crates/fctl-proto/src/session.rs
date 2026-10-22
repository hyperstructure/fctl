//! Summaries of the main resources (users, projects, sessions, runners) as
//! they appear in API responses and live updates.

use crate::{
    MemoryId, Millis, ProjectId, RetrospectiveId, RunnerId, SessionId, UserId, WorkspaceId,
};
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
    /// The primary instance owner, one per instance: everything admins
    /// do, and managing admins. Nobody else can demote, disable or remove
    /// them; they hand the role to an admin (`POST /api/v1/admin/transfer`).
    Owner,
    /// Manages people and invitations, runners, which workspaces use which
    /// GitHub accounts, and the instance's settings. Sees inside only the
    /// workspaces they are members of.
    Admin,
    /// Uses the workspaces they are members of.
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

/// Role within a workspace, from most to least privileged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRole {
    /// The primary admin, one per workspace: everything admins do, and
    /// deleting the workspace. No other admin can demote or remove them;
    /// they hand the role to another admin.
    Owner,
    /// Manages members, invitations, runners, GitHub accounts, settings
    /// and every project of the workspace.
    Admin,
    /// Works in the projects they are members of, and in every project with
    /// the workspace's base role.
    Member,
}

impl WorkspaceRole {
    /// Higher is more privileged.
    pub fn rank(self) -> u8 {
        match self {
            Self::Owner => 3,
            Self::Admin => 2,
            Self::Member => 1,
        }
    }

    /// The primary admin or an admin.
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

/// A workspace's color, from the brand palette: its avatar is its initial
/// on this color, until an admin uploads an image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceColor {
    #[default]
    Yellow,
    Pink,
    Blue,
    Ink,
}

impl WorkspaceColor {
    pub const ALL: [Self; 4] = [Self::Yellow, Self::Pink, Self::Blue, Self::Ink];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yellow => "yellow",
            Self::Pink => "pink",
            Self::Blue => "blue",
            Self::Ink => "ink",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// A workspace as projects, sessions and runners name it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WorkspaceRef {
    pub id: WorkspaceId,
    /// URL-safe, unique per instance.
    pub slug: String,
    pub name: String,
    pub color: WorkspaceColor,
    /// Its avatar image, a path in the app; absent: its initial on `color`.
    pub avatar_url: Option<String>,
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
/// stopped → queued (a message, or a resume)
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
    /// The sandbox is stopped, and its host takes other work. A message
    /// resumes the session.
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

    /// Whether a message resumes the session: it stopped (after being idle,
    /// or because someone stopped it), or it is stopping and resumes once it
    /// has stopped. A failed session needs its owner to resume it.
    pub fn resumes_for_messages(self) -> bool {
        matches!(self, Self::Stopping | Self::Stopped)
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

/// What a session's agent may do on GitHub in the repositories it may write
/// to (`Read` < `Comment` < `Push`): it only reads the others. A session
/// asks for it as it starts; its project's settings and the access token
/// that starts it can hold it lower. The tokens the session gets carry its
/// permissions, so GitHub enforces it.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GithubAccess {
    /// Read the code, pull requests, issues and CI results; write nothing.
    Read,
    /// Also review and comment on pull requests and issues, label them and
    /// resolve review threads; push nothing, merge nothing.
    Comment,
    /// Also push branches and open pull requests: what every session's
    /// agent did before sessions had a GitHub access.
    #[default]
    Push,
}

impl GithubAccess {
    pub const ALL: [Self; 3] = [Self::Push, Self::Comment, Self::Read];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Comment => "comment",
            Self::Push => "push",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Self::Read),
            "comment" => Some(Self::Comment),
            "push" => Some(Self::Push),
            _ => None,
        }
    }

    /// Whether its agent pushes, as every agent did before.
    pub fn pushes(self) -> bool {
        self == Self::Push
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

/// A sandbox image built from the project's repository, as its default
/// branch has it. Hosts build it again when the files it comes from change.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ImageBuild {
    /// A Dockerfile, or a dev container configuration (a file ending in
    /// `.json`, like `.devcontainer/devcontainer.json`), by its path in the
    /// repository.
    pub file: String,
    /// For a Dockerfile, the directory in the repository it builds in.
    /// Absent: the repository's root. A dev container configuration names
    /// its own.
    pub context: Option<String>,
}

impl ImageBuild {
    /// Whether `file` is a dev container configuration rather than a
    /// Dockerfile.
    pub fn is_devcontainer(&self) -> bool {
        self.file.to_ascii_lowercase().ends_with(".json")
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
    /// Build the sandbox image from a file in the project's repository
    /// instead, on hosts whose owners allow it (`allowed_builds`). It takes
    /// the place of `image`. Older hosts and control planes ignore it.
    /// Always sent, as `null` for none: an update without it keeps the
    /// stored build (see `fctl_domain::settings::keep_unsent`).
    #[serialize_always]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub image_build: Option<ImageBuild>,
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
    /// Minutes an idle session keeps its sandbox, and its slot on the host,
    /// before it stops (10 by default). A message resumes it.
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
    /// Agents keep the project's memory (`fctl memory`): each session's
    /// agent starts with the notes earlier agents and people left, adds
    /// what future agents should know, and marks notes that turned out
    /// wrong stale. On by default. Off, agents hear nothing of it, and the
    /// notes stay for people to read and delete.
    pub project_memory: bool,
    /// Agents record a retrospective when they finish a piece of work
    /// (`fctl retro`): what they worked on, how, what got in the way and
    /// what a future agent should know. On by default. Off, agents hear
    /// nothing of it, and the ones recorded stay.
    pub retrospectives: bool,
    /// The most its sessions' agents may do on GitHub, and what a session
    /// gets unless it asks for less. Absent: push. Running sessions get no
    /// more than this from their next GitHub token on (within the hour).
    /// An update that leaves it out keeps the stored value, so an older
    /// client can't lift it, and a value this version doesn't know reads
    /// as `read`.
    #[serde(deserialize_with = "least_if_unknown")]
    pub github_access: Option<GithubAccess>,
    /// Non-secret environment variables for sandboxes.
    pub env: std::collections::BTreeMap<String, String>,
}

/// A [`GithubAccess`] that a newer version may have stored, which reads as
/// the least an agent may do when this version doesn't know it, rather than
/// failing the whole of the settings it is in.
fn least_if_unknown<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<GithubAccess>, D::Error> {
    let v: Option<String> = Option::deserialize(d)?;
    Ok(v.map(|v| GithubAccess::parse(&v).unwrap_or(GithubAccess::Read)))
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            image: None,
            image_build: None,
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
            project_memory: true,
            retrospectives: true,
            github_access: None,
            env: Default::default(),
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Project {
    pub id: ProjectId,
    /// The workspace it belongs to (absent from control planes before
    /// workspaces).
    pub workspace: Option<WorkspaceRef>,
    /// URL-safe, unique per instance.
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub repo: Option<RepoRef>,
    /// How many repositories its agents may use besides `repo`, named one
    /// by one (`GET /api/v1/projects/{id}/repos` lists them). Absent from
    /// older control planes.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(as = "Option<u32>", optional))]
    pub other_repos: u32,
    /// The account whose every repository its agents may also read (an
    /// owner-wide grant, `owner/*`): the one `repo` belongs to. Absent
    /// without one.
    pub all_repos_of: Option<String>,
    pub default_branch: String,
    pub settings: ProjectSettings,
    /// The caller's role: their membership, their workspace's base role, or
    /// owner for the workspace's admins.
    pub role: Option<ProjectRole>,
    pub created_by: UserRef,
    pub created_at: Millis,
    pub updated_at: Millis,
    pub archived_at: Option<Millis>,
    /// Sessions currently provisioning, preparing, running, idle or paused.
    pub active_sessions: u32,
    pub total_sessions: u32,
    pub last_activity_at: Option<Millis>,
    /// The model the caller asked for when they last started a session in
    /// it (`CreateSessionRequest::model`), which the web app's composer
    /// starts on. Absent when they took the project's (`settings.model`)
    /// or never started one, and from control planes before it.
    pub last_model: Option<String>,
    /// The effort level they asked for then, as `last_model`.
    pub last_effort: Option<String>,
}

/// A pull request of a session, as its agent last looked it up and as
/// GitHub last told the control plane about it (`fctl_domain::pulls`).
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PullRequestRef {
    pub number: u64,
    pub url: String,
    pub title: Option<String>,
    /// `open`, `draft`, `queued` (in a merge queue), `merged` or `closed`;
    /// absent until someone looked it up. Only the control plane says
    /// `queued`, from GitHub: agents report a queued one as `open`.
    pub state: Option<String>,
    /// The repository it was opened in. A session's pull requests have it;
    /// agents leave it out of their git state, where its URL says it.
    pub repo: Option<RepoRef>,
    /// When it took its state: when it was opened, merged or closed, or
    /// joined a merge queue, as GitHub says, or else when fctl saw it
    /// change. Absent from agents and control planes before it.
    pub state_changed_at: Option<Millis>,
    /// When GitHub last changed it (`updatedAt`), as of the lookup its
    /// title and state come from: an older lookup doesn't undo a newer
    /// one. Absent from agents and control planes before it.
    pub updated_at: Option<Millis>,
    /// The checks of its head commit, as GitHub last told the control plane
    /// about them (its GitHub App's webhook). Absent until then, and where
    /// the webhook is off.
    pub checks: Option<PullRequestChecks>,
}

/// The checks of a pull request's head commit, check runs and commit
/// statuses, counted as GitHub counts them in a pull request's checks.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PullRequestChecks {
    /// Succeeded, or were skipped or neutral.
    pub passed: u32,
    /// Failed, errored, timed out or were cancelled, or wait for someone
    /// to act.
    pub failed: u32,
    /// Queued or running, or expected and not reported yet.
    pub pending: u32,
    /// The commit they are for.
    pub sha: Option<String>,
    /// When the control plane asked GitHub: a later answer replaces an
    /// earlier one.
    pub at: Millis,
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
    /// The person on whose Claude subscription the session ran when its
    /// agent reported this: who started the session, or who resumed it.
    /// Only they can open it in the Claude apps. The control plane adds it;
    /// agents, and control planes before it, leave it out.
    pub user: Option<UserRef>,
}

/// Counters shown on session lists. Tokens count each API call as it comes,
/// and settle on Claude's own totals at the end of each turn.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct SessionStats {
    /// Turns that ended (each with Claude's `result`).
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
    /// When the turn under way started (absent between turns). It isn't in
    /// `turns` or `busy_ms` until it ends.
    pub turn_started_at: Option<Millis>,
}

impl SessionStats {
    /// Turns taken, the one under way included.
    pub fn turns_taken(&self) -> u32 {
        self.turns
            .saturating_add(u32::from(self.turn_started_at.is_some()))
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Session {
    pub id: SessionId,
    pub project_id: ProjectId,
    pub project_slug: String,
    /// The workspace of its project (absent from control planes before
    /// workspaces).
    pub workspace: Option<WorkspaceRef>,
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
    /// The pull request of its branch in the primary repository, or in
    /// another repository when that has none: the last one its agent found.
    pub pull_request: Option<PullRequestRef>,
    /// Every pull request its agent found for the branches it worked on, in
    /// all of the project's repositories, in the order it found them
    /// (`fctl_domain::pulls`). Absent while it has none, and from control
    /// planes before it, which have `pull_request` only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<PullRequestRef>>", optional))]
    pub pull_requests: Vec<PullRequestRef>,
    pub runner: Option<RunnerRef>,
    pub model: Option<String>,
    /// The effort level Claude works at (`low` … `max`), as its agent last
    /// reported it. Absent until Claude starts, when its model takes no
    /// effort level, and from agents and control planes before it.
    pub effort: Option<String>,
    /// What its agent may do on GitHub, within its project's
    /// `github_access` as it is now. Absent: push, as from control planes
    /// before it.
    pub github_access: Option<GithubAccess>,
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

/// A session, as a memory or a retrospective names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionRef {
    pub id: SessionId,
    /// Sequential per project (`#12`).
    pub number: u32,
    pub title: String,
}

/// A note in a project's memory: something an agent, or a person, wants the
/// agents of later sessions to know that the repository doesn't say. Each
/// session's agent starts with the project's current notes
/// (`ProjectSettings::project_memory`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Memory {
    pub id: MemoryId,
    pub project_id: ProjectId,
    /// Markdown; its first line says what it is about.
    pub text: String,
    /// The session whose agent wrote it; absent when a person did.
    pub session: Option<SessionRef>,
    /// The person who wrote it, or who started the session whose agent
    /// did. Absent once their account is gone.
    pub created_by: Option<UserRef>,
    pub created_at: Millis,
    /// When a person last changed its text, and who.
    pub edited_at: Option<Millis>,
    pub edited_by: Option<UserRef>,
    /// Set once it is marked stale: wrong, or out of date. Agents no longer
    /// start with it.
    pub stale: Option<Staleness>,
}

/// When a memory went stale, why, and who said so.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Staleness {
    pub at: Millis,
    pub reason: Option<String>,
    /// The session whose agent marked it; absent when a person did.
    pub session: Option<SessionRef>,
    /// The person who marked it, or who started the session whose agent
    /// did.
    pub by: Option<UserRef>,
    /// The memory that took its place (`fctl memory add --replaces`).
    pub replaced_by: Option<MemoryId>,
    /// The memory it repeats, which was current when it was marked
    /// (`fctl memory duplicate`).
    pub duplicate_of: Option<MemoryId>,
}

/// How a piece of work ended, as the agent that did it sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RetrospectiveOutcome {
    /// What was asked is done: a pull request is open, a question answered.
    Completed,
    /// Part of it is done; the rest is left for later.
    Partial,
    /// It can't go on without a person: a decision, access, an answer.
    Blocked,
}

/// What an agent recorded as it finished a piece of its session's work
/// (`fctl retro`), for the project's people: what it worked on, the
/// approach it took, what got in the way and what a future agent should
/// know (`ProjectSettings::retrospectives`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Retrospective {
    pub id: RetrospectiveId,
    pub project_id: ProjectId,
    pub session: SessionRef,
    /// Who started the session. Absent once their account is gone.
    pub started_by: Option<UserRef>,
    pub created_at: Millis,
    pub outcome: RetrospectiveOutcome,
    /// What it worked on.
    pub worked_on: String,
    /// The approach it took, and why.
    pub approach: Option<String>,
    /// What got in the way.
    pub challenges: Option<String>,
    /// What a future agent should know: what it learned, or wishes it had
    /// known when it started.
    pub insights: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_github_access_this_version_does_not_know_reads_as_the_least()
    -> Result<(), serde_json::Error> {
        let newer: ProjectSettings =
            serde_json::from_str(r#"{"github_access":"triage","auto_pull_request":false}"#)?;
        assert_eq!(newer.github_access, Some(GithubAccess::Read));
        // The other settings stay as they are.
        assert!(!newer.auto_pull_request);
        let known: ProjectSettings = serde_json::from_str(r#"{"github_access":"comment"}"#)?;
        assert_eq!(known.github_access, Some(GithubAccess::Comment));
        let none: ProjectSettings = serde_json::from_str("{}")?;
        assert_eq!(none.github_access, None);
        let null: ProjectSettings = serde_json::from_str(r#"{"github_access":null}"#)?;
        assert_eq!(null.github_access, None);
        assert_eq!(
            serde_json::to_value(&known)?["github_access"],
            serde_json::json!("comment")
        );
        Ok(())
    }

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
        // Projects from before memory and retrospectives keep them.
        assert!(old.project_memory && old.retrospectives);
        let off: ProjectSettings =
            serde_json::from_str(r#"{"project_memory":false,"retrospectives":false}"#)?;
        assert!(!off.project_memory && !off.retrospectives);
        Ok(())
    }

    #[test]
    fn memories_and_retrospectives_on_the_wire() -> Result<(), serde_json::Error> {
        let memory: Memory = serde_json::from_value(serde_json::json!({
            "id": "mem_1", "project_id": "prj_1",
            "text": "The web app's tests need `npm ci` in web/ first.",
            "session": { "id": "ses_1", "number": 12, "title": "Fix the login redirect" },
            "created_by": { "id": "usr_1", "name": "Ada" },
            "created_at": 5
        }))?;
        assert_eq!(memory.stale, None);
        let json = serde_json::to_value(&memory)?;
        assert!(json.get("stale").is_none() && json.get("edited_at").is_none());
        assert_eq!(json["session"]["number"], 12);
        // A duplicate links the note it repeats; other stale notes, and
        // those from before duplicates, don't.
        let duplicate: Staleness = serde_json::from_value(serde_json::json!({
            "at": 6, "reason": "Duplicate of mem_0", "duplicate_of": "mem_0"
        }))?;
        assert_eq!(
            duplicate.duplicate_of.as_ref().map(|m| m.as_str()),
            Some("mem_0")
        );
        let replaced: Staleness =
            serde_json::from_value(serde_json::json!({ "at": 6, "replaced_by": "mem_2" }))?;
        assert_eq!(replaced.duplicate_of, None);
        assert!(
            serde_json::to_value(&replaced)?
                .get("duplicate_of")
                .is_none()
        );
        let retro: Retrospective = serde_json::from_value(serde_json::json!({
            "id": "rtr_1", "project_id": "prj_1",
            "session": { "id": "ses_1", "number": 12, "title": "Fix the login redirect" },
            "created_at": 5, "outcome": "blocked",
            "worked_on": "The login redirect."
        }))?;
        assert_eq!(retro.outcome, RetrospectiveOutcome::Blocked);
        assert_eq!(retro.insights, None);
        Ok(())
    }
}
