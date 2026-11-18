# 0085: Admin tokens provision people on self-hosted instances

2026-11-18 · accepted

## Context

An organization that runs fctl for its people wants their accounts to
follow its directory: an account for whoever joins, in their team's
workspaces and projects, with their name, address, picture, the address
their commits credit and the company's Claude seat, and gone when they
leave. Its identity provider or HR system does that through an API, with
a credential of its own.

fctl had nothing for it. Instance admins invite people, who make their
own accounts, and they run the instance, not what is inside workspaces
they aren't members of ([ADR 0024](0024-workspaces-hold-people-projects-runners-and-github-accounts.md)):
they can't place someone in a workspace, nor change what is someone's
own (their address, picture, commit email or Claude connection). A
person's access tokens act as them, and limited ones reach projects and
sessions alone ([ADR 0037](0037-access-tokens-and-sessions-limit-what-agents-do-on-github.md)).

On app.fctl.sh, where people from many organizations hold their own
accounts and workspaces, nobody should have that power, its admins
included.

## Decision

- **A setting of the deployment turns them on**: `FCTL_ADMIN_TOKENS=1`
  (`admin_tokens = true` in a Cloudflare instance's file; the standalone
  server passes the variable on, and refuses a value it can't read). Off,
  no admin token works and none can be made. app.fctl.sh's file doesn't
  set it, and a test checks that its rendered config never does. A
  setting of the app would let an admin turn it on from a browser.
- **An instance admin makes one** (`fctl_adm_…`, `admin_tokens` in D1),
  which takes confirming it's them where the instance asks for that
  ([ADR 0039](0039-sensitive-actions-take-a-passkey-or-a-code.md)), and
  every admin sees and revokes every one. It acts as its maker, while
  they are an active admin of the instance: demoted, disabled or deleted,
  their tokens stop working and go, as do their other tokens when their
  password changes.
- **It reaches the admin API's people endpoints alone**, by one rule in
  the router before a handler runs (`fctl_domain::admin_tokens::allows`):
  create an account, read one with its workspaces, projects and the
  Claude token a token set (never a token itself), change its names,
  primary address, commit email and status, set its picture and Claude
  token, add it to any workspace and project or take it out, make its
  password reset link, delete it, and list people and workspaces. What
  reaches past what admins do signed in takes a token: an admin in the
  browser still can't. A token changes no roles, nor the allowlist,
  invitations, the instance's settings, anything inside projects but
  who is in them, or other tokens.
- **Nobody's token changes the primary instance owner's account**, and
  only the primary owner's change admins', as only they rename, disable
  and delete admins: a leaked token can't point the owner's address
  elsewhere, reset the password and take the instance over.
- **What it does takes no confirming** on top: making it did, and it
  works unattended. The audit log records what it did as its maker, with
  which token.
- **Accounts stay on the allowlist**: a new account's address, or a new
  primary address, gets an entry of its own, as an invited one does, so
  that removing another entry doesn't disable it. A new primary address
  takes the old one's place everywhere, and the old one stops signing
  in.
- **One Claude token per person is the token's** (`provisioned`): setting
  it again replaces it in place, and the first becomes their default;
  their own connections stay theirs.

## Consequences

- A self-hosted instance can follow its organization's directory with a
  script, or with an identity provider that calls an HTTP API, and its
  people find their workspaces, projects and Claude seat ready when they
  first sign in.
- Whoever holds an admin token can take over a member's account (change
  their address, then reset their password) and place anyone anywhere:
  it is as sensitive as the identity provider's own secrets. Instances
  that don't need it leave it off.
- Self-hosted instances' admins hold a power app.fctl.sh's don't; the
  docs say which instances have it, and the web app shows it only there.
- The people endpoints take an address where they take an id, so that a
  script needs no lookup first.
