# 0002: Agents only ever run on Claude subscriptions

2026-10-17 · accepted

## Context

Claude Code can authenticate with API keys (billed per token) or with a
Claude Pro/Max subscription. A factory that runs agents around the clock
must never surprise anyone with API bills.

## Decision

Sessions authenticate only with subscription OAuth credentials: a
long-lived token from `claude setup-token`, or a full `claude auth login`
run on a host. The agent supervisor strips `ANTHROPIC_*` and the
Bedrock/Vertex/Foundry switches from Claude's environment, ignores project
settings that set `apiKeyHelper` or such variables, refuses to start if
Claude reports an API key as its credential source, and pauses a session
the moment Claude reports paid overage use.

## Consequences

- Each person's sessions use their own plan's limits; limits pause
  sessions, which resume when the window resets.
- Remote Control (following sessions in the Claude apps) needs the full
  login, so that path is the recommended one.
- Refresh tokens stay in the control plane; sandboxes get short-lived
  access tokens and ask for new ones through their host.
