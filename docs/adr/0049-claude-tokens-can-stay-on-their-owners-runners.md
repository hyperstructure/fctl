# 0049: Claude tokens can stay on their owner's runners

2026-10-23 · accepted

## Context

Every Claude connection kept its token in the control plane: a token from
`claude setup-token` pasted into fctl, or a full login's tokens, encrypted
in D1 under the master key. The control plane put an access token into
each session's spec, for the runner the session ran on.

[ADR 0036](0036-members-run-their-sessions-on-their-own-runners.md) let
people keep their sessions on runners they registered, so that no one
else's machine sees their token. The control plane still held it, though:
whoever reads D1 and has the master key (a breach, an operator of the
instance) has every token, and a token from `claude setup-token` lasts a
year. People who run their own runners asked to keep the token on them
alone, and never hand it to app.fctl.sh.

## Decision

- **A connection whose token stays on its owner's runners.** People
  connect "On your runners" (Settings → Claude, `fctl claude connect
  --on-runners`, `POST /api/v1/me/claude/on-runners`). Its row in D1 has no
  secret (`claude_credentials.on_runners`, `secret_enc` empty); its `kind`
  is `setup_token`, which is what it holds, so clients from before it show
  it as a long-lived token. One per person: a runner keeps one token, its
  owner's.
- **Runners keep the token.** `fctl host claude set-token` keeps a token
  from `claude setup-token` in `claude-token` next to `host.toml` (mode
  0600, or `claude_token_file`); `status` and `remove-token` go with it.
  Hosts say in their hello and heartbeats whether they keep one and when it
  was saved (`RunnerClaudeToken`), never what it is. Only full logins'
  tokens can be refreshed, and those need the control plane: runners keep
  tokens from `claude setup-token` alone, so sessions on them have no
  Remote Control, as with any such token.
- **Its sessions run only on its owner's runners that keep it.** Such a
  connection keeps its sessions on their person's runners as **Only on my
  runners** does (`fctl_domain::schedule::only_theirs`), in every
  workspace, and FleetDO starts them only on those runners whose host says
  it keeps a token. A session that waits says whose runner it waits for and
  why (`TokenWait`): none of theirs is in the workspace, none of those is
  online, or the ones online keep none (or run an fctl too old to).
- **The host fills the token in.** The spec and fresh credentials carry
  the connection with an empty token and `on_runners`; the host puts its
  own in before the agent gets them, or refuses the session, saying what to
  run. Fresh credentials still go through FleetDO, which checks that the
  session is the runner's and its person may still run it.
- **Hosts redact Claude tokens** from everything an agent sends to the
  control plane (events, logs, state, calls, and what Claude streams live,
  whose deltas may split a token), for every session: the token the
  session started with, any it got since, and the one the host keeps. An
  agent that prints its environment leaves `[Claude token, redacted by
  fctl]` in the transcript.

## Consequences

- The control plane is never given these tokens: D1 holds no secret for
  these connections, hosts tell it only whether they keep one, and they
  take the token out of what sessions send wherever it appears whole. Only
  the person's own machines keep it. Without a runner of theirs that keeps
  one online, their sessions wait; the app says so under the task box and
  on the session.
- The agent still has the token in its environment, so whoever directs it
  (the person, people they share the session with to message, the
  project's editors, text in a repository) can have it send the token
  elsewhere, or write it in a form the redaction doesn't know, as before. Hosts trust the control plane
  to send them only their owner's sessions on such a connection, as they
  trust it with whatever else they run, so a control plane taken over
  could do the same by starting sessions there: keeping the token on the
  runner protects it at rest, and from other people's machines.
- Removing the token from a runner applies to the runs that start after:
  sessions running there keep it until they stop. A replaced token reaches
  running sessions only when Claude asks for a fresh one.
- Hosts that can keep a token send `claude_token` in every hello; older
  hosts don't, and get none of these sessions. Agents need no change: the
  token reaches them as any token does. Older clients and control planes
  ignore the new fields.
