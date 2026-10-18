//! REST request and response bodies for `/api/v1`.
//!
//! Conventions: JSON in and out; errors are [`crate::ApiError`] with an HTTP
//! status from [`crate::ErrorCode::http_status`]; times are [`crate::Millis`];
//! list endpoints return objects (`{"sessions": [...]}`) so they can grow.

use crate::event::SessionEvent;
use crate::runner::{ClaudeCredentialKind, DriverInfo, SandboxLimits};
use crate::session::{
    InstanceRole, Project, ProjectRole, ProjectSettings, Session, SessionRole, UserRef,
};
use crate::{
    AllowId, AttachmentId, CredentialId, DeliveryId, IdentityId, InviteId, LoginJobId, Millis,
    ProjectId, RunnerId, SessionId, TokenId, UserId,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Me {
    pub user: User,
    pub identities: Vec<Identity>,
    pub claude: Vec<ClaudeCredentialInfo>,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateMeRequest {
    /// First and last name (send both; an empty last name clears it).
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    /// The full name alone, from older clients: the parts become unknown.
    pub name: Option<String>,
    pub avatar_url: Option<String>,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
