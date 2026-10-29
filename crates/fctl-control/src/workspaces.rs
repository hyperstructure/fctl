//! Workspaces in D1 (docs/architecture.md#workspaces): one with the
//! caller's membership, the GitHub accounts it works in, joining one, and
//! telling people's feeds when their workspaces change.

use crate::ctx::Ctx;
use crate::db::{Count, Query, int, int_bool, n, on, opt_int, os, q, s};
use crate::error::{AppError, AppResult, forbidden, not_found};
use crate::internal::{self, FeedIdentity};
use crate::model::{UserRow, WORKSPACE_ORDER, workspace_ref};
use fctl_domain::github::Binding;
use fctl_domain::repos::Scope;
use fctl_domain::workspace as policy;
use fctl_proto::api::{
    BaseRole, GithubConnection, GithubConnectionStatus, InstanceSettings, Workspace,
    WorkspaceInvitation,
};
use fctl_proto::session::{UserRef, WorkspaceRef, WorkspaceRole};
use fctl_proto::{ApiError, CredentialId, ErrorCode, UserId, WorkspaceInviteId};
use serde::Deserialize;
use worker::D1Database;

/// Who becomes the primary admin of workspace `w` when theirs, `?1`,
/// goes: its longest-standing admin, else member, people whose accounts
/// are active first. Not the person who made them go (`$actor`, the
/// parameter with their id) unless they are one of its admins already: an
/// instance admin who deletes or disallows a workspace's primary admin
/// would otherwise take over a workspace they are only a member of, which
/// they can't give themselves (`fctl_domain::workspace::check_assign_primary`).
/// None of them left, it waits for one (`WorkspaceRow::needs_primary`).
#[macro_export]
macro_rules! successor {
    ($actor:literal) => {
        concat!(
            "(SELECT o.user_id FROM workspace_members o JOIN users ou ON ou.id = o.user_id \
             WHERE o.workspace_id = w.id AND o.user_id != ?1 AND (o.user_id != ",
            $actor,
            " OR o.role = 'admin') \
             ORDER BY ou.status = 'active' DESC, o.role = 'admin' DESC, o.created_at LIMIT 1)"
        )
    };
}

/// SELECT for workspaces with the caller's membership (`wm`) and what lists
/// show of them. Bind: `?1` = caller user id.
pub const WORKSPACE_SELECT: &str = concat!(
    "SELECT w.id, w.slug, w.name, w.color, w.avatar_updated_at, w.base_role, \
    w.require_two_factor, w.skip_invites, w.created_at, wm.role AS role, wm.email AS email, wm.credential_id AS credential_id, wm.own_runners AS own_runners, \
    (SELECT COUNT(*) FROM workspace_members x WHERE x.workspace_id = w.id) AS members, \
    (SELECT COUNT(*) FROM projects p WHERE p.workspace_id = w.id AND p.archived_at IS NULL) AS projects, \
    (SELECT json_group_array(rw.runner_id) FROM runner_workspaces rw JOIN runners r ON r.id = rw.runner_id \
      WHERE rw.workspace_id = w.id AND r.enabled = 1 AND ",
    crate::serves!(),
    ") AS runner_ids, \
    (SELECT json_group_array(g.account) FROM workspace_github g WHERE g.workspace_id = w.id AND g.status = 'active') AS github, \
    pu.id AS primary_id, pu.name AS primary_name, pu.avatar_url AS primary_avatar, pu.status AS primary_status \
    FROM workspaces w \
    LEFT JOIN workspace_members wm ON wm.workspace_id = w.id AND wm.user_id = ?1 \
    LEFT JOIN workspace_members po ON po.workspace_id = w.id AND po.role = 'owner' \
    LEFT JOIN users pu ON pu.id = po.user_id"
);

#[derive(Clone, Debug, Deserialize)]
pub struct WorkspaceRow {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub color: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub avatar_updated_at: Option<i64>,
    pub base_role: String,
    #[serde(deserialize_with = "int_bool")]
    pub require_two_factor: bool,
    #[serde(deserialize_with = "int_bool")]
    pub skip_invites: bool,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    /// The caller's membership.
    pub role: Option<String>,
    pub email: Option<String>,
    pub credential_id: Option<String>,
    /// The caller keeps their sessions there on their own runners.
    #[serde(deserialize_with = "int_bool", default)]
    pub own_runners: bool,
    #[serde(deserialize_with = "int", default)]
    pub members: i64,
    #[serde(deserialize_with = "int", default)]
    pub projects: i64,
    #[serde(default)]
    pub runner_ids: Option<String>,
    #[serde(default)]
    pub github: Option<String>,
    pub primary_id: Option<String>,
    pub primary_name: Option<String>,
    pub primary_avatar: Option<String>,
    #[serde(default)]
    pub primary_status: Option<String>,
}

