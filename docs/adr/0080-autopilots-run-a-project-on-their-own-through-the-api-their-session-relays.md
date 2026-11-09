# 0080: Autopilots run a project on their own, through the API their session relays

2026-11-09 · accepted (experimental)

## Context

People start each session with a task, and an agent works on that task
alone. To have agents build something out over days (a CRM in a new
GitHub organization, the polish of an app, a backlog worked through), a
person had to plan the work, start each session, read what each did, set
up what the next needed (a setting, a secret, a repository, a schedule),
and come back when one stopped. Carl Oscar asked for an agent that does
that itself: one with the autonomy to start its own fctl agents and
manage the project's settings, secrets, repositories and schedules, that
keeps itself running, owns its backlog, asks people for input (credentials
among it) without getting stuck waiting, and takes their steering at any
time; on a project, or on a whole workspace, where it also creates
projects; marked as experimental.

Agents reach the control plane from their sandbox only through fctl-agent
and their host, for what the sandbox's `fctl` asks of their own session
(`fctl edit`, `fctl ask`, `fctl memory`): no fctl token enters a sandbox
([ADR 0007](0007-agents-name-their-sessions-through-fctl-agent.md)). An
agent that manages a project needs most of the API besides.

## Decision

- **An autopilot is a project's** (its home, one per project), on the
  subscription of the person who put it on, its owner, as whom it acts.
  Its scope is that project, or the whole workspace, from that project:
  one per workspace, put on by its admins, which also creates projects.
  Its workspace's admins let projects have autopilots first
  (`workspaces.autopilots`); turning that off pauses them. Putting a
  project on autopilot, or taking an autopilot over, is a sensitive
  action, as making a token that does everything is.
- **It works in sessions of its own** in its home project, which wakes
  go to while they started less than 12 hours ago and can take a message;
  after that, or once one failed, fctl starts it a fresh one with a
  briefing. It keeps what must outlast a conversation where people see it:
  a plan, a log, and the project's memory. Sessions it starts are its
  crew: up to as many at once as people allow, at most 20 an hour.
- **fctl wakes it** on the schedules' alarm, which ScheduleObject already
  rings: at least at its check-in (15 minutes to a day), at the time it
  asks for (5 minutes to a day ahead), a few seconds after people steer or
  answer it, and a minute after one of its sessions ends a turn, asks
  something or fails, gathered. Its own session failing three times in a
  row pauses it; its owner losing the project, their account or their
  Claude connection, or the project being archived, too.
- **Its sandbox's `fctl` reaches the API through its session**
  (`AgentCall::Api`): fctl-agent and the host pass each request on, and
  the control plane answers it through the same handlers as anyone's, as
  the owner, held to the autopilot's places (its project, or its
  workspace's), a maintainer's role, and a list of what an autopilot does
  (`fctl_domain::autopilot::allows`): read its projects and sessions,
  start sessions and steer only those it started, and as far as its powers
  go (settings, secrets, repositories, schedules, projects) change the
  rest. Never people, roles, shares, tokens, runners, GitHub accounts, the
  workspace's settings, deleting a project, or anything that takes
  confirming it's you. It replaces or removes only the secrets it set.
  The alternative, a short-lived token in the sandbox, would have put a
  credential where anything the agent runs reads it; this way access
  ends with the session, the runner and the autopilot's state, which the
  control plane checks at every request.
- **People steer and answer it without it waiting.** Notes stay with it
  until removed, and a newer note wins; its direction is theirs to change.
  It asks a question with answers to click and Markdown that helps answer
  (screenshots of two designs), or for a secret, which people type in on
  its tab and which goes into the project's secrets: the autopilot hears
  only that it's set. Asks notify the people who follow its session, its
  owner among them, and show in *Needs you*.
- **What it does shows where people look**: its tab (where it stands,
  what it asks, notes, direction, plan, its sessions live, its log), its
  own transcript (a card for each wake, a live card for each session it
  starts), the sessions it starts (its words as its own, a link back) and
  session lists (its mark).

## Consequences

- A new kind of caller, an autopilot, reaches a narrow part of the API
  from a sandbox. Whatever runs in that sandbox can ask what the autopilot
  may, as with `fctl memory`; prompt injection there can do no more than
  the autopilot's powers, within the owner's roles, while it is on.
- Hosts and fctl-agents from before autopilots refuse `AgentCall::Api`,
  and older images' `fctl` lacks `fctl autopilot`: the autopilot's session
  says so, and works once its runner updates.
- A project on autopilot spends its owner's Claude plan without anyone
  starting sessions: its check-in, crew size and model are theirs to
  choose, a change by anyone else runs it on theirs, and runs wait while
  the plan is at its limit.
- Its sessions' events go through SessionDO as any session's; the
  autopilot's log, notes and asks are rows in D1, kept per project and
  deleted with it.
