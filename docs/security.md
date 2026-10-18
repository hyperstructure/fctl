# Security

fctl runs code-writing agents with access to your repositories and your
Claude subscription. This page says what it protects, how, and what it
trusts.

## What it protects

| Asset | Where it lives | Who can use it |
|---|---|---|
| Passwords | argon2id hashes in D1 | nobody can read them back |
| Sessions and tokens | SHA-256 hashes in D1; the secret only with its holder | the holder |
| Claude credentials | encrypted (AES-256-GCM) in D1 | sessions their owner starts |
| Project secrets | encrypted in D1 | that project's sessions, as environment variables |
| GitHub App key | a Worker secret | the control plane, to mint short-lived tokens |
| Master key | a Worker secret | the control plane |

Encrypted values are bound to their row (the AES-GCM associated data names the
owner and purpose), so a value copied between rows does not decrypt.

## Access

- Accounts exist only for allowlisted emails or domains. Removing an
  allowlist entry disables the accounts it admitted and revokes their tokens.
- Browser sessions are `__Host-` cookies (`Secure`, `HttpOnly`,
  `SameSite=Lax`). Requests that change state must come from the app's own
  origin (`Origin` / `Sec-Fetch-Site`) and carry JSON; WebSocket upgrades are
  origin-checked too.
- The CLI signs in with a device flow (you approve a code in the browser) and
  gets a revocable token; Settings lists every device and token.
- Sign-in, signup and reset endpoints are rate limited per address and per
  account; unknown emails take as long to fail as known ones.
- The web app ships a strict Content-Security-Policy (`script-src 'self'`, no
  inline scripts), `frame-ancestors 'none'`, HSTS, and renders agent output
  as text (Markdown is parsed into elements, never injected as HTML).

## Sandboxes

Each session runs in its own Linux VM (Apple container or Lima) or container
(Docker). The agent runs Claude Code with permission prompts off, because the
sandbox is the boundary: it can do anything inside its VM, and nothing
outside it except through these channels:

- **Network.** Sandboxes have outbound network access (to install
  dependencies and reach GitHub and Claude). Do not run hosts on networks
  where an outbound connection from an untrusted process is a problem.
- **GitHub.** A token for one repository, with the permissions in
  [self-hosting](self-hosting.md#4-connect-github-optional-recommended),
  valid for an hour and refreshed on demand. It cannot reach other
  repositories.
- **Claude.** A short-lived access token for the session's owner. Refresh
  tokens never enter a sandbox: when the access token expires, Claude asks
  the control plane for a new one through its host.
- **claude.ai connectors.** A session on a full Claude login gets the
  connectors its owner has connected on claude.ai (mail, files, docs, chat),
  and the agent can use them with that person's access without asking, also
  when a repository or a web page tells it to. Long-lived tokens from
  `claude setup-token` don't reach connectors. The **claude.ai connectors**
  project setting turns them off for a project's sessions, and a
  repository's Claude settings can't turn them back on. Projects created in
  the app start with it off (from the CLI: `fctl projects create
  --no-connectors`); projects from before the setting keep their connectors
  until a maintainer turns them off.
- **fctl.** No fctl credentials enter a sandbox. Through fctl-agent's
  session socket, anything the agent runs can rename its own session and
  describe it (`fctl edit`), and nothing else; the control plane checks
  both values again.
- **Project secrets** are exported to the agent's environment. Treat them as
  readable by anything the agent runs.

The `process` driver has no isolation at all and is for development only.

## Billing

fctl is built so agents only ever use Claude **subscriptions**:

- `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, the
  Bedrock/Vertex/Foundry switches and any other `ANTHROPIC_*` variables are
  removed from the sandbox environment, and project secrets cannot set them.
- A repository whose `.claude/settings.json` sets `apiKeyHelper` or
  `ANTHROPIC_*` variables runs without project settings.
- The session refuses to start if Claude reports any API key as its
  credential source.
- When Claude reports that a turn draws on paid extra usage, the agent is
  interrupted and the session pauses until the plan's limit resets.

## Hosts

Hosts connect outbound to the control plane over TLS (rustls with the Mozilla
root store); they open no ports. A host authenticates with a runner token that
admins can rotate or revoke. Hosts receive a session's credentials only while
it runs, and keep nothing secret on disk but their runner token
(`~/.config/fctl/host-token`, mode 0600).

## Retention

- Deleting a project deletes its sessions with their transcripts (the event
  logs in their Durable Objects) right away; an hourly job retries any that
  fail.
- Expired browser sessions and tokens, password resets, device codes and
  invitations are deleted within the hour, and audit log entries after a
  year.
- People delete their own account in Settings (Danger zone) or with
  `fctl account delete`, and admins can delete anyone's but an owner's. Both
  delete the account's sign-in methods, tokens, Claude connections,
  memberships and shares, and the projects only it is a member of, with their
  sessions. Sessions and projects it created that others still use stay,
  credited to "Deleted user": the account remains only as a disabled
  placeholder, without its name, email or picture. An instance's last owner
  cannot delete their account.
- Settings → Your data (or `fctl account export`) downloads everything the
  instance keeps about you, as JSON.
- Hosts keep a stopped session's sandbox so it can resume, and remove it
  once it has been stopped for `keep_stopped_hours` (72 by default; 0 keeps
  them). Resuming after that starts a new sandbox on the session's branch,
  with everything that was pushed, and a new Claude conversation.

## Reporting a vulnerability

Report it privately through GitHub: **Security → Report a vulnerability** on
[hyperstructure/fctl](https://github.com/hyperstructure/fctl/security), with
details and a way to reproduce. Please do not open a public issue. We will
acknowledge within three working days and keep you updated until it is fixed.
