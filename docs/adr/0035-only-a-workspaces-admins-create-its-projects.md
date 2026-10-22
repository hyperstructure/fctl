# 0035: Only a workspace's admins create its projects

2026-10-22 · accepted

## Context

A workspace keeps its projects to the people its roles open them to: its
admins own every project, and a member sees the projects they are a member
of, or every one when the workspace has a base role
([ADR 0024](0024-workspaces-hold-people-projects-runners-and-github-accounts.md)).
Its GitHub accounts are the workspace's, though: a project uses the
repositories of those accounts its owners choose, and its sessions get
tokens that write to them.

Any member could create a project and so become its owner. A member whose
roles opened one project, or none, could create another, point it at any
repository the workspace's connections cover, and start sessions whose
agents read and push there, change its workflows, and see what runs in
them. A workspace with the base role `none` or `viewer` was meant to keep
some of its work to some of its people; for its repositories it didn't.

## Decision

- **Its admins create a workspace's projects**
  (`fctl_domain::workspace::check_create_project`). A member asks an admin,
  who creates the project and adds them in the role they need.
- **Without a workspace named**, a new project goes into the only one the
  caller is an admin of; a member of workspaces they don't administer is
  told to ask an admin, or to create a workspace of their own, where they
  are its primary admin.
- **The apps offer it only to admins**: the **+** beside a workspace in the
  sidebar, **New project** on its page and in the command palette, and the
  home page's setup step. The new project page lists the workspaces the
  person administers.
- **Projects that exist stay as they are**, with the owners they have.
- **Choosing a project's repositories stays with its maintainers and
  owners**, within the workspace's connections: admins, or the project's
  owners, give those roles.

## Consequences

- A workspace's admins decide which repositories its projects use, so that
  the base role and project roles bound what its members' agents reach on
  GitHub, not only what they see in fctl.
- Members can no longer start work in a new repository on their own; small
  teams that want that make them admins, or keep a workspace each.
- A base role of `maintainer`, or a project role of `maintainer` or
  `owner`, still lets its holders point their projects at any repository
  the workspace's connections cover: give those roles to people trusted
  with all of them.
