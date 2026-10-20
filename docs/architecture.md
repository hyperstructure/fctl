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
  user. The first account is created through a one-time setup link
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
  issued through the device flow), `fctl_pat_` (personal access token),
  `fctl_rt_` (runner).
- Cookie requests that change state must come from the app's origin
  (`Origin`/`Sec-Fetch-Site` checks) and carry JSON; file uploads carry
  `application/octet-stream`, which browsers cannot send cross-site without
  asking first either. WebSocket upgrades check the origin too.

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
   always does). Replacing the app, turning two-factor off and getting new
   recovery codes take a current code, from the app or a recovery code.
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
- **Signals.** Where browsers have them, the app tells the person's passkey
  provider which passkeys the account has, what the account is called, and
  which passkey fctl doesn't know any more (`signalAllAcceptedCredentials`,
  `signalCurrentUserDetails`, `signalUnknownCredential`), so that it stops
  offering removed ones.

Adding and removing a passkey go in the audit log, and adding one emails
the person when email is set up. A passkey that is the account's last way
in, or the second factor the instance requires, can't be removed until
another takes its place ([ADR 0023](adr/0023-passkeys-sign-in-on-their-own.md)).

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
  The project and session queries join the caller's membership
  (`model::PROJECT_SELECT`, `PROJECT_VISIBLE`).
- **Roles.** Admins invite and remove people, change roles, choose the
  runners and GitHub accounts, and change the settings; the primary admin
  also deletes the workspace and hands the role to another admin. Nobody
  else can demote or remove the primary admin, who leaves only after
  handing the role over (`fctl_domain::workspace::check_*`).
