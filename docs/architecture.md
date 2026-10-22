# Architecture

fctl is a software factory: you describe work, and Claude agents do it in
isolated sandboxes on machines you own. This document is the map. Read it
before changing anything that crosses a component boundary.

```
                         ┌──────── Cloudflare, or a container ────────┐
  browser (web app) ───► │  fctl-control  (Rust → wasm Worker)        │
  fctl CLI ────────────► │   ├─ REST API  /api/v1/*                   │
  Claude apps ◄─┐        │   ├─ static web app (Workers assets)       │
                │        │   ├─ D1: users, projects, sessions, …      │
                │        │   ├─ FleetDO: runners, scheduling, feed    │
                │        │   ├─ FilesDO (per project): attachments,   │
                │        │   │  published screenshots                 │
                │        │   ├─ NotifyDO (per person): notifications  │
                │        │   └─ SessionDO (one per session): events,  │
                │        │      viewers, message delivery             │
                │        └──────────────▲─────────────▲───────────────┘
                │                       │ control WS  │ session uplink WS (one per session)
                │        ┌──────────────┴─────────────┴───────────────┐
                │        │  fctl host  (Rust daemon on your machine)  │
                │        │   ├─ drivers: apple · docker · lima · proc │
                │        │   └─ per session: sandbox + attach channel │
                │        └──────────────┬─────────────────────────────┘
                │                       │ framed JSON over exec/unix socket
                │        ┌──────────────┴─────────────────────────────┐
                │        │  sandbox (VM)                               │
                │        │   fctl-agent (supervisor, journal)          │
                └──────── │   └─ claude -p --input/output-format stream-json
       Remote Control    └─────────────────────────────────────────────┘
```

## Components

| Path | What | Runs on |
|---|---|---|
| `crates/fctl-proto` | Wire types for every protocol below. TypeScript is generated from it. | everywhere |
| `crates/fctl-domain` | Pure logic: roles and permissions, validation, tokens, scheduling, event shaping. No I/O. | everywhere |
| `crates/fctl-control` | The control plane Worker: HTTP API, auth, Durable Objects, GitHub App client. | Cloudflare, or workerd anywhere (wasm32) |
| `deploy/standalone` | The control plane without Cloudflare: a server that runs the Worker on workerd through Miniflare, its container image and examples for Docker Compose and Kubernetes. | a container, any machine |
| `crates/fctl-host` | The host daemon library: control-plane client, session manager, sandbox drivers. | macOS, Linux |
| `crates/fctl-agent` | The supervisor inside each sandbox: runs `claude`, journals events, prepares the workspace. | Linux sandboxes |
| `crates/fctl` | The `fctl` binary: CLI for people and agents, plus `fctl host …`. | macOS, Linux |
| `crates/fake-claude` | A scripted stand-in for `claude` that speaks stream-json. Used by tests and `cargo xtask dev`. | dev, CI |
| `xtask` | Development automation (`cargo xtask dev`, `check`, `gen`, `deploy`, `dist`, `publish`). | dev, CI |
| `web/` | The web app (React, Vite, TypeScript, WebGL), installable on phones and computers. Served by fctl-control. | browser |