/// A JSON array of strings from `json_group_array` (nulls left out).
pub fn strings(json: Option<&str>) -> Vec<String> {
    json.and_then(|j| serde_json::from_str::<Vec<Option<String>>>(j).ok())
        .map(|v| v.into_iter().flatten().collect())
        .unwrap_or_default()
}

impl WorkspaceRow {
    pub fn to_ref(&self) -> WorkspaceRef {
        workspace_ref(
            &self.id,
            &self.slug,
            &self.name,
            self.color.as_deref(),
            self.avatar_updated_at,
        )
    }

    /// The caller's role.
    pub fn role(&self) -> Option<WorkspaceRole> {
        self.role.as_deref().and_then(WorkspaceRole::parse)
    }

    pub fn base_role(&self) -> BaseRole {
        BaseRole::parse(&self.base_role).unwrap_or_default()
    }

    pub fn runner_ids(&self) -> Vec<String> {
        strings(self.runner_ids.as_deref())
    }

    /// Nobody can hand its primary admin role over: theirs is disabled, or
    /// there is none.
    pub fn needs_primary(&self) -> bool {
        self.primary_status.as_deref() != Some("active")
    }

    pub fn primary(&self) -> Option<UserRef> {
        Some(UserRef {
            id: UserId(self.primary_id.clone()?),
            name: self.primary_name.clone().unwrap_or_default(),
            avatar_url: self.primary_avatar.clone(),
        })
    }

    /// Members need two-factor sign-in: its admins chose that, or the
    /// instance requires it.
    pub fn requires_two_factor(&self, settings: &InstanceSettings) -> bool {
        fctl_domain::two_factor::workspace_requires(settings, self.require_two_factor)
    }

    /// The workspace as its member, the caller, sees it; `online`: the
    /// runners online now.
    pub fn to_workspace(
        &self,
        settings: &InstanceSettings,
        blocked: bool,
        online: &[String],
    ) -> Workspace {
        let runners = self.runner_ids();
        Workspace {
            id: fctl_proto::WorkspaceId(self.id.clone()),
            slug: self.slug.clone(),
            name: self.name.clone(),
            color: self
                .color
                .as_deref()
                .and_then(fctl_proto::session::WorkspaceColor::parse)
                .unwrap_or_default(),
            avatar_url: crate::model::avatar_url(&self.id, self.avatar_updated_at),
            role: self.role().unwrap_or(WorkspaceRole::Member),
            email: self.email.clone().unwrap_or_default(),
            base_role: self.base_role(),
            require_two_factor: self.requires_two_factor(settings),
            instance_requires_two_factor: settings.require_two_factor,
            blocked,
            claude: self.credential_id.clone().map(CredentialId),
            primary: self.primary(),
            members: self.members.max(0) as u32,
            projects: self.projects.max(0) as u32,
            runners: runners.len() as u32,
            runners_online: runners.iter().filter(|r| online.contains(r)).count() as u32,
            github: strings(self.github.as_deref()),
            skip_invites: self.skip_invites,
            own_runners: self.own_runners,
            created_at: self.created_at,
        }
    }
}

/// A workspace with the caller's membership.
pub struct WorkspaceAccess {
    pub row: WorkspaceRow,
    pub role: Option<WorkspaceRole>,
    /// It requires two-factor sign-in, which the caller hasn't set up.
    pub blocked: bool,
}

impl WorkspaceAccess {
    pub fn id(&self) -> &str {
        &self.row.id
    }

    /// The caller's role, when the workspace is open to them.
    pub fn require_open(&self) -> AppResult<WorkspaceRole> {
        let role = self.role.ok_or_else(|| not_found("Workspace"))?;
        if self.blocked {
            return Err(AppError(ApiError::new(
                ErrorCode::Forbidden,
                format!(
                    "{} requires two-factor sign-in. Set it up in Settings → Security to open it.",
                    self.row.name
                ),
            )));
        }
        Ok(role)
    }

