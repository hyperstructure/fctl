# 0081: An autopilot's messages wait for room at its session limit

2026-11-15 · accepted (experimental)

## Context

An autopilot runs at most as many sessions at once as its people allow
([ADR 0080](0080-autopilots-run-a-project-on-their-own-through-the-api-their-session-relays.md)).
A message or a resume sets a session that rests working again, so the
control plane refused one to a resting session while as many worked:
`fctl send` failed, even for "approved, merge on green". An autopilot
that ran a project for days wrote scripts that tried `fctl send` and
`fctl run` every two minutes until one went through, which a fresh
session of its own lost at every handover, and asked for this to be
built in (fctl#550). It suggested two ways: hold such a message until
there is room, or let a message through to a resting session whatever
the limit.

## Decision

- **fctl holds the message**, rather than let it through: the limit is
  about how much of its owner's plan runs at once, and while a short
  approval rarely costs much, nothing tells how much a message will set
  going. A message from an autopilot to one of its sessions that rests
  (it ended its turn, stopped, or waits for its background work) while
  as many of its sessions work as its people allow is kept in D1
  (`autopilot_held`, at most 30 an autopilot), and so is one to any
  resting session while others are held, which go first, and one to a
  session that messages held for it wait for. A session that works
  takes messages at once, as it needs no room.
- **They go out in the order it sent them, as soon as there is room**:
  as one of its sessions ends its turn, stops or fails, or starts working
  again (SessionObject calls `autopilots::release` as it reports the
  move), at its wakes, as it sends another, and as people raise its
  limit. Each takes room once, with any later message to the same
  session. One for a session that failed or is gone is dropped, and its
  log says so. They wait while it is paused; taking the project off
  autopilot drops them, as they would arrive long after they were meant.
- **The API says it's held**: `SendMessageResponse.held` (its id and how
  many go before it), with the delivery it later goes out as. `fctl send`
  says "Held: it goes to shop#41 once one of your sessions ends its
  turn…", its briefing says not to send it again, its brief lists those
  held, and it (or those who steer it) takes one back (`fctl autopilot
  held --take-back`, `DELETE …/autopilot/held/{id}`). A retry of the
  request finds the one held.
- **Resuming a failed session and starting one still fail at the limit.**
  Holding those would mean replaying a start or a resume without the
  request that asked for it; a resume is rare, and a start is what the
  limit is for. The briefing keeps telling it to start the next session
  as fctl wakes it for one that ended its turn.

## Consequences

- The autopilot no longer needs scripts that poll to steer its sessions
  at the limit, and nothing it relies on is lost at a handover: held
  messages are fctl's, listed in its brief.
- A held message can arrive well after it was written, as the session it
  waits for may take hours to end its turn: its brief shows what waits,
  so that it takes back what no longer holds.
- Two releases that race (two sessions ending their turns at once) can
  each see the same room and set one session more working than the
  limit, as two messages sent at once could before; a delivery they both
  make arrives once, by its idempotency key.
