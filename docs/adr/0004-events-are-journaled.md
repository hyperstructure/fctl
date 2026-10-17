# 0004: Agent events are journaled in the sandbox and acknowledged

2026-10-17 · accepted

## Context

Sessions run for hours over home connections, laptops sleep, hosts
restart. Losing a tool call or a message silently would make transcripts
untrustworthy.

## Decision

`fctl-agent` numbers every event (`agent_seq`) and appends it to a journal
in the sandbox before sending it. The session's Durable Object stores
events, assigns the canonical `seq`, and acknowledges `agent_seq`; after any
reconnect it announces the last acknowledged number and the host replays the
rest. Messages to agents wait in the Durable Object's outbox until the agent
confirms them, and the agent remembers delivered ids, so delivery is at
least once and effectively exactly once. Streaming deltas are forwarded but
not stored.

## Consequences

- Reconnects, host restarts and control plane deploys lose nothing.
- The journal is compacted once events are acknowledged, so it stays small.
- Viewers can join late: the first frame is a snapshot including the text
  being streamed right now.