    pub fn require_admin(&self) -> AppResult<WorkspaceRole> {
        let role = self.require_open()?;
        if role.is_admin() {
            Ok(role)
        } else {
            Err(forbidden("Only the workspace's admins can do this"))
        }
    }

    pub fn require_primary(&self) -> AppResult<()> {
        if self.require_open()? == WorkspaceRole::Owner {
            Ok(())
        } else {
            Err(forbidden("Only the workspace's primary admin can do this"))
        }
    }
}

/// Whether a workspace that requires two-factor sign-in is closed to the
/// caller.
pub fn blocked_for(ctx: &Ctx, user: &UserRow, require_two_factor: bool) -> bool {
    user.blocked_by(&ctx.settings(), require_two_factor)
}

/// A workspace by id or slug, with the caller's membership. People who
/// aren't members are told it doesn't exist.
pub async fn load(ctx: &Ctx, id_or_slug: &str) -> AppResult<WorkspaceAccess> {
    let user = ctx.user()?;
    let row = find(&ctx.db, &user.id, id_or_slug)
        .await?
        .ok_or_else(|| not_found("Workspace"))?;
    let role = row.role();
    if role.is_none() {
        return Err(not_found("Workspace"));
    }
    let blocked = blocked_for(ctx, user, row.require_two_factor);
    Ok(WorkspaceAccess { row, role, blocked })
}

/// A workspace by id or slug, whoever asks (`user_id` for the membership
/// columns).
pub async fn find(
    db: &D1Database,
    user_id: &str,
    id_or_slug: &str,
) -> AppResult<Option<WorkspaceRow>> {
    q(
        db,
        &format!("{WORKSPACE_SELECT} WHERE w.id = ?2 OR w.slug = ?2"),
        &[s(user_id), s(id_or_slug)],
    )?
    .first()
    .await
}

/// The workspaces `user_id` is a member of, in their order
/// ([`WORKSPACE_ORDER`]).
pub async fn mine(db: &D1Database, user_id: &str) -> AppResult<Vec<WorkspaceRow>> {
    q(
        db,
        &format!("{WORKSPACE_SELECT} WHERE wm.user_id = ?1 ORDER BY {WORKSPACE_ORDER}"),
        &[s(user_id)],
    )?
    .all()
    .await
}

/// How many workspaces someone is a member of.
pub async fn membership_count(db: &D1Database, user_id: &str) -> AppResult<usize> {
    let c: Option<Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM workspace_members WHERE user_id = ?",
        &[s(user_id)],
    )?
    .first()
    .await?;
    Ok(c.map_or(0, |c| c.n.max(0) as usize))
}

#[derive(Deserialize)]
pub struct MemberRow {
    pub role: String,
    pub email: String,
    pub credential_id: Option<String>,
}

/// Someone's membership of a workspace.
pub async fn member(
    db: &D1Database,
    workspace_id: &str,
    user_id: &str,
) -> AppResult<Option<MemberRow>> {
    q(
        db,
        "SELECT role, email, credential_id FROM workspace_members WHERE workspace_id = ? AND user_id = ?",
        &[s(workspace_id), s(user_id)],
    )?
    .first()
    .await
}

/// The statement that makes `user_id` a member, as `email`, while they are
/// in fewer than `limit` workspaces (`None`: any number,
/// [`policy::workspace_limit`]).
#[allow(clippy::too_many_arguments)]
pub fn add_member(
    db: &D1Database,
    workspace_id: &str,
    user_id: &str,
    email: &str,
    role: WorkspaceRole,
    added_by: Option<&str>,
    limit: Option<usize>,
    now: i64,
) -> AppResult<Query> {
    // Within the limit when it runs, not only when it was checked: two
    // joins at once can't both take the last place.
    q(
        db,
        "INSERT INTO workspace_members (workspace_id, user_id, email, role, added_by, created_at) \
         SELECT ?1, ?2, ?3, ?4, ?5, ?6 \
         WHERE (?7 IS NULL OR (SELECT COUNT(*) FROM workspace_members WHERE user_id = ?2) < ?7) \
         AND EXISTS (SELECT 1 FROM workspaces WHERE id = ?1) \
         ON CONFLICT (workspace_id, user_id) DO NOTHING",
        &[
            s(workspace_id),
            s(user_id),
            s(email),
            s(role.as_str()),
            os(added_by),
            n(now),
            on(limit.map(|l| l as i64)),
        ],
    )
}

