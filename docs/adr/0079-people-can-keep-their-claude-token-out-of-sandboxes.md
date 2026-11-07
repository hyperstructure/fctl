# 0079: People can keep their Claude token out of sandboxes

2026-11-07 · accepted

## Context

Claude Code needs its owner's Claude token to reach the Anthropic API, and
fctl gave it the token as Claude Code expects it: `CLAUDE_CODE_OAUTH_TOKEN`
in its environment. Claude Code leaves that variable out of the commands it
runs, but those commands run as the same user: they read the token from
`/proc/<claude's pid>/environ` (`ps eww` shows it too) and from Claude's
memory, and the default image gives them `sudo`. So whoever can direct a
session's agent, or text in a repository or on a web page that steers it,
can have it hand the token over. With long-lived tokens from `claude
setup-token` now recommended, that is a year of access.

Nothing inside the sandbox can keep the token from the agent, since the
agent is root there. Passing the token on a file descriptor
(`CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR`) keeps it out of Claude's
environment, but a command still found 51 copies of it in Claude's memory;
Claude Code makes itself non-dumpable only for Anthropic's code reviews.
The token has to stay out of the sandbox.

Claude Code supports TLS-inspecting corporate proxies: it honors
`HTTPS_PROXY` for all its traffic and trusts certificates in
`NODE_EXTRA_CA_CERTS`. Run against such a proxy with a made-up token, Claude
Code 2.1.319 finished a turn, still counted as a subscription login
(`apiKeySource: none`, first party), and sent every request that carried
the token to `api.anthropic.com`.

The person who started the session that built this asked for it as an
opt-in setting, since how well it works with every Claude Code feature
isn't known yet.

## Decision

- **A setting of each person's**, off by default: **Keep my token out of
  sandboxes** (Settings → Claude, `fctl claude set --stand-in on`,
  `users.claude_stand_in`). It applies to all their connections, for the
  sessions that start after they change it. Turning it off again is a
  sensitive action ([ADR 0039](0039-sensitive-actions-take-a-passkey-or-a-code.md)),
  so a stolen session can't have agents hand the token over by turning it
  off first.
- **The control plane flags the credential** (`ClaudeCredential.stand_in`)
  and still sends the token, or for one kept on the person's runners lets
  the host fill it in ([ADR 0050](0050-claude-tokens-can-stay-on-their-owners-runners.md)):
  the host needs it.
- **The host keeps the token.** It gives the agent a fixed stand-in that
  Anthropic refuses (`CLAUDE_STAND_IN`), in the spec and in every
  credential it passes on, with the certificate of a certificate authority
  of its own (`AgentConfig.claude_relay`). The authority's key stays in a
  file next to `host.toml` (mode 0600), and it signs a certificate for
  `api.anthropic.com` alone. An agent too old to take a relay fails the
  session rather than get the token.
- **fctl-agent proxies Claude's HTTPS.** Claude gets `HTTPS_PROXY` at a
  loopback port of fctl-agent's, `NODE_EXTRA_CA_CERTS` with the host's
  certificate, a `NO_PROXY` that never covers the API, and the stand-in. A
  `CONNECT api.anthropic.com:443` waits for the host, which opens a channel
  to the sandbox's relay socket (`agent.relay.sock`) the way it attaches to
  fctl-agent, over the driver's exec channel, so every driver has it.
  Everything else Claude and its commands reach goes straight out, or
  through the proxy the session had.
- **The host puts the token in.** It answers Claude's TLS on the channel,
  reads each request, replaces `Bearer <stand-in>` with the session's token,
  and sends the request to `api.anthropic.com` over TLS that checks the
  API's certificate, as anything else the host reaches. Requests without the
  stand-in go on as they are; upgrades are refused. The token goes nowhere
  but the Anthropic API.
- **The control plane's flag decides, not the sandbox.** An agent the host
  doesn't relay for gets no credential flagged `stand_in`: neither a session
  that started before its owner turned the setting on, whose Claude keeps
  the token it has but gets no fresh one, nor a sandbox that says after a
  host restart that it doesn't relay, which a root agent could.
- **The stand-in is the same everywhere.** Its secrecy would add nothing:
  what a request may use is decided by the channel it comes on, which only
  that session's sandbox has. So a host that restarts needs no state to
  serve a session again: the agent's hello says it relays, and the host
  asks the control plane for the session's token.

## Consequences

- Nothing in the sandbox has the token: not Claude's environment or memory,
  nor fctl-agent's. An agent, root or not, has only the stand-in to give
  away. It can still use the token through the host while its session
  runs, for requests to the Anthropic API alone, as Claude does.
- Claude reaches the API only while the runner's host runs. A host that
  restarts (each update does) leaves Claude's requests failing for those
  seconds, which Claude Code retries. Each connection takes an exec of the
  driver's: tens of milliseconds with Docker, more with Lima.
- Claude's commands inherit `HTTPS_PROXY`, so their HTTPS goes through
  fctl-agent's proxy, which passes it straight out.
- Hosts older than this ignore the flag and give the agent the token, until
  they update themselves to the control plane's version
  ([ADR 0053](0053-hosts-update-themselves-to-the-control-planes-version.md)).
- The `process` driver gives Claude the token as before: its agents run as
  the host's user, who can read the host's memory, and fctl-agent's proxy
  would listen on the machine's loopback, where its other users reach it.
  Debug builds relay there all the same, so `cargo xtask dev` can try it.
- Full logins' Remote Control hasn't been tried with it, and claude.ai
  connectors reach `mcp-proxy.anthropic.com`, whose requests the host
  doesn't see, so they fail. The setting says so.
- Debug builds take `FCTL_DEV_CLAUDE_API` in place of the Anthropic API, so
  `cargo xtask dev` can run real Claude Code against a stand-in; release
  builds ignore it.
