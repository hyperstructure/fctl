# 0083: Schedules cap the sessions their events start, an hour and a day

2026-11-17 · accepted

## Context

A schedule that runs on events starts a session for each issue opened, pull
request merged or label added, on its owner's Claude plan
([ADR 0082](0082-schedules-run-on-events-on-github-through-its-webhook.md)).
Anyone can open issues on a public repository, and a script or an import
can open dozens at once. ADR 0082 capped each schedule at 20 runs an hour,
the same for everyone: 480 sessions a day at most, too many for most
plans, and nobody could choose fewer, or more for a busy repository whose
triage is cheap.

## Decision

- **Each schedule that runs on events says how many sessions its events
  start at most, in an hour and in a day**: `ScheduleEvent.max_per_hour`
  and `max_per_day`, set in the form ("At most [3] sessions an hour and
  [10] a day") and with `fctl schedules add` and `edit`'s
  `--max-per-hour` and `--max-per-day`. They go with the event, as what
  it waits for does, since only events start runs that need them.
- **3 an hour and 10 a day unless it says otherwise**, which a schedule
  keeps once saved; those from before read the same. A triage at
  `triage` costs little and can take more; one that opens pull requests
  for each labeled issue fewer. At most 100 an hour and 1,000 a day, and
  an hour never more than a day.
- **An hour and a day are the last 60 minutes and the last 24 hours**, not
  the clock's, so that no midnight lets a second day's worth through at
  once. A schedule keeps all of the last day's runs already (ADR 0082), so
  that what the caps count is never trimmed away.
- **Runs other deliveries are still deciding count too.** GitHub sends a
  burst's deliveries at once, and each run is claimed in D1 before its
  session starts; a run that counted only sessions already started would
  let each of them through. A claim counts for ten minutes at most, should
  its Worker stop before deciding.
- **Past either cap, events start none, and say until when**: the run's
  reason and, until then, the schedule's `problem` ("It started 10
  sessions on events in the last 24 hours, the most it starts in a day, so
  events until … start none."). Runs people and tools start don't count,
  as before: they have their own 20 an hour.

## Consequences

- Event schedules start at most 10 sessions a day unless their people
  raise it, where ADR 0082 let them start 480; a schedule set up before
  this change goes from 20 an hour to 3, and its people raise it if they
  want more.
- Events past the cap are skipped, not queued: a flood of issues leaves
  most of them untouched, and the runs list which. A schedule at a time
  can go through what happened since its last run to catch up.
- Control planes before this one ignore the caps and start up to 20 an
  hour.