// ───────────────────────────── invitations ─────────────────────────────

/// SELECT for invitations with their workspace and inviter.
pub const INVITATION_SELECT: &str = "SELECT i.id, i.workspace_id, i.email, i.role, i.invited_by, \
    iu.name AS inviter_name, iu.avatar_url AS inviter_avatar, i.created_at, i.expires_at, \
    w.slug, w.name, w.color, w.avatar_updated_at, w.require_two_factor \
    FROM workspace_invitations i JOIN workspaces w ON w.id = i.workspace_id \
    LEFT JOIN users iu ON iu.id = i.invited_by";

/// An ` AND …` keeping invitations (`i`) to those that still count: whoever
/// sent one is still an active admin of its workspace. Only admins invite,
/// so an invitation from one who was since removed, demoted or disabled
/// would outlast the role that let them send it, and let them back in, or
/// in as an admin again, through an address they hold.
pub const INVITER_STILL_ADMIN: &str = " AND EXISTS (SELECT 1 FROM workspace_members im \
    JOIN users imu ON imu.id = im.user_id WHERE im.workspace_id = i.workspace_id \
    AND im.user_id = i.invited_by AND im.role IN ('owner', 'admin') AND imu.status = 'active')";

#[derive(Clone, Debug, Deserialize)]
pub struct InvitationRow {
    pub id: String,
    pub workspace_id: String,
    pub email: String,
    pub role: String,
    pub invited_by: Option<String>,
    pub inviter_name: Option<String>,
    pub inviter_avatar: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "int")]
    pub expires_at: i64,
    pub slug: String,
    pub name: String,
    pub color: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub avatar_updated_at: Option<i64>,
    #[serde(deserialize_with = "int_bool")]
    pub require_two_factor: bool,
}

impl InvitationRow {
    pub fn role(&self) -> WorkspaceRole {
        WorkspaceRole::parse(&self.role).unwrap_or(WorkspaceRole::Member)
    }

    pub fn workspace(&self) -> WorkspaceRef {
        workspace_ref(
            &self.workspace_id,
            &self.slug,
            &self.name,
            self.color.as_deref(),
            self.avatar_updated_at,
        )
    }

    /// `needs_two_factor`: for the invitee, joining waits for two-factor.
    pub fn to_invitation(&self, needs_two_factor: bool) -> WorkspaceInvitation {
        WorkspaceInvitation {
            id: WorkspaceInviteId(self.id.clone()),
            workspace: self.workspace(),
            email: self.email.clone(),
            role: self.role(),
            invited_by: match (&self.invited_by, &self.inviter_name) {
                (Some(id), Some(name)) => Some(UserRef {
                    id: UserId(id.clone()),
                    name: name.clone(),
                    avatar_url: self.inviter_avatar.clone(),
                }),
                _ => None,
            },
            created_at: self.created_at,
            expires_at: self.expires_at,
            needs_two_factor,
            emailed: None,
        }
    }
}

/// The invitations waiting for any of `user_id`'s addresses.
pub async fn invitations_for(
    db: &D1Database,
    user_id: &str,
    now: i64,
) -> AppResult<Vec<InvitationRow>> {
    q(
        db,
        &format!(
            "{INVITATION_SELECT} WHERE i.email IN (SELECT email FROM user_emails WHERE user_id = ?1) \
             AND i.expires_at > ?2{INVITER_STILL_ADMIN} AND NOT EXISTS (SELECT 1 FROM workspace_members m \
             WHERE m.workspace_id = i.workspace_id AND m.user_id = ?1) ORDER BY i.created_at"
        ),
        &[s(user_id), n(now)],
    )?
    .all()
    .await
}

