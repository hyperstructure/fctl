//! Database rows and their conversion to wire types.

use crate::db::{int, int_bool, opt_int};
use crate::internal::LiveRunners;
use fctl_proto::api::{
    AllowEntry, ClaudeCredentialInfo, CredentialStatus, Identity, PlanLimit, ProjectRepo, Runner,
    RunnerStatus, User, UserStatus,
};
use fctl_proto::runner::{ClaudeAccount, ClaudeCredentialKind, HostHello};
use fctl_proto::session::{
    InstanceRole, Project, ProjectRole, ProjectSettings, PullRequestRef, RemoteControlState,
    RepoAccess, RepoGrant, RepoRef, RunnerRef, Session, SessionRole, SessionStats, SessionStatus,
    UserRef,
};
use fctl_proto::{AllowId, CredentialId, IdentityId, ProjectId, RunnerId, SessionId, UserId};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct UserRow {
    pub id: String,
    pub email: String,
    pub name: String,
    #[serde(default)]
    pub given_name: Option<String>,
    #[serde(default)]
    pub family_name: Option<String>,
    pub avatar_url: Option<String>,
    /// Commits of the sessions they start credit them with it. Only for them
    /// (`Me`), not in [`User`], which admins see.
    #[serde(default)]
    pub commit_email: Option<String>,
    pub password_hash: Option<String>,
    pub role: String,
    pub status: String,
    pub default_credential_id: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "opt_int", default)]
    pub last_login_at: Option<i64>,
}

impl UserRow {
    pub fn role(&self) -> InstanceRole {
        InstanceRole::parse(&self.role).unwrap_or(InstanceRole::Member)
    }

    pub fn is_admin(&self) -> bool {
        self.role().is_admin()
    }

    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    pub fn user_id(&self) -> UserId {
        UserId(self.id.clone())
    }

    pub fn to_user(&self) -> User {
        User {
            id: self.user_id(),
            email: self.email.clone(),
            name: self.name.clone(),
            given_name: self.given_name.clone(),
            family_name: self.family_name.clone(),
            avatar_url: self.avatar_url.clone(),
            role: self.role(),
            status: UserStatus::parse(&self.status).unwrap_or(UserStatus::Disabled),
            has_password: self.password_hash.is_some(),
            created_at: self.created_at,
            last_login_at: self.last_login_at,
        }
    }

    pub fn to_ref(&self) -> UserRef {
        UserRef {
            id: self.user_id(),
            name: self.name.clone(),
            avatar_url: self.avatar_url.clone(),
        }
    }
}

/// The SELECT list for [`UserRow`], prefixed with a table alias.
pub fn user_columns(alias: &str) -> String {
    [
        "id",
        "email",
        "name",
        "given_name",
        "family_name",
        "avatar_url",
        "commit_email",
        "password_hash",
        "role",
        "status",
        "default_credential_id",
        "created_at",
        "last_login_at",
    ]
    .iter()
    .map(|c| format!("{alias}.{c} AS {c}"))
    .collect::<Vec<_>>()
    .join(", ")
}

#[derive(Clone, Debug, Deserialize)]
pub struct IdentityRow {
    pub id: String,
    pub provider: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
}

