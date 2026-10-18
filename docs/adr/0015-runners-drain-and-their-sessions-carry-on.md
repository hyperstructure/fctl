# 0015: Runners drain, and their sessions carry on

2026-10-18 · accepted

## Context

There was no way to take a runner out of rotation and still serve the
sessions it ran ([#56](https://github.com/hyperstructure/fctl/issues/56)).
The tools were `fctl host uninstall` and disabling the runner, and both cut
the runner off: its agents kept working in their sandboxes, nothing of it
reached the app, and ten minutes later the sessions showed as stopped.
People reached for them for maintenance, or because a disk was filling up,
when all they wanted was no new work on that machine.

Two things shaped the answer. Runners are registered by admins, but the
person who looks after a machine may only have its runner token. And since
[0013](0013-idle-sessions-stop-and-a-message-resumes-them.md), a resume goes
back to the runner that has the session's sandbox, where Claude's
conversation is.

## Decision

The person who started the session that built this decided the open
questions in #56:

- **Both admins and the machine drain a runner, and it is one state.**
  Admins drain from the runners list and with `fctl admin drain`; the host
  drains itself with `fctl host drain`, using its runner token over HTTP
  (`POST /api/v1/runner/drain`). That works while the daemon is down,
  answers with what still runs, and needs no protocol change. FleetDO keeps
  the state, with who started the drain and when; either side can end it.
- **A resume of a session whose sandbox is on a draining runner starts
  there.** Draining stops new work; a resume continues work the runner
  already has, keeps Claude's conversation and needs no new sandbox. Only
  removing a machine has to move its sessions elsewhere.
- **Idle sessions on a draining runner stop after a minute**, not after
  their idle timeout, so a draining runner is empty whenever no turn runs,
  and `fctl host drain --wait` returns soon after the last turn ends. A
  message resumes them there. SessionDO decides, and marks the session
  stopping in the same step, so that none stops with a message on its way.
- `Runner` gets an optional `drain` (when, and the admin who drained it)
  rather than a new `RunnerStatus`: CLIs one version behind would refuse a
  status they don't know.
- Disable and Remove in the runners list ask first, and point to Drain when
  sessions run.

## Consequences

- People take a runner out of rotation without dropping its sessions, from
  the machine or from the app.
- A draining runner may never empty while people keep following up on its
  sessions. Removing the machine is a separate, explicit step.
- A follow-up to a session on a draining runner waits a few seconds for its
  sandbox to start again, rather than reaching a running agent, once the
  session has been idle for a minute.
- An admin can end a drain the machine's owner started, and the other way
  round. The runners list says who started it, and the audit log records
  both.
- Draining needs no host release, except for `fctl host drain` and
  `undrain` themselves. The session's last status reads "Stopped after
  being idle", since older hosts know no other reason to stop an idle
  session; the transcript shows "Stopping: … is draining" before it.
