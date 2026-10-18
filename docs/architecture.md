# Architecture

fctl is a software factory: you describe work, and Claude agents do it in
isolated sandboxes on machines you own. This document is the map. Read it
before changing anything that crosses a component boundary.

```
                         ┌──────────────── Cloudflare ────────────────┐
  browser (web app) ───► │  fctl-control  (Rust → wasm Worker)        │
  fctl CLI ────────────► │   ├─ REST API  /api/v1/*                   │
  Claude apps ◄─┐        │   ├─ static web app (Workers assets)       │
                │        │   ├─ D1: users, projects, sessions, …      │
                │        │   ├─ FleetDO: runners, scheduling, feed    │
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
| `crates/fctl-control` | The control plane Worker: HTTP API, auth, Durable Objects, GitHub App client. | Cloudflare (wasm32) |
| `crates/fctl-host` | The host daemon library: control-plane client, session manager, sandbox drivers. | macOS, Linux |
| `crates/fctl-agent` | The supervisor inside each sandbox: runs `claude`, journals events, prepares the workspace. | Linux sandboxes |
| `crates/fctl` | The `fctl` binary: CLI for people and agents, plus `fctl host …`. | macOS, Linux |
| `crates/fake-claude` | A scripted stand-in for `claude` that speaks stream-json. Used by tests and `cargo xtask dev`. | dev, CI |
| `xtask` | Development automation (`cargo xtask dev`, `check`, `gen`, `deploy`, `dist`, `publish`). | dev, CI |
| `web/` | The web app (React, Vite, TypeScript, WebGL). Served by fctl-control. | browser |
| `images/agent` | The default sandbox image (Debian, git, gh, node, python, claude). | sandboxes |

Two more repositories complete the picture:
[hyperstructure/design](https://github.com/hyperstructure/design)
(`@hyperstructure/design`: tokens, fonts, the WebGL scenes and the marks,
used by `web/`) and
[hyperstructure/fctl-site](https://github.com/hyperstructure/fctl-site)
(fctl.sh: the landing page, these docs rendered from `docs/`, the installer
and release downloads).

## Principles

1. **Subscription only.** Agents authenticate with Claude subscription OAuth
   tokens. API keys are stripped from every sandbox environment, the
   supervisor refuses to run when Claude reports an API-key credential source,
   and it interrupts a session the moment Claude reports paid overage use.
   See [Claude credentials](#claude-credentials).
2. **Your machines do the work.** Cloudflare holds state and routes messages.
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
  Google sign-in when the email is allowed).
- **Access is an allowlist.** Admins add exact emails or `*@domain` patterns.
  Signing up requires a matching entry; removing an exact entry disables that
  user. The first account is created through a one-time setup link
  (`/setup#token=…`, from the `FCTL_SETUP_TOKEN` secret) and becomes the owner.
- Instance roles: `owner`, `admin`, `member`. Project roles: `owner`,
  `maintainer`, `editor` (can start sessions and message agents), `viewer`.
  Sessions can also be shared with individual users.
- Credentials are opaque random tokens with a type prefix, stored as SHA-256
  hashes: `fctl_ws_` (web session cookie `__Host-fctl`), `fctl_cli_` (CLI,
  issued through the device flow), `fctl_pat_` (personal access token),
  `fctl_rt_` (runner).
- Cookie requests that change state must come from the app's origin
  (`Origin`/`Sec-Fetch-Site` checks) and carry JSON. WebSocket upgrades check
  the origin too.

## Data

**D1** (`crates/fctl-control/migrations`) holds relational state: users,
identities, allowlist, auth tokens, device codes, projects, members, sessions
(summary rows), shares, runners, encrypted credentials and project secrets,
and the audit log.

**SessionDO** (one per session, SQLite storage) holds the event log and the
live state of a session: connected viewers, the runner uplink, pending
deliveries, and the in-progress streaming message. It writes summary changes
through to D1.

**FleetDO** (one per instance) holds runner control connections, assigns
sessions to runners, and fans out session summaries to dashboard viewers.

**Deletion.** Deleting a project deletes its rows (sessions, members, secrets
and shares cascade) and, in the same batch, queues its sessions in
`session_purges`; their SessionDO storage, the event logs, is deleted right
after. An hourly cron trigger (`src/housekeeping.rs`) retries what failed and
deletes expired tokens, password resets, device codes and invites, abandoned
Claude logins, and audit log entries older than a year. Deleting an account
(`src/api/account.rs`) does the same for the projects only that account is a
member of, deletes its personal rows in the same batch, and keeps a nameless
placeholder row only while sessions or projects it created remain.

## Sessions

A session is one Claude conversation in one sandbox, on a branch of the
project's repository.

