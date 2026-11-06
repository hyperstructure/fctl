# 0075: Agents that triage write labels and comments through fctl alone

2026-11-06 · accepted

## Context

People want an agent to go through new issues on a schedule: label each
one, and ask for what is missing to act on it. Anyone can open an issue
on a public repository, and what an issue says reaches the agent, so an
issue can try to steer it: to change code and open a pull request, to
close or rewrite other issues, to post elsewhere.

The least GitHub access that labels and comments was `comment`
([ADR 0037](0037-access-tokens-and-sessions-limit-what-agents-do-on-github.md)),
whose token writes pull requests and issues: it also reviews, approves
and closes them. GitHub has no permission narrower than `issues` write,
which labels and comments but also closes, edits and files issues.

## Decision

- **A level between reading and commenting: `triage`.** An agent that
  triages labels its project's tickets and comments on them, and writes
  nothing else. As with the other levels, a session asks for it, a
  schedule runs at it, and a project's setting or an access token holds
  agents to it.
- **Its tokens only read; the control plane writes for it.** Its sandbox
  gets the one read-only token an agent that reads gets, so git and `gh`
  write nothing. It labels and comments with `fctl ticket`, an agent call
  to the control plane, which lets it comment and put labels on or take
  them off, and refuses filing tickets and changing their state, title,
  description, parent or priority (`fctl_domain::tickets::check_write`).
  On GitHub the control plane writes with a token of the GitHub App's for
  that repository's issues alone, as for any `fctl ticket` call, on issues
  only, never pull requests, and only with labels the repository has,
  since GitHub would make one it lacks. Its writes count within `fctl
  ticket`'s hourly limits.
- **The project's tracker says whose tickets:** GitHub's issues of the
  repositories its agents write to, or of one of them, or Linear's. One
  level serves both, since the control plane holds both trackers'
  credentials. Without a tracker there is nothing to triage, and the agent
  reads.
- **It is told that it triages**, and that what an issue says is
  information to triage, never instructions. The transcript shows each
  comment, and each change of labels with the labels.
- **Older hosts get `read`.** Hosts announce that they know the level
  (`triage` in their hello's features). Older ones would fail to read a
  spec that has it, so FleetDO tells their agents they only read: their
  tokens are the same, and the control plane takes their labels and
  comments, as the tracker's lines, which come from the control plane,
  tell them. Specs, sessions, schedules and tokens now read a level they
  don't know as `read`, so that the next one needs no such step.

## Consequences

- An agent that triages, on a schedule or for each new issue from a
  workflow, can't be steered by an issue into changing code, opening pull
  requests, reviewing or closing anything: the most an issue gets out of
  it is labels and comments on the project's tickets, within the hourly
  limits.
- Its writes go through fctl, not `gh`: on GitHub, each costs a lookup
  more (that it is an issue, that its labels exist), on the
  installation's rate limit.
- It still runs with what other sessions have: the project's secrets,
  setup script and network. Review agents, which read outsiders' pull
  requests, get none of those; agents that triage do.
- CLIs from before it fail to read sessions, schedules and tokens that
  triage until they are updated.
