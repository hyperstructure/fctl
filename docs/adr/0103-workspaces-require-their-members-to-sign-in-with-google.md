# 0103: Workspaces require their members to sign in with Google

2026-11-26 · accepted

## Context

[ADR 0099](0099-domains-require-signing-in-with-their-provider.md) let an
instance's admins require a provider of everyone at some domains, and
left a workspace's own requirement for later (#618). On an instance that
people of several organizations share, app.fctl.sh above all, an
organization's workspace has no instance admin of its own: it can require
two-factor sign-in of its members, but not that they sign in through its
Google Workspace, under its two-step verification and device checks, and
not once it closes their account.

A requirement that sends people to a provider from the sign-in page, or
holds them anywhere but the workspace, would let one customer claim
another's domain, so it would need the domain proved first. One that only
closes its own workspace needs nothing of the kind: its admins can
already remove anyone from it.

## Decision

- **A workspace's admins require Google of its members at some domains**
  (`workspaces.sign_in`): members who are in it as an address at one of
  them (the address it knows them by, `workspace_members.email`) open it
  only with a sign-in made with Google as such an address, or with a
  passkey alone where it says so. Their address there is the one its
  people see, and they leave the workspace if they disconnect it, so no
  member slips out by changing addresses.
- **It closes the workspace as its two-factor requirement does**: its
  projects and sessions too, and its page says why and links Google's
  sign-in. Members in it as other addresses (contractors) sign in as
  before; their other workspaces stay open.
- **It holds people's own sign-ins only.** Each request's token keeps how
  its sign-in was made (ADR 0099), and the account row the request
  carries keeps it (`UserRow::request_sign_in`). Service accounts',
  autopilots' and the control plane's own work for a member (their
  notifications) aren't held to it: their admins chose those.
- **No admin closes it to themselves**, and letting anyone in another
  way takes confirming it's you, as for the instance's rules.
- **No proof of the domains**: it closes only its own workspace. A
  workspace's own provider, or a domain whose people the sign-in page
  sends to it, needs proving the domain first, and waits for providers
  other than Google (#650, #653).
- **Google only**, the one provider today; the rule names its provider, as
  the instance's do.

## Consequences

- An organization on app.fctl.sh holds its people to its Google
  Workspace in its own workspace, and Google closing their account closes
  the workspace to them once their sign-in ends (session policies, ADR
  0102, set how soon on an instance of one's own).
- A member signed in with a password finds the workspace closed until
  they sign in with Google, then finds it open in every tab.
- A member in it as a covered address who has no Google account there
  can't open it: its admins invite them as another address, or don't
  require it.