```
queued → provisioning → preparing → running ⇄ idle → stopped
                                       │        │
                                       └─ paused (usage limit) ─┘   failed (any)
```

1. `POST /api/v1/projects/{id}/sessions` creates the row and asks FleetDO to
   schedule it. Without a free runner it stays `queued`.
2. FleetDO builds the `SessionSpec` just in time (repository, branch, agent
   settings, secrets, a fresh GitHub installation token, the starter's Claude
   credential) and sends `start_session` to a runner.
3. The host creates a sandbox with its driver, injects `fctl-agent`, starts it
   detached, attaches to it, and opens the session uplink to SessionDO.
4. fctl-agent clones the repository, creates the branch, runs the project's
   setup script, writes Claude's configuration, and starts
   `claude -p --input-format stream-json --output-format stream-json`.
5. Claude's messages flow back as events; messages from people flow down as
   deliveries. A finished turn makes the session `idle`; the sandbox stays up
   until the idle timeout, then stops. Stopped sessions can be resumed
   (`claude --resume`).

### Event flow and reliability

- fctl-agent assigns `agent_seq` to every event and appends it to a journal in
  the sandbox. The host forwards events in batches; SessionDO stores them,
  assigns the canonical `seq`, broadcasts them, and acknowledges `agent_seq`.
  After any reconnect SessionDO announces the last acknowledged `agent_seq`
  and the host replays the journal from there.
- Streaming deltas (`stream_event`) are forwarded but not stored; SessionDO
  keeps the in-progress text in memory so late viewers see it.
- Host status (`provisioning`, `preparing`) is sent as idempotent state, not
  as sequenced events.
- Deliveries (messages, interrupts, stops) wait in SessionDO's outbox until
  the agent confirms them; the agent remembers delivered ids.

### Viewers

Browsers and the CLI open `GET /api/v1/sessions/{id}/ws`. The first frame is a
snapshot (summary, recent events, presence, in-progress text); after that
frames are incremental. Older history is paged over HTTP.

## Claude credentials

Each user connects their own Claude subscription; a session always runs on
the subscription of the person who started it. Two kinds are supported:

| Kind | How it is created | Remote Control |
|---|---|---|
| Long-lived token | `claude setup-token` on any machine, pasted into fctl | no (inference-only scope) |
| Full login | `claude auth login` run by the official CLI in a sandbox; fctl shows the sign-in link and relays the code | yes |

Sandboxes receive only an access token (`CLAUDE_CODE_OAUTH_TOKEN`, plus the
granted scopes in `CLAUDE_CODE_OAUTH_SCOPES`). When it expires, Claude asks its
host with the `oauth_token_refresh` control request; fctl-agent forwards it to
the control plane, which refreshes once per user and answers. With a full
login, fctl-agent enables Remote Control with the `remote_control` control
request, so the session also appears in claude.ai/code and the Claude apps.

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

## Sandboxes and drivers

A driver turns a `SandboxSpec` (image, CPUs, memory, environment) into a
running Linux sandbox and can exec processes in it with piped stdio.

| Driver | Isolation | Host |
|---|---|---|
| `apple` | one lightweight VM per session (Apple `container`) | Apple silicon, macOS 26+ |
| `lima` | one full VM per session | macOS, Linux |
| `docker` | containers in Docker/OrbStack/Colima (a VM on macOS) | macOS, Linux |
| `process` | none: a directory and a process | development, trusted hosts |

fctl-agent runs detached inside the sandbox and listens on a unix socket; the
host talks to it through `fctl-agent attach` over the driver's exec channel.
Restarting or upgrading the host does not stop agents.

A stopped session keeps its sandbox, so resuming continues Claude's
conversation. The host records when each one stopped (`stopped.json` in its
data directory) and removes those stopped for longer than
`keep_stopped_hours` (`crates/fctl-host/src/expiry.rs`). Resuming one of those
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
- `agent` — host ⇄ fctl-agent.
- `event` — the session event model shared by all of them.

Changing a type means: update `fctl-proto`, run `cargo xtask gen` (regenerates
`web/src/gen`), and keep older peers working — hosts and agents may lag the
control plane by a version. Add fields as optional; never repurpose a tag.

## Deployment

- `app.<domain>` — fctl-control and the web app (one Worker).
- `<domain>` — the site Worker from hyperstructure/fctl-site (landing page,
  docs, `/install`, `/releases`). It reads releases from a KV namespace that
  `cargo xtask publish` writes (`deploy/releases.toml`); the two share the
  namespace, not code.
- Hosts install with `curl -fsSL https://<domain>/install | sh`, then
  `fctl login` and `fctl host install` (launchd on macOS, systemd on Linux).

Everything instance-specific (domain, GitHub App, Google client, email) is
configuration; see [self-hosting](self-hosting.md).
