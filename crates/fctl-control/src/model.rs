//! Database rows and their conversion to wire types.

use crate::db::{int, int_bool, opt_int};
use crate::internal::LiveRunners;
use fctl_proto::api::{
    AllowEntry, ClaudeCredentialInfo, CredentialStatus, Identity, PlanLimit, ProjectRepo, Runner,
    RunnerStatus, User, UserStatus,
};
use fctl_proto::runner::{ClaudeAccount, ClaudeCredentialKind, HostHello};
use fctl_proto::session::{
    GithubAccess, InstanceRole, Project, ProjectRole, ProjectSettings, PullRequestRef,
    RemoteControlState, RepoAccess, RepoGrant, RepoRef, RunnerRef, Session, SessionReview,
    SessionRole, SessionSchedule, SessionStats, SessionStatus, UserRef, WorkspaceColor,
    WorkspaceRef, WorkspaceRole,
};
use fctl_proto::{
    AllowId, CredentialId, IdentityId, ProjectId, RunnerId, ScheduleId, SessionId, UserId,
    WorkspaceId,
};
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
    /// What they asked to be called, if they chose a name for it.
    #[serde(default)]
    pub preferred_name: Option<String>,
    pub avatar_url: Option<String>,
    pub password_hash: Option<String>,
    pub role: String,
    pub status: String,
    pub default_credential_id: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "opt_int", default)]
    pub last_login_at: Option<i64>,
    /// When they last signed in or started a session themselves.
    #[serde(deserialize_with = "opt_int", default)]
    pub last_active_at: Option<i64>,
    /// Google sign-ins ask them for the second factor too (their choice).
    #[serde(deserialize_with = "int_bool", default)]
    pub two_factor_google: bool,
    /// When their authenticator app was set up, if it was.
    #[serde(deserialize_with = "opt_int", default)]
    pub totp_at: Option<i64>,
    /// How many passkeys they have.
    #[serde(deserialize_with = "int", default)]
    pub passkeys: i64,
    /// How many of their workspaces require two-factor sign-in.
    #[serde(deserialize_with = "int", default)]
    pub two_factor_workspaces: i64,
    /// Their other email addresses (JSON array).
    #[serde(default)]
    pub other_emails: Option<String>,
    /// How many workspaces they are a member of.
    #[serde(deserialize_with = "int", default)]
    pub workspace_count: i64,
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

    pub fn has_password(&self) -> bool {
        self.password_hash.is_some()
    }

    /// Whether they have an authenticator app: two-factor sign-in is on.
    pub fn has_second_factor(&self) -> bool {
        self.totp_at.is_some()
    }

    /// What they have beyond a password or Google.
    pub fn factors(&self) -> fctl_domain::two_factor::Factors {
        fctl_domain::two_factor::Factors {
            authenticator: self.totp_at.is_some(),
            passkeys: self.passkeys > 0,
        }
    }

    /// The instance's settings as two-factor sign-in applies them to this
    /// account: as if the instance required it, when one of their
    /// workspaces does.
    pub fn two_factor_settings(
        &self,
        settings: &fctl_proto::api::InstanceSettings,
    ) -> fctl_proto::api::InstanceSettings {
        fctl_domain::two_factor::with_workspaces(settings, self.two_factor_workspaces > 0)
    }

    /// Whether a workspace that requires two-factor sign-in is closed to
    /// them (they haven't set it up). `require_two_factor` is its admins'
    /// choice: the instance's requirement closes their whole account
    /// instead (`fctl_domain::two_factor::workspace_requires`).
    pub fn blocked_by(
        &self,
        settings: &fctl_proto::api::InstanceSettings,
        require_two_factor: bool,
    ) -> bool {
        require_two_factor
            && fctl_domain::two_factor::workspace_blocked(
                settings,
                self.has_password(),
                self.factors(),
            )
    }

    /// Their other email addresses.
    pub fn other_emails(&self) -> Vec<String> {
        self.other_emails
            .as_deref()
            .and_then(|j| serde_json::from_str::<Vec<Option<String>>>(j).ok())
            .map(|v| v.into_iter().flatten().collect())
            .unwrap_or_default()
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
            preferred_name: self.preferred_name.clone(),
            avatar_url: self.avatar_url.clone(),
            role: self.role(),
            status: UserStatus::parse(&self.status).unwrap_or(UserStatus::Disabled),
            has_password: self.has_password(),
            two_factor: self.has_second_factor(),
            passkeys: self.passkeys.max(0) as u32,
            created_at: self.created_at,
            last_login_at: self.last_login_at,
            last_active_at: self.last_active_at,
            emails: self.other_emails(),
            workspaces: self.workspace_count.max(0) as u32,
        }
    }

    pub fn to_ref(&self) -> UserRef {
        UserRef {
            id: self.user_id(),
            name: self.name.clone(),
            avatar_url: self.avatar_url.clone(),
        }
    }

    /// What agents call them (`fctl_domain::validate::called`); nothing,
    /// once their account is deleted.
    pub fn called(&self) -> Option<String> {
        if fctl_domain::account::is_tombstone(&self.email) {
            return None;
        }
        fctl_domain::validate::called(
            self.preferred_name.as_deref(),
            self.given_name.as_deref(),
            &self.name,
        )
    }

    /// What fctl calls them where it writes to them by name: what agents
    /// call them, or else their full name.
    pub fn addressed(&self) -> String {
        self.called().unwrap_or_else(|| self.name.clone())
    }
}