/// Joins `user` to the workspace an invitation is for, as the invited
/// address, and uses up the invitation. Fails when they are in as many
/// workspaces as anyone may be, or it requires two-factor sign-in they
/// haven't set up. Returns `false` when they were in it already (as
/// another of their addresses), which it leaves as it is.
pub async fn join(
    db: &D1Database,
    settings: &InstanceSettings,
    user: &UserRow,
    invitation: &InvitationRow,
    now: i64,
) -> AppResult<bool> {
    if member(db, &invitation.workspace_id, &user.id)
        .await?
        .is_some()
    {
        q(
            db,
            "DELETE FROM workspace_invitations WHERE id = ?",
            &[s(&invitation.id)],
        )?
        .run()
        .await?;
        return Ok(false);
    }
    // Only while whoever sent it may still invite (`INVITER_STILL_ADMIN`).
    let counts: Option<Count> = q(
        db,
        &format!(
            "SELECT COUNT(*) AS n FROM workspace_invitations i WHERE i.id = ?{INVITER_STILL_ADMIN}"
        ),
        &[s(&invitation.id)],
    )?
    .first()
    .await?;
    if counts.is_none_or(|c| c.n == 0) {
        q(
            db,
            "DELETE FROM workspace_invitations WHERE id = ?",
            &[s(&invitation.id)],
        )?
        .run()
        .await?;
        return Err(not_found("Invitation"));
    }
    let limit = policy::workspace_limit(user.role());
    policy::check_limit(membership_count(db, &user.id).await?, limit)?;
    if user.blocked_by(settings, invitation.require_two_factor) {
        return Err(AppError(ApiError::new(
            ErrorCode::Forbidden,
            format!(
                "{} requires two-factor sign-in of its members. Set it up in Settings → Security, then join.",
                invitation.name
            ),
        )));
    }
    crate::db::batch(
        db,
        vec![
            add_member(
                db,
                &invitation.workspace_id,
                &user.id,
                &invitation.email,
                invitation.role(),
                invitation.invited_by.as_deref(),
                limit,
                now,
            )?,
            // Once they are in: this invitation, and those of their other
            // addresses, which would let them back in after they leave.
            q(
                db,
                "DELETE FROM workspace_invitations WHERE workspace_id = ?1 \
                 AND (id = ?3 OR email IN (SELECT email FROM user_emails WHERE user_id = ?2)) \
                 AND EXISTS (SELECT 1 FROM workspace_members WHERE workspace_id = ?1 AND user_id = ?2)",
                &[s(&invitation.workspace_id), s(&user.id), s(&invitation.id)],
            )?,
        ],
    )
    .await?;
    if member(db, &invitation.workspace_id, &user.id)
        .await?
        .is_none()
    {
        // They joined another one in the meantime, or it was deleted.
        policy::check_limit(membership_count(db, &user.id).await?, limit)?;
        return Err(not_found("Workspace"));
    }
    Ok(true)
}

/// After someone signs up or connects an address: they join the
/// workspaces it was invited to, as far as they can (the limit, two-factor
/// sign-in), and the rest keep waiting for them. Returns the ones joined.
pub async fn join_invited(ctx: &Ctx, user: &UserRow, email: &str) -> AppResult<Vec<WorkspaceRef>> {
    let invitations: Vec<InvitationRow> = q(
        &ctx.db,
        &format!(
            "{INVITATION_SELECT} WHERE i.email = ? AND i.expires_at > ?{INVITER_STILL_ADMIN} ORDER BY i.created_at"
        ),
        &[s(email), n(ctx.now)],
    )?
    .all()
    .await?;
    let settings = crate::instance::settings(&ctx.db).await?;
    let mut joined = Vec::new();
    for invitation in invitations {
        match join(&ctx.db, &settings, user, &invitation, ctx.now).await {
            Ok(false) => {}
            Ok(true) => {
                ctx.audit_in(
                    Some(&invitation.workspace_id),
                    "workspace.joined",
                    Some(&user.id),
                    Some(serde_json::json!({ "email": invitation.email, "role": invitation.role })),
                );
                joined.push(invitation.workspace());
            }
            Err(e) => worker::console_log!("left an invitation waiting: {}", e.0.message),
        }
    }
    Ok(joined)
}

// ───────────────────────────── GitHub ─────────────────────────────

#[derive(Clone, Debug, Deserialize)]
pub struct ConnectionRow {
    pub workspace_id: String,
    pub account: String,
    #[serde(deserialize_with = "opt_int", default)]
    pub installation_id: Option<i64>,
    pub repos: Option<String>,
    pub status: String,
    pub requested_by: Option<String>,
    pub requester_name: Option<String>,
    pub requester_avatar: Option<String>,
    pub approved_by: Option<String>,
    pub approver_name: Option<String>,
    /// The GitHub account its admin signed in to GitHub as.
    pub github_user: Option<String>,
    /// GitHub's id of the account it is to (`None` from before ids were
    /// kept, until it is used).
    #[serde(deserialize_with = "opt_int", default)]
    pub account_id: Option<i64>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
}

