# 0013: Idle sessions stop, and a message resumes them

2026-10-18 · accepted

## Context

A session holds a slot on its runner from the moment it starts until its
sandbox stops. A session that finished its turn and waits for a reply kept
its slot for its project's idle timeout, two hours unless the project set
another, so busy runners filled up with sessions that did nothing, and new
work waited until someone stopped them by hand
([#56](https://github.com/hyperstructure/fctl/issues/56)).

Stopping sooner had two costs. A stopped session refused messages ("Resume
it to continue"): the web app resumed it for owners who typed into it, but
`fctl send`, `fctl attach` and editors got an error. And a resume went to
whichever runner was least loaded, so with more than one machine it could
land in a new sandbox, without Claude's conversation or the work the agent
had left uncommitted. The shorter the timeout, the more often both would
happen.

## Decision

- An idle session stops after ten minutes unless its project sets another
  idle timeout (`fctl_domain::settings::DEFAULT_IDLE_TIMEOUT_MINUTES`).
  Projects that set one keep it; sessions running when this shipped keep the
  timeout they started with until they next start.
- A message to a stopped session resumes it, for anyone who may message the
  agent and from every client: SessionDO queues the session when it stores
  the message, which the new run gets from its outbox. A message that comes
  while the session stops does the same.
- That run uses the Claude subscription the session ran on. A message to an
  idle session runs on it too, so going to sleep changes nothing about who
  pays. An explicit resume still runs on the resumer's subscription, and it
  is still what starts a failed session again: failures can need someone to
  look first.
- FleetDO remembers which runner last took each session and starts a resume
  there, waiting up to ten minutes while that runner is busy. Offline, or
  too small for the session now, and it starts elsewhere right away.
- Two runs of a session never overlap: FleetDO starts the next run once its
  runner no longer reports the last one, and SessionDO ignores what that
  run's uplink says about stopping while the session waits in the queue.

## Consequences

- Runners free their slots ten minutes after a session finishes its turn,
  so queued work starts without anyone stopping sessions, and following up
  works the same whether the session still runs or not.
- A follow-up after the timeout waits for the sandbox to start again, a few
  seconds to a minute depending on the driver, instead of reaching a
  running agent.
- A session waiting for a reply shows as stopped after ten minutes, and
  leaves the floor's "Working now" for "Recently".
- An editor's message can resume a session its owner stopped, on the
  subscription it ran on, as one can keep an idle session working. The
  composer says that a message resumes the session.
- With several runners, a resume may wait up to ten minutes for the runner
  with its sandbox. After that it starts without its conversation, and the
  sandbox it left behind is removed with the host's other stopped
  sandboxes (`keep_stopped_hours`).
- Draining a runner, the other part of #56, is still open; its decisions
  are in the issue.
