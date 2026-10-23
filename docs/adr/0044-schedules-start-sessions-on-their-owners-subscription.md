# 0044: Schedules start sessions on their owner's subscription, and other tools run them when asked

2026-10-23 · accepted

## Context

People started agents on a timetable from GitHub Actions: a workflow on a
`schedule`, with an access token that runs sessions
([Automation](../automation.md)), for nightly dependency updates and
weekly triage. That works, but each project has to set up a workflow and
a token, nothing in fctl shows what runs when or what came of it, and a
run that comes while the last is still at work, or while the plan it runs
on is used up, starts anyway (#9).

People asked for sessions that start on a schedule built in: once at a
set time or again and again, set with cron or with a form, for the
chores agents do well unattended: dependency updates, triaging issues,
tidying the project's memory, learning from the week's reviews. They also
asked that other tools, issue trackers first, can start agents for the
cases they are for, and that integrations with them be easy to add later.

## Decision

- **A schedule is a project's task with a time.** It has a name, the task
  its runs' agents get, and what its sessions start with (model, effort
  level, base branch, GitHub access, at most what the project allows
  now). Its time is a cron expression in an IANA time zone, a time to run
  once, or none: a schedule that runs only when asked.
- **On its owner's subscription.** People who start sessions in the
  project set up schedules, on their own Claude subscription, and its
  runs start as them. A change by anyone else (the project's maintainers
  and owners may) runs it on theirs from then on, as a resume does:
  nobody puts work on someone else's subscription. The owner may always
  pause it. Running one now starts a session on the subscription of
  whoever asks.
- **Runs don't pile up.** A run is skipped while the schedule's last
  session still works on its task, and while the owner's plan is at its
  limit, until it resets; a schedule that runs once waits for the reset
  instead. When the owner can no longer run it (they left, lost the role,
  have no Claude connection), fctl pauses it and says why, rather than
  skip it at every time to come.
- **An alarm, not the cron trigger.** A Durable Object of its own keeps an
  alarm for the earliest next run, which D1 keeps with each schedule. The
  hourly cron trigger would run late by up to an hour, and `wrangler dev`
  runs none; alarms ring on Cloudflare, in the standalone server and in
  development alike, to the second. Each run is claimed in D1 before it
  starts, so a retried alarm starts nothing twice.
- **Time zones from the platform.** The control plane reads time zones
  with `Intl`, as browsers do, so neither the Worker nor the web app
  carries a zone database, and both find the same runs.
- **A run says what it is.** Its first message is the task, then fctl's
  note: which schedule started it, when, its previous run, and that
  nobody watches it start, so the agent works without waiting for answers.
  The transcript shows the note apart from the task.
- **Other tools run schedules.** Any schedule runs on request, with what
  the asker knows of why (`context`): from people as theirs, from access
  tokens as information that isn't instructions, since an issue's text
  can come from anyone. A token that runs sessions may run schedules;
  setting one up or changing it takes one that does everything, since its
  sessions start after the token is gone. Pausing a schedule stops runs
  from tools too.

## Consequences

- Hosts and agents need no update: a schedule's run is a session like
  any other. Older CLIs and web apps ignore `Session.schedule`.
- Instances get a new Durable Object class (`ScheduleObject`, migration
  `v4`), which deploys and the standalone server pick up from the
  configuration.
- A schedule that runs every 15 minutes can still start many sessions in
  a day, on its owner's plan and the workspace's runners: the skip rules
  keep them from overlapping, not from adding up.
- A schedule run by a tool gets the tool's text in its first message:
  marking it as information makes it harder, not impossible, for that
  text to steer the agent, which can do no more than its project and the
  token allow.
- An integration that fctl offers itself, with an issue tracker say,
  becomes a trigger that runs a schedule with what it received, through
  the same path, and records itself as the run's trigger.
- Agents can't read their project's retrospectives from the sandbox yet,
  so learning from them on a schedule waits for that (the roadmap's
  "Learning from retrospectives").
