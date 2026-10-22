//! Who may do what.
//!
//! Projects and sessions belong to workspaces, and only the workspace's
//! members see them: its admins own every project, and its members have
//! the role they have in a project, or the workspace's base role
//! ([`crate::workspace::project_role`]). Instance admins manage people,
//! runners and the instance, not what is inside workspaces they aren't
//! members of.

use crate::workspace::{self, Membership};
use fctl_proto::session::{InstanceRole, ProjectRole, SessionRole};

/// What we know about a person's relation to a session.
#[derive(Clone, Copy, Debug)]
pub struct SessionAccess {
    /// Their membership of the session's workspace.
    pub membership: Option<Membership>,
    /// Their role as a member of the session's project.
    pub project_role: Option<ProjectRole>,
    /// A direct share of this session.
    pub share_role: Option<SessionRole>,
    /// The person started the session.
    pub is_creator: bool,
}

/// The effective role, or `None` for no access. Nothing in a workspace is
/// open to people outside it, whatever they started or was shared with
/// them. Starting a session makes someone its owner only while they may
/// start sessions in its project: one made a viewer of the project, or
/// who lost it, has the session as its viewers do, or not at all, so that
/// they can't go on steering its agent, or resume it with the project's
/// secrets and new tokens.
pub fn session_role(a: SessionAccess) -> Option<SessionRole> {
    session_role_within(a, ProjectRole::Owner)
}

/// [`session_role`] with their role in the session's project counting for
/// at most `cap`: what a limited access token does in its sessions
/// (`crate::limits`). Starting a session and shares count as they are.
pub fn session_role_within(a: SessionAccess, cap: ProjectRole) -> Option<SessionRole> {
    a.membership?.open()?;
    let mut best: Option<SessionRole> = None;
    let mut grant = |r: SessionRole| {
        best = Some(match best {
            Some(b) if rank(b) >= rank(r) => b,
            _ => r,
        });
    };
    let project = project_role(a.membership, a.project_role);
    if a.is_creator && can_start_sessions(project) {
        grant(SessionRole::Owner);
    }
    match project.map(|r| lower_project_role(r, cap)) {
        Some(ProjectRole::Owner | ProjectRole::Maintainer) => grant(SessionRole::Owner),
        Some(ProjectRole::Editor) => grant(SessionRole::Editor),
        Some(ProjectRole::Viewer) => grant(SessionRole::Viewer),
        None => {}
    }
    if let Some(r) = a.share_role {
        grant(r);
    }
    best
}

fn rank(r: SessionRole) -> u8 {
    match r {
        SessionRole::Viewer => 1,
        SessionRole::Editor => 2,
        SessionRole::Owner => 3,
    }
}

/// The lower of two session roles.
pub fn lower_session_role(a: SessionRole, b: SessionRole) -> SessionRole {
    if rank(a) <= rank(b) { a } else { b }
}

/// The lower of two project roles.
pub fn lower_project_role(a: ProjectRole, b: ProjectRole) -> ProjectRole {
    if a.at_least(b) { b } else { a }
}

/// The effective project role (see [`workspace::project_role`]).
pub fn project_role(
    membership: Option<Membership>,
    member_role: Option<ProjectRole>,
) -> Option<ProjectRole> {
    workspace::project_role(membership, member_role)
}

/// Starting sessions needs at least `editor`.
pub fn can_start_sessions(role: Option<ProjectRole>) -> bool {
    role.is_some_and(|r| r.at_least(ProjectRole::Editor))
}

/// Adding to a project's memory, changing its notes, marking them stale and
/// deleting them needs at least `editor`, as steering its agents does.
/// Everyone who sees the project reads it.
pub fn can_edit_memory(role: Option<ProjectRole>) -> bool {
    can_start_sessions(role)
}

/// Changing settings, secrets and members needs at least `maintainer`.
pub fn can_manage_project(role: Option<ProjectRole>) -> bool {
    role.is_some_and(|r| r.at_least(ProjectRole::Maintainer))
}

/// Deleting a project or changing owners needs `owner`.
pub fn can_own_project(role: Option<ProjectRole>) -> bool {
    role == Some(ProjectRole::Owner)
}

/// Whether `actor` may give `target_role` to someone in a project.
pub fn can_assign_project_role(actor: Option<ProjectRole>, target_role: ProjectRole) -> bool {
    match actor {
        Some(ProjectRole::Owner) => true,
        Some(ProjectRole::Maintainer) => target_role != ProjectRole::Owner,
        _ => false,
    }
}

