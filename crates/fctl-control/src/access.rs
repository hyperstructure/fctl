//! Loading resources together with the caller's role, and enforcing it.
//! Projects and sessions are open to the members of their workspace
//! (`fctl_domain::perm`); everyone else is told they don't exist. A limited
//! access token reaches only its projects, with its role there capped
//! (`fctl_domain::limits`).

use crate::ctx::Ctx;
use crate::db::{Val, q, s};
use crate::error::{AppResult, forbidden, not_found};
use crate::model::{PROJECT_SELECT, ProjectRow, SESSION_SELECT, SessionRow, UserRow};
use fctl_domain::limits::Limits;
use fctl_domain::perm::{self, SessionAccess};
use fctl_proto::api::InstanceSettings;
use fctl_proto::session::{GithubAccess, ProjectRole, SessionRole};
use worker::D1Database;

pub struct ProjectAccess {
    pub row: ProjectRow,
    pub role: Option<ProjectRole>,
}

/// An ` AND …` keeping a query of projects (`p`) to those a token limited
/// to `limits` reaches, binding its places after `params`; nothing for a
/// token that reaches every project. Lists filter in SQL, so that their
/// pages stay full.
pub fn places_sql(limits: Option<&Limits>, params: &mut Vec<Val>) -> String {
    let Some(places) = limits.and_then(|l| l.places.as_ref()) else {
        return String::new();
    };
    let json = |ids: &[String]| serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_owned());
    params.push(s(&json(&places.projects)));
    let projects = params.len();
    params.push(s(&json(&places.workspaces)));
    let workspaces = params.len();
    format!(
        " AND (p.id IN (SELECT value FROM json_each(?{projects})) \
         OR p.workspace_id IN (SELECT value FROM json_each(?{workspaces})))"
    )
}

/// A project's role for `user`, the caller, through a token limited to
/// `limits`.
pub fn project_role(
    user: &UserRow,
    settings: &InstanceSettings,
    limits: Option<&Limits>,
    row: &ProjectRow,
) -> Option<ProjectRole> {
    let role = perm::project_role(row.workspace.membership(user, settings), row.member_role());
    match limits {
        Some(l) => l.project_role(row.workspace_id(), &row.id, role),
        None => role,
    }
}

/// A project by id or slug, with the caller's effective role. Projects the
/// caller cannot see are reported as not found.
pub async fn project(ctx: &Ctx, id_or_slug: &str) -> AppResult<ProjectAccess> {
    let user = ctx.user()?;
    let sql = format!("{PROJECT_SELECT} WHERE p.id = ?2 OR p.slug = ?2");
    let row: Option<ProjectRow> = q(&ctx.db, &sql, &[s(&user.id), s(id_or_slug)])?
        .first()
        .await?;
    let row = row.ok_or_else(|| not_found("Project"))?;
    let role = project_role(user, &ctx.settings(), ctx.limits(), &row);
    if role.is_none() {
        return Err(not_found("Project"));
    }
    Ok(ProjectAccess { row, role })
}

impl ProjectAccess {
    /// The project's workspace.
    pub fn workspace_id(&self) -> &str {
        self.row.workspace_id()
    }

    pub fn require_editor(&self) -> AppResult<()> {
        if perm::can_start_sessions(self.role) {
            Ok(())
        } else {
            Err(forbidden("You need the editor role in this project"))
        }
    }

    pub fn require_maintainer(&self) -> AppResult<()> {
        if perm::can_manage_project(self.role) {
            Ok(())
        } else {
            Err(forbidden("You need the maintainer role in this project"))
        }
    }

    pub fn require_owner(&self) -> AppResult<()> {
        if perm::can_own_project(self.role) {
            Ok(())
        } else {
            Err(forbidden("Only project owners can do this"))
        }
    }
}

pub struct SessionAccessRow {
    pub row: SessionRow,
    pub role: SessionRole,
    /// The caller's access token lets its sessions' agents do less on
    /// GitHub than this one's, which leaves the session read only to it:
    /// what it lets them do, for what refusals say.
    pub held_to: Option<GithubAccess>,
}

/// A session with the caller's effective role; hidden ones are not found.
pub async fn session(ctx: &Ctx, id: &str) -> AppResult<SessionAccessRow> {
    let user = ctx.user()?;
    let sql = format!("{SESSION_SELECT} WHERE s.id = ?2");
    let row: Option<SessionRow> = q(&ctx.db, &sql, &[s(&user.id), s(id)])?.first().await?;
    let row = row.ok_or_else(|| not_found("Session"))?;
    let role = session_role(user, &ctx.settings(), ctx.limits(), &row)
        .ok_or_else(|| not_found("Session"))?;
    let held_to = ctx
        .limits()
        .and_then(|l| l.github_access)
        .filter(|most| row.github_access() > *most);
    Ok(SessionAccessRow { row, role, held_to })
}

/// The role `user` has in session `id` now (`settings`: the instance's;
/// `limits`: those of the token they opened it with), or `None` once they
/// can't see it: for sockets that stay open after the Worker checked them
/// (`SessionObject`).
pub async fn session_role_now(
    db: &D1Database,
    user: &UserRow,
    settings: &InstanceSettings,
    limits: Option<&Limits>,
    id: &str,
) -> AppResult<Option<SessionRole>> {
    let sql = format!("{SESSION_SELECT} WHERE s.id = ?2");
    let row: Option<SessionRow> = q(db, &sql, &[s(&user.id), s(id)])?.first().await?;
    Ok(row.and_then(|r| session_role(user, settings, limits, &r)))
}

/// A session's role for `user`, the caller of the query that read `row`,
/// through a token limited to `limits`.
pub fn session_role(
    user: &UserRow,
    settings: &InstanceSettings,
    limits: Option<&Limits>,
    row: &SessionRow,
) -> Option<SessionRole> {
    let access = SessionAccess {
        membership: row.workspace.membership(user, settings),
        project_role: row.member_role.as_deref().and_then(ProjectRole::parse),
        share_role: row.share_role.as_deref().and_then(SessionRole::parse),
        is_creator: row.created_by == user.id,
    };
    match limits {
        Some(l) => l.session_role(
            row.workspace.workspace_id.as_deref().unwrap_or_default(),
            &row.project_id,
            access,
            row.github_access(),
        ),
        None => perm::session_role(access),
    }
}

impl SessionAccessRow {
    /// The session's workspace.
    pub fn workspace_id(&self) -> &str {
        self.row
            .workspace
            .workspace_id
            .as_deref()
            .unwrap_or_default()
    }

    /// Why a token whose agents may do less on GitHub than this session's
    /// can't act on it.
    fn held(&self) -> AppResult<()> {
        match self.held_to {
            Some(most) => Err(forbidden(&format!(
                "This access token's sessions' agents may at most {} on GitHub, and this session's agent may {}: the token only reads it.",
                most.as_str(),
                self.row.github_access().as_str()
            ))),
            None => Ok(()),
        }
    }

    pub fn require_editor(&self) -> AppResult<()> {
        if self.role.can_message() {
            Ok(())
        } else {
            self.held()?;
            Err(forbidden(
                "You can view this session but not message the agent",
            ))
        }
    }

    pub fn require_owner(&self) -> AppResult<()> {
        if self.role.can_manage() {
            Ok(())
        } else {
            self.held()?;
            Err(forbidden("Only the session's owner can do this"))
        }
    }
}
