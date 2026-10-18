//! REST request and response bodies for `/api/v1`.
//!
//! Conventions: JSON in and out; errors are [`crate::ApiError`] with an HTTP
//! status from [`crate::ErrorCode::http_status`]; times are [`crate::Millis`];
//! list endpoints return objects (`{"sessions": [...]}`) so they can grow.

use crate::event::SessionEvent;
use crate::runner::{ClaudeCredentialKind, DriverInfo, SandboxLimits};
use crate::session::{
    InstanceRole, Project, ProjectRole, ProjectSettings, RepoAccess, Session, SessionRole, UserRef,
};
use crate::{
    AllowId, AttachmentId, CredentialId, DeliveryId, IdentityId, InviteId, LoginJobId, Millis,
    ProjectId, PushDeviceId, RunnerId, SessionId, TokenId, UserId,
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

// ───────────────────────────── instance ─────────────────────────────

/// `GET /api/v1/instance` (public): what the sign-in page needs to know.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InstanceInfo {
    pub name: String,
    pub version: String,
    /// No owner yet: the setup link must be used first.
    pub setup_required: bool,
    pub features: Features,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Features {
    /// "Continue with Google" is configured.
    pub google: bool,
    /// Transactional email is configured (resets and invites are emailed).
    pub email: bool,
    /// A GitHub App is configured.
    pub github: bool,
    /// Admins let projects publish screenshots ([`InstanceSettings`]).
    #[serde(default)]
    pub publish_screenshots: bool,
}

/// `POST /api/v1/setup`: create the owner account with the setup token.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SetupRequest {
    pub token: String,
    pub email: String,
    /// First and last name; or `name`, the full name (older clients).
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub name: Option<String>,
    pub password: String,
}

// ───────────────────────────── users and auth ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    Disabled,
}

impl UserStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct User {
    pub id: UserId,
    pub email: String,
    /// The full name everyone sees.
    pub name: String,
    /// First names, all of them ("Carl Oscar"); what greetings use.
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub avatar_url: Option<String>,
    pub role: InstanceRole,
    pub status: UserStatus,
    /// Whether the account can sign in with a password.
    pub has_password: bool,
    pub created_at: Millis,
    pub last_login_at: Option<Millis>,
}

impl User {
    pub fn to_ref(&self) -> UserRef {
        UserRef {
            id: self.id.clone(),
            name: self.name.clone(),
            avatar_url: self.avatar_url.clone(),
        }
    }
}

/// A linked external account.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Identity {
    pub id: IdentityId,
    /// `google`.
    pub provider: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub created_at: Millis,
}

/// `GET /api/v1/auth/me`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Me {
    pub user: User,
    pub identities: Vec<Identity>,
    pub claude: Vec<ClaudeCredentialInfo>,
    /// The address the commits of sessions you start credit you with, in a
    /// `Co-authored-by` trailer. Only you see it here; absent when you have
    /// none, and then commits don't name you.
    pub commit_email: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SignupRequest {
    pub email: String,
    /// First and last name; or `name`, the full name (older clients).
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub name: Option<String>,
    pub password: String,
    /// The token from an invitation link.
    pub invite: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Returned by setup, signup, login and password reset. The web session is
/// set as a cookie; `token` is only present for non-browser clients
/// (requested with `"token": true`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AuthResponse {
    pub me: Me,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateMeRequest {
    /// First and last name (send both; an empty last name clears it).
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    /// The full name alone, from older clients: the parts become unknown.
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    /// The address commits credit you with (`Me.commit_email`); an empty
    /// one removes it.
    pub commit_email: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ChangePasswordRequest {
    /// Required when the account already has a password.
    pub current_password: Option<String>,
    pub new_password: String,
}

/// `GET /api/v1/me/deletion`: what deleting your account would delete.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AccountDeletion {
    /// Why the account cannot be deleted now; absent when it can.
    pub blocked: Option<String>,
    /// Projects only you are a member of: deleted, with their sessions.
    pub projects: Vec<DeletedProject>,
    /// Sessions you started in projects others are members of: they stay
    /// with those projects, attributed to a deleted user.
    pub kept_sessions: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeletedProject {
    pub slug: String,
    pub name: String,
    pub sessions: u32,
}

/// `POST /api/v1/me/delete`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeleteAccountRequest {
    /// The account's email address, typed to confirm.
    pub email: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    /// A browser session.
    Web,
    /// Issued to the CLI through the device flow.
    Cli,
    /// A personal access token.
    Pat,
}

impl TokenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Cli => "cli",
            Self::Pat => "pat",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "web" => Some(Self::Web),
            "cli" => Some(Self::Cli),
            "pat" => Some(Self::Pat),
            _ => None,
        }
    }
}

