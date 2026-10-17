//! Who may do what.

use fctl_proto::session::{InstanceRole, ProjectRole, SessionRole};

/// What we know about a person's relation to a session.
#[derive(Clone, Copy, Debug)]
pub struct SessionAccess {
    pub instance_role: InstanceRole,
    pub project_role: Option<ProjectRole>,
    /// A direct share of this session.
    pub share_role: Option<SessionRole>,
    /// The person started the session.
    pub is_creator: bool,
}

/// The effective role, or `None` for no access.
pub fn session_role(a: SessionAccess) -> Option<SessionRole> {
    let mut best: Option<SessionRole> = None;
    let mut grant = |r: SessionRole| {
        best = Some(match best {
            Some(b) if rank(b) >= rank(r) => b,
            _ => r,
        });
    };
    if a.instance_role.is_admin() || a.is_creator {
        grant(SessionRole::Owner);
    }
    match a.project_role {
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

/// The effective project role: instance admins act as project owners.
pub fn project_role(instance_role: InstanceRole, member_role: Option<ProjectRole>) -> Option<ProjectRole> {
    if instance_role.is_admin() {
        Some(ProjectRole::Owner)
    } else {
        member_role
    }
}

/// Starting sessions needs at least `editor`.
pub fn can_start_sessions(role: Option<ProjectRole>) -> bool {
    role.is_some_and(|r| r.at_least(ProjectRole::Editor))
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

/// Whether `actor` may change `target`'s instance role to `new_role`.
pub fn can_assign_instance_role(actor: InstanceRole, target: InstanceRole, new_role: InstanceRole) -> bool {
    match actor {
        InstanceRole::Owner => true,
        InstanceRole::Admin => target == InstanceRole::Member && new_role != InstanceRole::Owner,
        InstanceRole::Member => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access(i: InstanceRole, p: Option<ProjectRole>, s: Option<SessionRole>, c: bool) -> SessionAccess {
        SessionAccess { instance_role: i, project_role: p, share_role: s, is_creator: c }
    }

    #[test]
    fn session_roles() {
        use InstanceRole::*;
        assert_eq!(session_role(access(Member, None, None, false)), None);
        assert_eq!(session_role(access(Member, Some(ProjectRole::Viewer), None, false)), Some(SessionRole::Viewer));
        assert_eq!(session_role(access(Member, Some(ProjectRole::Viewer), Some(SessionRole::Editor), false)), Some(SessionRole::Editor));
        assert_eq!(session_role(access(Member, Some(ProjectRole::Editor), Some(SessionRole::Viewer), false)), Some(SessionRole::Editor));
        assert_eq!(session_role(access(Member, Some(ProjectRole::Editor), None, true)), Some(SessionRole::Owner));
        assert_eq!(session_role(access(Admin, None, None, false)), Some(SessionRole::Owner));
        assert_eq!(session_role(access(Member, Some(ProjectRole::Maintainer), None, false)), Some(SessionRole::Owner));
    }

    #[test]
    fn project_management() {
        assert!(can_start_sessions(Some(ProjectRole::Editor)));
        assert!(!can_start_sessions(Some(ProjectRole::Viewer)));
        assert!(can_manage_project(project_role(InstanceRole::Admin, None)));
        assert!(!can_assign_project_role(Some(ProjectRole::Maintainer), ProjectRole::Owner));
        assert!(can_assign_project_role(Some(ProjectRole::Maintainer), ProjectRole::Editor));
        assert!(!can_assign_instance_role(InstanceRole::Admin, InstanceRole::Admin, InstanceRole::Member));
        assert!(can_assign_instance_role(InstanceRole::Admin, InstanceRole::Member, InstanceRole::Admin));
        assert!(!can_assign_instance_role(InstanceRole::Admin, InstanceRole::Member, InstanceRole::Owner));
    }
}
