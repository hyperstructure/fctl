//! Workspaces (docs/architecture.md#workspaces): people, the projects they
//! work on, the runners that serve them and the GitHub accounts they work
//! in. Who may do what in one, how many one person joins, and what a
//! workspace's settings mean for the roles people have in its projects.

use crate::naming;
use crate::validate::{self, Result};
use fctl_proto::ApiError;
use fctl_proto::api::{BaseRole, InstanceSettings};
use fctl_proto::session::{InstanceRole, ProjectRole, WorkspaceRole};

/// The most workspaces one person can be a member of, but the primary
/// instance owner ([`workspace_limit`]).
pub const MAX_WORKSPACES: usize = 5;
/// How long an invitation to a workspace waits for an answer.
pub const INVITATION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

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

/// Whether an instance admin may give the primary admin role of a workspace
/// whose primary admin's account is disabled or gone to someone (`to_self`:
/// to themselves, whose role there is `own`). Instance admins can disable
/// any member, so an instance admin who could take the role would take any
/// workspace they are in: they give it to themselves only where they are
/// one of its admins already, and to anyone else of its members.
pub fn check_assign_primary(to_self: bool, own: Option<WorkspaceRole>) -> Result<()> {
    if to_self && !own.is_some_and(WorkspaceRole::is_admin) {
        return Err(denied(
            "Instance admins make themselves the primary admin only of workspaces they are an admin of. Give the role to one of its admins, or have another instance admin give it to you.",
        ));
    }
    Ok(())
}

/// Whether a workspace's admins may set its two-factor requirement to
/// `require`: not off while the instance requires two-factor sign-in, as
/// every workspace then does ([`crate::two_factor::workspace_requires`]).
pub fn check_two_factor(settings: &InstanceSettings, require: bool) -> Result<()> {
    if !require && settings.require_two_factor {
        return Err(ApiError::conflict(
            "This instance requires two-factor sign-in, so every workspace does too. Instance admins can turn that off in Admin → Settings, or with `fctl admin set --require-two-factor off`.",
        ));
    }
    Ok(())
}

/// The most workspaces someone whose instance role is `role` can be a
/// member of: [`MAX_WORKSPACES`], or any number (`None`) for the primary
/// instance owner, whose instance it is. Admins are held to it as members
/// are, and so is an owner who hands the role over: they stay in every
/// workspace they are in, and join no more until they are in fewer.
pub fn workspace_limit(role: InstanceRole) -> Option<usize> {
    (role != InstanceRole::Owner).then_some(MAX_WORKSPACES)
}