impl IdentityRow {
    pub fn to_identity(&self) -> Identity {
        Identity {
            id: IdentityId(self.id.clone()),
            provider: self.provider.clone(),
            email: self.email.clone(),
            name: self.name.clone(),
            avatar_url: self.avatar_url.clone(),
            created_at: self.created_at,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct AllowRow {
    pub id: String,
    pub pattern: String,
    pub role: String,
    pub note: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    pub created_by_id: Option<String>,
    pub created_by_name: Option<String>,
    pub user_id: Option<String>,
}

impl AllowRow {
    pub fn to_entry(&self) -> AllowEntry {
        AllowEntry {
            id: AllowId(self.id.clone()),
            pattern: self.pattern.clone(),
            role: InstanceRole::parse(&self.role).unwrap_or(InstanceRole::Member),
            note: self.note.clone(),
            created_by: match (&self.created_by_id, &self.created_by_name) {
                (Some(id), Some(name)) => Some(UserRef {
                    id: UserId(id.clone()),
                    name: name.clone(),
                    avatar_url: None,
                }),
                _ => None,
            },
            created_at: self.created_at,
            user_id: self.user_id.clone().map(UserId),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProjectRow {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub default_branch: String,
    pub settings: String,
    pub created_by: String,
    pub creator_name: Option<String>,
    pub creator_avatar: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "int")]
    pub updated_at: i64,
    #[serde(deserialize_with = "opt_int", default)]
    pub archived_at: Option<i64>,
    /// The caller's membership role (from a LEFT JOIN).
    pub member_role: Option<String>,
    #[serde(deserialize_with = "int", default)]
    pub active_sessions: i64,
    #[serde(deserialize_with = "int", default)]
    pub total_sessions: i64,
    #[serde(deserialize_with = "opt_int", default)]
    pub last_activity_at: Option<i64>,
}

impl ProjectRow {
    pub fn settings(&self) -> ProjectSettings {
        serde_json::from_str(&self.settings).unwrap_or_default()
    }

    pub fn repo(&self) -> Option<RepoRef> {
        match (&self.repo_owner, &self.repo_name) {
            (Some(owner), Some(name)) => Some(RepoRef {
                owner: owner.clone(),
                name: name.clone(),
            }),
            _ => None,
        }
    }

    pub fn member_role(&self) -> Option<ProjectRole> {
        self.member_role.as_deref().and_then(ProjectRole::parse)
    }

    pub fn to_project(&self, effective_role: Option<ProjectRole>) -> Project {
        Project {
            id: ProjectId(self.id.clone()),
            slug: self.slug.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            repo: self.repo(),
            default_branch: self.default_branch.clone(),
            settings: self.settings(),
            role: effective_role,
            created_by: UserRef {
                id: UserId(self.created_by.clone()),
                name: self.creator_name.clone().unwrap_or_default(),
                avatar_url: self.creator_avatar.clone(),
            },
            created_at: self.created_at,
            updated_at: self.updated_at,
            archived_at: self.archived_at,
            active_sessions: self.active_sessions.max(0) as u32,
            total_sessions: self.total_sessions.max(0) as u32,
            last_activity_at: self.last_activity_at,
        }
    }
}

/// A repository a project's agents may use besides its primary one
/// (`project_repos`; `name` is `*` for an owner-wide grant).
#[derive(Clone, Debug, Deserialize)]
pub struct RepoRow {
    pub owner: String,
    pub name: String,
    pub access: String,
    pub added_by: Option<String>,
    pub adder_name: Option<String>,
    pub adder_avatar: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
}

/// SELECT for [`RepoRow`]. Bind: `?1` = project id.
pub const REPO_SELECT: &str = "SELECT r.owner, r.name, r.access, r.added_by, u.name AS adder_name, \
    u.avatar_url AS adder_avatar, r.created_at FROM project_repos r LEFT JOIN users u ON u.id = r.added_by \
    WHERE r.project_id = ?1 ORDER BY r.owner, r.name = '*' DESC, r.name";

/// `name` of an owner-wide grant in `project_repos`.
pub const ALL_REPOS: &str = "*";

impl RepoRow {
    pub fn grant(&self) -> RepoGrant {
        RepoGrant {
            owner: self.owner.clone(),
            name: (self.name != ALL_REPOS).then(|| self.name.clone()),
            access: RepoAccess::parse(&self.access).unwrap_or_default(),
        }
    }

    pub fn to_repo(&self) -> ProjectRepo {
        let grant = self.grant();
        ProjectRepo {
            owner: grant.owner,
            name: grant.name,
            access: grant.access,
            primary: false,
            added_by: match (&self.added_by, &self.adder_name) {
                (Some(id), Some(name)) => Some(UserRef {
                    id: UserId(id.clone()),
                    name: name.clone(),
                    avatar_url: self.adder_avatar.clone(),
                }),
                _ => None,
            },
            added_at: Some(self.created_at),
        }
    }
}

/// SELECT for projects with the caller's role and session counts.
/// Bind: `?1` = caller user id.
pub const PROJECT_SELECT: &str = "SELECT p.id, p.slug, p.name, p.description, p.repo_owner, p.repo_name, \
    p.default_branch, p.settings, p.session_counter, p.created_by, cu.name AS creator_name, \
    cu.avatar_url AS creator_avatar, p.created_at, p.updated_at, p.archived_at, m.role AS member_role, \
    (SELECT COUNT(*) FROM sessions s WHERE s.project_id = p.id AND s.status IN ('provisioning','preparing','running','idle','paused','stopping')) AS active_sessions, \
    (SELECT COUNT(*) FROM sessions s WHERE s.project_id = p.id) AS total_sessions, \
    (SELECT MAX(COALESCE(s.last_activity_at, s.created_at)) FROM sessions s WHERE s.project_id = p.id) AS last_activity_at \
    FROM projects p \
    LEFT JOIN users cu ON cu.id = p.created_by \
    LEFT JOIN project_members m ON m.project_id = p.id AND m.user_id = ?1";

#[derive(Clone, Debug, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub project_id: String,
    pub project_slug: String,
    #[serde(deserialize_with = "int")]
    pub number: i64,
    pub title: String,
    pub description: Option<String>,
    pub prompt: String,
    /// The task's attachments, JSON.
    #[serde(default)]
    pub prompt_attachments: Option<String>,
    pub status: String,
    pub status_reason: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub resume_at: Option<i64>,
    pub created_by: String,
    pub creator_name: Option<String>,
    pub creator_avatar: Option<String>,
    pub credential_id: Option<String>,
    pub runner_id: Option<String>,
    pub runner_name: Option<String>,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub claude_session_id: String,
    #[serde(deserialize_with = "opt_int", default)]
    pub pr_number: Option<i64>,
    pub pr_url: Option<String>,
    pub pr_title: Option<String>,
    pub pr_state: Option<String>,
    pub remote_control: Option<String>,
    pub stats: String,
    #[serde(deserialize_with = "int_bool")]
    pub archived: bool,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "int")]
    pub updated_at: i64,
    #[serde(deserialize_with = "opt_int", default)]
    pub started_at: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    pub ended_at: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    pub last_activity_at: Option<i64>,
    /// The caller's project membership role and share role (LEFT JOINs).
    pub member_role: Option<String>,
    pub share_role: Option<String>,
    /// Whether the caller follows it (a LEFT JOIN).
    #[serde(deserialize_with = "int_bool", default)]
    pub following: bool,
}

impl SessionRow {
    pub fn status(&self) -> SessionStatus {
        SessionStatus::parse(&self.status).unwrap_or(SessionStatus::Failed)
    }