/// The SELECT list for [`UserRow`], prefixed with a table alias.
pub fn user_columns(alias: &str) -> String {
    let mut columns: Vec<String> = [
        "id",
        "email",
        "name",
        "given_name",
        "family_name",
        "preferred_name",
        "avatar_url",
        "password_hash",
        "role",
        "status",
        "default_credential_id",
        "created_at",
        "last_login_at",
        "last_active_at",
        "two_factor_google",
    ]
    .iter()
    .map(|c| format!("{alias}.{c} AS {c}"))
    .collect();
    columns.push(format!(
        "(SELECT a.created_at FROM authenticator_apps a WHERE a.user_id = {alias}.id) AS totp_at"
    ));
    columns.push(format!(
        "(SELECT COUNT(*) FROM passkeys p WHERE p.user_id = {alias}.id) AS passkeys"
    ));
    columns.push(format!(
        "(SELECT COUNT(*) FROM workspace_members wm JOIN workspaces w ON w.id = wm.workspace_id \
         WHERE wm.user_id = {alias}.id AND w.require_two_factor = 1) AS two_factor_workspaces"
    ));
    columns.push(format!(
        "(SELECT json_group_array(e.email) FROM user_emails e WHERE e.user_id = {alias}.id \
         AND e.email != {alias}.email) AS other_emails"
    ));
    columns.push(format!(
        "(SELECT COUNT(*) FROM workspace_members wm WHERE wm.user_id = {alias}.id) AS workspace_count"
    ));
    columns.join(", ")
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

/// The workspace of a project or session row, and the caller's membership.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct WorkspaceCols {
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub workspace_slug: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub workspace_color: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub workspace_avatar_at: Option<i64>,
    #[serde(default)]
    pub workspace_base_role: Option<String>,
    #[serde(deserialize_with = "int_bool", default)]
    pub workspace_two_factor: bool,
    /// The caller's role in it.
    #[serde(default)]
    pub workspace_role: Option<String>,
}

impl WorkspaceCols {
    pub fn to_ref(&self) -> Option<WorkspaceRef> {
        Some(workspace_ref(
            self.workspace_id.as_deref()?,
            self.workspace_slug.as_deref()?,
            self.workspace_name.as_deref()?,
            self.workspace_color.as_deref(),
            self.workspace_avatar_at,
        ))
    }