Three more repositories complete the picture:
[hyperstructure/design](https://github.com/hyperstructure/design)
(`@hyperstructure/design`: tokens, fonts, the WebGL scenes and the marks,
used by `web/`),
[hyperstructure/fctl-site](https://github.com/hyperstructure/fctl-site)
(fctl.sh: the landing page, these docs rendered from `docs/`, the installer
and release downloads) and
[hyperstructure/sandbox](https://github.com/hyperstructure/sandbox) (the
default sandbox image: Debian, git, gh, Python, Rust, Node.js, Bun, claude
and fctl, published to `ghcr.io/hyperstructure/sandbox`).

## Principles

1. **Subscription only.** Agents authenticate with Claude subscription OAuth
   tokens. API keys are stripped from every sandbox environment, the
   supervisor refuses to run when Claude reports an API-key credential source,
   and it interrupts a session the moment Claude reports paid overage use.
   See [Claude credentials](#claude-credentials).
2. **Your machines do the work.** The control plane (on Cloudflare, or in
   [a container of your own](#deployment)) holds state and routes messages.
   Hosts connect outbound only: no open ports, no tunnels, NAT friendly.
3. **Nothing is lost on a reconnect.** Agent events carry a sequence number
   assigned in the sandbox and are journaled there until the control plane
   acknowledges them. Messages to agents are delivered at least once and
   de-duplicated by id.
4. **One owner per secret.** The control plane owns credentials and mints
   short-lived, narrowly scoped tokens for each session. Sandboxes never see
   refresh tokens or the GitHub App key.
5. **Agent first.** Every capability is reachable from the CLI with `--json`,
   every component runs locally with one command, and the repository explains
   itself (AGENTS.md, this file, ADRs).

## Identity and access

- Users sign in with email and password (argon2id, OWASP parameters) or with a
  linked Google account (OIDC, linked from Settings or created on first
  Google sign-in when the email is allowed), or with a
  [passkey](#passkeys). With [two-factor sign-in](#two-factor-sign-in), a
  code from an authenticator app follows the password. An account has one
  or more [email addresses](#email-addresses), and any of them signs in.
- **Access is an allowlist.** Admins add exact emails or `*@domain` patterns.
  Signing up requires a matching entry; removing an exact entry disables that
  user. Inviting an address adds its entry, which revoking the invitation
  removes again while no account has the address; an entry gives the role
  it names, so demoting or removing an admin makes those of their addresses
  give a member's. The first account is created through a one-time setup link
  (`/setup#token=…`, from the `FCTL_SETUP_TOKEN` secret) and becomes the
  primary instance owner.
- **Early access**, where the deployment turns it on (as app.fctl.sh's
  does): people without an account ask for an invitation on the sign-in
  page, and admins invite them from a list ([Early access](#early-access)).
- Instance roles: `owner` (the primary instance owner, one per instance),
  `admin`, `member`. Instance admins run the instance (people, invitations,
  runners, GitHub, settings), not what is inside
  [workspaces](#workspaces) they aren't members of. Workspace roles: `owner`
  (the primary admin, one per workspace), `admin`, `member`. Project roles:
  `owner`, `maintainer`, `editor` (can start sessions and message agents),
  `viewer`. Sessions can also be shared with people in their workspace.
- Credentials are opaque random tokens with a type prefix, stored as SHA-256
  hashes: `fctl_ws_` (web session cookie `__Host-fctl`, or `fctl` over plain
  http outside localhost, where browsers refuse the prefix:
  `fctl_domain::cookies`), `fctl_cli_` (CLI,
  issued through the device flow), `fctl_pat_` (personal access token,
  which may be limited: [Access tokens](#access-tokens)), `fctl_rt_`
  (runner).
- Cookie requests that change state must come from the app's origin
  (`Origin`/`Sec-Fetch-Site` checks) and carry JSON; file uploads carry
  `application/octet-stream`, which browsers cannot send cross-site without
  asking first either. WebSocket upgrades check the origin too.
- **Sensitive actions take a passkey or a code**, where the primary
  instance owner turns that on: deleting a project, making an access token
  and the like ask the person to confirm it's them first, which then lasts
  15 minutes for the browser, CLI or token that confirmed
  ([Confirming it's you](#confirming-its-you)).

### Early access

Where the deployment turns it on (`early_access` in its file in
`deploy/instances`, which sets `FCTL_EARLY_ACCESS`; app.fctl.sh's does,
and the standalone server doesn't take it), the sign-in page links to
`/early-access`, where people without an account leave their email address
(`POST /api/v1/access-requests`). The answer is the same whether the
address asked before, was invited or has an account, so that it tells
nobody who uses the instance: an address that has an account isn't
recorded, and asking again moves the request's `asked_at`. A rate limit per
client address applies, as for sign-ups. The answer has a token, which the
browser keeps with the address, so that the page says it is on the list
when they come back and can withdraw the request (`POST
/api/v1/access-requests/withdraw`): D1 keeps its SHA-256 for an address's
first request only (`withdraw_hash`), so that nobody withdraws someone
else's by asking with their address, and the others' tokens look the same
and withdraw nothing.

Instance admins see the requests under Admin → Early access, and with
`fctl admin early-access` (`GET /api/v1/admin/access-requests`): those
waiting first, longest waiting first, then those invited, with whether
their link still works. Inviting one is an instance invitation like any
other (`POST /api/v1/admin/invites`), which works once for 14 days; the
list shows its link, to copy or to send from the admin's mail app (a
`mailto:` link) where the instance sends no email, and the invitation page
greets them as having asked. Any invitation to the address marks its
request invited, and by whom. A request goes once its address has an
account (the hourly cron trigger), when an admin removes it (`DELETE
/api/v1/admin/access-requests/{id}`, in the audit log) or the browser that
asked withdraws it (which the audit log doesn't record), or a year after it
last asked (`access_requests`).

### Two-factor sign-in

People turn it on in Settings, or with `fctl account two-factor enable`,
with an authenticator app: TOTP (RFC 6238) with HMAC-SHA1, six digits and
30-second steps, the parameters every app takes
(`crates/fctl-domain/src/totp.rs`). The CLI signs in through the browser
(the device flow), so two-factor happens there
([ADR 0022](adr/0022-two-factor-sign-in-with-authenticator-apps.md)).

1. **Setting it up.** The control plane makes a 160-bit secret and returns
   it as a QR code, an `otpauth://` link (which phones open in their
   authenticator app) and a key to type in, together with the secret sealed
   for that person (AES-GCM, bound to `totp-setup:<user>`, for 15 minutes).
   Nothing is stored until a code from the app confirms it. Then D1 keeps
   the secret sealed again (`authenticator_apps`, bound to `totp:<user>`),
   and the first app comes with ten recovery codes, of which D1 keeps the
   SHA-256. Setting up an app takes the password unless this browser
   signed in within the last 15 minutes (with a CLI or access token, it
   always does). Without a password, a first app takes that sign-in, which
   nothing stands in for: its codes then add passkeys, so a stolen session
   or token must not set one up (`two_factor::setup_proof`). Replacing the
   app, turning two-factor off and getting new recovery codes take a
   current code, from the app or a recovery code.
2. **Signing in.** A sign-in with a password then starts no web session. It
   leaves a challenge (`sign_in_challenges`, for ten minutes) that the
   browser holds in the `__Host-fctl-2fa` cookie, and the page asks for a
   code: `POST /api/v1/auth/two-factor/verify` takes one from the app, for
   the time steps around now, or a recovery code, and signs in. Each works
   once: codes of a time step up to the last one used are refused
   (`last_step`). A Google sign-in asks for a code when the person chose
   that, or the instance's admins did for everyone, and lands on
   `/two-factor`; otherwise Google's own sign-in is enough. A password reset
   asks for a code too, and its new password takes effect only once the
   code checks out.
3. **Wrong codes.** Five in a row make codes from the app wait a minute,
   and each further one twice as long, up to an hour
   (`fctl_domain::two_factor::lockout_ms`), besides the rate limits per
   address and account. Recovery codes, which can't be guessed, still work
   meanwhile, so whoever has the password can't lock the person out, and
   the person gets an email (when email is set up): whoever it was knew the
   password.

A workspace's admins require it of its members (the workspace's Settings,
or `fctl workspaces set --require-two-factor on`): for them, it works as
the instance's requirement below, except that it closes the workspace to a
member without a second factor, not their account (`two_factor::workspace_blocked`).
Their sign-ins ask for a passkey as the second factor too, and their last
factor can't go while they are in it (`two_factor::with_workspaces`).
While the instance requires it, every workspace does, and their admins
can't turn it off (`two_factor::workspace_requires`,
`workspace::check_two_factor`). Someone without a second factor then finds
their whole account closed, not only the workspace; once the instance stops
requiring it, each workspace's own choice applies again.

Admins require it of everyone (Admin → Settings → Sign-in, or `fctl admin
set --require-two-factor on`): of every account that signs in with a
password and, once Google sign-ins ask for codes too
(`--two-factor-google`), of every account. There, a [passkey](#passkeys)
counts as the second factor too, after the password or Google, in place of
a code. An account it is required of that has neither an authenticator app
nor a passkey can do nothing else until it has one: the control plane
answers its other requests, from browser sessions and CLI and access tokens
alike, with `two_factor_setup_required` (`api::two_factor::open_while_required`),
and the web app shows the page that sets it up instead of the app. Admins
reset it for someone who lost their phone and recovery codes (`fctl admin
reset-two-factor`; only the primary instance owner resets an admin's, and
nobody theirs), which removes their authenticator app and recovery
codes, and their passkeys where those are the second factor required
(unless a passkey is their only way in). Turning it on, replacing the app, turning it off, an
admin's reset and a sign-in with a recovery code go in the audit log, and
email the person when email is set up.

### Passkeys

Passkeys are WebAuthn credentials that the person's device or password
manager keeps and finds by itself (discoverable), and that sign in after a
fingerprint, face or screen lock: two factors in one, which no other site
can use. fctl asks for no attestation, takes ES256, EdDSA and RS256 keys,
and belongs to the host name of `FCTL_PUBLIC_URL` (its relying party id).
`fctl_domain::webauthn` checks what browsers send (with a small CBOR
decoder, `fctl_domain::cbor`), and WebCrypto checks the signatures
(`crypto::verify_passkey`).

- **Making one** (Settings → Passkeys, the page that requires two-factor,
  or the card after a sign-in without one): `POST /api/v1/me/passkeys/options`
  returns WebAuthn's creation options, with a challenge of 256 random bits
  that works once, for ten minutes (`passkey_challenges`, by its SHA-256).
  It takes the password unless this browser signed in within the last 15
  minutes, or without a password a code from the authenticator app: a
  passkey signs in without either, so a stolen session must not add one.
  `POST /api/v1/me/passkeys` checks the answer (the challenge, the origin,
  the relying party, user presence and verification) and keeps the public
  key (`passkeys`). The authenticator's model (AAGUID) names it: "1Password",
  "Apple Passwords"; otherwise the browser's suggestion, "Chrome on macOS".
- **Signing in.** The sign-in page asks for options on load
  (`POST /api/v1/auth/passkey/options`), and again before they run out.
  Their challenge is signed with the master key rather than stored, so
  that page loads write nothing; it is recorded once used, so it works
  once. The
  browser offers the person's passkeys in the email field's suggestions
  (conditional mediation, `autocomplete="username webauthn"`), and **Sign in
  with a passkey** opens its passkey sheet with the same options.
  `POST /api/v1/auth/passkey` checks the signature with the stored key and
  the counter, which must move on unless it stays 0 (synced passkeys' do),
  and signs in without a second factor.
- **As a second factor**, where two-factor is required, or for someone who
  has an authenticator app too: the step after the password offers it
  (`POST /api/v1/auth/two-factor/passkey`, then
  `POST /api/v1/auth/two-factor/verify` with the passkey's answer).
- **Confirming it's you** before sensitive actions, where the instance
  asks for it ([Confirming it's you](#confirming-its-you)):
  `POST /api/v1/auth/verify/passkey`, then `POST /api/v1/auth/verify`
  with the passkey's answer.
- **Signals.** Where browsers have them, the app tells the person's passkey
  provider which passkeys the account has, what the account is called, and
  which passkey fctl doesn't know any more (`signalAllAcceptedCredentials`,
  `signalCurrentUserDetails`, `signalUnknownCredential`), so that it stops
  offering removed ones. Settings gives the account's email and name only
  when they changed since this browser last gave them or made a passkey
  with them (it keeps a hash of them for each account): Apple's Passwords
  notifies the person of each time, changed or not.

Adding and removing a passkey go in the audit log, and adding one emails
the person when email is set up. A passkey that is the account's last way
in, or the second factor the instance requires, can't be removed until
another takes its place ([ADR 0023](adr/0023-passkeys-sign-in-on-their-own.md)).

### Confirming it's you

Where the primary instance owner turns it on (Admin → Settings → Sensitive
actions, or `fctl admin set --verify-sensitive-actions on`; off by
default), sensitive actions take the person's passkey or a code from their
authenticator app first, so that a stolen browser session, CLI or access
token can't do them alone
([ADR 0039](adr/0039-sensitive-actions-take-a-passkey-or-a-code.md)).
`fctl_domain::verify::Sensitive` lists them:

- inviting someone to the instance as an admin, or allowing an address
  as one, making someone an admin, and handing over the primary instance
  owner's role;
- turning off what makes people prove it's them: requiring two-factor
  sign-in, of the instance or a workspace, asking for codes on Google
  sign-in, and this confirmation itself; resetting someone's two-factor
  sign-in, and removing a passkey;
- adding one of your runners to a workspace, or letting it run
  everyone's sessions there (registering a new one, as `fctl host install`
  does, doesn't ask);
- making someone a workspace's admin, or inviting them as one, and
  handing over or giving a workspace's primary admin role;
- making an access token, and approving a CLI's sign-in;
- exporting your data, and deleting your account, someone else's, a
  project or a workspace.

Only the primary instance owner turns it on or off, and turning it off is
one of those actions (`verify::check_verify_setting`). Turning off your
own authenticator app, replacing it and new recovery codes take a current
code already, and adding a passkey or an app the password or a recent
sign-in ([Two-factor sign-in](#two-factor-sign-in)).

1. **Who it asks.** People with a passkey or an authenticator app
   (`verify::required`). People with neither have nothing to confirm it
   with, and aren't asked: requiring two-factor sign-in gives everyone one.
2. **Where.** Each sensitive action's handler checks it once the action is
   allowed and checked, right before it changes anything
   (`Ctx::verified`), so that nobody confirms for something that then
   fails; actions whose request decides (a role, a setting turned off) ask
   only when they are sensitive. A browser, CLI or token that hasn't
   confirmed within the last 15 minutes gets `verification_required` (HTTP
   403), which says what to confirm and with what. D1 keeps when each
   token last confirmed (`tokens.verified_at`), and the token lookup reads
   it with the token, so the check costs no query.
3. **How.** `POST /api/v1/auth/verify` takes a passkey's answer to
   `POST /api/v1/auth/verify/passkey`'s options (a challenge for that
   person, `verify`, which the device must verify them for), a code from
   the app or a recovery code, used up and counted towards the wait after
   wrong ones as at sign-in. The token that sent it counts as confirmed for
   15 minutes, and the audit log records it (`user.verified`). A browser
   that signs in with a passkey, or with a second factor after the
   password or Google, counts as confirmed from its sign-in
   (`auth::start_web_session`). `GET /api/v1/auth/verify` says whether
   sensitive actions ask the caller, with what, and until when they count
   as confirmed.
4. **The web app** answers `verification_required` with one dialog, the
   passkey first where the browser can use one, then the app's code or a
   recovery code, and sends the request again once the person confirmed
   (`web/src/lib/api.ts`, `components/two-factor/verify.tsx`); requests
   that wait at the same time share it. The export, a download the
   browser makes, asks first.
5. **The CLI** asks in a terminal, and sends the request again: for a
   code from the app, or, for someone with passkeys, to approve a code in
   the browser, as `fctl login` does (`POST /api/v1/auth/verify/device`,
   the `/device` page). Approving that code confirms the CLI's token
   instead of signing a new one in, and the device flow's poll answers
   `verified`; such a code is its person's alone. Without a terminal the
   command fails with the server's answer, and `fctl account verify --code`
   confirms first. Access tokens with limits can't confirm; of these
   actions they reach only deleting a project, which they leave to a
   person.

### Access tokens

People make personal access tokens (`fctl_pat_`) for scripts, CI and bots
in Settings → Access tokens or with `fctl token`, and list and revoke them
there or with `fctl tokens`. A token may be limited, in three ways
([ADR 0037](adr/0037-access-tokens-and-sessions-limit-what-agents-do-on-github.md)),
which D1 keeps with it (`tokens.scope`, `fctl_domain::limits::Limits`):

- **What it does** (`access`): `read` reads projects and sessions as a
  viewer does, with their transcripts, changes, memory and retrospectives;
  `run` also starts sessions, messages and interrupts their agents, stops
  and resumes the sessions its person started and keeps projects' memory,
  as an editor does; `full` does whatever its person may in its projects.
- **Where** (`places`): every project its person sees, or the projects of
  some workspaces (those made later too) and some projects.
- **What its sessions' agents may do on GitHub** (`github_access`), for a
  token that runs sessions: the sessions it starts get at most that
  ([GitHub access](#github-access)), and a session whose agent started able
  to do more is read only to it, so that a token for review agents can't
  steer an agent that pushes, also once its project holds that agent
  lower, while its sandbox still has the tokens it started with
  (`SessionRow::started_github`). A token that does everything can't have
  this limit (`limits::check`): it could change its projects' settings,
  and so what every agent there does.

A token with any limit reaches projects and sessions alone. The router
refuses it everything else before a handler runs (`limits::allows`): the
account and its sign-in methods, Claude connections, workspaces' people,
settings, runners and GitHub accounts, notifications, other tokens and
administration, so that a token handed to CI can't make itself another.
`GET /auth/me` shows it whose it is (their name and primary address) and
the token itself, nothing more.
Its role in a project is its person's, at most a viewer's (`read`) or an
editor's (`run`); in a session, its person's with their project role so
capped, and at most a viewer's for a token that reads
(`limits::session_role`). Lists of projects, sessions and workspaces keep
to its places in SQL (`access::places_sql`), so that their pages stay
full, and sockets opened with it check its limits again as they check its
person ([Viewers](#viewers)); it opens no feed. Limits that don't read as
limits, as a newer version's might, reach nothing. A place that is deleted, or
that its person no longer sees, drops out of its reach, and a token whose
places are all gone reaches nothing. Its limits hold however it comes, as
a bearer token or in a cookie. Browser sessions and the CLI's sign-ins
have none. A control plane from before limits ignores them and makes a
token without any, so `fctl token` revokes such a token and says so. The
audit log records each token's limits as it is made, and which token
started a session.

## Workspaces

A workspace is the people who work together, their projects, the runners
their sessions run on and the GitHub accounts their projects use
([ADR 0024](adr/0024-workspaces-hold-people-projects-runners-and-github-accounts.md)).
It has a name, an address (`/w/<slug>`), a color and an avatar image (at most
256 KB, which the web app makes 256 px square; `GET
/api/v1/workspaces/{id}/avatar`, at a URL that changes with it). Someone can
be in at most five (`fctl_domain::workspace::MAX_WORKSPACES`), and anyone on
the instance creates one, as its primary admin.

- **Who sees what.** Every project belongs to one workspace, and only its
  members see it (`fctl_domain::workspace::project_role`): its admins own
  every project; a member has their role as the project's member or the
  workspace's base role (`none`, `viewer`, `editor`, `maintainer`),
  whichever is higher. New workspaces start with `editor`. Starting a
  session, a share or a project role counts for nothing once someone leaves
  the workspace, or while its two-factor requirement closes it to them.
  Starting a session makes someone its owner only while they may start
  sessions in its project: made a viewer of it, or taken out of it, they
  have the session as its viewers do, or not at all, so that they neither
  steer its agent nor resume it with the project's secrets and new tokens
  (`fctl_domain::perm::session_role`). Nobody shares a session with
  themselves, and a share someone gave themselves before counts for
  nothing: it would outlast the role that let them make it.
  The project and session queries join the caller's membership
  (`model::PROJECT_SELECT`, `PROJECT_VISIBLE`).
- **Roles.** Admins invite and remove people, change roles, choose the
  runners and GitHub accounts, create its projects
  ([ADR 0035](adr/0035-only-a-workspaces-admins-create-its-projects.md)),
  and change the settings; the primary admin
  also deletes the workspace and hands the role to another admin. Nobody
  else can demote or remove the primary admin, who leaves only after
  handing the role over (`fctl_domain::workspace::check_*`). Once the
  primary admin's account is disabled or gone, an instance admin gives
  the role to one of its members (`fctl admin set-primary`), themselves
  only where they are one of its admins already, since they can disable
  anyone (`check_assign_primary`).
- **Invitations** (`workspace_invitations`) are for an address and last 30
  days. Whoever has it (an account's, or connected to one) sees it on their
  Agent Floor and in `fctl workspaces`, and joins as that address. A
  workspace's admins invite any address and get the same answer whether it
  can sign in to the instance, has an account or is another address of a
  member, so that inviting tells them nothing about who uses the instance:
  the email goes out after the answer, and only to an address that can sign
  in and isn't a member's. One the instance doesn't admit yet waits for an
  instance invitation; instance admins, who see who can sign in anyway, are
  told so, and send the instance's invitation together with their
  workspaces'. People invited before they had an account join when they
  make it, or connect the address, as far as the limit and two-factor
  allow; the rest keep waiting.
- **Your preferences.** Each member chooses the Claude connection their
  sessions in it run on (`workspace_members.credential_id`), else their
  default one, so that a company's seat pays for the company's work. They
  also put their workspaces, and each one's projects, in the order they
  want, by dragging them in the app's sidebar or with `fctl workspaces
  order` and `fctl projects order` (`PUT /api/v1/me/workspace-order`, and
  `project_order` in `PATCH /api/v1/workspaces/{id}/membership`). The
  membership keeps it (`workspace_members.position`, `project_order`), and
  lists of workspaces and projects come in it (`model::WORKSPACE_ORDER`,
  `PROJECT_ORDER`): what someone hasn't placed follows, oldest first, so
  nothing moves unless they move it. Which workspaces they fold in the
  sidebar stays in their browser, and so does a copy of what the sidebar
  last showed them (names, colors and avatars, their role, each project's
  active sessions; `web/src/lib/sidebar.ts`): when the app opens, the
  sidebar shows it at once, until the lists of workspaces and projects
  have both come. Signing out forgets it.
- **Audit log.** What happens in a workspace (people, roles, runners, GitHub
  accounts, settings, projects, sessions) goes in its log
  (`audit_log.workspace_id`), which its admins read; the rest is the
  instance's.
- **Feeds.** A feed's identity holds the projects its person saw when it
  opened, and the workspaces where they see every project
  (`internal::FeedIdentity`). When someone's workspaces or project roles
  change, FleetDO recomputes it on their sockets (`/workspaces-changed`)
  and sends `workspaces`, after which the app fetches them again; runners
  and their updates go only to the people of their workspaces, their owners
  and instance admins. It also checks a feed's token and recomputes what it
  shows on its pings, at most once a minute (`recheck_feed`), and closes a
  feed whose token no longer signs anyone in, as SessionDO does with
  [viewers](#viewers).
- **Deleting** a workspace (its primary admin, or an instance admin, typing
  its name) deletes its projects with their sessions and files, as deleting
  a project does, once none of its sessions runs.

### Email addresses

Each address is invited to the instance on its own. Accepting an
invitation while signed in (`POST /api/v1/invites/{token}/connect`)
connects its address to that account instead of making a new one
(`user_emails`). That takes a browser sign-in from the last 15 minutes,
and the account's primary address is emailed about it, since the address
then signs in and receives password reset links: an invitation link
forwarded to someone else would otherwise give its holder their account.
Every address signs in, and has its own commit email
([Commits](#commits)); the account's primary one (`users.email`) is where
fctl writes to the person. A workspace knows its member by the address they
joined as (`workspace_members.email`), the only one its people see:
project members, shares and invitations look people up by it
(`api::auth::member_by_email`). Disconnecting an address
(`DELETE /api/v1/me/emails/{email}`) leaves the workspaces joined as it.

### GitHub accounts

The instance has one GitHub App. A workspace uses an account the app is
installed on once one of its admins connects it (`workspace_github`), with
every repository the app reaches there or only some
([ADR 0029](adr/0029-workspaces-connect-github-accounts-with-a-github-sign-in.md),
[ADR 0031](adr/0031-admins-choose-a-github-accounts-repositories-after-signing-in.md)).
Connecting takes a GitHub sign-in: the admin is sent to GitHub to sign in
with the app's OAuth client (PKCE), and the user access token that comes
back decides what the workspace may get, because the app's tokens can write
and GitHub's setup redirect can name anyone's installation. fctl reads
which of the installation's repositories the person can see and administers
(`GET /user/installations/{id}/repositories`), and `fctl_domain::github::offer`
turns that into what they may connect: every repository of their own
account, or of an organization whose every repository the app reaches and
they administer all of (as its owners do); otherwise the repositories they
administer, by name. The installation is found among those the token can
see (`GET /user/installations`), never by id alone, so connecting reveals
nothing about where else the app is installed; a sign-in that names no
account offers every installation the token sees (at most 20). fctl uses
the token once and revokes it. What it showed waits ten minutes for the
admin to choose from (`github_offers`, theirs alone): the workspace's
GitHub tab lists it, with what the workspace works in there now checked,
else all of it, and connects what they choose (`POST
/api/v1/workspaces/{id}/github/offers/{account}`), which must come from the
offer (`fctl_domain::github::choose`). A connection records who connected
it and the account they signed in as (`api::github`). Repositories named
before the sign-in (`fctl workspaces connect acme --repo web`) are
connected at once instead, as far as GitHub says the admin may
(`fctl_domain::github::grant`). The sign-in's details ride in a signed
cookie (`__Host-fctl-github`), its PKCE state checked on the way back.

A connection is to an account by GitHub's id (`workspace_github.account_id`),
since a login can change hands: an account renamed or deleted, and its
name taken by someone else, who installs the app there. The control plane
mints a session's tokens, links and creates repositories, and tells
sessions what pull request lookups and webhooks say only through the
installation on a login that still belongs to the account the workspace
connected (`workspaces::installation`, `fctl_domain::github::binding`).
Otherwise sessions fail, and links refuse, saying that the workspace's
admins connect the account again, by its name now. A connection from
before ids were kept takes the account of the installation it recorded,
when GitHub still has it, else the one that has the login, the first time
it is used.

Installing the app from a workspace, or giving it more repositories, goes
through `/api/v1/github/install`, which keeps the workspace in a signed
cookie (`__Host-fctl-github-install`, for an hour) on the way to GitHub.
GitHub sends people to the app's setup URL (`…/api/v1/github/installed`)
after they install the app, with that state, and after they change an
installation (Redirect on update), without it: there the cookie stands in.
The GitHub sign-in for the installation GitHub names follows, and the
browser comes back to the workspace's GitHub tab to choose. A setup URL
reached without either (the app installed or changed on GitHub itself)
goes to the GitHub tab of the person's workspace that uses the
installation, or of the only one they're an admin of, where they connect
with GitHub.

Signing in to GitHub needs the app's client ID and secret
(`GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`), the callback URL
`…/api/v1/github/authorized` and, to come back from installing it, the
setup URL with Redirect on update; without the first two a workspace keeps
the accounts it has but connects no new one (`can_connect` is false). Instance admins
connect and approve nothing: Admin → GitHub lists the installations and the
workspaces that use each (`fctl admin github`), and an instance admin can
take an account from a workspace, not give one. Project repositories must
be within the workspace's connections (`fctl_domain::repos::check_scope`),
and every token a session gets is minted within them (`repos::narrow`), for
the runner the session runs on only, so that disconnecting an account takes
it from running sessions at their next refresh, within the hour. New
repositories are created in an organization the workspace uses all of.

### Runners and workspaces

A runner belongs to the person who registered it (`runners.created_by`) and
runs the sessions of the workspaces it is added to (`runner_workspaces`):
everyone's there, or only its owner's (`personal`). A workspace trusts a
runner for everyone's sessions with their code, secrets and the tokens of
the people who start them, so its owner adds one only as an admin of the
workspace; a member adds theirs for their own sessions, which show it
nothing they couldn't read in the session
([ADR 0036](adr/0036-members-run-their-sessions-on-their-own-runners.md)).
Admins add theirs for everyone's sessions unless they ask otherwise
(`--personal`), and adding a runner again changes whose; a demoted admin's
runners run only their own sessions from then on. A workspace's admins take
any runner out of it; its owner and instance admins rename, drain, disable
and remove it.

A session runs as the owner of its Claude connection. A member may keep
their sessions in a workspace on runners they registered
(`workspace_members.own_runners`, **Only on my runners** in its Settings,
`fctl workspaces own-runners`): no one else's machine then sees their
Claude token or their sessions' work. FleetDO reads which workspaces each
runner serves, and for whom, before it dispatches, and starts a session
only on a runner of its workspace that runs the sessions of its person
(`Slot.serves`, `fctl_domain::schedule::runs_for`). A session that no
online runner takes says why: the workspace has no runner, none online,
only its members' own, or none of its person's, when they keep theirs on
their own runners. A runner serves a workspace while its owner is an active
member there (`serves!`). Once it stops running a session (taken out of the
workspace, its owner left, was disabled, or the session's person turned on
own runners), the session stops and resumes on another runner, whatever
its host does: FleetDO stops it (`/runner-changed`), closes the runner's
uplink to it, and the runner gets no credentials, uplinks, files or
screenshots for it any more (`spec::runner_serves`). Disabling a runner or
replacing its token closes its control connection and its sessions'
uplinks. A Claude sign-in runs on one of the person's own runners when one
is online, else on a runner that runs their sessions in one of their
workspaces.

The Agent Floor tells the person who registered a runner when it is online
with an older fctl than the control plane: the version in its hello against
the one `GET /api/v1/instance` reports, which on app.fctl.sh is the latest
release, since each release redeploys it. It shows the command that updates
the runner, `fctl self update`. Skipping the notice hides it for that
version of the control plane, in that browser (localStorage, per person).

## Data

**D1** (`crates/fctl-control/migrations`) holds relational state: users and
their email addresses, identities, two-factor sign-in (sealed authenticator
secrets, hashed recovery codes, sign-ins waiting for a code), passkeys'
public keys and their challenges, allowlist, early access requests, auth
tokens (what access tokens are limited to, and when each last confirmed
it's its person), device codes, workspaces
(with their avatar images), their members,
invitations, GitHub
accounts (and what GitHub sign-ins offer to connect, while admins choose)
and runners, projects, their members and
repositories, sessions (summary rows), which sessions list which pull
request, shares, who follows which session,
notification settings and browsers (push subscriptions), runners, encrypted
credentials and project secrets, the latest plan usage of each Claude
credential, the links of published screenshots, projects' memories and
sessions' retrospectives, the instance's settings and sealed Web Push key,
and the audit log.

**SessionDO** (one per session, SQLite storage) holds the event log and the
live state of a session: connected viewers, the runner uplink, pending
deliveries, and the in-progress streaming message. It writes summary changes
through to D1.

**FleetDO** (one per instance) holds runner control connections, assigns
sessions to runners, and fans out session summaries to dashboard viewers.

**FilesDO** (one per project, SQLite storage) holds the files people attach
to messages, in 1 MiB chunks, and which session each was sent in (see
[Attachments](#attachments)), and the screenshots its agents publish until
they expire (see [Published screenshots](#published-screenshots)).

**NotifyDO** (one per person, SQLite storage) holds their notifications
while they wait to go out, and forgets them once sent (see
[Notifications](#notifications)).

**Deletion.** Deleting a project deletes its rows (sessions, members, secrets,
shares, its memory and its sessions' retrospectives cascade) and, in the
same batch, queues its sessions in
`session_purges` and the project in `file_purges`; their Durable Object
storage, the event logs and the attached files and published screenshots,
is deleted right after. An hourly cron trigger (`src/housekeeping.rs`; the
standalone server runs it outside Cloudflare) retries what failed and
deletes expired tokens, password resets, sign-ins waiting for a code,
passkey challenges, device codes and invites, early access requests whose
address has an account or that are a year old, the links of expired
published screenshots, abandoned Claude logins, and audit
log entries older than a year. It also looks up pull requests that GitHub
hasn't told about for a while
([Pull request state and checks](#pull-request-state-and-checks)).
Deleting a workspace does the same for all its projects. Deleting an
account (`src/api/account.rs`) does it for the workspaces only that
account is in, passes the primary admin role of its
other workspaces to their longest-standing admin, or member, deletes its
personal rows (its addresses too) in the same batch, and keeps a nameless
placeholder row only while sessions or projects it created remain.

## Sessions

A session is one Claude conversation in one sandbox, on a branch of the
project's repository (and the same branch in the project's other
repositories it changes; see [Repositories](#repositories)).

```
queued → provisioning → preparing → running ⇄ idle → stopped
   ▲                                   │        │        │
   │                                   └─ paused (usage limit) ─┘   failed (any)
   └──────────── a message or a resume ──────────────────┘
```

1. `POST /api/v1/projects/{id}/sessions` creates the row and asks FleetDO to
   schedule it, on a runner of its workspace that runs or builds its
   project's image, gives its sandbox the size it asks for and has the free
   disk its host keeps. Without a free one it stays `queued`, and says why.
2. FleetDO builds the `SessionSpec` just in time (repositories, branch, agent
   settings, secrets, fresh GitHub installation tokens, the starter's Claude
   credential) and sends `start_session` to a runner.
3. The host creates a sandbox with its driver (building the project's image
   first when it builds one from its repository and the files changed),
   injects `fctl-agent`, starts it detached, attaches to it, and opens the
   session uplink to SessionDO.
4. fctl-agent clones the primary repository, creates the branch, runs the
   project's setup script, checks that Claude Code is 2.1.101 or later,
   writes Claude's configuration over the image's, and starts
   `claude -p --input-format stream-json --output-format stream-json`.
5. Claude's messages flow back as events; messages from people flow down as
   deliveries. A finished turn makes the session `idle`; the sandbox stays up
   until the idle timeout, then stops, and a message resumes the session
   (`claude --resume`; see [Idle sessions](#idle-sessions)).

### Idle sessions

A session holds a slot on its runner from the moment it starts until its
sandbox stops. One that waits for a message stops after its project's idle
timeout (`idle_timeout_minutes`: ten minutes unless the project sets
another), so that its runner takes other work. fctl-agent ends Claude, and
the host stops the sandbox and keeps it (see
[Sandboxes and drivers](#sandboxes-and-drivers)).

Background work keeps a session up: a shell command Claude runs with
`run_in_background`, a subagent, a Monitor
([ADR 0018](adr/0018-background-work-keeps-a-session-up-for-an-hour.md)).
Claude Code reports each task starting (`system/task_started`) and ending
(`system/task_notification`), and fctl-agent follows them
(`crates/fctl-agent/src/tasks.rs`). While any runs after a turn, the
session waits for it ("Waiting for 2 background tasks"). When background
work ends, Claude Code hands Claude the result in a turn of its own.
fctl-agent counts it as any other: the session runs, and a stop or an
interrupt reaches the turn. It tells these turns by their first message:
`system/init`, which starts each turn, or the conversation's first message
after a `result`. Claude Code also reports its state
(`system/session_state_changed`, which fctl-agent turns on with
`CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS`), but not each turn: from a turn
that starts subagents in the background, it says `running` until they
have all reported and the last turn that takes their reports has ended,
and only then `idle`. So the idle timeout counts from when the last task
has ended and Claude Code has said `idle`. Background work keeps a session
up for at most an hour after its last turn, or for the idle timeout when
that is longer, so that a forgotten dev server doesn't hold the runner's
slot for ever. When a session stops with background tasks still running
(that limit, a person's stop, a drain, an uninstall), the transcript
names them, and the next message tells Claude they didn't finish. The
system prompt says how long background work may run after a turn ends.
Such a session holds its slot as any other does, and a draining runner
stops it after a minute, as it stops sessions that wait for a message.

A message to a stopped session resumes it: from the web app, `fctl send`,
`fctl attach` or the API, for anyone who may message the agent. SessionDO
queues the session again and keeps the message in its outbox for the new
run, which runs on the Claude subscription the session ran on, as a
message to an idle session does. That subscription's owner must still be
an active member of the session's workspace, as a runner's owner must
(`spec::credential_serves`): once they left it, were removed or disabled,
the session's runs and token refreshes get no credential, and it fails
with a reason instead; leaving, or being disabled, stops the sessions
that run on their subscription. A resume (**Resume** for the session's
owners, `fctl resume`) runs it on the resumer's subscription instead, and
also starts a failed session, which a message doesn't. A message that comes
while the session stops queues it too: FleetDO starts the new run once the
last one has ended, so that two runs of a session never overlap, and
SessionDO ignores what the last run's uplink still says about stopping.
Messages for a session that stops wait in the outbox for the next run
instead of going to the agent, and an interrupt for one is dropped, since
there is nothing left to interrupt. FleetDO marks a session stopping before
it asks its host to stop it. When an agent stops on its own (its idle
timeout) just as a message comes, before SessionDO knows it is stopping, the
agent leaves the message, and the session resumes for it once it has
stopped.

FleetDO remembers which runner last took each session (`sandboxes`, in its
storage) and starts a resume there, where the sandbox has Claude's
conversation and whatever the agent left uncommitted. While that runner
runs as many sessions as it may, the session waits for it, for up to ten
minutes. Then, or right away when that runner is offline or no longer
gives the sandbox the session asks for, the session starts on another
runner in a new sandbox: the agent clones the session's branch and starts
a new conversation from the session's task.

### Draining a runner

A runner that drains takes no new sessions, and the sessions it has carry on
there, so that it can be taken out of rotation (for maintenance) without
dropping anything. A host whose disk runs low takes no new sessions by
itself, without a drain ([Sandboxes and drivers](#sandboxes-and-drivers)).
Its owner and instance admins drain one from the runners list or with
`fctl admin drain`; its host drains itself with `fctl host drain`, using
the runner token (`POST /api/v1/runner/drain`). Both set the same state,
kept in FleetDO's storage with who started the drain and when, and either
can end it.

- FleetDO places new sessions on runners that don't drain; a session that
  waits because every runner online drains says so.
- A resume still goes back to the draining runner that has the session's
  sandbox ([Idle sessions](#idle-sessions)): it keeps Claude's conversation
  and needs no new sandbox.
- Sessions that wait for a message stop once they have waited a minute,
  instead of after the idle timeout. FleetDO asks each idle session it sees
  in the runner's heartbeats, and SessionDO agrees only when no message is
  on its way to the agent, and marks the session stopping in the same step,
  so that a message that comes later waits for the next run. A message
  resumes the session on the same runner. So a draining runner is empty
  whenever no turn runs.

The runners list, `fctl admin runners` and `fctl host status`
(`GET /api/v1/runner`, with the runner token) show who started the drain,
when, and the sessions that still run.

Removing a machine is the step that moves its sessions elsewhere.
`fctl host uninstall` asks the control plane which sessions run on the
host; with any, it drains the runner and waits for them (`--wait`) or
stops them (`--force`, or yes when it asks). Stopping drains the runner
with `stop_sessions`: FleetDO marks each session stopping and asks the host
to stop it (`StopReason::Shutdown`), so its agent pushes its work, and it
forgets the runner as the home of every session whose sandbox it has, so a
message resumes them on other runners. When the host isn't connected,
FleetDO stops its sessions at once and the CLI stops the sandboxes on the
machine with the driver.

### A runner that goes away

A host that stops answering (a laptop gone to sleep, a host that hangs or
crashed, a network that dropped) leaves its agents at work in their
sandboxes, journaling what they do, and the session page says since when
the runner hasn't been heard from ([Viewers](#viewers)). Once its control
connection has been closed for ten minutes, FleetDO marks its sessions
stopped, so that a message resumes them on another runner, and remembers
which ones it stopped this way (`stopped_away`, in its storage).

When the runner comes back and its hello reports one of them still
running, and no other run of it has started since, the session carries
on there. FleetDO assigns it to the runner again and gives it back its
runner, so that the host's uplink gets in and replays the agent's journal,
and the transcript says the runner is back. That includes a session
resumed meanwhile that no runner took: the agent that still runs gets the
messages. A session that runs on another runner by then, that someone
stopped or archived, or whose project was deleted, stops instead, with a
reason that says which (`fctl_domain::reconcile`).

### Repositories

A project has a primary repository and may list more of the same GitHub
account (`project_repos`), each read only or read and write, or grant read
access to every repository of the account the GitHub App reaches
(`owner/*`), all within what its workspace may use
([GitHub accounts](#github-accounts)): an owner-wide grant reads only the
workspace's repositories of the account. One installation mints a session's
tokens: one that can write (the primary repository and the writable ones),
as far as the session's agent may on GitHub ([GitHub access](#github-access)),
and one that can only read. A token carries one set of permissions, hence
two; they are refreshed together (`CredentialKind::Github`). A listed
repository the workspace may no longer use, or the installation no longer
reaches, is left out of them and marked unavailable in the spec, and the
agent posts a notice; a primary repository out of reach fails the session.

- **Credentials.** git asks fctl-agent's credential helper, which reads the
  repository path git sends (`credential.useHttpPath`): writable
  repositories get the token that can write, others the read-only one. `gh`
  holds one token per host, the one that can write.
- **Clones.** The primary repository is cloned into `~/workspace` before
  Claude starts. The system prompt lists the others, and Claude clones them
  when it needs them, into `~/repos/<owner>/<name>`.
- **After each turn** fctl-agent finds those clones by their `origin`. In a
  writable one, work on the default branch moves to the session branch
  (which Claude is told to use), new commits on it are pushed, and its pull
  request is looked up. The `git` event carries the primary repository in
  its fields and the others in `repos`.
- **Changes** are what each branch changed since it left its base: the
  diff from the merge base of the branch and `origin/<base>`, uncommitted
  and untracked work included, so the base's later commits never show as
  the session's. Once the branch's pull request merged, from the base as it
  was just before (the pull request's `baseRefOid`), so that the work it
  merged still shows, however it merged.
- **Pull requests.** fctl-agent follows every pull request of its session
  (`crates/fctl-agent/src/followed.rs`): those of the branches it looks
  up, those whose URL `gh pr create` printed, whichever branch, worktree or
  clone they came from, and those the session lists when a sandbox starts
  (the spec's `pull_requests`). It keeps them in its state file, reports
  them all in the `git` event's `pull_requests`, and looks each up again
  when a turn ends until it is merged, so the session lists them with their
  state (`fctl_domain::pulls`), and GitHub keeps that current
  ([Pull request state and checks](#pull-request-state-and-checks)). The
  system prompt tells agents to make another pull request's branch where
  the repository is checked out, not in a worktree, since Changes shows
  the checked-out branches.
- **Merged branches** aren't pushed again: GitHub deletes a merged pull
  request's branch in repositories set up to, and a push would bring it
  back. Before it pushes a
  branch whose pull request it saw open, or whose upstream a fetch pruned
  (`[gone]`), fctl-agent looks the pull request up. Commits after a merged
  one stay in the sandbox: the transcript says so, and Changes shows the
  branch merged, with the commits since (`unmerged`) not pushed
  ([ADR 0030](adr/0030-sessions-follow-every-pull-request-their-agent-opens.md)).
- **While Claude works** fctl-agent reports the same state soon after tool
  calls that can change the repositories (`Bash`, `Write`, `Edit`,
  `NotebookEdit`, a subagent's too) and when background tasks end: a burst
  costs one refresh, refreshes come at most every ten seconds
  (`crates/fctl-agent/src/refresh.rs`), and a `git` event goes out only
  when the state changed. These refreshes only read: they push nothing,
  move no work, and look up a branch's pull request only once it was
  pushed or its pushed head moved, or after `gh pr create`, along with the
  pull requests it printed. They read a
  copy of each clone's index, since `git diff` rewrites the index it reads
  under its lock, which would fail the agent's own `git commit`
  ([ADR 0027](adr/0027-agents-report-their-repositories-while-claude-works.md)).
  The session page's Changes section and the session lists take the newest
  state; the transcript shows one when a branch gets a commit, a push or a
  new pull request (`fctl_domain::branches`).

Older hosts and agents ignore the list and the second token, and work on the
primary repository alone
([ADR 0009](adr/0009-projects-span-an-accounts-repositories.md)).

### GitHub access

A session's agent pushes, comments or only reads on GitHub
(`GithubAccess`, `sessions.github_access`) in the repositories the session
may write to; it only reads the others
([ADR 0037](adr/0037-access-tokens-and-sessions-limit-what-agents-do-on-github.md)):

| Access | Its token for those repositories | The agent |
|---|---|---|
| `push`, as every session's before | contents, pull requests, issues and workflows: write | commits, pushes its branch and opens pull requests |
| `comment` | pull requests and issues: write; contents: read | reviews, comments, labels and resolves review threads; pushes and merges nothing |
| `read` | none: the read-only token covers them too | reads; writes nothing |

A session asks for one as it starts (`CreateSessionRequest.github_access`,
`fctl run --github`, the list beside the composer's model and effort), at
most what its project's settings allow (`ProjectSettings.github_access`,
`fctl projects set --github`) and the access token that starts it, and
gets as much as they allow when it doesn't ask (`limits::session_github`).
Its tokens carry what it may do now: what it started with, within its
project's setting as that is now (`limits::effective`), so that lowering a
project's setting reaches its running sessions with their next token,
within the hour, as removing a repository does. A value this version
doesn't know, stored by a newer one, reads as `read`. GitHub holds the agent to
it, since git and `gh` have no token that can do more.

fctl-agent reads it in the spec (`SessionSpec.github_access`). For an agent
that doesn't push, it pushes nothing, moves no work onto session branches
and treats the other repositories as read only, and it tells Claude what
the agent may do instead of how to push and open pull requests; the
transcript says so as the session starts. Hosts from before it leave it
out of the agent's configuration: their agents try to push, and GitHub
refuses.

GitHub lets only tokens that can write a repository's contents resolve the
review threads of its pull requests, so an agent that comments resolves
them through fctl: `fctl resolve PRRT_…` in the sandbox goes to the control
plane as an agent call (`AgentCall::ResolveThread`), as `fctl memory`
does. The control plane looks the thread up through the project's
installation, checks that its pull request is in a repository the session
may write to and its workspace may use (and refuses one elsewhere as it
refuses one that doesn't exist, naming no repository), and resolves it, or
opens it again, with a token of its own for that repository, which never
leaves the control plane and which it revokes at once
(`fctl_domain::threads`, `repos::resolve_permissions`). `~/AGENTS.md` tells agents that comment
how, where the sandbox's fctl has the command.

### Pull request state and checks

Session lists, the session page and `fctl show` give each pull request its
state, `open`, `draft`, `queued` (in a merge queue), `merged` or `closed`,
since when, and the checks of its head commit: how many passed, failed and
are still to finish. Agents look their pull requests up as turns end, but a
session that stopped looks up nothing, so GitHub tells the control plane
([ADR 0032](adr/0032-githubs-webhook-keeps-sessions-pull-requests-current.md)):

1. **The webhook.** Where the GitHub App's webhook is on, with its secret
   in `GITHUB_WEBHOOK_SECRET` ([Self-hosting](self-hosting.md#connect-github-optional-recommended)),
   GitHub posts to `/api/v1/github/webhook` when a pull request changes
   (`pull_request`: opened, pushed to, marked ready or a draft, queued,
   merged, closed…) and when the checks of a commit do (`check_run`,
   `check_suite`, `status`). A delivery counts only with the signature of
   that secret (`fctl_domain::webhooks::signed`); without the secret the
   instance takes none and sessions show what their agents last looked up.
2. **Which sessions.** D1 keeps which sessions list which pull request
   (`pull_request_sessions`), by repository and number, with its head
   commit, which commit statuses name instead of a pull request. SessionDO
   adds a session's rows as its agent finds pull requests.
3. **A lookup.** For the pull requests sessions list that a delivery
   concerns, the control plane asks GitHub with one GraphQL query
   (`fctl_domain::webhooks::query`, with a token of the installation that
   sent it): for each, its title, state, merge queue entry, times and its
   head commit's checks counted by state, each check once: GitHub counts
   every run of a check, but the pull request shows its latest run alone,
   so a run that a re-run, or the next run of its workflow for the same
   commit, replaced counts no more. Deliveries come out of order and some
   never come; a lookup says how things are. A pull request is queued
   when the delivery's action is `enqueued` (or `queued`), GitHub lists its
   entry in the merge queue, or its merge state status (else its
   `mergeable_state`) is `queued` or `in_merge_queue`. When the lookup
   fails, a `pull_request` delivery's own payload gives the state.
4. **The sessions.** The control plane sends what it found to the
   SessionDO of each session that lists the pull request
   (`/pull-requests`), which takes what is newer (`fctl_domain::pulls::update`):
   GitHub's `updatedAt` orders what GitHub and agents say of its title and
   state, an agent's `open` leaves a queued one queued (`gh` can't see merge
   queues), and checks go by when the control plane asked. It keeps when
   the state changed (`state_changed_at`): when GitHub says it merged,
   closed or joined the queue, or else when fctl saw it change. Viewers and
   the dashboards hear of it at once, and D1 within seconds.

The control plane also looks up the pull requests a session finds as it
finds them, and the hourly job up to 500 that aren't merged or closed and
that nobody asked GitHub about for six hours (`pull_requests::sweep`), in
case a delivery went missing: that also brings the pull requests of
sessions from before the webhook up to date, all in its first run. One
query asks about up to 50 pull requests of an account, for one point of
the rate limit of the app's installation there (5,000 an hour), which the
agents of its sessions share
([ADR 0033](adr/0033-the-control-plane-looks-pull-requests-up-50-at-a-time.md)):
the job leaves an account's other pull requests to its next run when
GitHub doesn't answer or the installation has fewer than 1,000 points left.
Each session hears of all of its pull requests at once, and
`pull_request_sessions` keeps a pull request's old state until every
session that lists it heard, so that the job asks again for one that
didn't. The web app shows the states and checks with GitHub's octicons and
colors (the design system's `pr-*` and `checks-*` tokens).

### Commits

Commits are authored by the GitHub App's bot (`git_author` in the spec).
Two project settings, both on unless a maintainer turns them off, decide
who else they credit with `Co-authored-by` trailers:

- **Credit people in commits** (`credit_people`): the person who started
  the session, with an address they chose: the commit email of the address
  they are in the workspace as, in Settings → Email addresses (`fctl account
  set --for <address> --commit-email …`), kept with the address and shown
  only to them. FleetDO puts their name and that address in the spec's
  `co_authored_by`; when the project credits nobody, it doesn't look the
  address up, so it never leaves the control plane.
- **Credit Claude in commits** (`credit_claude`): Claude, as Claude Code
  does by default.

fctl-agent turns them into Claude Code's `attribution` in Claude's user
settings. With both, `attribution.commit` is Claude's trailer followed by
the person's:

```
Co-authored-by: Claude <noreply@anthropic.com>
Co-authored-by: Ada Lovelace <1234+ada@users.noreply.github.com>
```

With Claude alone it stays unset, so Claude Code's own trailer, which names
the model, ends commits. Without Claude it holds only the people's
trailers, or nothing, and `attribution.sessionUrl` is `false`, so Remote
Control sessions leave out the claude.ai session link too. Agents from
before `credit_claude` always credit Claude.

GitHub links a commit to the account that has the address, and a squash
merge keeps the trailers of the commits it squashes. Claude Code's setting
reaches the commits Claude makes, not those a script makes, and a
repository's own Claude settings or CLAUDE.md can replace it. Sign-in
emails never go into commits: `co_authors`, which older control planes
filled with one, stays empty, and agents ignore it
([ADR 0011](adr/0011-commits-credit-people-with-an-address-they-choose.md)).

### Titles

A session's title starts as the first line of its task. Once the agent
understands the work it renames the session, and says in a sentence or two
what it is about, with `fctl edit` in its sandbox; it does so again when
the work turns to something else. fctl reaches fctl-agent over the session
socket, and fctl-agent keeps the title and description in the session
state it reports. SessionDO applies them when they differ from the agent's
previous report, so a person's rename stands until the agent renames the
session again, and writes them through to D1. fctl-agent tells Claude how
in `~/AGENTS.md`, which Claude's user memory (`~/.claude/CLAUDE.md`)
imports so it loads in every repository
([ADR 0007](adr/0007-agents-name-their-sessions-through-fctl-agent.md)).

### Model and effort

A project's settings name the model its sessions run on (Claude's
default; `opus`, `sonnet`, `haiku` or `fable`, Claude Code's aliases for
the latest model of each family; or a model's ID) and the effort level
(Claude's default, or `low` … `max`), which fctl-agent passes to Claude as
`--model` and `--effort`. A session can ask for others as it starts
(`CreateSessionRequest`, `fctl run --model … --effort …`), and the
composers on the Agent Floor and a project's page have a list for each.
They start on what the person asked for when they last started a session
in the project, else on the project's settings, and a session asks only
for what differs from those, so that where it takes the project's it
follows them, when it resumes too, and the next composer starts on them
again (`web/src/lib/models.ts`). D1 keeps what each person's last session
in a project asked for (`project_choices`, written as sessions start), and
projects return the caller's as `last_model` and `last_effort`. A person's
rows go when they leave the project's workspace or delete their account.

The session page and `fctl show` show the model and the effort level
Claude works at. The model comes in Claude's `system/init` message, which
in `-p` mode has no effort level, so fctl-agent asks Claude for it with
the `get_settings` control request once Claude has started, and again
after each turn, since it can change during a session: Claude Code's
`/effort high`, in a message from the person who started the session, sets
it without a model call, and so can the Claude apps over Remote Control.
The answer's `applied.effort` is what Claude sends the API, after the
session's `--effort`, Claude's settings and environment, the
organization's limits and the model's default: `low` … `max`, or none for
a model that takes no effort level. fctl-agent reports it in the session
state, and SessionDO writes it through to D1's `claude_effort`. `effort`
stays the level the session asked for, which its runs pass to Claude, so a
resumed session still follows its project's setting. Claude Code before
`get_settings`, and agents before this, report none, and the session shows
no effort level.

The Work section of the session page and `fctl show` show how full
Claude's context is, against the window and where Claude compacts the
conversation on its own, and how often it did. SessionDO counts the
context with the other counters: the input of the conversation's latest
API call (subagents' calls have contexts of their own), cached or not, as
Claude Code's status line counts it, and what a compaction left
(`compact_boundary`'s `post_tokens`) until the next call says. The window
and the threshold depend on the model, Claude's settings and environment
and its experiments (Claude Code 2.1.308 gives Opus 5.5 a window of 1M
tokens and compacts at 967K), so fctl-agent asks Claude for them with the
`get_context_usage` control request once Claude has started, and again
after each turn, since `/model` changes them. It asks for a `summary`,
which Claude answers from the last call's usage and its own estimates,
without asking the API to count each part of the context. fctl-agent
reports the answer's `maxTokens` and `autoCompactThreshold` in the session
state (`ContextWindow`), and SessionDO keeps them in the session's stats.
Claude doesn't compact before the window is full when auto-compact is off
or when it leaves that to the API (`autocompactSource` `auto`: it compacts
once the API says the prompt is too long, and its `/context` shows no
buffer for it), so fctl-agent reports no threshold then. Claude Code
before `get_context_usage`, and agents before this, report no window, and
the session shows the context alone.

### Event flow and reliability

- fctl-agent assigns `agent_seq` to every event and appends it to a journal in
  the sandbox. The host forwards events in batches; SessionDO stores them,
  assigns the canonical `seq`, broadcasts them, and acknowledges `agent_seq`.
  After any reconnect SessionDO announces the last acknowledged `agent_seq`
  and the host replays the journal from there. People's messages,
  interrupts and status changes are SessionDO's own to record: from an
  uplink, where a host or what runs in its sandbox could send them as
  anyone, they are acknowledged and dropped.
- SessionDO stores events in `agent_seq` order. After a reconnect, newer
  events can arrive before the replay of older ones, which went over a link
  to the agent or an uplink that closed: they wait for it, for at most a
  minute, rather than acknowledge the older ones unseen
  (`crates/fctl-domain/src/journal.rs`).
- Streaming deltas (`stream_event`), tool heartbeats (`tool_progress`) and
  Claude Code's word on how far the agent's thinking got
  (`system/thinking_tokens`, about once a second while it thinks, which the
  session page counts after "Thinking") are forwarded but not stored.
  SessionDO keeps the response in progress in memory so late viewers see
  it, with only the newest thinking progress: each counts the whole
  thought (`fctl_domain::events::Live`). Older agents store the thinking
  progress; SessionDO acknowledges it and drops it.
- The session's counters (`SessionStats`) move as the agent works. Each
  API call's input and cache reads count as its response starts
  (`message_start`), or with its stored messages if that stream event went
  missing, and its output with the `message_delta` that ends it. The
  input of a call of the conversation's own, cached or not, is what
  Claude's context holds. Claude
  reports each several times, so SessionDO remembers the calls it counted.
  A turn is under way while the session runs (`turn_started_at`), so
  viewers count its time up. Each turn's `result` adds its duration and
  settles the tokens on Claude's own totals (`modelUsage`), which also have
  the calls that stream nothing and the output lost while the uplink was
  down (`crates/fctl-domain/src/events.rs`).
- Host status (`provisioning`, `preparing`) is sent as idempotent state, not
  as sequenced events.
- Deliveries (messages, interrupts, stops) wait in SessionDO's outbox until
  the agent confirms them; the agent remembers delivered ids. The host keeps
  them too, and sends the unconfirmed ones again over a new link to the
  agent.

### Viewers

Browsers and the CLI open `GET /api/v1/sessions/{id}/ws`. The first frame is a
snapshot (summary, recent events, presence, in-progress text); after that
frames are incremental. Older history is paged over HTTP. The session page
pages it in as the reader scrolls up near the start of what it has, without
a press, and as it opens, too, until it has about a screenful to show: the
recent events can show next to nothing, since the tool calls in a row fold
into one line, and older agents stored Claude Code's word on how far the
agent's thinking got, about once a second.

The Worker checks access as the socket opens, and SessionDO checks it again
while it stays open: the token it was opened with and the role it gives,
on the viewer's pings at most once a minute and before every message or
interrupt (`recheck_viewer`). A viewer whose person can no longer see the
session (they left the workspace or were removed, their role or share went,
they signed out or were disabled) closes with 4403, after which the apps
stop reconnecting; one whose role changed gets it in its next `session`
frame. A viewer that stopped pinging closes after five minutes at the next
broadcast and reconnects through the Worker.

The snapshot and `runner_connected` frames say whether the agent's work can
come through:

- **The runner is connected** when its uplink is open and SessionDO heard
  from it within the last minute. Hosts ping every 20 seconds, but one that
  is frozen or cut off (a laptop gone to sleep) can leave its socket open
  for a long time. SessionDO has no timer while its sockets hibernate, so
  viewers' pings, every 25 seconds, notice an uplink that went quiet.
  Otherwise the frames say when the runner was last heard from.
- **The runner can't reach the agent** when its host says so
  (`UplinkUp::AgentLink`): once it has known for 10 seconds that no link to
  fctl-agent works, with when it last heard from it and what went wrong. Hosts
  say it again after every reconnect, and SessionDO forgets it when the
  uplink closes. Older hosts never say it.

The session page and `fctl attach` say which, and since when, once it has
lasted 10 seconds (a host that restarts reconnects within them), and show
nothing in progress meanwhile.

### Attachments

A task, message or resume can carry files: up to 10, of at most 20 MB each.

1. Each file uploads on its own, the bytes as an `application/octet-stream`
   body, into the project's FilesDO: `POST /api/v1/projects/{id}/attachments`
   to start a session, `POST /api/v1/sessions/{id}/attachments` for a
   message in one (for anyone who may message it). The answer is an
   `Attachment` (id, name, media type, size, and pixel size for images).
2. The message sends the ids. The control plane claims the uploads for the
   session (they must be the sender's and not sent in another session) and
   records them on the user message and the delivery; a task's files are
   kept on the session row and travel in the `SessionSpec`. Uploads no
   message claims are deleted after a day.
3. Before forwarding a message, or configuring the agent with the task, the
   host fetches the files with its runner token
   (`GET /api/v1/runner/sessions/{id}/attachments/{attachment}`), in the
   order the messages came, and hands them to fctl-agent as `file` frames.
4. The agent saves each file as `~/attachments/<id>/<name>` in the sandbox,
   outside the repository, and lists the paths after the message's text.
   Images Claude accepts as they are (PNG, JPEG, GIF and WebP within the
   API's size limit and 2000 px a side, 5 MB of them per message) go into
   the message too; Claude reads anything else with its tools, which resize
   large images.

Viewers of a session read its files with
`GET /api/v1/sessions/{id}/attachments/{attachment}`. It answers a single
byte range (`Range`, `206 Partial Content`) from only the chunks that hold
it, because Safari plays a video only from a server that does: the session
page plays attached videos, and shows images, in a lightbox. Hosts and
agents announce the `attachments` feature in their hellos; an older one
passes the text alone, and the transcript says so.

### Published screenshots

Files go the other way too: agents show people what a pull request changes
with screenshots in its description. GitHub takes those uploads (`gh pr
edit --attach`) only with a person's token, and sessions have the GitHub
App's, so in projects that publish screenshots the agent publishes them on
the control plane instead and puts the links in the pull request
([ADR 0014](adr/0014-agents-publish-screenshots-at-unguessable-links.md)).

1. `fctl publish shot.png` in the sandbox sends the image to fctl-agent over
   the session socket, which checks that it is a PNG, JPEG, GIF or WebP of
   at most 5 MB (the most GitHub's image proxy fetches) and hands it to the
   host as a `publish` frame.
2. The host uploads it with its runner token
   (`POST /api/v1/runner/sessions/{id}/published`). The control plane
   checks that the session is assigned to that runner and that publishing
   is allowed, stores the bytes in the project's FilesDO, and indexes them
   in D1's `published_files` by the SHA-256 of a new token of 256 random
   bits. Its answer, a `PublishedFile`, has the link,
   `https://<instance>/api/v1/published/<token>/<name>`, and when it
   expires.
3. fctl-agent answers fctl, which prints the link and Markdown that shows
   it, and adds a `screenshot_published` notice with the file to the
   transcript.
4. Anyone with the link gets the image (`GET /api/v1/published/{token}/…`):
   GitHub's image proxy for the pull request, the transcript in the web
   app. Responses are `public, no-cache` with an `ETag`, so caches ask
   every time, `nosniff`, sandboxed by their Content-Security-Policy,
   `Cross-Origin-Resource-Policy: cross-origin` and `noindex`.

Publishing is off twice over by default: the instance's admins allow it
(Admin → Settings, `InstanceSettings.publish_screenshots`) and each
project's maintainers turn it on (`publish_screenshots` in its settings),
which also picks how long links work: 30 days unless `screenshot_days`
says 1 to 60. The control plane checks both when a screenshot is
published and whenever a link is opened, so turning either off takes the
screenshots offline at once, until they expire. A session publishes at most
100. The FilesDO's alarm deletes a screenshot's bytes when it expires and
the hourly cron trigger its row; people who manage the session delete one
sooner (the transcript, `fctl unpublish`, `DELETE
/api/v1/published/{token}`).

The setting reaches the agent in its `SessionSpec`, only when the instance
allows it too, and through hosts that know it: an older host leaves it out
of the agent's configuration, so its agents never offer `fctl publish`.
When it is on, and the sandbox's fctl has the command, `~/AGENTS.md` tells
Claude to try `--attach` and publish when GitHub refuses.

### Project memory

Every session starts in a new conversation, most in a new sandbox, so what
an agent learned that the repository doesn't say went with it. A project
keeps a memory instead: notes its agents, and its people, write for the
agents of its later sessions, such as a command that works, a trap, how its
tests or deploys really behave, or a decision people made in conversation
([ADR 0034](adr/0034-agents-keep-a-project-memory-and-record-retrospectives.md)).
D1 keeps them (`memories`): the text (Markdown, at most 2,000 characters),
the session whose agent wrote it and the person who started that session,
or the person who wrote it, who last changed it, and for a stale note when,
why and who marked it, and the note that took its place or that it repeats
(`fctl_domain::memory`).

1. **What agents start with.** As FleetDO builds a run's spec, it adds the
   project's current notes, newest first, as many as fit in 24 KB and 200
   notes, and how many more there are (`SessionSpec.memory`,
   `memory::for_prompt`): about what Claude Code loads of its own memory.
   fctl-agent lists them in `~/AGENTS.md`, each with its id, where it came
   from and how old it is, as notes that don't override the person's
   request or the rules, and says how to keep them. A resumed run starts
   with the memory as it is then. It also sets
   `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`: Claude Code's own memory would live
   in the sandbox's `~/.claude/projects/` and go with it.
2. **What agents do.** In the sandbox, `fctl memory` lists the notes as
   they are now, those other sessions added since the run started too;
   `fctl memory add` writes one, in place of others that go stale with
   `--replaces` (as many as 20, for a note that says what overlapping
   notes did); `fctl memory stale` marks one stale, with the reason agents
   must give (not one someone else marked stale already); and `fctl memory
   duplicate` marks one stale as a duplicate of another, a current note
   that agents go on starting with, and links the two. Agents can't change
   or delete notes. fctl reaches
   fctl-agent over the session socket, which hands the call to its host
   (`AgentToHost::Call`); the host sends it with its runner token (`POST
   /api/v1/runner/sessions/{id}/calls`, an `AgentCall`), and the control
   plane checks that the session is assigned to that runner, that the
   runner still serves its workspace and that the project keeps a memory,
   as for published screenshots. The answer comes back the same way, and
   fctl-agent adds a `memory_added` or `memory_stale` notice with the note
   to the transcript, one for each note a new one replaced. The host
   retries reading the memory and marking a note stale (a duplicate too)
   on errors that may pass, and not a new note, which the control plane
   may have kept already. The first note a new one replaces travels as
   `replaces` and the others as `also_replaces`, and a duplicate as
   `mark_stale` with `duplicate_of` and the reason `Duplicate of mem_…`:
   a sandbox's fctl may be newer than its host, and fctl-agents, hosts and
   control planes from before them replace the first note alone (fctl
   says which notes are still current) and mark a duplicate stale with
   that reason.
3. **What people do.** Everyone who sees the project reads its memory (its
   Memory tab, `fctl memory -p …`, `GET /api/v1/projects/{id}/memories`);
   editors and up add notes, in place of others too, change them, mark
   them stale (duplicates too) or current again and delete them (`POST
   /api/v1/projects/{id}/memories`, `PATCH` and `DELETE
   /api/v1/memories/{id}`), which the workspace's audit log records. A
   project keeps at most 500 notes, stale ones included. Session pages ask
   for the notes their agent wrote or marked stale (`?session=`), and the
   web app reads the memory again when a session of the project changes
   status, as when a turn ends. Deleting a note leaves the transcripts that
   showed it as they are.

The **Project memory** setting (`project_memory`, on by default) decides
whether agents keep it. Off, the spec has no memory, agents hear nothing
of it and the control plane refuses their calls; the notes stay for
people. An update that leaves the field out keeps the stored value. Hosts
and agents from before it never offer `fctl memory`, nor does a sandbox
whose fctl lacks the command (fctl-agent asks `fctl memory --help` and
says so once in the transcript's log).

### Retrospectives

As an agent finishes a piece of work (it opened or updated a pull request,
answered what was asked, or stops because it is blocked), `~/AGENTS.md`
asks it to record a retrospective for the project's people: how the work
ended (`completed`, `partial` or `blocked`), what it worked on, the
approach it took, what got in the way and what a future agent should know
(`fctl retro --outcome … --worked-on … --approach … --challenges …
--insights …`). The call travels as the memory's do, under the
**Retrospectives** setting (`retrospectives`, on by default). D1 keeps it
with its session (`retrospectives`), at most 50 a session
(`fctl_domain::retrospectives`), and the transcript shows it as a
`retrospective` notice.

A retrospective is of a piece of work, not of a turn
([ADR 0038](adr/0038-agents-keep-one-retrospective-for-each-piece-of-work.md)).
When a later turn carries on with work the session recorded one for
(review comments, a failed check, the answer that unblocks a blocked one,
the rest of a partial one), `~/AGENTS.md` asks the agent to update that
one rather than record another: `fctl retro update rtr_… --outcome
completed …` replaces the outcome and the parts it gives and keeps the
others, and `fctl retro list` in the sandbox lists the session's own, with
their ids, which the agent checks before it records one (a resumed run may
start without its conversation). The control plane changes only
retrospectives of the calling session, in place, and keeps when
(`updated_at`); the transcript shows the retrospective as it now is in a
`retrospective_updated` notice, and keeps the earlier notices as they
were. A new one is for different work. Where the sandbox's fctl can't
update them (fctl-agent asks `fctl retro update --help`), `~/AGENTS.md`
asks instead for one for each follow-up that changes the work.

The session page shows the newest beside the transcript and all of them a
click away, each saying when its agent last updated it, and the project's
Retrospectives tab lists the project's, newest first (as they were
recorded), with how many ended each way and a filter by outcome,
leaving out those of archived sessions (`GET
/api/v1/projects/{id}/retrospectives`, a page at a time after the id of
the last one, which sort as they were recorded; `GET
/api/v1/sessions/{id}/retrospectives`; `fctl retro list`). Editors put an
insight in the project's memory from there. People who manage a session
delete its retrospectives (`DELETE /api/v1/retrospectives/{id}`), which the
workspace's audit log records; the transcript keeps what the agent
recorded.

## Notifications

A session notifies the people who follow it when it waits for a reply (a
turn ends: `running` → `idle`), pauses on a usage limit, or fails. Starting
or resuming a session follows it, unless the person turned that off in
Settings; anyone who can see a session can follow it (the bell on its page,
`fctl follow`). Notifications are Web Push messages (RFC 8030) to the
browsers where people turned them on, so they need no account with a push
provider; on iPhone and iPad they reach fctl on the Home Screen.

1. SessionDO sees the change. It leaves out the people who can see the
   session right now (a viewer whose tab is visible: the web app sends
   `visibility`) and whoever interrupted the turn, and asks D1 for the other
   followers who can still see the session, want this kind of news
   (`notification_settings`) and have a browser for it. A finished turn
   carries the start of the agent's last message.
2. It hands the news to each one's NotifyDO, which waits ten seconds for
   more, so that a burst goes out as one, and sends at most one notification
   a minute: what arrives in between goes out together as a digest ("3
   sessions need you"). Sessions that moved on meanwhile (a reply made one
   stop waiting) are left out.
3. The control plane encrypts the message for each browser (RFC 8291,
   `aes128gcm`) and posts it to the browser's push service, signed with a
   VAPID token (RFC 8292). The instance's VAPID key is made on first use and
   kept in D1's `settings`, sealed with the master key; WebCrypto does the
   elliptic-curve work. Push services see when a message goes and how big it
   is, not what it says.
4. The service worker (`web/public/sw.js`) shows it. A click opens the
   session, in a window of the app that is already open when there is one;
   a digest replaces the notifications about the sessions it sums up.

A browser's subscription (`push_subscriptions`) belongs to the sign-in that
made it: signing out, revoking that sign-in or its expiry deletes it, and so
do removing the browser in Settings and a push service that answers it is
gone. When the app starts in a browser whose subscription the server no
longer has for that sign-in, it ends the subscription there too, so it never
comes back by itself; when the instance's VAPID key changed, it subscribes
again with the new one.

## Claude credentials

Each user connects their own Claude subscription; a session always runs on
the subscription of the person who started it: the connection they chose
for the session's workspace, else their default one. Two kinds are
supported:

| Kind | How it is created | Remote Control |
|---|---|---|
| Long-lived token | `claude setup-token` on any machine, pasted into fctl | no (inference-only scope) |
| Full login | `claude auth login` run by the official CLI in a sandbox; fctl shows the sign-in link and relays the code | yes |

Sandboxes receive only an access token (`CLAUDE_CODE_OAUTH_TOKEN`, plus the
granted scopes in `CLAUDE_CODE_OAUTH_SCOPES`). When it expires, Claude asks its
host with the `oauth_token_refresh` control request; fctl-agent forwards it to
the control plane, which refreshes once per user and answers. With a full
login, fctl-agent enables Remote Control with the `remote_control` control
request, so the session also appears in claude.ai/code and the Claude apps
of the person it runs for, and nobody else can open it there. SessionDO
names that person in each Remote Control state the agent reports
(`RemoteControlState.user`): who started the session, or who resumed it
last, which the resume tells it (`ControlState.claude_user`). The web app
shows the links, and the transcript's note, to that person alone; states
from before it named anyone count as the starter's.

A full login also brings the person's claude.ai connectors (Claude Code
fetches them with the `user:mcp_servers` scope). A project can turn them off
(`claude_ai_connectors` in its settings): the control plane then adds
`ENABLE_CLAUDEAI_MCP_SERVERS=false` to the environment in the session's
spec, which every fctl-agent passes to Claude, and fctl-agent writes
`disableClaudeAiConnectors` into Claude's user settings, which no
repository settings can undo. An update that leaves the field out keeps
the stored value, so a client from before it can't turn connectors back on.

Agents can list and message other Claude Code sessions with Claude Code's
[cross-session messaging](https://code.claude.com/docs/en/cross-session-messaging)
(the `ListAgents` and `SendMessage` tools). On its own machine, Claude Code
finds only the sessions registered in its `~/.claude/sessions`, and every
sandbox has its own home directory, so an agent finds there only the Claude
sessions in its sandbox, such as its subagents. Claude Code finds sessions
on other machines and in the cloud only while connected to Remote Control,
so a session on a long-lived token can't list, read or message any of the
person's other sessions, whether in fctl, on their own machines or at
claude.ai/code. A session on a full login joins Remote Control unless its
project turns it off, and its agent can then list the person's other Remote
Control sessions (fctl sessions on a full login, and those on their own
machines) with their titles and status, and message them. The `process`
driver isolates nothing: there an agent can reach whatever the host's user
can.

Guards against API billing: `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`,
`ANTHROPIC_BASE_URL`, Bedrock/Vertex/Foundry switches and `apiKeyHelper` never
reach Claude; the `system/init` message must report no API key source; a
`rate_limit_event` showing overage use interrupts the session and pauses it
until the limit resets.

### Plan usage

Settings shows how much of each connection's plan is used, and the Agent Floor
shows it for the connection new sessions run on, so people see what is left
before they start work. Sessions report it; fctl asks Anthropic for nothing.
Claude Code writes a `rate_limit_event` when a limit's status changes and,
in recent versions, whenever the five-hour or weekly window moves by a
percent (`unifiedWindows`). fctl-agent passes it on like every other Claude
message.

1. SessionDO turns each one into readings, one per limit
   (`fctl_domain::usage`): the five-hour window, the week, and a model's own
   weekly limit when Claude names it, each with the share used, when it
   resets, and Claude's status for the limit closest to stopping requests.
   Paid extra usage is not a plan limit and is left out.
2. It keeps them with the Claude credential the session runs on now (a
   resumed session runs on the resumer's), in D1's `claude_usage`: one row
   per credential and limit, which a later reading replaces.
3. `/auth/me` returns them on each credential (`ClaudeCredentialInfo.usage`,
   `fctl claude status`), and FleetDO sends the credential's owner a
   `claude_usage` feed message, so Settings and the Agent Floor change while
   sessions run.

Readings change only while sessions run, so the apps say when a session last
reported them, and a window whose reset time has passed counts as started
over. Removing the connection deletes them. Hosts and agents need no update:
they already pass every Claude message on.

The apps go by the share used. From 80 % of a limit, the Agent Floor says how
much of it is used, in amber, and from 90 % in red. At 100 %, or when Claude
refuses requests (`rejected`), the limit is reached, and only then does a
session's transcript say so. Claude's warning status (`allowed_warning`)
doesn't count: Claude Code sends it when a limit goes faster than its window
passes, as with a quarter of the week used in its first day.

## Sandboxes and drivers

A driver turns a `SandboxSpec` (image, CPUs, memory, environment) into a
running Linux sandbox and can exec processes in it with piped stdio.

The image is the project's or, without one, the host's: unless `host.toml`
sets another, the default image of the host's own version,
`ghcr.io/hyperstructure/sandbox:<version>`. hyperstructure/sandbox's Image
workflow publishes it for arm64 and amd64 within the hour after every
release, and rebuilds the latest release's when Claude Code has a newer
release, since fctl turns off Claude Code's auto-updater. The host pulls the
image when it starts and when a sandbox needs one it doesn't have, and falls
back to `latest` when its version's can't be pulled. On the default image,
it pulls its tag again when it starts and every 6 hours, unless the image
there was built on the host: only the Image workflow sets the
`org.opencontainers.image.revision` label. It also removes the default
images that sessions no longer start from, without force, so that Docker
keeps those a sandbox uses (`crates/fctl-host/src/refresh.rs`,
[Sandbox images](sandbox-images.md)). `fctl host image build` builds it on
the host instead, from that repository's `main`, which it downloads; the
Lima driver's base VM gets the image's `provision.sh`.

A project names an image (`ProjectSettings.image`) or a file in its
repository to build one from (`image_build`: a Dockerfile and its context,
or a dev container configuration). Hosts send the images they run besides
their own (`HostHello.allowed_images`: references, `/*` prefixes, digest
pins; absent: any) and the repositories they build images from
(`allowed_builds`; absent: none, and always absent with Lima or the process
driver). FleetDO reads each queued session's image with its size
(`fctl_domain::image::ImageWant`) and only assigns it to a runner whose
lists take it (`fctl_domain::schedule::pick`); a session that none online
takes stays queued with the reason. The host checks its lists again before
it accepts a session. To build (`crates/fctl-host/src/build.rs`), it reads
the file at the tip of the project's default branch through GitHub's API
with the session's token, keys the build by the file, the Dockerfile and
the git tree of the context, and builds only when no image has that key:
it downloads the repository's archive, unpacks the context and the
Dockerfile, and runs `docker build` or `container build`, one at a time,
within `build_timeout_minutes`, removing an image over `max_build_mb`. It
records each project's last build in `builds.json` in its data directory,
removes the one before, and starts from the last build while GitHub can't
be reached.

| Driver | Isolation | Host |
|---|---|---|
| `apple` | one lightweight VM per session (Apple `container`) | Apple silicon, macOS 26+ |
| `lima` | one full VM per session | macOS, Linux |
| `docker` | containers in Docker/OrbStack/Colima (a VM on macOS) | macOS, Linux |
| `process` | none: a directory and a process | development, trusted hosts |

A sandbox gets the CPUs and memory its project's settings ask for, or the
host's default. Hosts send the default and a per-sandbox maximum in their
hello (`HostHello.sandbox`): the configured `max_cpus` and `max_memory_mb`,
within what the sandboxes run on (the Docker daemon's VM with Docker,
otherwise the machine). FleetDO only assigns a session to a host whose
maximum covers its size; one that fits nowhere stays queued with the reason,
and the sessions queued behind it may start meanwhile. Hosts that send no
limits (older versions) take any size. The host passes the size it applied
to fctl-agent (`AgentConfig.sandbox`), which tells Claude: with Docker,
`nproc` and `free` show the VM, not the sandbox's cgroup limits, and with
Apple's `container`, `nproc` counts one CPU more than the sandbox may use.
The process driver limits nothing.

An Apple `container` sandbox is a VM of its own. Apple's runtime gives the
VM one CPU and 128 MiB more than the sandbox's size, for the guest kernel
and its init, and holds the container to the size with cgroups. It sets
the VM's clock from the Mac's every 30 seconds, so the clock is right
again within half a minute of the Mac waking, and the VM resolves names
through the Mac. The root filesystem is a sparse 512 GiB disk image cloned
from the image's, so `df` shows about 500 GB free whatever the Mac has.
The image grows as the session writes, and shrinks only where the guest
discards blocks: the guest mounts it without `discard`, and the container
can't run `fstrim` without `CAP_SYS_ADMIN`, which it doesn't have. So as
the host stops a sandbox, it runs `sync` in it, then `container clean`
(Apple's `container` 1.4 and later), which has the VM's init trim the
root filesystem, and the Mac gets back the space of the files the
session deleted
([ADR 0025](adr/0025-hosts-trim-apple-sandboxes-as-they-stop-them.md)).
While a sandbox runs, it holds the space of the files deleted in it; with
an older `container`, until the host removes the sandbox.

fctl-agent runs detached inside the sandbox and listens on a unix socket; the
host talks to it through `fctl-agent attach` over the driver's exec channel.
Restarting or upgrading the host does not stop agents. The host pings the
agent every 15 seconds and replaces a link that carries nothing for a
minute, or gets no hello within 30 seconds of attaching: it kills the exec,
starts fctl-agent again if it is gone, and attaches again, waiting longer
after each failure (`crates/fctl-host/src/liveness.rs`). Once it has known
for 10 seconds that no link works, the session's viewers hear that the
agent can't be reached, and since when ([Viewers](#viewers)); a host that
wakes from sleep to silent links attaches again within them.

A host that restarts starts fctl-agent again too, before it attaches, in
case the sandbox restarted with it. A supervisor holds a lock on
`~/.fctl/agent.lock` while it runs, so a second one sees the first and
exits without connecting to its socket: the supervisor takes every
connection for a host link, and the newest replaces the host's. Agents from
before the lock are found on the socket instead.

Driver commands that hang, as they do in an overloaded Docker VM, are
killed after a minute (image pulls and VM boots after 30), and the control
connection never waits on one: the hello reports what the drivers said
when the host last probed them (at start and every five minutes), and
sweeps and cleanups of sandboxes run in tasks of their own, so heartbeats
keep flowing. A host that starts while its driver doesn't answer waits to
connect until it can list its sandboxes, rather than report sessions as
gone that may still run. A driver command that can't start says as little
as one that hangs about the image or sandbox it asked about: the host
reports the error instead of pulling the image, falling back to `latest`,
or taking a sandbox it couldn't stop or remove for gone.

A second socket beside
it, the session socket (`FCTL_SESSION_SOCKET` in Claude's environment),
serves programs in the sandbox: `fctl edit` names and describes the session
there, `fctl publish` publishes screenshots, `fctl memory` and `fctl retro`
keep the project's memory and record and update retrospectives, and
`fctl resolve` resolves review threads ([GitHub access](#github-access)),
with no fctl credentials in the sandbox.

A stopped session keeps its sandbox, so resuming it on the same host
continues Claude's conversation ([Idle sessions](#idle-sessions)). The host
records when each one stopped (`stopped.json` in its data directory) and
removes those stopped for longer than `keep_stopped_hours`
(`crates/fctl-host/src/expiry.rs`), when it starts and every hour.

It also keeps `min_free_disk_gb` free on the disk where the sandboxes'
files are: Apple's `container` app root (from `container system status`),
Lima's home, Docker's root directory, or with Docker on macOS the disk in
its VM, where `df` runs in a short-lived container of the host's image, and
the data directory with the process driver. It measures every 30 seconds
(in Docker's VM every five minutes). Below the floor:

- The sweep removes the other stopped sandboxes too, those stopped longest
  first, one at a time, and measures after each, so that it stops once
  there is room. A session that starts in one meanwhile keeps it.
- The host takes no new sessions, only resumes of the sessions whose
  sandboxes it has. Its hello carries the floor (`HostHello.min_free_disk_mb`),
  and FleetDO compares it with each heartbeat's `disk_available_mb`: a
  runner below it gets resumes into its sandboxes, as while it drains, and
  no other session (`fctl_domain::disk`, `schedule::pick`). The host refuses
  the others itself, for a control plane that hasn't had its heartbeat yet.
  Unlike a drain, it stops no idle sessions early, and it ends when there is
  room again ([ADR 0019](adr/0019-hosts-make-room-when-their-disk-runs-low.md)).

Older hosts send no floor and remove sandboxes by age only.

Resuming a session whose sandbox is gone, or resuming on another host,
creates a new sandbox on the session's branch: the agent finds no
conversation to resume, so Claude starts a new one with the session's
original task, and the agent numbers its events after the ones the control
plane already has (`first_agent_seq`).

## Protocols

All protocols are JSON with a `type` tag and live in `fctl-proto`:

- `api` — REST request and response bodies.
- `viewer` — SessionDO ⇄ browser/CLI.
- `runner` — FleetDO ⇄ host (control connection).
- `uplink` — SessionDO ⇄ host (one per session).
- `agent` — host ⇄ fctl-agent (attachments' bytes travel here as `file`
  frames, published screenshots' as `publish` frames, what agents ask of
  the control plane about their project as `call` frames), and `fctl` ⇄
  fctl-agent in the sandbox (the session socket).
- `event` — the session event model shared by all of them.

Changing a type means: update `fctl-proto`, run `cargo xtask gen` (regenerates
`web/src/gen`), and keep older peers working — hosts and agents may lag the
control plane by a version. Add fields as optional; never repurpose a tag.

## Deployment

- `app.<domain>` — fctl-control and the web app (one Worker).
- Or, without Cloudflare, the same Worker and web app in one container
  (`ghcr.io/hyperstructure/fctl-control`, `deploy/standalone`), on any
  machine or Kubernetes cluster
  ([ADR 0026](adr/0026-the-control-plane-runs-without-cloudflare-on-workerd.md)).
  `server.mjs` runs them on workerd, Cloudflare's open-source Workers
  runtime, through Miniflare, with the bindings of `wrangler.toml` and its
  state (D1 and every Durable Object, as SQLite files) in a data directory.
  It does what Cloudflare does around the Worker: applies the D1 migrations
  as it starts, sets `CF-Connecting-IP` (from the connection, or from
  `X-Forwarded-For` behind trusted proxies) and drops what clients must not
  set (`CF-*`, `MF-*`, `/cdn-cgi/`), and runs the hourly cron trigger. A
  lock file keeps a second server off the same data, and the server exits
  when workerd stops answering, so that Docker or Kubernetes starts it
  again. `request.cf` is empty there, so sign-ins record no city or
  country.
- `<domain>` — the site Worker from hyperstructure/fctl-site (landing page,
  docs, `/install`, `/releases`). It reads releases from a KV namespace that
  `cargo xtask publish` writes (`deploy/releases.toml`); the two share the
  namespace, not code.
- Hosts install with `curl -fsSL https://<domain>/install | sh`, then
  `fctl login` and `fctl host install` (launchd on macOS, systemd on Linux).

Everything instance-specific (domain, GitHub App, Google client, email) is
configuration, the same for both; see [self-hosting](self-hosting.md).
