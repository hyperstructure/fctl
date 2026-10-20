# 0027: Agents report their repositories while Claude works

2026-10-20 · accepted

## Context

fctl-agent reported what a session's repositories hold (changed files,
commits, whether the branch is pushed, pull requests) in a `git` event in
two places: after each turn, when it also pushes new commits and looks up
pull requests, and when the session stops. Everything people see about
the repositories comes from the latest of those events: the session page's
Changes and Pull requests sections, and the session lists' change counts
and pull requests.

Agents often do their whole task in their first turn, and wait for CI in
it too, since nothing wakes a session when CI finishes. One session ran
its task as a single 30-minute turn: it committed after 19 minutes, pushed
and opened a pull request after 28, and all that time its page said "No
changes yet" while its Work counters moved
([#132](https://github.com/hyperstructure/fctl/issues/132)).

## Decision

- **fctl-agent refreshes the state while Claude works.** After the
  results of tool calls that can change the repositories (`Bash`,
  `Write`, `Edit`, `NotebookEdit`, a subagent's too), and when a
  background task ends. A burst of them costs one refresh, two seconds
  after the last, and refreshes come at most every ten seconds. A refresh
  emits a `git` event only when the state changed; a turn's end and a
  stop emit one as before.
- **It only reads.** fctl-agent pushes when a turn ends, as it tells
  agents, and not in between; work in a writable clone moves onto the
  session branch then too.
- **GitHub isn't polled.** A refresh looks up a branch's pull request
  (`gh pr view`, one API call per repository) only once the branch was
  pushed or its pushed head moved since the last report, or after
  `gh pr create` ran. After a turn every pushed branch's is looked up, as
  before. A pull request found stays until a lookup finds another.
- **The agent keeps the index lock.** `git diff` against the working tree
  rewrites the index it reads when files' stat data is stale, under the
  index lock, even with `GIT_OPTIONAL_LOCKS=0`; an agent's `git commit` at
  that moment would fail. A refresh reads a copy of each clone's index,
  and runs `git status` without optional locks.
- **The transcript shows a line when a branch changes:** a commit of its
  own, a push, or a pull request the session hadn't had, in the
  repositories that changed. Reports that only move files show in the
  sidebar's Changes section alone, which takes the newest.

## Consequences

- Commits, pushes and pull requests show up on the session page and in
  the lists during the turn, seconds after they happen and at most about
  ten.
- More `git` events: at most one per ten seconds while files change. Each
  is stored and broadcast and can list 500 files, so a long turn of edits
  adds to the event log; reports that change nothing add nothing.
- A turn without commits, pushes or pull requests no longer has a line in
  the transcript, in old sessions too: the web app and `fctl attach`
  decide which states to show, so agents of older versions need nothing,
  and the protocol is unchanged.
- The refresh is fctl-agent's: sessions show changes during turns once
  their hosts run agents of this release. Older agents keep reporting
  after each turn.
- A pull request's title and state can be up to a turn old until a push
  or `gh pr create` calls for a lookup.
