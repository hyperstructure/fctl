# 0074: Sessions keep their changes' diffs for their viewers

2026-11-05 · accepted

## Context

The session page lists the files a session's agent changed, with lines
added and taken out (`git diff --numstat`), but not the changes
themselves. People open the pull request to read them, which exists only
once the agent opened one and shows only what it pushed: not the work in
progress, uncommitted work, or a repository the agent doesn't push to.
The diffs are in the sandbox, which runs only while the session does: most
sessions people read have stopped, their sandboxes stopped with them.

## Decision

- **The agent reads the diffs with the state it reports.** fctl-agent
  reads each changed file's diff when it reads the repositories' state,
  from the same commit, and gives each changed file the digest of its diff
  in the `git` event (`FileChange.digest`). Untracked files' are all of
  their lines added.
- **The control plane keeps the latest, not a history.** Diffs aren't
  events: SessionDO keeps the latest diff of each changed file
  (`diff_files`), replaces it when the agent reads another, and forgets
  those of files no longer changed. A session keeps at most 512 KB of a
  file's diff and 8 MB of them all; the others are kept as too large, and
  `fctl diff` prints them where the agent's runner reaches the session.
- **They travel over the uplink, only what changed.** The agent hands its
  host every diff when they change, before the `git` event that has their
  digests; the host sends SessionDO, over the session's uplink, those it
  lacks: all of them over a new connection, then those that changed, in
  frames well within a WebSocket message. On the same connection as the
  events, a diff comes before the state that lists it.
- **Anyone who can see the session reads them**: `GET
  /api/v1/sessions/{id}/diff`, `fctl diff`, and the session page, which
  opens a file's diff from Changes, highlighted as its language is.

## Consequences

- Diffs show for sessions that stopped, as their agents last read them,
  and for work no pull request has.
- The control plane keeps code it didn't before: the changes themselves,
  beside the transcript's edits and the files the agent read. They go
  with the session, when its project is deleted.
- A refresh that changes a file costs that file's diff on the uplink, and
  a new uplink connection all of them.
- Agents of hosts from before send no diffs, and the page says why it
  shows none; a host from before ignores an agent's diffs, which then
  never asks for them.