pub const CONNECTION_SELECT: &str = "SELECT g.workspace_id, g.account, g.installation_id, g.repos, g.status, \
    g.requested_by, ru.name AS requester_name, ru.avatar_url AS requester_avatar, g.approved_by, \
    au.name AS approver_name, g.github_user, g.account_id, g.created_at FROM workspace_github g \
    LEFT JOIN users ru ON ru.id = g.requested_by LEFT JOIN users au ON au.id = g.approved_by";

impl ConnectionRow {
    pub fn active(&self) -> bool {
        self.status == "active"
    }

    /// Its repositories, `None` for every one. A list that doesn't read
    /// is none of them, never every one.
    pub fn repos(&self) -> Option<Vec<String>> {
        self.repos
            .as_deref()
            .map(|j| serde_json::from_str(j).unwrap_or_default())
    }

    pub fn status(&self) -> GithubConnectionStatus {
        GithubConnectionStatus::parse(&self.status).unwrap_or(GithubConnectionStatus::Requested)
    }

    pub fn requester(&self) -> Option<UserRef> {
        Some(UserRef {
            id: UserId(self.requested_by.clone()?),
            name: self.requester_name.clone().unwrap_or_default(),
            avatar_url: self.requester_avatar.clone(),
        })
    }

    pub fn to_connection(&self) -> GithubConnection {
        GithubConnection {
            account: self.account.clone(),
            repos: self.repos(),
            status: self.status(),
            requested_by: self.requester(),
            approved_by: match (&self.approved_by, &self.approver_name) {
                (Some(id), Some(name)) => Some(UserRef {
                    id: UserId(id.clone()),
                    name: name.clone(),
                    avatar_url: None,
                }),
                _ => None,
            },
            github_user: self.github_user.clone(),
            created_at: self.created_at,
        }
    }
}

/// The GitHub accounts a workspace works in.
pub async fn connections(db: &D1Database, workspace_id: &str) -> AppResult<Vec<ConnectionRow>> {
    q(
        db,
        &format!("{CONNECTION_SELECT} WHERE g.workspace_id = ? ORDER BY g.created_at"),
        &[s(workspace_id)],
    )?
    .all()
    .await
}

/// What a workspace may use on `owner`: `None` when it doesn't work there.
pub fn scope(connections: &[ConnectionRow], owner: &str) -> Option<Scope> {
    connections
        .iter()
        .find(|c| c.active() && c.account.eq_ignore_ascii_case(owner))
        .map(|c| Scope::of(c.repos()))
}

/// The installation of the GitHub App that serves a workspace's connection
/// to `account`: the one on that login as GitHub says now (the app may have
/// been installed again since), while the login still belongs to the
/// account the workspace connected, by id (`fctl_domain::github::binding`).
/// A connection from before ids were kept takes the account of the
/// installation it recorded, when GitHub still has it, else the one that
/// has the login now. The installation is remembered with the workspace's
/// connection and the others bound to that account; never with another
/// workspace's connection from before ids were kept, whose recorded
/// installation is what tells it which account it connected. A workspace
/// without an active connection to `account` gets none.
pub async fn installation(
    db: &D1Database,
    gh: &crate::github::Github<'_>,
    connections: &[ConnectionRow],
    account: &str,
) -> AppResult<u64> {
    let Some(connection) = connections
        .iter()
        .find(|c| c.active() && c.account.eq_ignore_ascii_case(account))
    else {
        return Err(not_used(account));
    };
    let found = gh.installation(account).await?;
    let known = connection
        .installation_id
        .and_then(|id| u64::try_from(id).ok());
    let stored = connection.account_id.and_then(|id| u64::try_from(id).ok());
    let mut connected = stored;
    if stored.is_none()
        && let Some(recorded) = known.filter(|k| *k != found.id)
        && let Some(info) = gh.installation_by_id(recorded).await?
    {
        connected = info.account_id;
    }
    if fctl_domain::github::binding(connected, found.account_id) == Binding::Other {
        return Err(not_connected(account));
    }
    // A connection from before ids were kept keeps the account from now on.
    if stored.is_none()
        && let Some(id) = found.account_id
    {
        q(
            db,
            "UPDATE workspace_github SET account_id = ? WHERE workspace_id = ? AND account = ? AND account_id IS NULL",
            &[n(id as i64), s(&connection.workspace_id), s(&connection.account)],
        )?
        .run()
        .await?;
    }
    if known != Some(found.id) {
        q(
            db,
            "UPDATE workspace_github SET installation_id = ?1 \
             WHERE (workspace_id = ?2 AND account = ?3) OR (?4 IS NOT NULL AND account_id = ?4)",
            &[
                n(found.id as i64),
                s(&connection.workspace_id),
                s(&connection.account),
                on(found.account_id.map(|id| id as i64)),
            ],
        )?
        .run()
        .await?;
    }
    Ok(found.id)
}