    /// The caller's membership, `user` being the caller.
    pub fn membership(
        &self,
        user: &UserRow,
        settings: &fctl_proto::api::InstanceSettings,
    ) -> Option<fctl_domain::workspace::Membership> {
        let role = WorkspaceRole::parse(self.workspace_role.as_deref()?)?;
        Some(fctl_domain::workspace::Membership {
            role,
            base_role: self
                .workspace_base_role
                .as_deref()
                .and_then(fctl_proto::api::BaseRole::parse)
                .unwrap_or_default(),
            blocked: user.blocked_by(settings, self.workspace_two_factor),
        })
    }
}

/// The path of a workspace's uploaded avatar, which changes with it.
pub fn avatar_url(id: &str, updated_at: Option<i64>) -> Option<String> {
    updated_at.map(|v| format!("/api/v1/workspaces/{id}/avatar?v={v}"))
}

pub fn workspace_ref(
    id: &str,
    slug: &str,
    name: &str,
    color: Option<&str>,
    avatar_at: Option<i64>,
) -> WorkspaceRef {
    WorkspaceRef {
        id: WorkspaceId(id.to_owned()),
        slug: slug.to_owned(),
        name: name.to_owned(),
        color: color.and_then(WorkspaceColor::parse).unwrap_or_default(),
        avatar_url: avatar_url(id, avatar_at),
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
    /// Repositories in `project_repos` named one by one.
    #[serde(deserialize_with = "int", default)]
    pub other_repos: i64,
    /// The account of an owner-wide grant in `project_repos`.
    #[serde(default)]
    pub all_repos_of: Option<String>,
    /// What the caller asked for when they last started a session in it
    /// (`project_choices`, a LEFT JOIN).
    #[serde(default)]
    pub last_model: Option<String>,
    #[serde(default)]
    pub last_effort: Option<String>,
    #[serde(flatten)]
    pub workspace: WorkspaceCols,
}

impl ProjectRow {
    /// The workspace's id ("" for a project from before workspaces, which
    /// the migration gives one).
    pub fn workspace_id(&self) -> &str {
        self.workspace.workspace_id.as_deref().unwrap_or_default()
    }

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
            workspace: self.workspace.to_ref(),
            slug: self.slug.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            repo: self.repo(),
            other_repos: self.other_repos.max(0) as u32,
            all_repos_of: self.all_repos_of.clone(),
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
            last_model: self.last_model.clone(),
            last_effort: self.last_effort.clone(),
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

/// SELECT for projects with their workspace, the caller's roles and what
/// they last started a session with, session counts and their other
/// repositories. Bind: `?1` = caller user id.
pub const PROJECT_SELECT: &str = "SELECT p.id, p.slug, p.name, p.description, p.repo_owner, p.repo_name, \
    p.default_branch, p.settings, p.session_counter, p.created_by, cu.name AS creator_name, \
    cu.avatar_url AS creator_avatar, p.created_at, p.updated_at, p.archived_at, m.role AS member_role, \
    (SELECT COUNT(*) FROM sessions s WHERE s.project_id = p.id AND s.status IN ('provisioning','preparing','running','idle','paused','stopping') \
      AND NOT EXISTS (SELECT 1 FROM reviews rv WHERE rv.session_id = s.id)) AS active_sessions, \
    (SELECT COUNT(*) FROM sessions s WHERE s.project_id = p.id \
      AND NOT EXISTS (SELECT 1 FROM reviews rv WHERE rv.session_id = s.id)) AS total_sessions, \
    (SELECT MAX(COALESCE(s.last_activity_at, s.created_at)) FROM sessions s WHERE s.project_id = p.id \
      AND NOT EXISTS (SELECT 1 FROM reviews rv WHERE rv.session_id = s.id)) AS last_activity_at, \
    (SELECT COUNT(*) FROM project_repos r WHERE r.project_id = p.id AND r.name != '*') AS other_repos, \
    (SELECT r.owner FROM project_repos r WHERE r.project_id = p.id AND r.name = '*' LIMIT 1) AS all_repos_of, \
    w.id AS workspace_id, w.slug AS workspace_slug, w.name AS workspace_name, w.color AS workspace_color, \
    w.avatar_updated_at AS workspace_avatar_at, w.base_role AS workspace_base_role, \
    w.require_two_factor AS workspace_two_factor, wm.role AS workspace_role, \
    c.model AS last_model, c.effort AS last_effort \
    FROM projects p \
    LEFT JOIN users cu ON cu.id = p.created_by \
    LEFT JOIN workspaces w ON w.id = p.workspace_id \
    LEFT JOIN workspace_members wm ON wm.workspace_id = p.workspace_id AND wm.user_id = ?1 \
    LEFT JOIN project_members m ON m.project_id = p.id AND m.user_id = ?1 \
    LEFT JOIN project_choices c ON c.project_id = p.id AND c.user_id = ?1";

/// What lets the caller see a project, for `WHERE` clauses with
/// [`PROJECT_SELECT`] (or [`SESSION_SELECT`]): they are in its workspace,
/// and an admin there, or its base role or their membership gives them a
/// role. Workspaces closed to them (two-factor) are left out in Rust.
pub const PROJECT_VISIBLE: &str = "wm.user_id IS NOT NULL AND (wm.role IN ('owner', 'admin') OR w.base_role != 'none' OR m.role IS NOT NULL)";

/// The order the caller puts their workspaces in (`w`, with their
/// membership `wm`), for `ORDER BY`: theirs (`wm.position`), then the ones
/// they haven't placed, in the order they joined them.
#[macro_export]
macro_rules! workspace_order {
    () => {
        "wm.position IS NULL, wm.position, wm.created_at, w.name, w.id"
    };
}

/// [`workspace_order!`], for queries with [`crate::workspaces::WORKSPACE_SELECT`].
pub const WORKSPACE_ORDER: &str = workspace_order!();

/// The order the caller puts projects in, for `ORDER BY` with
/// [`PROJECT_SELECT`]: by workspace ([`WORKSPACE_ORDER`]), then as they put
/// each one's projects (`wm.project_order`), the ones they haven't placed
/// last, oldest first.
pub const PROJECT_ORDER: &str = concat!(
    workspace_order!(),
    ", COALESCE((SELECT o.key FROM json_each(wm.project_order) o WHERE o.value = p.id), \
    json_array_length(wm.project_order), 0), p.created_at, p.id"
);

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
    /// The effort level the session asked for (`fctl run --effort`), which
    /// its runs pass to Claude.
    pub effort: Option<String>,
    /// The effort level Claude reported it works at (NULL until it did, and
    /// when its model takes none).
    #[serde(default)]
    pub claude_effort: Option<String>,
    pub claude_session_id: String,
    #[serde(deserialize_with = "opt_int", default)]
    pub pr_number: Option<i64>,
    pub pr_url: Option<String>,
    pub pr_title: Option<String>,
    pub pr_state: Option<String>,
    /// Every pull request it found, JSON (NULL before it found any, and for
    /// sessions from before the column).
    #[serde(default)]
    pub pull_requests: Option<String>,
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
    /// What its agent may do on GitHub, as it started (NULL: push).
    #[serde(default)]
    pub github_access: Option<String>,
    /// What its project's settings let agents do on GitHub now.
    #[serde(default)]
    pub project_github_access: Option<String>,
    /// Its agent reaches the GitHub issues of the repositories it writes
    /// to, whatever its project's tracker says
    /// (`fctl_domain::tickets::session_issues`).
    #[serde(deserialize_with = "int_bool", default)]
    pub github_issues: bool,
    /// The caller's project membership role and share role (LEFT JOINs).
    pub member_role: Option<String>,
    pub share_role: Option<String>,
    /// Whether the caller follows it (a LEFT JOIN).
    #[serde(deserialize_with = "int_bool", default)]
    pub following: bool,
    /// For a review agent's session, the pull request it reviews (a LEFT
    /// JOIN of `reviews`): its repository, number, URL, title and commit.
    #[serde(default)]
    pub review_repo: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub review_number: Option<i64>,
    #[serde(default)]
    pub review_pr_url: Option<String>,
    #[serde(default)]
    pub review_title: Option<String>,
    #[serde(default)]
    pub review_sha: Option<String>,
    /// The schedule it started from, and its name while it exists (a LEFT
    /// JOIN of `schedules`).
    #[serde(default)]
    pub schedule_id: Option<String>,
    #[serde(default)]
    pub schedule_name: Option<String>,
    /// The ticket it started from (JSON: `crate::delegations::TicketColumn`).
    #[serde(default)]
    pub ticket: Option<String>,
    #[serde(flatten)]
    pub workspace: WorkspaceCols,
}

impl SessionRow {
    /// The pull request a review agent's session reviews; `None` for other
    /// sessions.
    pub fn review(&self) -> Option<SessionReview> {
        Some(SessionReview {
            repo: RepoRef::parse(self.review_repo.as_deref()?)?,
            number: u64::try_from(self.review_number?).ok()?,
            url: self.review_pr_url.clone()?,
            title: self.review_title.clone(),
            head_sha: self.review_sha.clone()?,
        })
    }

    /// Whether it is a review agent's session, which takes no messages.
    pub fn is_review(&self) -> bool {
        self.review_repo.is_some()
    }

    /// The schedule it started from; `None` for other sessions.
    pub fn schedule(&self) -> Option<SessionSchedule> {
        Some(SessionSchedule {
            id: ScheduleId(self.schedule_id.clone()?),
            name: self.schedule_name.clone(),
        })
    }

    /// What its agent may do on GitHub now, as its tokens are minted and
    /// its pages say: what it started with, within its project's settings
    /// as they are now. A value this version doesn't know reads as the
    /// least.
    pub fn github_access(&self) -> GithubAccess {
        let known = |v: &Option<String>| {
            v.as_deref()
                .map(|v| GithubAccess::parse(v).unwrap_or(GithubAccess::Read))
        };
        fctl_domain::limits::effective(
            known(&self.github_access),
            known(&self.project_github_access),
        )
    }

    /// What its agent may have done on GitHub since it started, for whom
    /// may steer it (`fctl_domain::limits::Limits::session_role`): what it
    /// started with, even once its project holds it lower, since the
    /// tokens in its sandbox last until they expire. A value this version
    /// doesn't know counts as the most.
    pub fn started_github(&self) -> GithubAccess {
        self.github_access
            .as_deref()
            .map_or(GithubAccess::Push, |v| {
                GithubAccess::parse(v).unwrap_or(GithubAccess::Push)
            })
    }

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
        let mut s = Session {
            id: self.session_id(),
            project_id: ProjectId(self.project_id.clone()),
            project_slug: self.project_slug.clone(),
            workspace: self.workspace.to_ref(),
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
                    ..Default::default()
                }),
                _ => None,
            },
            pull_requests: self
                .pull_requests
                .as_deref()
                .and_then(|j| serde_json::from_str::<Vec<PullRequestRef>>(j).ok())
                .unwrap_or_default(),
            runner: match (&self.runner_id, &self.runner_name) {
                (Some(id), Some(name)) => Some(RunnerRef {
                    id: RunnerId(id.clone()),
                    name: name.clone(),
                }),
                _ => None,
            },
            model: self.model.clone(),
            effort: self.claude_effort.clone(),
            github_access: Some(self.github_access()),
            github_issues: self.github_issues.then_some(true),
            remote_control: self
                .remote_control
                .as_deref()
                .and_then(|s| serde_json::from_str::<RemoteControlState>(s).ok()),
            stats: serde_json::from_str::<SessionStats>(&self.stats).unwrap_or_default(),
            role,
            archived: self.archived,
            following: Some(self.following),
            review: self.review(),
            schedule: self.schedule(),
            ticket: self
                .ticket
                .as_deref()
                .and_then(|j| serde_json::from_str::<fctl_proto::session::SessionTicket>(j).ok()),
        };
        fctl_domain::pulls::backfill(&mut s);
        s
    }
}

