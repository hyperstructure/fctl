# 0102: Instances limit how long sign-ins last

2026-11-25 · accepted

## Context

A browser signed in to fctl stayed signed in while it was used, until 30
days unused; a CLI until 180 days unused; an access token as long as its
maker chose, for ever included. Once an organization requires its
provider of its domains ([ADR 0099](0099-domains-require-signing-in-with-their-provider.md)),
closing someone's account at the provider stops their next sign-in, but
not the sign-ins they have: someone who left kept fctl for as long as
their laptop's browser or CLI stayed in use. Google offers OpenID
Connect apps no back-channel logout, and its Cross-Account Protection
events, which could tell fctl of some closed accounts, take a service
account of each organization's to receive. Security
teams also ask for limits of their own: a browser that signs in again
every working day, a CLI every month, no access token older than 90
days, and no tokens made by people outside a few admins (#618).

## Decision

- **Instance admins set session policies**, beside the rules: how long a
  browser stays signed in from its sign-in however much it's used, how
  long one may sit unused (30 days, the default, at most), how long a
  CLI stays signed in from `fctl login`, the oldest an access token may
  be, and whether members make access tokens. Each limit is off by
  default but the browser's unused one, which keeps its 30 days: nothing
  changes until an admin says so.
- **A policy holds every sign-in, those made before it too.** Every
  request checks its token against it, as it checks the rules, and saving
  one deletes at once what it ends, so that nothing it ended comes back
  when it's relaxed. Admin → Sign-in and `fctl admin sign-in policies`
  say what saving ends before it saves, and letting anything last longer
  is a sensitive action.
- **Signing in again goes through the provider.** That is how a closed
  account ends: the shorter the policy, the sooner. Access tokens don't
  sign in again, so their limit and who makes them are part of the
  policy.
- **Unused means no request.** The control plane sees requests, not
  people: a tab left open on a page that updates itself counts as use,
  and the pages say so.
- **Admin tokens and service accounts' secrets aren't held to it.** They
  are machines' credentials, made by admins for a directory sync or a bot
  with an end of their own, and ending them with people's would stop the
  sync that offboards people.
- **The primary instance owner is held to it like everyone else.** A
  policy ends sign-ins and nothing more: whoever it signs out signs in
  again, the owner with their password while the provider is down.

## Consequences

- An organization that requires Google and signs browsers out after 12
  hours has someone it closed out of Google out of fctl within 12 hours,
  and out of their CLI within its limit; their access tokens last until
  their own limit unless an admin disables the account.
- A policy that shortens sign-ins signs people out the moment it's saved,
  the admin who saves it among them if theirs is older: the preview says
  so.
- Members' access tokens that go when members may no longer make them
  don't come back when they may again: their people make new ones.
- Signals from a provider that someone signed out or left (Okta's
  Universal Logout, OpenID back-channel logout, Google's Cross-Account
  Protection), which would end sign-ins at once, remain later work.
