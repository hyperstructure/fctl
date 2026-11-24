# 0100: Autopilots keep trying after their sessions fail

2026-11-23 · accepted

## Context

An autopilot's own session that failed made fctl try again 5, 15 and 60
minutes later, and the fourth failure in a row paused the autopilot
([ADR 0080](0080-autopilots-run-a-project-on-their-own-through-the-api-their-session-relays.md)):
it woke again only once someone resumed it. On 2026-11-23 a runner's
sandboxes lost their network, every session failed to clone its
repository, and an autopilot paused, though nothing about it needed a
person: once the runner had its network back, it stayed paused until
someone noticed and clicked *Resume*. Autopilots are meant to run a
project on their own, and carry on through what mends by itself.

What fails an autopilot's own session is mostly of that kind: a runner
that lost its network or went away, GitHub or Anthropic down for a
while, a sandbox that couldn't start. What doesn't mend by itself (its
owner gone, their Claude connection removed, the project archived,
autopilots turned off in the workspace) pauses it as it wakes, saying
why, whatever its sessions do. A session that fails before Claude starts,
as most do, costs its owner's plan nothing.

## Decision

- **Failures never pause an autopilot.** After its own session's failure,
  or a failure to wake it, fctl tries again 5, 15 and 60 minutes later,
  then every hour, until a session of its own ends a turn, which counts
  its failures from nothing again, or people pause it
  (`fctl_domain::autopilot::retry_in`). Its log says each failure, and
  when fctl tries again. This supersedes ADR 0080's "its own session
  failing three times in a row pauses it".
- **A retry takes the place of its check-in**, which a wake sets counting
  from then: one with a 15-minute check-in tries again no more often than
  the retries say, unless people or its sessions wake it sooner, or it
  asked to wake sooner.
- **People hear of it**: at the fourth failure in a row, where it paused
  before, those who follow its own session get a notification that its
  sessions keep failing, why the last did, and that it tries again every
  hour (`notify::Kind::AutopilotFailing`), and again every 24 failures,
  a day of hourly tries, while they go on. A new autopilot that has had
  no session yet has no one to tell, until it has one. Its sessions'
  other failures notify nobody, as before.
- Autopilots that fctl paused this way before stay paused until people
  resume them.

## Consequences

- An autopilot whose sessions failed for a while carries on within an
  hour of what failed them mending, without anyone resuming it.
- One whose sessions fail for good starts a session an hour, which fails,
  until people pause it or fix what fails it: a sandbox on a runner, and
  for a failure after Claude started, some of its owner's plan, at most
  24 times a day. Its page says that its last session failed and when it
  tries again, and its people hear of it each day.