/// SELECT for sessions with their workspace, the caller's workspace,
/// project and share roles, and whether they follow it. Bind: `?1` =
/// caller user id. A share someone gave themselves (as an owner, before
/// they were demoted) counts for nothing.
pub const SESSION_SELECT: &str = "SELECT s.id, s.project_id, p.slug AS project_slug, s.number, s.title, s.description, s.prompt, s.prompt_attachments, \
    s.status, s.status_reason, s.resume_at, s.created_by, cu.name AS creator_name, cu.avatar_url AS creator_avatar, \
    s.credential_id, s.runner_id, r.name AS runner_name, s.branch, s.base_branch, s.model, s.effort, \
    s.claude_effort, s.claude_session_id, s.pr_number, s.pr_url, s.pr_title, s.pr_state, s.pull_requests, s.remote_control, s.stats, s.archived, \
    s.github_access, json_extract(p.settings, '$.github_access') AS project_github_access, s.github_issues, \
    s.created_at, s.updated_at, s.started_at, s.ended_at, s.last_activity_at, \
    m.role AS member_role, sh.role AS share_role, f.user_id IS NOT NULL AS following, \
    rv.repo AS review_repo, rv.number AS review_number, rv.url AS review_pr_url, rv.title AS review_title, \
    rv.head_sha AS review_sha, s.schedule_id, sc.name AS schedule_name, s.ticket, \
    w.id AS workspace_id, w.slug AS workspace_slug, w.name AS workspace_name, w.color AS workspace_color, \
    w.avatar_updated_at AS workspace_avatar_at, w.base_role AS workspace_base_role, \
    w.require_two_factor AS workspace_two_factor, wm.role AS workspace_role \
    FROM sessions s \
    JOIN projects p ON p.id = s.project_id \
    LEFT JOIN reviews rv ON rv.session_id = s.id \
    LEFT JOIN schedules sc ON sc.id = s.schedule_id \
    LEFT JOIN workspaces w ON w.id = p.workspace_id \
    LEFT JOIN workspace_members wm ON wm.workspace_id = p.workspace_id AND wm.user_id = ?1 \
    LEFT JOIN users cu ON cu.id = s.created_by \
    LEFT JOIN runners r ON r.id = s.runner_id \
    LEFT JOIN project_members m ON m.project_id = s.project_id AND m.user_id = ?1 \
    LEFT JOIN session_shares sh ON sh.session_id = s.id AND sh.user_id = ?1 \
      AND (sh.created_by IS NULL OR sh.created_by != sh.user_id) \
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
    /// The workspaces it serves (JSON, from [`RUNNER_SELECT`]).
    #[serde(default)]
    pub workspaces: Option<String>,
}