/// A web session, CLI token or personal access token (never the secret).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TokenInfo {
    pub id: TokenId,
    pub kind: TokenKind,
    pub name: Option<String>,
    pub created_at: Millis,
    pub last_used_at: Option<Millis>,
    pub expires_at: Option<Millis>,
    pub user_agent: Option<String>,
    /// The token used for this request.
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TokenList {
    pub tokens: Vec<TokenInfo>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateTokenRequest {
    pub name: String,
    pub expires_in_days: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreatedToken {
    pub token: TokenInfo,
    /// Shown once.
    pub secret: String,
}

// ───────────────────────────── device flow (CLI sign-in) ─────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceStartRequest {
    /// Shown to the person approving, e.g. "fctl on carls-macbook".
    pub client_name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceStartResponse {
    pub device_code: String,
    /// Typed or confirmed in the browser, e.g. `KFTW-QXPL`.
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u32,
    /// Seconds between polls.
    pub interval: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DevicePollRequest {
    pub device_code: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DevicePollResponse {
    Pending,
    /// Polling too fast; wait `interval` more seconds.
    SlowDown,
    Denied,
    Expired,
    Approved {
        token: String,
        me: Box<Me>,
    },
}

/// `GET /api/v1/auth/device/{user_code}`: what the approval page shows.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct DeviceInfo {
    pub user_code: String,
    pub client_name: String,
    pub created_at: Millis,
    pub expires_at: Millis,
    pub ip: Option<String>,
    pub location: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceDecisionRequest {
    pub approve: bool,
}

// ───────────────────────────── admin ─────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UserList {
    pub users: Vec<User>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateUserRequest {
    pub role: Option<InstanceRole>,
    pub status: Option<UserStatus>,
    pub name: Option<String>,
}

/// Who may sign up: an exact email (`ada@example.com`) or a domain
/// (`*@example.com`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AllowEntry {
    pub id: AllowId,
    pub pattern: String,
    /// Role given to people who sign up through this entry.
    pub role: InstanceRole,
    pub note: Option<String>,
    pub created_by: Option<UserRef>,
    pub created_at: Millis,
    /// For exact emails: the account, once it exists.
    pub user_id: Option<UserId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AllowList {
    pub entries: Vec<AllowEntry>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateAllowRequest {
    pub pattern: String,
    pub role: Option<InstanceRole>,
    pub note: Option<String>,
    /// Also create an invitation link (exact emails only).
    pub invite: Option<bool>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreatedAllow {
    pub entry: AllowEntry,
    pub invite: Option<Invite>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Invite {
    pub id: InviteId,
    pub email: String,
    /// The link to share (contains the secret; shown once).
    pub url: Option<String>,
    pub role: InstanceRole,
    pub expires_at: Millis,
    /// Whether it was emailed.
    pub emailed: bool,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateInviteRequest {
    pub email: String,
    pub role: Option<InstanceRole>,
}

/// `GET /api/v1/invites/{token}` (public): who an invitation is for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InvitePreview {
    pub email: String,
    pub instance: String,
    pub invited_by: Option<String>,
    pub expires_at: Millis,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AuditEntry {
    pub id: String,
    pub ts: Millis,
    pub actor: Option<UserRef>,
    pub action: String,
    pub target: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub data: Option<serde_json::Value>,
    pub ip: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AuditList {
    pub entries: Vec<AuditEntry>,
    pub next_before: Option<Millis>,
}

/// `GET /api/v1/admin/settings`: what admins decide for the whole instance.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct InstanceSettings {
    /// Projects may let their agents publish screenshots at links anyone
    /// who has one can open (each project turns it on in its settings:
    /// [`ProjectSettings::publish_screenshots`]). Off by default. Turning
    /// it off takes the published ones offline too; they are back if it is
    /// turned on again before they expire.
    pub publish_screenshots: bool,
}

/// `PATCH /api/v1/admin/settings`: absent fields stay as they are.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateInstanceSettingsRequest {
    pub publish_screenshots: Option<bool>,
}

// ───────────────────────────── runners ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RunnerStatus {
    Online,
    Offline,
    Disabled,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Runner {
    pub id: RunnerId,
    pub name: String,
    pub status: RunnerStatus,
    pub version: Option<String>,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub driver: Option<String>,
    pub drivers: Vec<DriverInfo>,
    pub capacity: u32,
    pub active_sessions: u32,
    /// The machine's CPUs and memory (MiB).
    pub cpus: Option<u32>,
    pub memory_mb: Option<u64>,
    /// Free memory and disk (MiB) at its last heartbeat, while it is connected.
    pub memory_available_mb: Option<u64>,
    pub disk_available_mb: Option<u64>,
    /// What it gives each sandbox (absent from older hosts).
    pub sandbox: Option<SandboxLimits>,
    pub labels: Vec<String>,
    pub last_seen_at: Option<Millis>,
    pub created_at: Millis,
    pub created_by: Option<UserRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunnerList {
    pub runners: Vec<Runner>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreateRunnerRequest {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunnerRegistration {
    pub runner: Runner,
    /// `fctl_rt_…`; shown once. Goes into the host's config.
    pub token: String,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateRunnerRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
}

// ───────────────────────────── projects ─────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectList {
    pub projects: Vec<Project>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateRepoRequest {
    /// Repository name in the GitHub App's organization.
    pub name: String,
    pub private: bool,
    pub description: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateProjectRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    /// An existing repository, `owner/name`.
    pub repo: Option<String>,
    /// Or create a new one.
    pub create_repo: Option<CreateRepoRequest>,
    pub default_branch: Option<String>,
    pub settings: Option<ProjectSettings>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    /// `owner/name`, or `""` to unlink.
    pub repo: Option<String>,
    pub default_branch: Option<String>,
    /// Replaces all settings.
    pub settings: Option<ProjectSettings>,
    pub archived: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectMember {
    pub user: UserRef,
    pub email: String,
    pub role: ProjectRole,
    pub added_at: Millis,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemberList {
    pub members: Vec<ProjectMember>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AddMemberRequest {
    pub email: String,
    pub role: ProjectRole,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SetRoleRequest {
    pub role: ProjectRole,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SecretInfo {
    pub name: String,
    pub updated_at: Millis,
    pub updated_by: Option<UserRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SecretList {
    pub secrets: Vec<SecretInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SetSecretRequest {
    pub value: String,
}

// ───────────────────────────── project repositories ─────────────────────────────

/// A repository a project's agents may use. All of a project's repositories
/// belong to one GitHub account, so one installation of the GitHub App mints
/// their tokens.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ProjectRepo {
    pub owner: String,
    /// Absent for an owner-wide grant: every repository of `owner` the
    /// GitHub App can reach.
    pub name: Option<String>,
    pub access: RepoAccess,
    /// The project's primary repository, cloned into each agent's working
    /// directory. Agents clone the others when they need them.
    pub primary: bool,
    pub added_by: Option<UserRef>,
    pub added_at: Option<Millis>,
}

/// `GET /api/v1/projects/{id}/repos`: the primary repository first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectRepoList {
    pub repos: Vec<ProjectRepo>,
}

/// `POST /api/v1/projects/{id}/repos`: adds a repository, or changes its
/// access. `DELETE /api/v1/projects/{id}/repos/{owner}/{name}` (`*` for an
/// owner-wide grant) removes one.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AddProjectRepoRequest {
    /// `owner/name`, or `owner/*` for every repository of `owner` the GitHub
    /// App can reach.
    pub repo: String,
    /// `read` when absent. Owner-wide grants are always read-only.
    pub access: Option<RepoAccess>,
}

// ───────────────────────────── sessions ─────────────────────────────

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SessionList {
    pub sessions: Vec<Session>,
    /// Pass as `?before=` for the next page.
    pub next_before: Option<Millis>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateSessionRequest {
    /// The task. May be empty when `attachments` are given.
    pub prompt: String,
    pub title: Option<String>,
    /// Branch to start from (default: the project's default branch).
    pub base_branch: Option<String>,
    /// Branch to work on (default: `fctl/<number>-<slug of title>`).
    pub branch: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Files to send with it: ids from `POST /api/v1/projects/{id}/attachments`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<AttachmentId>>", optional))]
    pub attachments: Vec<AttachmentId>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
    /// What the session is about; an empty string clears it.
    pub description: Option<String>,
    pub archived: Option<bool>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SendMessageRequest {
    /// May be empty when `attachments` are given.
    pub text: String,
    /// Idempotency key; retries with the same key deliver once.
    pub client_id: Option<String>,
    /// Files to send with it: ids from `POST /api/v1/sessions/{id}/attachments`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<AttachmentId>>", optional))]
    pub attachments: Vec<AttachmentId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SendMessageResponse {
    pub delivery_id: DeliveryId,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ResumeSessionRequest {
    pub prompt: Option<String>,
    /// Files to send with the prompt: ids from `POST /api/v1/sessions/{id}/attachments`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<AttachmentId>>", optional))]
    pub attachments: Vec<AttachmentId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EventPage {
    pub events: Vec<SessionEvent>,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionShare {
    pub user: UserRef,
    pub email: String,
    pub role: SessionRole,
    pub created_at: Millis,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ShareList {
    pub shares: Vec<SessionShare>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreateShareRequest {
    pub email: String,
    pub role: SessionRole,
}

// ───────────────────────────── GitHub ─────────────────────────────

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct GithubInfo {
    pub configured: bool,
    pub app_slug: Option<String>,
    /// Where to install the app on more accounts.
    pub install_url: Option<String>,
    pub accounts: Vec<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct GithubRepo {
    pub owner: String,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub default_branch: String,
    pub description: Option<String>,
    pub pushed_at: Option<String>,
    /// The project already linked to it.
    pub project_id: Option<ProjectId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GithubRepoList {
    pub repos: Vec<GithubRepo>,
}

// ───────────────────────────── Claude credentials ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CredentialStatus {
    Active,
    /// Expired or revoked; connect again.
    Invalid,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ClaudeCredentialInfo {
    pub id: CredentialId,
    pub kind: ClaudeCredentialKind,
    pub label: Option<String>,
    pub account_email: Option<String>,
    pub subscription_type: Option<String>,
    /// Whether sessions on this credential can enable Remote Control.
    pub remote_control: bool,
    pub status: CredentialStatus,
    /// Used for new sessions.
    pub is_default: bool,
    pub created_at: Millis,
    pub last_used_at: Option<Millis>,
    pub expires_at: Option<Millis>,
    /// How much of the plan's usage limits is used, as the sessions on this
    /// credential last reported it: one entry per limit, the five-hour
    /// window first. Empty until a session reports one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<PlanLimit>>", optional))]
    pub usage: Vec<PlanLimit>,
}

/// Where a Claude plan limit stands, as Claude reported it for the limit
/// that is closest to stopping requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LimitStatus {
    Allowed,
    /// Close to the limit.
    AllowedWarning,
    /// The limit is reached: sessions pause until it resets.
    Rejected,
}

impl LimitStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::AllowedWarning => "allowed_warning",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "allowed" => Some(Self::Allowed),
            "allowed_warning" => Some(Self::AllowedWarning),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// One of a Claude plan's usage limits, from the `rate_limit_event`s that
/// sessions report.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PlanLimit {
    /// The limit's window, as Claude names it: `five_hour`, `seven_day`
    /// (the week, every model), or a model's own weekly limit
    /// (`seven_day_opus`, `seven_day_sonnet`, …).
    pub window: String,
    /// The share of the limit used, from 0 to 1. Usage can run a little past
    /// the limit, so it can be more.
    pub utilization: Option<f64>,
    /// When the window starts over.
    pub resets_at: Option<Millis>,
    /// Present when Claude reported this limit as the one closest to
    /// stopping requests.
    pub status: Option<LimitStatus>,
    /// When a session reported it.
    pub observed_at: Millis,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AddClaudeTokenRequest {
    /// The output of `claude setup-token` (`sk-ant-oat01-…`).
    pub token: String,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClaudeLoginState {
    /// Waiting for a runner to pick it up.
    Pending,
    Starting,
    /// Open `url`, approve, then submit the code claude.ai shows.
    AwaitingCode {
        url: String,
    },
    Verifying,
    Succeeded {
        credential_id: CredentialId,
    },
    Failed {
        error: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ClaudeLoginJob {
    pub id: LoginJobId,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub state: ClaudeLoginState,
    pub created_at: Millis,
    pub updated_at: Millis,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ClaudeLoginCodeRequest {
    pub code: String,
}

// ───────────────────────────── notifications ─────────────────────────────

/// What notifies a person, in every browser where they turned notifications
/// on. A field that is missing has its default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct NotificationSettings {
    /// A session they follow finished its turn and waits for a reply.
    pub waiting: bool,
    /// A session they follow paused on a Claude usage limit.
    pub paused: bool,
    /// A session they follow failed.
    pub failed: bool,
    /// Starting or resuming a session follows it.
    pub follow_started: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            waiting: true,
            paused: true,
            failed: true,
            follow_started: true,
        }
    }
}

/// A browser that shows your notifications: its Web Push subscription.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PushDevice {
    pub id: PushDeviceId,
    /// What the browser was called when it subscribed ("Chrome on macOS").
    pub name: Option<String>,
    /// The push service that delivers to it, e.g. `fcm.googleapis.com`.
    pub service: String,
    pub created_at: Millis,
    /// When a notification last went to it.
    pub last_sent_at: Option<Millis>,
    /// It subscribed with the sign-in this request uses: this browser.
    pub current: bool,
}

/// `GET /api/v1/me/notifications`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Notifications {
    pub settings: NotificationSettings,
    pub devices: Vec<PushDevice>,
    /// The instance's VAPID public key (base64url), for
    /// `PushManager.subscribe`. Absent when it cannot send notifications.
    pub public_key: Option<String>,
}

/// `PATCH /api/v1/me/notifications`: the settings to change.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateNotificationsRequest {
    pub waiting: Option<bool>,
    pub paused: Option<bool>,
    pub failed: Option<bool>,
    pub follow_started: Option<bool>,
}

/// A browser's keys for encrypting notifications to it, from
/// `PushSubscription.toJSON()`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PushKeys {
    /// Its P-256 public key, base64url.
    pub p256dh: String,
    /// Its authentication secret, base64url.
    pub auth: String,
}

/// `POST /api/v1/me/notifications/devices`: notifications for a browser.
/// Sending the same endpoint again updates it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AddPushDeviceRequest {
    /// `PushSubscription.endpoint`.
    pub endpoint: String,
    pub keys: PushKeys,
    /// What to call it, e.g. "Chrome on macOS".
    pub name: Option<String>,
}

/// `POST /api/v1/me/notifications/test`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PushTestResult {
    /// Browsers whose push service took the notification.
    pub sent: u32,
    /// Browsers it could not reach. Expired ones are removed.
    pub failed: u32,
}

/// A notification as the control plane sends it to a browser (encrypted,
/// RFC 8291), for its service worker (`web/public/sw.js`) to show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PushMessage {
    pub title: String,
    pub body: String,
    /// Replaces a notification with the same tag that is still shown.
    pub tag: String,
    /// The page a click opens, a path in the app.
    pub url: String,
    /// Tags of the notifications this one sums up: showing it closes them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub replaces: Vec<String>,
    /// When it happened.
    pub at: Millis,
}

// ───────────────────────────── feed (dashboard live updates) ─────────────────────────────

/// `GET /api/v1/feed/ws`: live updates for the dashboard.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FeedServer {
    Hello {
        runners: Vec<Runner>,
        server_time: Millis,
    },
    /// A session the person can see changed.
    Session {
        session: Box<Session>,
    },
    /// A session was deleted or the person lost access.
    SessionRemoved {
        session_id: SessionId,
    },
    Runner {
        runner: Box<Runner>,
    },
    /// Agent activity for the visualization: one pulse per event, by session.
    Activity {
        session_id: SessionId,
        project_id: ProjectId,
        /// Event kind or Claude message type (`assistant`, `tool_use`, …).
        kind: String,
        /// Rough size, for the visual weight of the pulse.
        weight: u32,
    },
    ClaudeLogin {
        job: Box<ClaudeLoginJob>,
    },
    /// A session reported new plan usage for one of the person's Claude
    /// credentials: all its limits, as `ClaudeCredentialInfo.usage`.
    ClaudeUsage {
        credential_id: CredentialId,
        usage: Vec<PlanLimit>,
    },
    Pong,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FeedClient {
    Ping,
}

/// `{}` responses.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Empty {}

/// `{"ok": true}` responses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Ok {
    pub ok: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_settings_default_to_everything() -> Result<(), serde_json::Error> {
        let stored: NotificationSettings = serde_json::from_str(r#"{"waiting":false}"#)?;
        assert_eq!(
            stored,
            NotificationSettings {
                waiting: false,
                ..Default::default()
            }
        );
        assert!(stored.paused && stored.failed && stored.follow_started);
        Ok(())
    }

    #[test]
    fn push_messages_leave_out_what_they_do_not_replace() -> Result<(), serde_json::Error> {
        let m = PushMessage {
            title: "Fix the login test".into(),
            body: "my-app#12 waits for your reply".into(),
            tag: "ses_1".into(),
            url: "/s/ses_1".into(),
            replaces: vec![],
            at: 1,
        };
        let json = serde_json::to_string(&m)?;
        assert!(!json.contains("replaces"));
        assert_eq!(serde_json::from_str::<PushMessage>(&json)?, m);
        Ok(())
    }
}