- **Invitations** (`workspace_invitations`) are for an address and last 30
  days. Whoever has it (an account's, or connected to one) sees it on their
  home page and in `fctl workspaces`, and joins as that address; one the
  instance doesn't admit yet needs an instance invitation first, which
  instance admins send together with their workspaces' invitations. People
  invited before they had an account join when they make it, or connect the
  address, as far as the limit and two-factor allow; the rest keep waiting.
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
  sidebar stays in their browser.
- **Audit log.** What happens in a workspace (people, roles, runners, GitHub
  accounts, settings, projects, sessions) goes in its log
  (`audit_log.workspace_id`), which its admins read; the rest is the
  instance's.
- **Feeds.** A feed's identity holds the projects its person saw when it
  opened, and the workspaces where they see every project
  (`internal::FeedIdentity`). When someone's workspaces change, FleetDO
  recomputes it on their sockets (`/workspaces-changed`) and sends
  `workspaces`, after which the app fetches them again; runners and their
  updates go only to the people of their workspaces, their owners and
  instance admins.
- **Deleting** a workspace (its primary admin, or an instance admin, typing
  its name) deletes its projects with their sessions and files, as deleting
  a project does, once none of its sessions runs.

### Email addresses

Each address is invited to the instance on its own. Accepting an
invitation while signed in (`POST /api/v1/invites/{token}/connect`)
connects its address to that account instead of making a new one
(`user_emails`). Every address signs in, and has its own commit email
([Commits](#commits)); the account's primary one (`users.email`) is where
fctl writes to the person. A workspace knows its member by the address they
joined as (`workspace_members.email`), the only one its people see:
project members, shares and invitations look people up by it
(`api::auth::member_by_email`). Disconnecting an address
(`DELETE /api/v1/me/emails/{email}`) leaves the workspaces joined as it.

### GitHub accounts

The instance has one GitHub App. A workspace uses an account the app is
installed on once it is connected to it (`workspace_github`), with every
repository the app reaches there or only some. A workspace's admins ask:
they install the app from the workspace (its setup URL,
`GET /api/v1/github/installed`, brings them back with the installation and a
signed state naming the workspace) or name an account it is installed on.
An instance admin approves the request, or connects accounts directly
(Admin → GitHub, `fctl admin github`): GitHub's setup URL can be called
with anyone's installation id, and the app's tokens can write. Project
repositories must be within the workspace's connections
(`fctl_domain::repos::check_scope`), and every token a session gets is
minted within them (`repos::narrow`), so that disconnecting an account takes
it from running sessions at their next refresh, within the hour. New
repositories are created in an organization the workspace uses all of.

### Runners and workspaces

A runner belongs to the person who registered it (`runners.created_by`) and
runs the sessions of the workspaces it is added to (`runner_workspaces`).
Its owner adds it to workspaces they are an admin of, since a workspace
trusts its runners with its code, secrets and the tokens of the people who
start its sessions; a workspace's admins take any runner out of it; its
owner and instance admins rename, drain, disable and remove it. FleetDO
reads which workspaces each runner serves before it dispatches, and starts
a session only on a runner of its workspace (`Slot.serves`); a session whose
workspace has no runner online says so. A Claude sign-in runs only on a
runner of the person's workspaces, or their own.

## Data

**D1** (`crates/fctl-control/migrations`) holds relational state: users and
their email addresses, identities, two-factor sign-in (sealed authenticator
secrets, hashed recovery codes, sign-ins waiting for a code), passkeys'
public keys and their challenges, allowlist, early access requests, auth
tokens, device codes, workspaces (with their avatar images), their members,
invitations, GitHub
accounts and runners, projects, their members and
repositories, sessions (summary rows), shares, who follows which session,
notification settings and browsers (push subscriptions), runners, encrypted
credentials and project secrets, the latest plan usage of each Claude
credential, the links of published screenshots, the instance's settings
and sealed Web Push key, and the audit log.

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

**Deletion.** Deleting a project deletes its rows (sessions, members, secrets
and shares cascade) and, in the same batch, queues its sessions in
`session_purges` and the project in `file_purges`; their Durable Object
storage, the event logs and the attached files and published screenshots,
is deleted right after. An hourly cron trigger (`src/housekeeping.rs`; the
standalone server runs it outside Cloudflare) retries what failed and
deletes expired tokens, password resets, sign-ins waiting for a code,
passkey challenges, device codes and invites, early access requests whose
address has an account or that are a year old, the links of expired
published screenshots, abandoned Claude logins, and audit
log entries older than a year. Deleting a workspace does the same for all
its projects. Deleting an account (`src/api/account.rs`) does it for the
workspaces only that account is in, passes the primary admin role of its
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
session waits for it ("Waiting for 2 background tasks"), and the idle
timeout counts from when the last one ends. When background work ends,
Claude Code hands Claude the result in a turn of its own. fctl-agent
counts it as any other: the session runs, and a stop or an interrupt
reaches the turn. It learns of these turns from
`system/session_state_changed` (it sets
`CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS`), or from a turn's first message
when Claude Code doesn't report them. Background work keeps a session up
for at most an hour after its last turn, or for the idle timeout when that
is longer, so that a forgotten dev server doesn't hold the runner's slot
for ever. When a session stops with background tasks still running (that
limit, a person's stop, a drain, an uninstall), the transcript names them,
and the next message tells Claude they didn't finish. The system prompt
says how long background work may run after a turn ends. Such a session
holds its slot as any other does, and a draining runner stops it after a
minute, as it stops sessions that wait for a message.

A message to a stopped session resumes it: from the web app, `fctl send`,
`fctl attach` or the API, for anyone who may message the agent. SessionDO
queues the session again and keeps the message in its outbox for the new
run, which runs on the Claude subscription the session ran on, as a
message to an idle session does. A resume (**Resume** for the session's
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
tokens: one that can write (the primary repository and the writable ones)
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
- **While Claude works** fctl-agent reports the same state soon after tool
  calls that can change the repositories (`Bash`, `Write`, `Edit`,
  `NotebookEdit`, a subagent's too) and when background tasks end: a burst
  costs one refresh, refreshes come at most every ten seconds
  (`crates/fctl-agent/src/refresh.rs`), and a `git` event goes out only
  when the state changed. These refreshes only read: they push nothing,
  move no work, and look up a branch's pull request only once it was
  pushed or its pushed head moved, or after `gh pr create`. They read a
  copy of each clone's index, since `git diff` rewrites the index it reads
  under its lock, which would fail the agent's own `git commit`
  ([ADR 0027](adr/0027-agents-report-their-repositories-while-claude-works.md)).
  The session page's Changes section and the session lists take the newest
  state; the transcript shows one when a branch gets a commit, a push or a
  new pull request (`fctl_domain::branches`).

Older hosts and agents ignore the list and the second token, and work on the
primary repository alone
([ADR 0009](adr/0009-projects-span-an-accounts-repositories.md)).

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

### Event flow and reliability

- fctl-agent assigns `agent_seq` to every event and appends it to a journal in
  the sandbox. The host forwards events in batches; SessionDO stores them,
  assigns the canonical `seq`, broadcasts them, and acknowledges `agent_seq`.
  After any reconnect SessionDO announces the last acknowledged `agent_seq`
  and the host replays the journal from there.
- SessionDO stores events in `agent_seq` order. After a reconnect, newer
  events can arrive before the replay of older ones, which went over a link
  to the agent or an uplink that closed: they wait for it, for at most a
  minute, rather than acknowledge the older ones unseen
  (`crates/fctl-domain/src/journal.rs`).
- Streaming deltas (`stream_event`) are forwarded but not stored; SessionDO
  keeps the in-progress text in memory so late viewers see it.
- The session's counters (`SessionStats`) move as the agent works. Each
  API call's input and cache reads count as its response starts
  (`message_start`), or with its stored messages if that stream event went
  missing, and its output with the `message_delta` that ends it. Claude
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
frames are incremental. Older history is paged over HTTP.

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
last, which the resume tells it (`ControlState.claude_user`). A link it
already had keeps its person, so a late report from the run before a
resume doesn't pass it on. The web app shows the links, and the
transcript's note, to that person alone; states from before it named
anyone count as the starter's.

A full login also brings the person's claude.ai connectors (Claude Code
fetches them with the `user:mcp_servers` scope). A project can turn them off
(`claude_ai_connectors` in its settings): the control plane then adds
`ENABLE_CLAUDEAI_MCP_SERVERS=false` to the environment in the session's
spec, which every fctl-agent passes to Claude, and fctl-agent writes
`disableClaudeAiConnectors` into Claude's user settings, which no
repository settings can undo. An update that leaves the field out keeps
the stored value, so a client from before it can't turn connectors back on.

Guards against API billing: `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`,
`ANTHROPIC_BASE_URL`, Bedrock/Vertex/Foundry switches and `apiKeyHelper` never
reach Claude; the `system/init` message must report no API key source; a
`rate_limit_event` showing overage use interrupts the session and pauses it
until the limit resets.

### Plan usage

Settings shows how much of each connection's plan is used, and the home page
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
   `claude_usage` feed message, so Settings and the home page change while
   sessions run.

Readings change only while sessions run, so the apps say when a session last
reported them, and a window whose reset time has passed counts as started
over. Removing the connection deletes them. Hosts and agents need no update:
they already pass every Claude message on.

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
there, and `fctl publish` publishes screenshots, with no fctl credentials
in the sandbox.

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
  frames, published screenshots' as `publish` frames), and `fctl` ⇄
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