/// Who looks at runners: what of a runner they see.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunnerView<'a> {
    pub user_id: Option<&'a str>,
    /// The workspaces they are a member of; `None` for instance admins, who
    /// see every workspace a runner serves.
    pub workspaces: Option<&'a [String]>,
}

#[derive(Deserialize)]
struct RunnerWorkspace {
    id: String,
    slug: String,
    name: String,
    color: Option<String>,
    #[serde(default)]
    avatar_at: Option<f64>,
    /// It runs only its owner's sessions there.
    #[serde(deserialize_with = "int_bool", default)]
    personal: bool,
}

impl RunnerRow {
    /// The ids of the workspaces it serves.
    pub fn workspace_ids(&self) -> Vec<String> {
        self.workspace_list().into_iter().map(|w| w.id).collect()
    }

    fn workspace_list(&self) -> Vec<RunnerWorkspace> {
        self.workspaces
            .as_deref()
            .and_then(|j| serde_json::from_str::<Vec<RunnerWorkspace>>(j).ok())
            .unwrap_or_default()
    }

    /// Whether `view` may see it: it serves one of their workspaces, they
    /// registered it, or they are an instance admin.
    pub fn visible_to(&self, view: &RunnerView) -> bool {
        match view.workspaces {
            None => true,
            Some(mine) => {
                view.user_id.is_some() && self.created_by.as_deref() == view.user_id
                    || self.workspace_ids().iter().any(|w| mine.contains(w))
            }
        }
    }

