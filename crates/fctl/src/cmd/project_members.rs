//! `fctl projects members …`: the people a project has, and their roles in
//! it. Its workspace's admins own it besides, and the workspace's base role
//! gives every member of the workspace a role in it.

use crate::style;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use fctl_client::Api;
use fctl_proto::api::{AddMemberRequest, MemberList, ProjectMember};
use fctl_proto::session::ProjectRole;

#[derive(Args)]
pub struct MembersArgs {
    /// The project (slug or id); default from `fctl projects use`.
    #[arg(short, long, global = true)]
    pub project: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<MembersCmd>,
}

#[derive(Subcommand)]
pub enum MembersCmd {
    /// Its members and their roles (the default).
    #[command(visible_alias = "ls")]
    List,
    /// Add someone in the project's workspace, or change the role of a
    /// member (maintainers; only owners make owners).
    Add {
        /// The address they are in the workspace as.
        email: String,
        /// Their role.
        #[arg(long, value_enum, default_value_t = Role::Editor)]
        role: Role,
    },
    /// Change a member's role (maintainers; only owners change an owner's).
    /// A project keeps at least one owner.
    Role {
        /// Their address in the workspace, or their user id.
        person: String,
        /// Their new role.
        #[arg(value_enum)]
        role: Role,
    },
    /// Remove someone from the project (maintainers; only owners remove an
    /// owner), or leave it: your own address.
    #[command(visible_alias = "rm")]
    Remove {
        /// Their address in the workspace, or their user id.
        person: String,
    },
}

/// A role in a project, as `--role` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Role {
    /// Reads its sessions.
    Viewer,
    /// Also starts sessions and messages agents.
    Editor,
    /// Also changes its settings, secrets and members.
    Maintainer,
    /// Everything, deleting the project included.
    Owner,
}

impl From<Role> for ProjectRole {
    fn from(r: Role) -> Self {
        match r {
            Role::Viewer => Self::Viewer,
            Role::Editor => Self::Editor,
            Role::Maintainer => Self::Maintainer,
            Role::Owner => Self::Owner,
        }
    }
}

/// "a viewer", "an editor", "a maintainer", "an owner".
fn a(role: ProjectRole) -> &'static str {
    match role {
        ProjectRole::Viewer => "a viewer",
        ProjectRole::Editor => "an editor",
        ProjectRole::Maintainer => "a maintainer",
        ProjectRole::Owner => "an owner",
    }
}

/// A member, by their address in the workspace or their user id.
fn find<'a>(members: &'a [ProjectMember], who: &str) -> Option<&'a ProjectMember> {
    let who = who.trim();
    members
        .iter()
        .find(|m| m.user.id.as_str() == who || m.email.eq_ignore_ascii_case(who))
}

async fn member(api: &Api, project: &str, who: &str) -> Result<ProjectMember> {
    let members = api.project_members(project).await?.members;
    find(&members, who).cloned().with_context(|| {
        format!("{project} has no member {who}. `fctl projects members -p {project}` lists them.")
    })
}

fn print(list: &MemberList) {
    for m in &list.members {
        println!(
            "{:<28} {:<36} {}",
            style::bold(&m.user.name),
            m.email,
            style::dim(m.role.as_str())
        );
    }
}

pub async fn run(api: &Api, args: MembersArgs, json: bool) -> Result<()> {
    let project = super::sessions::project_or_default(args.project)?;
    match args.cmd.unwrap_or(MembersCmd::List) {
        MembersCmd::List => {
            let list = api.project_members(&project).await?;
            if json {
                return style::print_json(&list);
            }
            print(&list);
        }
        MembersCmd::Add { email, role } => {
            let role = ProjectRole::from(role);
            let list = api
                .add_project_member(
                    &project,
                    &AddMemberRequest {
                        email: email.trim().to_owned(),
                        role,
                    },
                )
                .await?;
            if json {
                return style::print_json(&list);
            }
            let name = find(&list.members, &email).map_or(email.trim(), |m| m.user.name.as_str());
            println!("{} {name} is {} of {project}", style::accent("✓"), a(role));
        }
        MembersCmd::Role { person, role } => {
            let m = member(api, &project, &person).await?;
            let role = ProjectRole::from(role);
            let list = api
                .set_project_role(&project, m.user.id.as_str(), role)
                .await?;
            if json {
                return style::print_json(&list);
            }
            println!(
                "{} {} is {} of {project}",
                style::accent("✓"),
                m.user.name,
                a(role)
            );
        }
        MembersCmd::Remove { person } => {
            let m = member(api, &project, &person).await?;
            let out = api
                .remove_project_member(&project, m.user.id.as_str())
                .await?;
            if json {
                return style::print_json(&out);
            }
            // Leaving answers `{"ok": true}`, removing someone the members left.
            if out.get("ok").is_some() {
                println!("{} Left {project}", style::accent("✓"));
            } else {
                println!(
                    "{} Removed {} from {project}",
                    style::accent("✓"),
                    m.user.name
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Fctl {
        #[command(flatten)]
        members: MembersArgs,
    }

    fn parse(args: &[&str]) -> Option<MembersArgs> {
        Fctl::try_parse_from(std::iter::once("members").chain(args.iter().copied()))
            .ok()
            .map(|f| f.members)
    }

    #[test]
    fn members_are_listed_unless_a_command_says_otherwise() {
        let listed = parse(&["-p", "web"]);
        assert!(matches!(
            listed,
            Some(MembersArgs { project: Some(ref p), cmd: None }) if p == "web"
        ));
        assert!(matches!(
            parse(&["ls"]),
            Some(MembersArgs {
                cmd: Some(MembersCmd::List),
                ..
            })
        ));
    }

    #[test]
    fn people_are_added_as_editors_unless_a_role_is_given() {
        let role = |args: &[&str]| match parse(args) {
            Some(MembersArgs {
                cmd: Some(MembersCmd::Add { role, .. }),
                ..
            }) => Some(ProjectRole::from(role)),
            _ => None,
        };
        assert_eq!(
            role(&["add", "grace@example.com"]),
            Some(ProjectRole::Editor)
        );
        assert_eq!(
            role(&[
                "add",
                "grace@example.com",
                "--role",
                "maintainer",
                "-p",
                "web"
            ]),
            Some(ProjectRole::Maintainer)
        );
        assert_eq!(role(&["add", "grace@example.com", "--role", "admin"]), None);
        assert!(matches!(
            parse(&["role", "grace@example.com", "owner"]),
            Some(MembersArgs {
                cmd: Some(MembersCmd::Role {
                    role: Role::Owner,
                    ..
                }),
                ..
            })
        ));
        assert!(matches!(
            parse(&["rm", "usr_01ABC"]),
            Some(MembersArgs {
                cmd: Some(MembersCmd::Remove { .. }),
                ..
            })
        ));
    }

    #[test]
    fn members_are_found_by_address_or_id() -> anyhow::Result<()> {
        let members: Vec<ProjectMember> = serde_json::from_value(serde_json::json!([
            { "user": { "id": "usr_1", "name": "Ada" }, "email": "ada@example.com",
              "role": "owner", "added_at": 1 },
            { "user": { "id": "usr_2", "name": "Grace" }, "email": "Grace@Example.com",
              "role": "editor", "added_at": 2 }
        ]))?;
        let name = |who: &str| find(&members, who).map(|m| m.user.name.as_str());
        assert_eq!(name("grace@example.com"), Some("Grace"));
        assert_eq!(name(" usr_1 "), Some("Ada"));
        assert_eq!(name("Ada"), None);
        assert_eq!(name("usr_3"), None);
        Ok(())
    }
}
