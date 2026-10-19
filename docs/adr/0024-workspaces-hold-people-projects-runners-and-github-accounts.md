# 0024: Workspaces hold people, projects, runners and GitHub accounts

2026-10-19 · accepted

## Context

An instance had one set of everything: one GitHub App whose every
installation any project could use, one pool of runners that took every
session, and one group of admins who saw every project. That fits one
team. It doesn't fit one person who builds a company's software on
app.fctl.sh, keeps private projects beside it and works on a side project
with friends in another GitHub organization: each of those has its own
people, its own machines and its own repositories, and the people in one
shouldn't see, or run their agents with, what belongs to another. People
also come with more than one email address: a company address for the
company's work and a private one for their own.

## Decision

- **Workspaces.** A workspace has members, projects, the runners its
  sessions run on, the GitHub accounts its projects use, a two-factor
  requirement, a base role and its own audit log, besides a name, an
  address (slug), a color and an avatar image. Projects belong to exactly
  one workspace, and everything in a project (sessions, transcripts,
  files, secrets) with it. Its members see its projects as their role
  there says: admins own every project; members have their role as a
  project's member, or the workspace's base role (none, viewer, editor,
  maintainer) when that is higher. New workspaces start with editor, so a
  small team works in every project; the first workspace, which the
  migration makes, starts with none, so nobody sees more than before.
  Someone can be a member of at most five workspaces.
- **One primary admin each.** A workspace has one primary admin, and the
  instance one primary instance admin (the `owner` role, as before, now
  one of them). Other admins can't demote, disable or remove them; they
  hand the role to another admin themselves and stay an admin. Deleting a
  primary admin's account passes a workspace's role to its longest-standing
  admin, or member.
- **Instance admins run the instance, not what is inside workspaces.**
  They invite people (and, at the same time, to the workspaces they are an
  admin of), manage the allowlist, the instance's settings and its
  two-factor requirement, every runner as a machine, and which workspaces
  use which GitHub accounts, and they read the instance's audit log. They
  see what each workspace is and who runs it, and can delete one, but not
  its projects and sessions unless they are a member.
- **One account, several addresses.** Each address is invited to the
  instance on its own. Accepting an invitation while signed in connects its
  address to that account instead of making a new one. Every address signs
  in, each has its own commit email, and a workspace knows its member by the
  address they joined as, which is the only one it sees. One account means
  one set of sign-in factors, so a workspace's two-factor requirement holds
  however its member signs in.
- **Workspaces require two-factor sign-in as the instance does.** For its
  members, a workspace's requirement works like the instance's: password
  sign-ins ask for the second factor (a passkey counts), the last factor
  can't be removed, and Google sign-ins ask only when the instance says so.
  Unlike the instance's, it closes only the workspace to someone without a
  factor, not their account; joining one takes a factor first.
- **GitHub accounts are connected per workspace, with an instance admin's
  approval.** The instance keeps one GitHub App. A workspace's admins
  install it on an account, or name an account it is installed on, and the
  workspace works there once an instance admin approves; an instance admin
  connects accounts directly. A connection can be narrowed to some of the
  account's repositories. Sessions' tokens are minted within the
  connection's repositories at every refresh, so a disconnected account
  stops working within the hour.
- **Runners belong to the person who registered them.** A runner runs the
  sessions of the workspaces its owner adds it to, which takes being an
  admin there: the workspace trusts the machine with its code, secrets and
  tokens. A workspace's admins take any runner out of it. Sessions start
  only on their workspace's runners, and a Claude sign-in runs only on a
  runner of the person's workspaces, or their own.
- **The first workspace takes everything that exists.** The migration makes
  one, named after the GitHub account most projects' repositories belong
  to, with everyone in it in the role they had, every project and runner,
  and the GitHub accounts the projects use. Owners beyond the first become
  admins.

## Alternatives

- **Separate accounts per address, signed in side by side** (as Google and
  Slack do). Each account would keep its own password and factors, so a
  workspace's two-factor requirement would hold only for the account that
  joined it while a weaker sign-in of another opened the same browser; the
  CLI, the feed and notifications would multiplex accounts; and nothing
  would stop someone from being five workspaces' member per address. One
  account with several addresses keeps one identity per person.
- **A workspace admin's own installation, verified by GitHub sign-in.**
  GitHub sends the installation's id to the app's setup URL, where anyone
  can forge it. Verifying that the person may manage the installation
  takes a user token and permission to read organization memberships, which
  the app doesn't ask for. Approval by an instance admin, who holds the
  app's key anyway, is the safe step until then; an instance admin's own
  request connects at once.
- **Workspaces as projects' only owners, without project roles.** Project
  roles stay: some projects in a workspace are only for some of its
  people, and the base role gives everyone the rest.

## Consequences

- Instance admins no longer see the projects and sessions of workspaces
  they aren't members of, and their instance role no longer makes them
  owners of every project.
- Project slugs stay unique per instance, so links and `fctl run -p` keep
  working; a name another workspace took gets a number.
- An account's addresses can't be merged with another account's yet:
  connecting an address that has an account of its own takes deleting that
  account first.
- Connecting a GitHub account takes an instance admin's approval for
  everyone else. The GitHub App must be public to be installed on other
  accounts, and its setup URL points at `/api/v1/github/installed` to bring
  people back to their workspace.
- Older CLIs register runners for the one workspace their user is an admin
  of; with more, `fctl host install` needs `--workspace`, which only newer
  ones have, or a token from the workspace's Runners tab (`--token`).