/// Whether someone who is a member of `count` workspaces, of at most
/// `limit` ([`workspace_limit`]), may join, or create, another one.
pub fn check_limit(count: usize, limit: Option<usize>) -> Result<()> {
    match limit {
        Some(max) if count >= max => Err(ApiError::conflict(if count == max {
            format!("You are a member of {max} workspaces, the most you can be. Leave one first.")
        } else {
            format!(
                "You are a member of {count} workspaces, and you can be in {max} at most. Leave {} first.",
                count + 1 - max
            )
        })),
        _ => Ok(()),
    }
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
/// Apostrophes go: "Ada's lab" is `adas-lab`.
pub fn slug_from_name(name: &str) -> String {
    let s = naming::slugify(&name.replace(['\'', '\u{2019}'], ""), 48);
    if s.len() < 2 {
        format!("workspace-{s}").trim_end_matches('-').to_owned()
    } else {
        s
    }
}

/// Whether someone whose role in a workspace is `role` may create a project
/// in it: only its admins. A project's owners choose its repositories among
/// those of the workspace's GitHub accounts, and its sessions get write
/// tokens for them, so a member who could create one would reach every
/// repository the workspace works in, whatever the base role and their
/// project roles let them see.
pub fn check_create_project(role: Option<WorkspaceRole>) -> Result<()> {
    if role.is_some_and(WorkspaceRole::is_admin) {
        Ok(())
    } else {
        Err(denied(
            "Only the workspace's admins create its projects. Ask one of them to create it and add you to it.",
        ))
    }
}

/// Whether someone may add a runner to a workspace: they registered the
/// runner (it is their machine) and they are a member of the workspace. A
/// runner that runs everyone's sessions there sees their code, secrets and
/// tokens, so only its admins add one; a member adds theirs for their own
/// sessions alone (`personal`), which see nothing they couldn't.
pub fn check_add_runner(
    registered_it: bool,
    role: Option<WorkspaceRole>,
    personal: bool,
) -> Result<()> {
    let Some(role) = role else {
        return Err(denied("Only the workspace's members can add runners to it"));
    };
    if !registered_it {
        return Err(denied(
            "Only the person who registered a runner can add it to workspaces",
        ));
    }
    if !personal && !role.is_admin() {
        return Err(denied(
            "Only the workspace's admins add runners that run everyone's sessions. Add yours for your own sessions.",
        ));
    }
    Ok(())
}

/// Whether someone may replace a runner's token, which they then hold: only
/// the person who registered it. The token runs the sessions of the
/// workspaces the runner serves, with their code, secrets and tokens, and
/// an instance admin need not be a member of those; they disable, drain or
/// remove the runner instead.
pub fn check_replace_runner_token(registered_it: bool) -> Result<()> {
    if registered_it {
        Ok(())
    } else {
        Err(denied(
            "Only the person who registered a runner can replace its token, since the new token runs its workspaces' sessions. Instance admins can disable or remove it.",
        ))
    }
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
    fn five_workspaces_at_most_but_for_the_primary_instance_owner() {
        for role in [InstanceRole::Admin, InstanceRole::Member] {
            let limit = workspace_limit(role);
            assert_eq!(limit, Some(MAX_WORKSPACES));
            assert!(check_limit(0, limit).is_ok());
            assert!(check_limit(MAX_WORKSPACES - 1, limit).is_ok());
            let full = check_limit(MAX_WORKSPACES, limit).err().map(|e| e.message);
            assert_eq!(
                full.as_deref(),
                Some("You are a member of 5 workspaces, the most you can be. Leave one first.")
            );
        }
        let owner = workspace_limit(InstanceRole::Owner);
        assert_eq!(owner, None);
        for count in [0, MAX_WORKSPACES, 100] {
            assert!(check_limit(count, owner).is_ok());
        }
        // An owner who handed the role over, in more than the limit.
        let over = check_limit(7, workspace_limit(InstanceRole::Admin));
        assert_eq!(
            over.err().map(|e| e.message).as_deref(),
            Some("You are a member of 7 workspaces, and you can be in 5 at most. Leave 3 first.")
        );
    }

    #[test]
    fn two_factor_stays_on_while_the_instance_requires_it() {
        let optional = InstanceSettings::default();
        assert!(check_two_factor(&optional, true).is_ok());
        assert!(check_two_factor(&optional, false).is_ok());
        let required = InstanceSettings {
            require_two_factor: true,
            ..Default::default()
        };
        assert!(check_two_factor(&required, true).is_ok());
        let refused = check_two_factor(&required, false).err();
        assert_eq!(
            refused.map(|e| e.code),
            Some(fctl_proto::ErrorCode::Conflict)
        );
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
        assert_eq!(slug_from_name("Ada's side project"), "adas-side-project");
        assert_eq!(slug_from_name("Ada\u{2019}s lab"), "adas-lab");
        assert_eq!(slug_from_name("X"), "workspace-x");
        assert_eq!(slug_from_name("✨"), "workspace");
    }

    #[test]
    fn only_admins_create_projects() {
        assert!(check_create_project(Some(Owner)).is_ok());
        assert!(check_create_project(Some(Admin)).is_ok());
        assert!(check_create_project(Some(Member)).is_err());
        assert!(check_create_project(None).is_err());
    }

    #[test]
    fn members_add_their_runners_for_themselves_and_admins_for_everyone() {
        // Admins add theirs for everyone's sessions or their own.
        for personal in [false, true] {
            assert!(check_add_runner(true, Some(Admin), personal).is_ok());
            assert!(check_add_runner(true, Some(Owner), personal).is_ok());
            assert!(check_add_runner(false, Some(Admin), personal).is_err());
            assert!(check_add_runner(true, None, personal).is_err());
        }
        // Members only for their own.
        assert!(check_add_runner(true, Some(Member), true).is_ok());
        assert!(check_add_runner(true, Some(Member), false).is_err());
        assert!(check_add_runner(false, Some(Member), true).is_err());
        assert!(can_remove_runner(true, None, false));
        assert!(can_remove_runner(false, Some(Admin), false));
        assert!(can_remove_runner(false, None, true));
        assert!(!can_remove_runner(false, Some(Member), false));
    }

    #[test]
    fn only_a_runners_owner_holds_its_new_token() {
        assert!(check_replace_runner_token(true).is_ok());
        // An instance admin who didn't register it would run the sessions
        // of workspaces they may not be in.
        assert!(check_replace_runner_token(false).is_err());
    }

    #[test]
    fn instance_admins_take_a_primary_admin_role_only_as_its_admins() {
        // To one of its members: theirs to give.
        assert!(check_assign_primary(false, None).is_ok());
        assert!(check_assign_primary(false, Some(Member)).is_ok());
        // To themselves, once they disabled its primary admin: only where
        // they are an admin already.
        assert!(check_assign_primary(true, Some(Admin)).is_ok());
        assert!(check_assign_primary(true, Some(Member)).is_err());
        assert!(check_assign_primary(true, None).is_err());
    }
}
