# 0018: Background work keeps a session up, for an hour at most

2026-10-18 · accepted

## Context

A session that waits for a message stops after its project's idle timeout,
ten minutes unless the project sets another
([ADR 0013](0013-idle-sessions-stop-and-a-message-resumes-them.md)).
Agents also wait for work they start in the background: a shell command
with `run_in_background`, a subagent, a Monitor. They end their turn to
wait for it, and Claude Code hands them the result in a turn of its own.

fctl-agent saw none of it. The idle clock ran from the end of the turn,
and ten minutes later the session stopped while the work still ran, and
the work died with the sandbox. Turns that Claude started on its own went
unnoticed too: the session kept saying it waited for a message, its idle
clock kept running, and a stop ended Claude at once instead of
interrupting the turn
([#87](https://github.com/hyperstructure/fctl/issues/87)).

Claude Code reports each background task starting (`system/task_started`)
and ending, for any reason (`system/task_notification`). It reports its
turns too (`system/session_state_changed`) when
`CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS` is set. 2.1.101, the oldest
release fctl runs, has all three.

## Decision

- **fctl-agent follows background tasks.** While any runs after a turn,
  the session waits for it and says so ("Waiting for 2 background tasks"),
  and the idle clock starts when the last one ends. Claude Code's own
  housekeeping (tasks it marks `ambient`) doesn't count.
- **A turn Claude starts on its own is a turn.** The session runs, the
  idle clock stops, and a stop or an interrupt reaches the turn. A session
  paused for a usage limit interrupts it, since it waits for the reset.
  fctl-agent learns of these turns from `session_state_changed`, or from a
  turn's first message when Claude Code doesn't report its turns.
- **For an hour at most.** Background work keeps a session up for an hour
  after its last turn, or for the project's idle timeout when that is
  longer, so that it never stops a session sooner than waiting for a
  message would. Then the session stops anyway. A fixed hour covers a
  review or a long build and needs no new setting; a project setting can
  come later if an hour proves too short or too long.
- **It holds its slot.** A session that waits for background tasks counts
  against its runner's capacity as any session with a sandbox does: the
  work uses the sandbox's CPUs and memory.
- **Nothing stops unsaid.** When a session stops while background tasks
  run, the transcript names them, and the next message tells Claude they
  didn't finish. The system prompt says how long background work may run
  after a turn ends, so that agents know when waiting is safe.

## Consequences

- An agent can end its turn to wait for a review or a build of up to an
  hour, and carries on when it finishes, without anyone sending a message.
- A session whose agent leaves a dev server running holds its runner's
  slot for an hour after the turn instead of ten minutes. The system
  prompt asks agents to stop what they no longer need.
- The reason a session waits is text, so hosts, the control plane and the
  protocol are unchanged. The web app shows the reason beside a waiting
  session's status when it is anything but waiting for a message. Agents
  of older versions keep stopping at the idle timeout.
- Draining runners still stop these sessions after a minute, as they stop
  sessions that wait for a message. Letting a drain wait for background
  work would need the control plane to know about it.
- A Claude Code that exits unexpectedly loses its background tasks, and
  fctl-agent stops following them; the restarted Claude Code doesn't know
  them either.
