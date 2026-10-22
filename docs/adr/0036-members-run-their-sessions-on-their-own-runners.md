# 0036: Members run their sessions on their own runners

2026-10-22 · accepted

## Context

A session's sandbox holds what its agent works with: the project's code and
secrets, GitHub tokens for its repositories, and the Claude token of the
person it runs as, an access token that lasts hours from a full login or a
token from `claude setup-token` that lasts a year. The machine of the
runner it runs on sees all of it. Only a workspace's admins added runners
([ADR 0024](0024-workspaces-hold-people-projects-runners-and-github-accounts.md)),
and each one ran everyone's sessions there, so every member's Claude token
reached every machine the admins added. A full Claude sign-in, which passes
the login's long-lived refresh token through a host, ran on any runner of
the person's workspaces.

Trusting a workspace's machines with its code is not trusting them with
one's Claude account, and some people want their sessions only on machines
they control. Members couldn't bring one: adding runners took an admin.

## Decision

- **Runners for one person.** A runner added to a workspace runs everyone's
  sessions there or only its owner's (`runner_workspaces.personal`). A
  member may add theirs for their own sessions, which show the machine
  nothing the member can't read in the session: its agent reads the
  project's secrets and uses its tokens. Runners for everyone's sessions
  stay with admins, who add theirs for their own sessions with
  `--personal`. Adding a runner again changes whose sessions it runs, and a
  demoted admin's runners run only their own sessions from then on, rather
  than leave the workspace.
- **A session runs as its Claude connection's owner,** since that is whose
  token the runner sees. A message that resumes a session keeps its
  connection; Resume switches it to the resumer's, and the runner choice
  follows.
- **Only on my runners.** Each member chooses, per workspace, to run their
  sessions there only on runners they registered
  (`workspace_members.own_runners`; in the workspace's Settings, `fctl
  workspaces own-runners`, `PATCH /api/v1/workspaces/{id}/membership`). It
  is per workspace because the trust is: a company's machines for its
  workspace, a friend's for a side project. Turning it on stops their
  sessions on others' runners, which resume on theirs.
- **One rule decides.** `fctl_domain::schedule::runs_for` says whether a
  runner runs a session, for FleetDO's scheduling, `spec::runner_serves`
  (credentials, uplinks, files, screenshots) and Claude sign-ins alike, so
  a runner that stops running a session gets nothing more for it.
- **Sign-ins prefer the person's runners.** A full Claude sign-in runs on
  one of the person's own runners when one is online, else on one that
  runs their sessions in one of their workspaces.
- **Waiting says why.** A queued session that no runner takes says whose
  runner it waits for: any of the workspace's, one for everyone's
  sessions, or one of its person's.

## Consequences

- A member whose sessions run only on their runners waits while none is
  online; the app says so under the task box, and the session says so.
- Members bring machines into a workspace without its admins. The Runners
  tab marks them, and admins take any out. A workspace that wants all of
  its work on its own machines has no switch to refuse them yet; its
  members could take the same code and tokens out through their sessions.
- Hosts and agents don't change: the control plane decides which runner
  gets what. An older CLI that registers or adds a member's runner gets one
  for their own sessions.
- FleetDO reads, for each queued session, whom it runs as and whether they
  keep their sessions on their own runners.
