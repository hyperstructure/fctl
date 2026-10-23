# 0039: Sensitive actions take a passkey or a code, where the primary instance owner asks for it

2026-10-22 · accepted · new runners ask too since [0046](0046-registering-a-runner-takes-confirming-its-you.md)

## Context

A browser session, a CLI sign-in or an access token is all it takes to
act as someone in fctl: whoever has one, or the computer it's on, can
invite admins, make access tokens, add their own machine as a runner of
a workspace, export someone's data or delete their projects. Two-factor
sign-in ([0022](0022-two-factor-sign-in-with-authenticator-apps.md)) and
passkeys ([0023](0023-passkeys-sign-in-on-their-own.md)) protect the
sign-in, not what happens after it. People asked that the most sensitive
actions take a passkey, or a code from the authenticator app, at the
moment they are done, and that the primary instance owner can turn that
off, which suits some self-hosted instances.

## Decision

- **Where the primary instance owner turns it on** (`verify_sensitive_actions`,
  off by default; Admin → Settings, `fctl admin set
  --verify-sensitive-actions`), the actions in
  `fctl_domain::verify::Sensitive` take the person's passkey or a code
  first: inviting and promoting admins, handing over the primary owner's
  and workspaces' primary admin roles, turning off what makes people
  prove it's them (requiring two-factor sign-in, codes on Google sign-in,
  this confirmation itself), resetting someone's two-factor sign-in,
  removing a passkey, adding one's runner to a workspace, new access
  tokens and CLI sign-ins, exporting one's data, and deleting accounts,
  projects and workspaces. Only the primary instance owner changes the
  setting, and turning it off is one of the actions.
- **What people have.** A passkey (the device verifies the person:
  `userVerification: "required"`), a code from the authenticator app, or
  a recovery code, as a sign-in's second step takes them. People with
  neither have nothing to confirm it with, and aren't asked; requiring
  two-factor sign-in gives everyone one. Not the password: a second
  factor is what a stolen session or token lacks, also when whoever has
  it knows the password.
- **For 15 minutes, for the token that confirmed.** D1 keeps when each
  token last confirmed it's its person (`tokens.verified_at`), and the
  token lookup reads it with the token, so the check costs no query. A
  confirmation belongs to the browser, CLI or token that made it: one in
  the browser lets no stolen CLI token through. A sign-in with a passkey,
  or with a second factor, counts as one, so a browser that just signed in
  that way isn't asked again.
- **The server decides, at the action.** Each handler checks right before
  it changes anything, once the action is allowed and checked
  (`Ctx::verified`), so that nobody confirms for something that then
  fails, and an action whose body decides (a role, a setting turned off)
  asks only when it is sensitive. The answer is `verification_required`
  (HTTP 403), with what to confirm and with what; clients confirm
  (`POST /api/v1/auth/verify`) and send the request again. The web app
  does that with one dialog for every request, so no screen has to know
  which actions are sensitive.
- **The CLI confirms with a code, or in the browser.** A terminal can't
  use passkeys: the CLI asks for a code from the app, or sends its person
  to approve a code in the browser, as `fctl login` does
  (`POST /api/v1/auth/verify/device`), which confirms its token instead
  of signing a new one in. Approving a CLI's sign-in is itself sensitive,
  so that a stolen browser session can't get a CLI token that way.
- **New runners don't ask.** Adding a runner its owner already has to a
  workspace asks, as people asked; registering a new one (`fctl host
  install`) doesn't, so that installing a host stays one command. Its
  token runs nothing until it runs a host.

## Consequences

- With it on, scripts that delete projects, export data or make tokens
  with an access token fail unless they confirm first (`fctl account
  verify --code`). Access tokens with limits can't confirm; of these
  actions they reach only deleting a project, which they then leave to a
  person.
- A browser that can't use passkeys, of someone whose only second factor
  is a passkey, can't do these actions; another browser, or an
  authenticator app, can.
- Someone who has a person's session and their authenticator codes, or
  their unlocked device, still gets through: this guards against a stolen
  session or token, not a stolen phone.
- Older CLIs show `verification_required` as an HTTP 403 with its
  message, since they don't know the error code, and can't confirm; the
  browser, or an updated CLI, can.
- Registering a new runner for a workspace stays a way to run its
  sessions on a machine of one's own; only its admins can, for everyone's
  sessions, as before.