    pub fn hello(&self) -> Option<HostHello> {
        self.info
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
    }

    pub fn to_runner(&self, live: &LiveRunners, view: &RunnerView) -> Runner {
        let hello = self.hello();
        let seen: Vec<RunnerWorkspace> = self
            .workspace_list()
            .into_iter()
            .filter(|w| view.workspaces.is_none_or(|mine| mine.contains(&w.id)))
            .collect();
        let personal = seen
            .iter()
            .filter(|w| w.personal)
            .map(|w| WorkspaceId(w.id.clone()))
            .collect();
        let workspaces = seen
            .into_iter()
            .map(|w| {
                workspace_ref(
                    &w.id,
                    &w.slug,
                    &w.name,
                    w.color.as_deref(),
                    w.avatar_at.map(|t| t as i64),
                )
            })
            .collect();
        let online = live.online.contains(&self.id);
        let free = live.free.get(&self.id).copied().unwrap_or_default();
        let yours = view.user_id.is_some() && self.created_by.as_deref() == view.user_id;
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
            workspaces,
            yours,
            personal,
            // Whether its host keeps its owner's Claude token is theirs to see.
            claude_token: hello
                .as_ref()
                .and_then(|h| h.claude_token)
                .filter(|_| yours),
            auto_update: hello.as_ref().and_then(|h| h.auto_update),
        }
    }
}

