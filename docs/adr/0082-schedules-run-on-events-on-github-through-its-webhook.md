# 0082: Schedules run on events on GitHub, through its webhook

2026-11-17 · accepted

## Context

Schedules start sessions at their times, or when people and tools run
them ([ADR 0044](0044-schedules-start-sessions-on-their-owners-subscription.md)).
Much of what people want an agent for starts with something happening
instead: an issue opened, to triage it; a pull request merged, to learn
from what people said in its review and keep it in AGENTS.md or a skill;
the default branch's checks failing, to find why and fix it. Until now
that took a workflow in GitHub Actions on `issues: [opened]` with an
access token that runs sessions, or a schedule that went through what
happened since its last run once a day, late and in a batch.

fctl's GitHub App already gets those events through its webhook, signed,
for the repositories of the accounts workspaces connected
([ADR 0032](0032-githubs-webhook-keeps-sessions-pull-requests-current.md)).
ADR 0044 expected integrations to become triggers that run a schedule
with what they received, through the same path.

## Decision

- **An event is another kind of "when".** A schedule runs at its times,
  on an event, or only when asked; never at times and on events both. Its
  owner, the rights to change and pause it, its model, effort level,
  GitHub access, hidden sessions, runs and Run now stay as they are, so
  that people learn one thing: the Schedules tab's form gets **On an
  event**, and `fctl schedules add` gets `--on`.
- **Six kinds to start with**, each one a delivery fctl's app gets: an
  issue opened (`issues`), an issue labeled, a pull request opened or a
  draft marked ready (`pull_request`), a pull request merged, a release
  published (`release`), and checks failing on a branch (`check_suite`,
  and commit statuses, once all the checks of the branch's newest commit
  ended, looked up as for the checks after merges). The kind is a string
  that older clients read as `other`, so that more (Linear's issues, a
  discussion, a deployment, fctl's own events) need no new shape.
- **Narrowed down in a few words**: the project's repositories (those its
  agents write to unless named; a read-only one for a dependency's
  releases), labels, branches, and who opened it (anyone, people who may
  write to the repository, or fctl's agents). The form says it as a
  sentence, as the control plane does (`triggers::describe`).
- **One run for each event, on its owner's subscription.** Each event
  starts a session of its own, rather than one at a time as a cron's runs
  do: two new issues are two pieces of work. A key claimed in D1 keeps a
  delivery GitHub sends again from starting a second. Runs don't pile up
  on the same thing: while a run works on an issue, a pull request, a
  release or a branch's checks, events about it start none. A schedule's
  events start at most 20 runs an hour, past which they are skipped and
  its runs say why; a schedule keeps all of the last day's runs, so that
  a flood can't trim away what the cap counts. One delivery starts at
  most ten sessions, so that GitHub gets its answer in time.
- **What GitHub says is information.** The run's agent gets the task,
  fctl's note on what happened, then what GitHub said of it (title,
  author, labels, branches, text cut at 8,000 characters) inside
  `<context>`, as for a tool's runs: anyone can open an issue on a public
  repository. A pull request opened starts runs only for people who may
  write to its repository unless the schedule says anyone, as reviews
  start; the triage idea runs at `triage`, which labels and comments and
  does nothing else ([ADR 0075](0075-agents-that-triage-write-labels-and-comments-through-fctl-alone.md)).
- **Only the workspaces the account is connected to hear of it**, through
  the installation that sent the delivery, as sessions do
  (`pull_requests::in_reach`).
- **Say why it wouldn't run, before anyone chooses.** Each kind needs its
  webhook event, which an app made before may not be subscribed to
  (app.fctl.sh's had no **Issues**): the control plane asks GitHub what
  the app is subscribed to (`GET /app`, at most every ten minutes) and
  the API lists the kinds that wouldn't start runs, with why
  (`ScheduleList.unheard`, and each schedule's `problem`).

## Consequences

- Triaging each issue as it's opened, working on issues people label
  `fctl`, learning from merged pull requests, fixing failing checks on
  main and trying each release need neither a workflow nor a token; the
  Schedules tab offers each as an idea.
- An instance whose app isn't subscribed to **Issues** or **Release** has
  its admins subscribe it under **Permissions & events**; installations
  accept nothing new, since the app has the permissions already. Until
  then the tab says so, and those schedules start nothing.
- A busy public repository can start up to 20 sessions of one schedule an
  hour on its owner's plan: the hourly cap, the per-issue rule and
  hiding its sessions keep that bounded and out of the way, but not free.
- Checks failing costs a lookup of the commit, a point of the
  installation's GraphQL rate limit, for each check suite that completes
  on a branch a schedule watches.
- CLIs from before it fail to read a schedule's runs that an event
  started (`trigger: "event"`) until they are updated; triggers a later
  version adds read as `other`.