    pub fn session_id(&self) -> SessionId {
        SessionId(self.id.clone())
    }

    pub fn creator_ref(&self) -> UserRef {
        UserRef {
            id: UserId(self.created_by.clone()),
            name: self.creator_name.clone().unwrap_or_default(),
            avatar_url: self.creator_avatar.clone(),
        }
    }

    pub fn to_session(&self, role: Option<SessionRole>) -> Session {
        Session {
            id: self.session_id(),
            project_id: ProjectId(self.project_id.clone()),
            project_slug: self.project_slug.clone(),
            number: self.number.max(0) as u32,
            title: self.title.clone(),
            description: self.description.clone(),
            status: self.status(),
            status_reason: self.status_reason.clone(),
            resume_at: self.resume_at,
            created_by: self.creator_ref(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            started_at: self.started_at,
            ended_at: self.ended_at,
            last_activity_at: self.last_activity_at,
            branch: self.branch.clone(),
            base_branch: self.base_branch.clone(),
            pull_request: match (self.pr_number, &self.pr_url) {
                (Some(number), Some(url)) => Some(PullRequestRef {
                    number: number.max(0) as u64,
                    url: url.clone(),
                    title: self.pr_title.clone(),
                    state: self.pr_state.clone(),
                }),
                _ => None,
            },
            runner: match (&self.runner_id, &self.runner_name) {
                (Some(id), Some(name)) => Some(RunnerRef {
                    id: RunnerId(id.clone()),
                    name: name.clone(),
                }),
                _ => None,
            },
            model: self.model.clone(),
            remote_control: self
                .remote_control
                .as_deref()
                .and_then(|s| serde_json::from_str::<RemoteControlState>(s).ok()),
            stats: serde_json::from_str::<SessionStats>(&self.stats).unwrap_or_default(),
            role,
            archived: self.archived,
            following: Some(self.following),
        }
    }
}

/// SELECT for sessions with the caller's project and share roles, and
/// whether they follow it. Bind: `?1` = caller user id.
pub const SESSION_SELECT: &str = "SELECT s.id, s.project_id, p.slug AS project_slug, s.number, s.title, s.description, s.prompt, s.prompt_attachments, \
    s.status, s.status_reason, s.resume_at, s.created_by, cu.name AS creator_name, cu.avatar_url AS creator_avatar, \
    s.credential_id, s.runner_id, r.name AS runner_name, s.branch, s.base_branch, s.model, s.effort, \
    s.claude_session_id, s.pr_number, s.pr_url, s.pr_title, s.pr_state, s.remote_control, s.stats, s.archived, \
    s.created_at, s.updated_at, s.started_at, s.ended_at, s.last_activity_at, \
    m.role AS member_role, sh.role AS share_role, f.user_id IS NOT NULL AS following \
    FROM sessions s \
    JOIN projects p ON p.id = s.project_id \
    LEFT JOIN users cu ON cu.id = s.created_by \
    LEFT JOIN runners r ON r.id = s.runner_id \
    LEFT JOIN project_members m ON m.project_id = s.project_id AND m.user_id = ?1 \
    LEFT JOIN session_shares sh ON sh.session_id = s.id AND sh.user_id = ?1 \
    LEFT JOIN session_followers f ON f.session_id = s.id AND f.user_id = ?1";

#[derive(Clone, Debug, Deserialize)]
pub struct RunnerRow {
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "int_bool")]
    pub enabled: bool,
    pub info: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub last_seen_at: Option<i64>,
    pub created_by: Option<String>,
    pub creator_name: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "int", default)]
    pub active_sessions: i64,
}

