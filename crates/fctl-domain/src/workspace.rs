//! Workspaces (docs/architecture.md#workspaces): people, the projects they
//! work on, the runners that serve them and the GitHub accounts they work
//! in. Who may do what in one, how many one person joins, and what a
//! workspace's settings mean for the roles people have in its projects.

use crate::files;
use crate::naming;
use crate::validate::{self, Result};
use fctl_proto::ApiError;
use fctl_proto::api::BaseRole;
use fctl_proto::session::{ProjectRole, WorkspaceRole};

/// The most workspaces one person can be a member of.
pub const MAX_WORKSPACES: usize = 5;
/// How long an invitation to a workspace waits for an answer.
pub const INVITATION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// The largest avatar image, in bytes. The web app makes them 256 px square.
pub const MAX_AVATAR_BYTES: usize = 256 * 1000;
/// The widest and tallest avatar image, in pixels.
pub const MAX_AVATAR_SIDE: u32 = 1024;

/// Someone's membership of a workspace, as access checks need it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Membership {
    pub role: WorkspaceRole,
    /// The workspace's base role.
    pub base_role: BaseRole,
    /// The workspace requires two-factor sign-in, which they haven't set up
    /// ([`crate::two_factor::workspace_blocked`]): nothing in it is open to
    /// them until they do.
    pub blocked: bool,
}

impl Membership {
    /// The membership, when it opens the workspace.
    pub fn open(self) -> Option<Self> {
        (!self.blocked).then_some(self)
    }
}