/// The error for an account a workspace doesn't work in (now).
fn not_used(account: &str) -> AppError {
    AppError(ApiError::new(
        ErrorCode::Forbidden,
        format!(
            "This workspace doesn't use the GitHub account {account}. Its admins connect it under the workspace's GitHub tab."
        ),
    ))
}

/// The error for a connection whose login belongs to another GitHub
/// account now.
pub fn not_connected(account: &str) -> AppError {
    AppError(ApiError::new(
        ErrorCode::Forbidden,
        format!(
            "The GitHub account {account} isn't the one this workspace connected: it was renamed or deleted, and its name may be someone else's now. The workspace's admins connect the account again under its GitHub tab, by its name now."
        ),
    ))
}

// ───────────────────────────── feeds ─────────────────────────────

/// What someone's feed shows: the projects they see, and the workspaces
/// where they see everything or are members (for projects made later).
pub async fn feed_identity(
    db: &D1Database,
    user: &UserRow,
    settings: &InstanceSettings,
) -> AppResult<FeedIdentity> {
    let workspaces = mine(db, &user.id).await?;
    let open: Vec<&WorkspaceRow> = workspaces
        .iter()
        .filter(|w| !user.blocked_by(settings, w.require_two_factor))
        .collect();
    #[derive(Deserialize)]
    struct P {
        id: String,
    }
    let projects: Vec<P> = q(
        db,
        &format!(
            "SELECT p.id FROM projects p JOIN workspaces w ON w.id = p.workspace_id \
             JOIN workspace_members wm ON wm.workspace_id = p.workspace_id AND wm.user_id = ?1 \
             LEFT JOIN project_members m ON m.project_id = p.id AND m.user_id = ?1 \
             WHERE {} AND p.workspace_id IN (SELECT value FROM json_each(?2))",
            crate::model::PROJECT_VISIBLE
        ),
        &[
            s(&user.id),
            s(&serde_json::to_string(
                &open.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            )?),
        ],
    )?
    .all()
    .await?;
    Ok(FeedIdentity {
        user_id: user.id.clone(),
        admin: user.is_admin(),
        projects: projects.into_iter().map(|p| p.id).collect(),
        sees_all: open
            .iter()
            .filter(|w| {
                w.role().is_some_and(WorkspaceRole::is_admin) || w.base_role() != BaseRole::None
            })
            .map(|w| w.id.clone())
            .collect(),
        workspaces: open.iter().map(|w| w.id.clone()).collect(),
        token_id: None,
        checked_at: None,
    })
}

/// Tells the feeds of `user_ids` that their workspaces changed (after the
/// response): they see what they now may, and fetch their workspaces again.
pub fn changed(ctx: &Ctx, user_ids: Vec<String>) {
    if user_ids.is_empty() {
        return;
    }
    let env = ctx.env.clone();
    ctx.defer(async move {
        let stub = internal::fleet_stub(&env)?;
        let _: serde_json::Value = internal::post(
            &stub,
            "/workspaces-changed",
            &internal::WorkspacesChanged { user_ids },
        )
        .await?;
        Ok(())
    });
}

/// Everyone in a workspace.
pub async fn member_ids(db: &D1Database, workspace_id: &str) -> AppResult<Vec<String>> {
    #[derive(Deserialize)]
    struct M {
        user_id: String,
    }
    let rows: Vec<M> = q(
        db,
        "SELECT user_id FROM workspace_members WHERE workspace_id = ?",
        &[s(workspace_id)],
    )?
    .all()
    .await?;
    Ok(rows.into_iter().map(|m| m.user_id).collect())
}