impl RunnerRow {
    pub fn hello(&self) -> Option<HostHello> {
        self.info
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
    }

    pub fn to_runner(&self, live: &LiveRunners) -> Runner {
        let hello = self.hello();
        let online = live.online.contains(&self.id);
        let free = live.free.get(&self.id).copied().unwrap_or_default();
        Runner {
            id: RunnerId(self.id.clone()),
            name: self.name.clone(),
            status: if !self.enabled {
                RunnerStatus::Disabled
            } else if online {
                RunnerStatus::Online
            } else {
                RunnerStatus::Offline
            },
            version: hello.as_ref().map(|h| h.version.clone()),
            hostname: hello.as_ref().map(|h| h.hostname.clone()),
            os: hello.as_ref().map(|h| h.os.clone()),
            arch: hello.as_ref().map(|h| h.arch.clone()),
            driver: hello.as_ref().map(|h| h.driver.clone()),
            drivers: hello
                .as_ref()
                .map(|h| h.drivers.clone())
                .unwrap_or_default(),
            capacity: hello.as_ref().map_or(0, |h| h.capacity),
            active_sessions: self.active_sessions.max(0) as u32,
            cpus: hello.as_ref().map(|h| h.cpus),
            memory_mb: hello.as_ref().map(|h| h.memory_mb),
            memory_available_mb: free.memory_mb,
            disk_available_mb: free.disk_mb,
            min_free_disk_mb: hello.as_ref().and_then(|h| h.min_free_disk_mb),
            sandbox: hello.as_ref().and_then(|h| h.sandbox),
            image: hello.as_ref().and_then(|h| h.image.clone()),
            allowed_images: hello.as_ref().and_then(|h| h.allowed_images.clone()),
            allowed_builds: hello.as_ref().and_then(|h| h.allowed_builds.clone()),
            labels: hello.as_ref().map(|h| h.labels.clone()).unwrap_or_default(),
            last_seen_at: self.last_seen_at,
            drain: live.drains.get(&self.id).cloned(),
            created_at: self.created_at,
            created_by: match (&self.created_by, &self.creator_name) {
                (Some(id), Some(name)) => Some(UserRef {
                    id: UserId(id.clone()),
                    name: name.clone(),
                    avatar_url: None,
                }),
                _ => None,
            },
        }
    }
}

