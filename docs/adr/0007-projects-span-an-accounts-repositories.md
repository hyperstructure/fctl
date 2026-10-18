# 0007: Projects span the repositories of one GitHub account

2026-10-18 · accepted

## Context

A project was one repository, and its sessions got an installation token
for that repository alone. An agent could not change a service and its
client library in one session, read a private dependency (`npm ci` in this
repository needs hyperstructure/design), or work across an organization's
repositories ([#17](https://github.com/hyperstructure/fctl/issues/17)).

## Decision

- A project keeps its primary repository, cloned into the agent's working
  directory, and can list more, each read only or read and write, or grant
  read access to every repository of its account that the GitHub App can
  reach (`owner/*`). Owner-wide grants stay read-only: agents write only to
  repositories that are named.
- All of a project's repositories belong to one account, so one
  installation of the GitHub App mints a session's tokens. A token carries
  one set of permissions, so a session gets two, refreshed together: one
  that can write (the primary repository and the writable ones) and one that
  can only read (the read-only ones, or the whole account).
- git picks the token per repository through fctl-agent's credential
  helper, which reads the path git sends (`credential.useHttpPath`). `gh`
  takes one token per host: the one that can write.
- Agents clone the other repositories when they need them, into
  `~/repos/<owner>/<name>`. After each turn fctl-agent finds the clones,
  keeps the work in writable ones on the session branch, pushes it, and
  tracks one pull request per repository (`GitState.repos`).
- When a listed repository leaves the installation (or is renamed or
  deleted), sessions leave it out of their tokens and say so in a notice,
  and carry on. Only an unreachable primary repository fails a session.
- Everything new on the wire is optional: older hosts and agents ignore it
  and work on the primary repository only.

## Consequences

- One stale entry in a project's list does not stop its sessions; people
  see the notice and fix the list.
- A session's tokens reach more than one repository, and with `owner/*`
  every repository the app can read, private ones included.
  [docs/security.md](../security.md) says what each token can do. Write
  access, with workflows, is granted repository by repository.
- Repositories on several accounts need a project per account: several
  installations per project would mean tokens per installation and access
  level.
- Changes to the list apply to sessions that start afterwards; a running
  session loses a removed repository at its next token refresh, within the
  hour.
