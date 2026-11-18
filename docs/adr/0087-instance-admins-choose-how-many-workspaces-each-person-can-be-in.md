# 0087: Instance admins choose how many workspaces each person can be in

2026-11-18 · accepted

## Context

Someone can be a member of at most five workspaces, those they create and
those they join, and the primary instance owner of any number
([ADR 0058](0058-the-primary-instance-owner-can-be-in-any-number-of-workspaces.md)).
Five suits app.fctl.sh, where anyone may sign up and every workspace takes
its own members, projects, audit log and search index. An organization
that runs fctl for its own people can have them work across many teams'
workspaces, and five is too few there; ADR 0058 left a limit each
instance sets open, and session #1482 was asked for it.

## Decision

- **An instance setting**: `InstanceSettings::max_workspaces`, five
  (`DEFAULT_MAX_WORKSPACES`) until an admin chooses another number, or 0
  for any number. Instance admins change it under Admin → Settings →
  Workspaces or with `fctl admin set --max-workspaces 50` (`any` for 0),
  as they change the instance's other settings; it takes no confirming
  it's you, as it opens nothing to anyone but the people already on the
  instance.
- **It holds everyone ADR 0058 held**: members and instance admins, never
  the primary instance owner. `workspace_limit` reads it with the
  person's role, and creating a workspace, accepting an invitation,
  joining invited workspaces at sign-up and an admin token placing
  someone check it as they checked five, their SQL too.
- **Lowering it takes nobody out of a workspace.** Those in more stay in
  every one and create and join no more until they are in fewer, as an
  owner who hands the role over does; the error says how many to leave.
- **The workspace list says the instance's number.**
  `WorkspaceList.limit` is the setting, 0 for any number, and `unlimited`
  is true for whoever it doesn't hold: the owner, or everyone at 0. The
  web app says "As the primary instance owner, you can be in any number"
  only when the limit holds the others.
- **No ceiling but `u32`.** An instance that wants no limit chooses any
  number; one that wants a large one names it.

## Alternatives

- **A limit per person**, raised for those who need more: more to keep
  and explain, and nobody asked for it. It stays possible on top of this.
- **Only the primary instance owner changes it**, as with confirming
  sensitive actions: it guards nobody's account, and the instance's other
  admins already decide who may sign up at all.
- **`WorkspaceList.limit` absent for any number**: CLIs and web apps from
  before read it as a number, and `unlimited` already tells them there is
  room.

## Consequences

- app.fctl.sh keeps five, and every instance does until its admins
  choose otherwise.
- At any number, nothing bounds how many workspaces each person creates;
  each takes what any workspace does
  ([0056](0056-sessions-are-searched-in-an-index-per-workspace.md)).
- Control planes before this one ignore the setting and hold everyone but
  the owner to five; a CLI before it shows no line for it in `fctl admin
  settings`.
