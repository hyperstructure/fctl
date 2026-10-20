//! Loading resources together with the caller's role, and enforcing it.
//! Projects and sessions are open to the members of their workspace
//! (`fctl_domain::perm`); everyone else is told they don't exist.

use crate::ctx::Ctx;
use crate::db::{q, s};
use crate::error::{AppResult, forbidden, not_found};
use crate::model::{PROJECT_SELECT, ProjectRow, SESSION_SELECT, SessionRow, UserRow};
use fctl_domain::perm::{self, SessionAccess};
use fctl_proto::api::InstanceSettings;
use fctl_proto::session::{ProjectRole, SessionRole};

pub struct ProjectAccess {
    pub row: ProjectRow,
    pub role: Option<ProjectRole>,
}

/// A project's role for `user`, the caller.
pub fn project_role(
    user: &UserRow,
    settings: &InstanceSettings,
    row: &ProjectRow,
) -> Option<ProjectRole> {
    perm::project_role(row.workspace.membership(user, settings), row.member_role())
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
    let role = project_role(user, &ctx.settings(), &row);
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
}

/// A session with the caller's effective role; hidden ones are not found.
pub async fn session(ctx: &Ctx, id: &str) -> AppResult<SessionAccessRow> {
    let user = ctx.user()?;
    let sql = format!("{SESSION_SELECT} WHERE s.id = ?2");
    let row: Option<SessionRow> = q(&ctx.db, &sql, &[s(&user.id), s(id)])?.first().await?;
    let row = row.ok_or_else(|| not_found("Session"))?;
    let role = session_role(user, &ctx.settings(), &row).ok_or_else(|| not_found("Session"))?;
    Ok(SessionAccessRow { row, role })
}

/// A session's role for `user`, the caller of the query that read `row`.
pub fn session_role(
    user: &UserRow,
    settings: &InstanceSettings,
    row: &SessionRow,
) -> Option<SessionRole> {
    perm::session_role(SessionAccess {
        membership: row.workspace.membership(user, settings),
        project_role: row.member_role.as_deref().and_then(ProjectRole::parse),
        share_role: row.share_role.as_deref().and_then(SessionRole::parse),
        is_creator: row.created_by == user.id,
    })
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

    pub fn require_editor(&self) -> AppResult<()> {
        if self.role.can_message() {
            Ok(())
        } else {
            Err(forbidden(
                "You can view this session but not message the agent",
            ))
        }
    }

    pub fn require_owner(&self) -> AppResult<()> {
        if self.role.can_manage() {
            Ok(())
        } else {
            Err(forbidden("Only the session's owner can do this"))
        }
    }
}