pub const RUNNER_SELECT: &str = "SELECT r.id, r.name, r.enabled, r.info, r.last_seen_at, r.created_by, \
    u.name AS creator_name, r.created_at, \
    (SELECT COUNT(*) FROM sessions s WHERE s.runner_id = r.id AND s.status IN ('provisioning','preparing','running','idle','paused','stopping')) AS active_sessions \
    FROM runners r LEFT JOIN users u ON u.id = r.created_by";

#[derive(Clone, Debug, Deserialize)]
pub struct CredentialRow {
    pub id: String,
    pub user_id: String,
    pub kind: String,
    pub label: Option<String>,
    pub secret_enc: String,
    pub scopes: String,
    pub account_email: Option<String>,
    pub subscription_type: Option<String>,
    pub account: Option<String>,
    pub status: String,
    #[serde(deserialize_with = "opt_int", default)]
    pub expires_at: Option<i64>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "opt_int", default)]
    pub last_used_at: Option<i64>,
    /// Its plan usage, from [`USAGE_JSON`] when the query selects it.
    #[serde(default)]
    pub usage: Option<String>,
}

/// A credential's plan usage as one JSON array of its `claude_usage` rows,
/// for queries that name the credential `c`:
/// `SELECT c.*, {USAGE_JSON} AS usage FROM claude_credentials c …`.
pub const USAGE_JSON: &str = "(SELECT json_group_array(json_object('window', u.window, \
    'utilization', u.utilization, 'resets_at', u.resets_at, 'status', u.status, \
    'observed_at', u.observed_at)) FROM claude_usage u WHERE u.credential_id = c.id)";

/// Plan usage selected with [`USAGE_JSON`], in display order.
pub fn plan_usage(json: Option<&str>) -> Vec<PlanLimit> {
    json.map(fctl_domain::usage::from_json).unwrap_or_default()
}

impl CredentialRow {
    pub fn kind(&self) -> ClaudeCredentialKind {
        ClaudeCredentialKind::parse(&self.kind).unwrap_or(ClaudeCredentialKind::SetupToken)
    }

    pub fn scopes(&self) -> Vec<String> {
        self.scopes.split_whitespace().map(str::to_owned).collect()
    }

    pub fn account(&self) -> Option<ClaudeAccount> {
        self.account
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
    }

    /// Remote Control needs a full-scope login (with `user:profile`).
    pub fn supports_remote_control(&self) -> bool {
        self.kind() == ClaudeCredentialKind::Oauth
            && self.scopes().iter().any(|s| s == "user:profile")
    }

    /// The AAD binding a credential's ciphertext to its record.
    pub fn aad(id: &str, user_id: &str) -> String {
        format!("claude:{id}:{user_id}")
    }

    pub fn to_info(&self, default_id: Option<&str>) -> ClaudeCredentialInfo {
        ClaudeCredentialInfo {
            id: CredentialId(self.id.clone()),
            kind: self.kind(),
            label: self.label.clone(),
            account_email: self.account_email.clone(),
            subscription_type: self.subscription_type.clone(),
            remote_control: self.supports_remote_control(),
            status: if self.status == "active" {
                CredentialStatus::Active
            } else {
                CredentialStatus::Invalid
            },
            is_default: default_id == Some(self.id.as_str()),
            created_at: self.created_at,
            last_used_at: self.last_used_at,
            expires_at: self.expires_at,
            usage: plan_usage(self.usage.as_deref()),
        }
    }
}

/// The secret part of a stored Claude credential.
#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub struct CredentialSecret {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}