/// Whether runner `r` serves the workspace of `runner_workspaces` row `rw`:
/// while its owner is an active member there (a runner whose owner's
/// account is gone serves where it was put before workspaces had owners).
#[macro_export]
macro_rules! serves {
    () => {
        "(r.created_by IS NULL OR EXISTS (SELECT 1 FROM workspace_members sm JOIN users su ON su.id = sm.user_id \
         WHERE sm.workspace_id = rw.workspace_id AND sm.user_id = r.created_by AND su.status = 'active'))"
    };
}

pub const RUNNER_SELECT: &str = concat!(
    "SELECT r.id, r.name, r.enabled, r.info, r.last_seen_at, r.created_by, \
    u.name AS creator_name, r.created_at, \
    (SELECT COUNT(*) FROM sessions s WHERE s.runner_id = r.id AND s.status IN ('provisioning','preparing','running','idle','paused','stopping')) AS active_sessions, \
    (SELECT json_group_array(json_object('id', w.id, 'slug', w.slug, 'name', w.name, 'color', w.color, \
      'avatar_at', w.avatar_updated_at, 'personal', rw.personal)) FROM runner_workspaces rw JOIN workspaces w ON w.id = rw.workspace_id \
      WHERE rw.runner_id = r.id AND ",
    serves!(),
    ") AS workspaces \
    FROM runners r LEFT JOIN users u ON u.id = r.created_by"
);

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
    /// Its owner keeps its token on their runners: `secret_enc` is empty,
    /// and its sessions run only on those of their runners that keep one.
    #[serde(deserialize_with = "int_bool", default)]
    pub on_runners: bool,
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
            on_runners: self.on_runners,
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
