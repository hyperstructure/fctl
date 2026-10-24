# 0058: The primary instance owner can be in any number of workspaces

2026-10-24 · accepted

## Context

Since [0024](0024-workspaces-hold-people-projects-runners-and-github-accounts.md)
someone can be a member of at most five workspaces, and that held the
primary instance owner too: the one person whose instance it is, who set
it up, decides who its admins are and alone hands the role on. They keep
workspaces of their own and are asked into other people's, and five can be
too few for them; session #263 was asked to let them be in more.

## Decision

- **The primary instance owner has no limit.**
  `fctl_domain::workspace::workspace_limit` gives `MAX_WORKSPACES` (five)
  for every instance role but `owner`, and none for it. Creating a
  workspace, accepting an invitation and joining invited workspaces at
  sign-up check it with the person's role, and their SQL still holds the
  limit when it runs, so that two joins at once can't both take the last
  place (`?7 IS NULL OR (SELECT COUNT(*) …) < ?7`).
- **Instance admins are held to five, as members are.** They run the
  instance; its owner is the one whose it is.
- **Handing the role over keeps every membership.** An owner who hands it
  to an admin stays in every workspace they are in, and joins or creates
  no more until they are in fewer than five; the error says how many to
  leave. The new owner has no limit from then on.
- **The workspace list says so.** `WorkspaceList.unlimited` is true for
  the owner and absent otherwise, beside `limit`, which stays the five
  everyone else is held to. The web app offers **New workspace** by it,
  and Settings and the New workspace page say how many the person is in
  and that the owner can be in any number.

## Alternatives

- **A limit each instance sets** (Admin → Settings): it would change the
  limit for everyone, which nobody needed; the owner alone did. It stays
  possible on top of this.
- **Instance admins too.** They are trusted with people, runners and
  settings, but an instance can have any number of them, whom the owner
  makes: the limit would then hold members alone.
- **`limit` as the caller's own, `null` for the owner.** A CLI from before
  reads `limit` as a number and would fail to read the owner's list.

## Consequences

- Nothing bounds how many workspaces the owner creates or joins. Each
  takes what any workspace does: its members, projects and audit log, and
  its own search index
  ([0056](0056-sessions-are-searched-in-an-index-per-workspace.md)).
- An owner who hands the role over may stay in more workspaces than anyone
  else can be in, until they leave some.
