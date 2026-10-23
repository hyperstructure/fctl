# 0046: Registering a runner, or replacing its token, takes confirming it's you

2026-10-23 · accepted

## Context

A runner runs the sessions of the workspaces it serves, with their code,
the projects' secrets and the tokens of the people who start them: their
Claude access token and the sessions' GitHub tokens. Whoever holds a
runner's token holds all of that.

[0039](0039-sensitive-actions-take-a-passkey-or-a-code.md) made adding a
runner to a workspace a sensitive action, which takes a passkey or a code
where the primary instance owner asks for that, so that a stolen browser
session, CLI or access token can't do it alone. It left out registering a
new runner, so that `fctl host install` stays one command, since a new
runner's token "runs nothing until it runs a host". But the token comes
back in the answer, and running a host with it is one more command for
whoever stole the session: they registered a runner for the workspace's
sessions, or replaced the token of the runner the workspace already
trusts (`POST /api/v1/runners/{id}/token`), instead of adding one. The
action that asked guarded nothing that the ones next to it didn't give
away.

A workspace's two-factor requirement had the same hole. It closes the
workspace to a member without a second factor, so that their password
alone doesn't open it. Replacing a runner's token asked only that its
owner signed in, so their password alone got a token for every session
the runner runs there.

## Decision

- **Registering a runner for a workspace takes confirming it's you**
  (`Sensitive::AddRunner`), whether for everyone's sessions or only the
  registrant's, as adding one does. Registering one for no workspace
  (instance admins) doesn't: it runs nothing until someone adds it.
- **So does replacing a runner's token**, when the runner serves a
  workspace. The CLI asks in the terminal, as it does for other sensitive
  actions; scripts confirm first with `fctl account verify --code`.
- **Not while a workspace it serves is closed to its owner** by that
  workspace's two-factor requirement: the token is refused until they set
  up a second factor.

## Consequences

- `fctl host install` asks for a code or a passkey on instances that ask
  for confirmation, once in 15 minutes, for people who have one. People
  without one aren't asked, as for every sensitive action.
- Installing hosts from a script, without a terminal, takes `fctl account
  verify --code` first on such instances, or a token from the web app.
- A member who hasn't set up the second factor their workspace requires
  can't replace their runner's token until they do; the runner keeps
  running the sessions it has with the token it has.
