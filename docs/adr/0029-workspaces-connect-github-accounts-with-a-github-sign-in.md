# 0029: Workspaces connect GitHub accounts with a GitHub sign-in

2026-10-20 · accepted

## Context

[0024](0024-workspaces-hold-people-projects-runners-and-github-accounts.md)
made GitHub accounts a workspace's own: a workspace works in an account
the instance's one GitHub App is installed on, and its projects use that
account's repositories. But it left connecting an account to an instance
admin: a workspace's admin installed the app or named an account, and an
instance admin approved it, because GitHub's setup redirect carries an
`installation_id` that anyone can forge ("you should not rely on the
validity of the `installation_id` parameter"), so naming an installation
proved nothing.

That is too much power in the wrong place, and too little safety on an
instance with more than one team. An instance admin had to approve every
connection, and could connect any installation to any workspace: on
app.fctl.sh, where strangers share an instance, an instance admin could
point one person's workspace at another person's organization and have
that workspace's agents write to repositories its people never had access
to. The instance's one GitHub App can mint a writing token for every
account it is installed on, so the only thing standing between a workspace
and someone else's code was the instance admin's judgement. A workspace's
own admin naming an account was no safer: they, too, could name any
installed account, their own organization or not.

GitHub already knows who may touch what. The missing step was asking it,
as the person, rather than trusting a redirect or an admin.

## Decision

- **Connecting an account takes a GitHub sign-in.** A workspace's admin who
  connects an account is sent to GitHub to sign in with the instance's
  GitHub App (OAuth, PKCE). fctl trades the code for a short-lived user
  access token, which acts as that person within the app, uses it once and
  revokes it. What it reads with that token decides what the workspace
  gets; no refresh token is kept, and the token never leaves the control
  plane.
- **A workspace gets only what its admin may give on GitHub.** From the
  user token fctl reads which repositories of the installation the person
  can see and which they administer (`GET
  /user/installations/{id}/repositories`, whose per-repository
  `permissions.admin` is the signed-in user's own access):
  - **Their own account**, or **an organization whose every repository the
    app reaches and they administer all of** (as its owners do): every
    repository, also those the app reaches there later. This is the only
    way to a connection that follows the account, because only here is
    "everything, now and later" theirs to give.
  - **Otherwise**: the repositories they administer, by name, at the moment
    they connect. A connection to some repositories never widens on its
    own.
  - Named repositories (`--repo`) are kept only where the person both sees
    them through the installation and administers them; the rest are
    refused by name.
- **Nobody connects what GitHub doesn't show them.** The installation is
  found among those the user token can see (`GET /user/installations`),
  never by its id alone, so connecting reveals nothing about where else the
  app is installed, and a forged `installation_id` finds nothing. An
  instance admin has no shortcut: they connect their own workspaces'
  accounts the same way, by signing in to GitHub.
- **Instance admins no longer connect or approve.** There are no requests
  to approve. Admin → GitHub lists the app's installations and the
  workspaces that use each, and an instance admin can take an account from
  a workspace (its running sessions lose their tokens for it within the
  hour, as before) — a brake on misuse, not a way in. Only the workspace's
  own admins connect it again.
- **The instance needs the GitHub App's client secret.** Signing in to
  GitHub takes the app's OAuth client: its client ID (already configured)
  and a client secret (`GITHUB_CLIENT_SECRET`, new). Without the secret a
  workspace keeps the accounts it has but connects no new one, and the app
  needs a callback URL (`…/api/v1/github/authorized`) registered. No new
  GitHub App permissions: reading the user's installations and their
  repositories needs only Metadata, which the app already has.
- **Existing connections stand; requests go.** The migration drops the
  connections still waiting for an instance admin (their admins connect
  them again, now with a sign-in) and keeps the active ones, which have no
  recorded GitHub user. A session's tokens are still minted within a
  connection's repositories at every refresh, so nothing about what a
  running session may touch changes.

## Alternatives

- **Trust a workspace admin naming an account, without GitHub.** What 0024
  rejected for its own reason: naming an installed account proves no
  authority over it, so one workspace could still name another's
  organization.
- **Keep the instance admin's approval.** It doesn't scale past one team
  and still lets an instance admin reach into a workspace's code; the point
  of workspaces is that they don't.
- **Read the user's organization membership (`GET
  /user/memberships/orgs/{org}`) instead of repository admin.** Membership,
  even as an owner, is coarser than repository admin and needs the app to
  ask for the "Members" organization permission, which it doesn't.
  Repository admin, from the Metadata the app already has, is both finer
  and cheaper.
- **Verify once and remember the GitHub identity.** fctl keeps no GitHub
  login: the token is used once and revoked. A connection records only who
  connected it and the account they signed in as, for the audit log and
  the workspace's GitHub tab. Re-checking a person's standing is a new
  sign-in.

## Consequences

- An instance admin can no longer give a workspace access to an account; a
  workspace's admins do it themselves, and only to what they administer on
  GitHub. There is no path by which one workspace reaches another's
  repositories through the shared GitHub App.
- app.fctl.sh's admins must set `GITHUB_CLIENT_SECRET` and register the
  callback URL on its GitHub App before anyone can connect a new account;
  until then the GitHub tab says so. Self-hosted instances that linked
  repositories do the same.
- A workspace that works in some of an organization's repositories, and
  wants more, connects the organization again after an admin of the rest
  has been added on GitHub, or an owner connects all of it.
- `fctl workspaces connect` prints a link and opens it in a browser, where
  the person finishes the connection by signing in to GitHub; it no longer
  connects in one call.
- A separate, related fix stops the fleet from handing a session's GitHub
  or Claude tokens to any runner but the one the session runs on, so a
  runner of one workspace cannot ask for another workspace's session
  tokens.
