# 0055: Maintainers start sessions that reach GitHub issues beyond the tracker

2026-10-24 · accepted

## Context

Since [0052](0052-projects-choose-a-ticket-tracker-and-linear-delegates-tickets-to-fctl.md)
a project's GitHub tokens carry the issues permission only where its
tracker says: nowhere for a project that tracks its work in Linear or
nowhere. A team that moved to Linear asked an agent of such a project to
move the repository's open GitHub issues into Linear, labelled, and close
each with a comment naming its new ticket. The agent could reach Linear,
but GitHub refused it the issues (`Resource not accessible by
integration`), and the repository was private, so it stopped.

The person who started that session asked for agents to be able to move
tickets from one tracker to another: not often, but without changing the
project's tracker and back, which would reach every running session.

## Decision

- **A session can reach GitHub's issues beyond its tracker**
  (`CreateSessionRequest::github_issues`, `fctl run --github-issues`,
  **GitHub issues too** in the start form's GitHub menu). Its tokens
  carry the issues permission as for GitHub Issues of every repository it
  writes to (`fctl_domain::tickets::session_issues`), at the level its
  GitHub access gives it, for as long as it runs.
- **Only maintainers start one.** They may change the tracker anyway; an
  editor could otherwise lift a tracker of none, which keeps agents away
  from GitHub's issues, for any session. A limited access token, whose
  role is an editor's at most, can't.
- **The tracker stays the project's.** The session reaches it as others
  do, `fctl ticket` included, with `gh` where the tracker is one
  repository's issues among several, which its tokens then reach. Its
  agent is told that it reaches GitHub's issues too and to use them only
  as its task asks, to move them into the tracker, say.
- **Agents without it hear how to get it.** Where their tokens reach no
  issues, they are told that a maintainer can start a session that does,
  so that a task like that one ends with a way on rather than a dead end.

## Alternatives

- **A project setting that adds GitHub's issues to Linear.** It would
  reach every session, those that Linear's delegations start too, for a
  task that comes once.
- **Changing the tracker for the move.** Possible before this, but it
  reaches every running session's tokens and what their agents are told,
  and is easily left behind.
- **Moving tickets in the control plane** (`fctl ticket import`). It would
  hold one more tracker's credentials per call and map GitHub's issues onto
  Linear's fields, labels and people itself, where an agent judges them.

## Consequences

- A maintainer moves a repository's issues into Linear with one session.
  Its tokens can read, open, comment on, label and close the issues of the
  repositories it writes to, for as long as it runs, whoever messages or
  resumes it.
- Sessions record whether they got it (`sessions.github_issues`), and
  their pages and `fctl show` say so. Control planes from before ignore
  it, so `fctl run --github-issues` checks the answer and stops a session
  that didn't get it.