/// Whether `actor` may change `target`'s instance role to `new_role`. The
/// primary instance owner (`owner`) manages admins; admins manage members.
/// Nobody becomes the primary instance owner this way: they hand the role
/// over (`POST /api/v1/admin/transfer`), and nobody else changes theirs.
pub fn can_assign_instance_role(
    actor: InstanceRole,
    target: InstanceRole,
    new_role: InstanceRole,
) -> bool {
    if target == InstanceRole::Owner || new_role == InstanceRole::Owner {
        return false;
    }
    match actor {
        InstanceRole::Owner => true,
        InstanceRole::Admin => target == InstanceRole::Member,
        InstanceRole::Member => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::api::BaseRole;
    use fctl_proto::session::WorkspaceRole;

    fn member(role: WorkspaceRole) -> Option<Membership> {
        Some(Membership {
            role,
            base_role: BaseRole::None,
            blocked: false,
        })
    }

    fn access(
        m: Option<Membership>,
        p: Option<ProjectRole>,
        s: Option<SessionRole>,
        c: bool,
    ) -> SessionAccess {
        SessionAccess {
            membership: m,
            project_role: p,
            share_role: s,
            is_creator: c,
        }
    }

    #[test]
    fn session_roles() {
        use WorkspaceRole::*;
        let m = member(Member);
        assert_eq!(session_role(access(m, None, None, false)), None);
        assert_eq!(
            session_role(access(m, Some(ProjectRole::Viewer), None, false)),
            Some(SessionRole::Viewer)
        );
        assert_eq!(
            session_role(access(
                m,
                Some(ProjectRole::Viewer),
                Some(SessionRole::Editor),
                false
            )),
            Some(SessionRole::Editor)
        );
        assert_eq!(
            session_role(access(
                m,
                Some(ProjectRole::Editor),
                Some(SessionRole::Viewer),
                false
            )),
            Some(SessionRole::Editor)
        );
        assert_eq!(
            session_role(access(m, Some(ProjectRole::Editor), None, true)),
            Some(SessionRole::Owner)
        );
        assert_eq!(
            session_role(access(member(Admin), None, None, false)),
            Some(SessionRole::Owner)
        );
        assert_eq!(
            session_role(access(m, Some(ProjectRole::Maintainer), None, false)),
            Some(SessionRole::Owner)
        );
    }

    #[test]
    fn a_cap_lowers_the_project_role_but_not_starting_or_shares() {
        let admin = member(WorkspaceRole::Admin);
        let editor = ProjectRole::Editor;
        // A maintainer's sessions, as an editor's.
        assert_eq!(
            session_role_within(access(admin, None, None, false), editor),
            Some(SessionRole::Editor)
        );
        // Their own, still theirs.
        assert_eq!(
            session_role_within(access(admin, None, None, true), editor),
            Some(SessionRole::Owner)
        );
        assert_eq!(
            session_role_within(
                access(
                    member(WorkspaceRole::Member),
                    None,
                    Some(SessionRole::Owner),
                    false
                ),
                ProjectRole::Viewer
            ),
            Some(SessionRole::Owner)
        );
        assert_eq!(
            session_role_within(access(admin, None, None, false), ProjectRole::Viewer),
            Some(SessionRole::Viewer)
        );
        assert_eq!(
            lower_session_role(SessionRole::Owner, SessionRole::Viewer),
            SessionRole::Viewer
        );
        assert_eq!(
            lower_project_role(ProjectRole::Viewer, ProjectRole::Owner),
            ProjectRole::Viewer
        );
    }

    #[test]
    fn starting_a_session_counts_while_they_may_start_sessions() {
        use WorkspaceRole::*;
        let m = member(Member);
        // Started as an editor: theirs.
        assert_eq!(
            session_role(access(m, Some(ProjectRole::Editor), None, true)),
            Some(SessionRole::Owner)
        );
        // Made a viewer of the project since: as its viewers.
        assert_eq!(
            session_role(access(m, Some(ProjectRole::Viewer), None, true)),
            Some(SessionRole::Viewer)
        );
        // Taken out of it: nothing, unless someone shares it with them.
        assert_eq!(session_role(access(m, None, None, true)), None);
        assert_eq!(
            session_role(access(m, None, Some(SessionRole::Editor), true)),
            Some(SessionRole::Editor)
        );
        // A workspace admin owns every project, and so their sessions.
        assert_eq!(
            session_role(access(member(Admin), None, None, true)),
            Some(SessionRole::Owner)
        );
    }

    #[test]
    fn a_workspaces_base_role_reaches_every_session() {
        let editors = Some(Membership {
            role: WorkspaceRole::Member,
            base_role: BaseRole::Editor,
            blocked: false,
        });
        assert_eq!(
            session_role(access(editors, None, None, false)),
            Some(SessionRole::Editor)
        );
    }

    #[test]
    fn nothing_is_open_outside_the_workspace() {
        // Starting a session, a share or a project role count for nothing
        // once the person left the workspace, or while it is closed to them.
        assert_eq!(
            session_role(access(
                None,
                Some(ProjectRole::Owner),
                Some(SessionRole::Owner),
                true
            )),
            None
        );
        let blocked = Some(Membership {
            role: WorkspaceRole::Owner,
            base_role: BaseRole::Maintainer,
            blocked: true,
        });
        assert_eq!(session_role(access(blocked, None, None, true)), None);
    }

    #[test]
    fn project_management() {
        assert!(can_start_sessions(Some(ProjectRole::Editor)));
        assert!(!can_start_sessions(Some(ProjectRole::Viewer)));
        assert!(can_manage_project(project_role(
            member(WorkspaceRole::Admin),
            None
        )));
        assert!(!can_manage_project(project_role(
            member(WorkspaceRole::Member),
            None
        )));
        assert!(!can_assign_project_role(
            Some(ProjectRole::Maintainer),
            ProjectRole::Owner
        ));
        assert!(can_assign_project_role(
            Some(ProjectRole::Maintainer),
            ProjectRole::Editor
        ));
    }

    #[test]
    fn instance_roles() {
        use InstanceRole::*;
        assert!(!can_assign_instance_role(Admin, Admin, Member));
        assert!(can_assign_instance_role(Admin, Member, Admin));
        assert!(can_assign_instance_role(Owner, Admin, Member));
        // Nobody makes or unmakes the primary instance owner by changing roles.
        assert!(!can_assign_instance_role(Admin, Member, Owner));
        assert!(!can_assign_instance_role(Owner, Admin, Owner));
        assert!(!can_assign_instance_role(Owner, Owner, Admin));
        assert!(!can_assign_instance_role(Member, Member, Admin));
    }
}