/// The role someone has in a project of a workspace: owner for the
/// workspace's admins; for its members, the higher of their role as the
/// project's member and the workspace's base role. Nothing for people
/// outside the workspace, or whom it is closed to.
pub fn project_role(
    membership: Option<Membership>,
    member_role: Option<ProjectRole>,
) -> Option<ProjectRole> {
    let m = membership?.open()?;
    if m.role.is_admin() {
        return Some(ProjectRole::Owner);
    }
    match (member_role, m.base_role.project_role()) {
        (Some(a), Some(b)) => Some(if a.at_least(b) { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn denied(message: &str) -> ApiError {
    ApiError::forbidden(message)
}

/// Whether workspace `actor` may make a member whose role is `from` an
/// `to`. Admins make members admins and admins members; the primary admin
/// hands their own role over instead ([`check_transfer`]).
pub fn check_role_change(
    actor: Option<WorkspaceRole>,
    from: WorkspaceRole,
    to: WorkspaceRole,
) -> Result<()> {
    if !actor.is_some_and(WorkspaceRole::is_admin) {
        return Err(denied("Only the workspace's admins can change roles"));
    }
    if from == WorkspaceRole::Owner {
        return Err(denied(
            "Nobody can change the primary admin's role. They can hand it to another admin.",
        ));
    }
    if to == WorkspaceRole::Owner {
        return Err(ApiError::validation(
            "role",
            "A workspace has one primary admin, who hands the role over",
        ));
    }
    Ok(())
}

/// Whether `actor` may remove a member whose role is `target` from a
/// workspace (`own`: they leave). Everyone may leave, except the primary
/// admin, who hands their role over first; admins remove anyone but the
/// primary admin.
pub fn check_removal(actor: Option<WorkspaceRole>, target: WorkspaceRole, own: bool) -> Result<()> {
    if target == WorkspaceRole::Owner {
        return Err(if own {
            ApiError::conflict(
                "You are this workspace's primary admin. Hand the role to another admin first, or delete the workspace.",
            )
        } else {
            denied("Nobody can remove a workspace's primary admin")
        });
    }
    if own || actor.is_some_and(WorkspaceRole::is_admin) {
        Ok(())
    } else {
        Err(denied("Only the workspace's admins can remove people"))
    }
}

/// Whether `actor` may hand the primary admin role of a workspace to
/// someone whose role there is `to` (`None`: they aren't a member).
pub fn check_transfer(actor: Option<WorkspaceRole>, to: Option<WorkspaceRole>) -> Result<()> {
    if actor != Some(WorkspaceRole::Owner) {
        return Err(denied("Only the primary admin can hand the role over"));
    }
    match to {
        Some(WorkspaceRole::Admin) => Ok(()),
        Some(WorkspaceRole::Owner) => Err(ApiError::conflict("They are the primary admin already")),
        Some(WorkspaceRole::Member) => Err(ApiError::validation(
            "user_id",
            "Make them an admin first: the primary admin role goes to an admin",
        )),
        None => Err(ApiError::validation(
            "user_id",
            "They aren't a member of this workspace",
        )),
    }
}

/// Whether someone who is a member of `count` workspaces may join, or
/// create, another one.
pub fn check_limit(count: usize) -> Result<()> {
    if count >= MAX_WORKSPACES {
        return Err(ApiError::conflict(format!(
            "You are a member of {MAX_WORKSPACES} workspaces, the most anyone can be. Leave one first."
        )));
    }
    Ok(())
}

/// The role an invitation gives: a member or an admin, never the primary
/// admin.
pub fn invitation_role(role: Option<WorkspaceRole>) -> Result<WorkspaceRole> {
    match role.unwrap_or(WorkspaceRole::Member) {
        WorkspaceRole::Owner => Err(ApiError::validation(
            "role",
            "Invite them as an admin; the primary admin hands the role over",
        )),
        r => Ok(r),
    }
}

/// A workspace's name.
pub fn name(input: &str) -> Result<String> {
    validate::name("name", input)
}

/// A slug from a workspace's name (callers check that it is free).
pub fn slug_from_name(name: &str) -> String {
    let s = naming::slugify(name, 48);
    if s.len() < 2 {
        format!("workspace-{s}").trim_end_matches('-').to_owned()
    } else {
        s
    }
}

/// Checks an avatar image and returns its media type: a PNG, JPEG, GIF or
/// WebP of at most [`MAX_AVATAR_BYTES`] and [`MAX_AVATAR_SIDE`] a side.
pub fn avatar(bytes: &[u8]) -> Result<&'static str> {
    if bytes.len() > MAX_AVATAR_BYTES {
        return Err(ApiError::validation(
            "avatar",
            format!(
                "Use an image of at most {}",
                files::human_size(MAX_AVATAR_BYTES as u64)
            ),
        ));
    }
    let info = files::sniff_image(bytes)
        .ok_or_else(|| ApiError::validation("avatar", "Use a PNG, JPEG, GIF or WebP image"))?;
    if info.width > MAX_AVATAR_SIDE || info.height > MAX_AVATAR_SIDE {
        return Err(ApiError::validation(
            "avatar",
            format!("Use an image of at most {MAX_AVATAR_SIDE}×{MAX_AVATAR_SIDE} pixels"),
        ));
    }
    Ok(info.media_type)
}

/// Whether someone may add a runner to a workspace: they registered the
/// runner (it is their machine) and they are an admin of the workspace (it
/// trusts the machine with its code and secrets).
pub fn check_add_runner(registered_it: bool, role: Option<WorkspaceRole>) -> Result<()> {
    if !role.is_some_and(WorkspaceRole::is_admin) {
        return Err(denied(
            "Only the workspace's admins can choose the runners its sessions run on",
        ));
    }
    if !registered_it {
        return Err(denied(
            "Only the person who registered a runner can add it to workspaces",
        ));
    }
    Ok(())
}

/// Whether someone may take a runner out of a workspace: they registered
/// it, they are an admin of the workspace, or of the instance.
pub fn can_remove_runner(
    registered_it: bool,
    role: Option<WorkspaceRole>,
    instance_admin: bool,
) -> bool {
    registered_it || instance_admin || role.is_some_and(WorkspaceRole::is_admin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use WorkspaceRole::*;

    fn member(role: WorkspaceRole, base_role: BaseRole) -> Option<Membership> {
        Some(Membership {
            role,
            base_role,
            blocked: false,
        })
    }

    #[test]
    fn admins_own_every_project_and_members_get_the_higher_role() {
        assert_eq!(
            project_role(member(Admin, BaseRole::None), None),
            Some(ProjectRole::Owner)
        );
        assert_eq!(
            project_role(member(Owner, BaseRole::None), Some(ProjectRole::Viewer)),
            Some(ProjectRole::Owner)
        );
        assert_eq!(project_role(member(Member, BaseRole::None), None), None);
        assert_eq!(
            project_role(member(Member, BaseRole::Editor), None),
            Some(ProjectRole::Editor)
        );
        assert_eq!(
            project_role(member(Member, BaseRole::Editor), Some(ProjectRole::Viewer)),
            Some(ProjectRole::Editor)
        );
        assert_eq!(
            project_role(
                member(Member, BaseRole::Viewer),
                Some(ProjectRole::Maintainer)
            ),
            Some(ProjectRole::Maintainer)
        );
    }

    #[test]
    fn people_outside_a_workspace_or_blocked_by_it_get_nothing() {
        assert_eq!(project_role(None, Some(ProjectRole::Owner)), None);
        let blocked = Some(Membership {
            role: Admin,
            base_role: BaseRole::Maintainer,
            blocked: true,
        });
        assert_eq!(project_role(blocked, Some(ProjectRole::Owner)), None);
    }

    #[test]
    fn admins_change_roles_but_not_the_primary_admins() {
        assert!(check_role_change(Some(Admin), Member, Admin).is_ok());
        assert!(check_role_change(Some(Owner), Admin, Member).is_ok());
        assert!(check_role_change(Some(Admin), Admin, Member).is_ok());
        assert!(check_role_change(Some(Member), Member, Admin).is_err());
        assert!(check_role_change(None, Member, Admin).is_err());
        assert!(check_role_change(Some(Admin), Owner, Admin).is_err());
        assert!(check_role_change(Some(Owner), Owner, Admin).is_err());
        assert!(check_role_change(Some(Owner), Admin, Owner).is_err());
    }

    #[test]
    fn nobody_removes_the_primary_admin_and_everyone_else_may_leave() {
        assert!(check_removal(Some(Admin), Member, false).is_ok());
        assert!(check_removal(Some(Admin), Admin, false).is_ok());
        assert!(check_removal(Some(Member), Member, true).is_ok());
        assert!(check_removal(Some(Admin), Admin, true).is_ok());
        assert!(check_removal(Some(Member), Member, false).is_err());
        assert!(check_removal(Some(Admin), Owner, false).is_err());
        assert!(check_removal(Some(Owner), Owner, true).is_err());
        assert!(check_removal(None, Member, false).is_err());
    }

    #[test]
    fn the_primary_admin_hands_the_role_to_an_admin() {
        assert!(check_transfer(Some(Owner), Some(Admin)).is_ok());
        assert!(check_transfer(Some(Admin), Some(Admin)).is_err());
        assert!(check_transfer(Some(Owner), Some(Member)).is_err());
        assert!(check_transfer(Some(Owner), None).is_err());
        assert!(check_transfer(Some(Owner), Some(Owner)).is_err());
    }

    #[test]
    fn five_workspaces_at_most() {
        assert!(check_limit(0).is_ok());
        assert!(check_limit(MAX_WORKSPACES - 1).is_ok());
        assert!(check_limit(MAX_WORKSPACES).is_err());
    }

    #[test]
    fn invitations_make_members_or_admins() {
        assert_eq!(invitation_role(None).ok(), Some(Member));
        assert_eq!(invitation_role(Some(Admin)).ok(), Some(Admin));
        assert!(invitation_role(Some(Owner)).is_err());
    }

    #[test]
    fn slugs_from_names() {
        assert_eq!(slug_from_name("hyperstructure"), "hyperstructure");
        assert_eq!(slug_from_name("Ada's side project"), "ada-s-side-project");
        assert_eq!(slug_from_name("X"), "workspace-x");
        assert_eq!(slug_from_name("✨"), "workspace");
    }

    #[test]
    fn avatars_are_small_images() {
        // A 1×1 PNG.
        let png =
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";
        assert_eq!(avatar(png).ok(), Some("image/png"));
        assert!(avatar(b"<svg xmlns='http://www.w3.org/2000/svg'/>").is_err());
        let mut big = png.to_vec();
        big.resize(MAX_AVATAR_BYTES + 1, 0);
        assert!(avatar(&big).is_err());
        // 2000 px wide.
        let wide =
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\x07\xd0\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";
        assert!(avatar(wide).is_err());
    }

    #[test]
    fn runners_join_workspaces_with_their_owner_and_its_admins() {
        assert!(check_add_runner(true, Some(Admin)).is_ok());
        assert!(check_add_runner(true, Some(Owner)).is_ok());
        assert!(check_add_runner(true, Some(Member)).is_err());
        assert!(check_add_runner(false, Some(Admin)).is_err());
        assert!(check_add_runner(true, None).is_err());
        assert!(can_remove_runner(true, None, false));
        assert!(can_remove_runner(false, Some(Admin), false));
        assert!(can_remove_runner(false, None, true));
        assert!(!can_remove_runner(false, Some(Member), false));
    }
}
