# 0030: Sessions follow every pull request their agent opens

2026-10-20 · accepted

## Context

fctl-agent looked up only the pull request of the branch each repository
had checked out (`gh pr view <branch>`), and the control plane learns of
pull requests only from its reports. A session that opened a second pull
request from a branch in a worktree never listed it
([#155](https://github.com/hyperstructure/fctl/issues/155)).

Changes compared the working tree with the tip of `origin/<base>`, which
moves whenever the clone fetches. Once the session's pull request was
squash-merged, its page said "No changes yet". When someone else's commits
reached the base, they showed among the session's, reversed. And when the
session resumed, `git fetch --prune` removed the merged branch GitHub had
deleted, so fctl-agent pushed it again and brought the branch back.

## Decision

- **fctl-agent follows every pull request of its session:** those of the
  branches it looks up, those whose URL `gh pr create` printed (in the
  tool's result), and those the session lists when a sandbox starts (the
  spec's `pull_requests`), in repositories the session may use. It keeps
  them in its state file, reports them all in the git state
  (`pull_requests`), and looks each up again by its URL when a turn ends,
  until it is merged. This amends ADR 0027: a pull request found doesn't
  stand until the next lookup of its branch, which may never come.
- **Changes are what the branch changed since it left its base:** the diff
  from the merge base of the branch and `origin/<base>`, uncommitted and
  untracked work included. Once its pull request merged, the diff is from
  the base as it was just before the merge (`baseRefOid`), so the work it
  merged still shows, whichever way it merged, and Changes says it merged.
- **A branch whose pull request merged isn't pushed again.** Before it
  pushes a branch whose upstream a fetch pruned (`[gone]`), or whose pull
  request it saw open, fctl-agent looks the pull request up when a turn
  ends. Commits after a merged one stay in the sandbox: the transcript says
  so, and Changes counts them (`unmerged`) as not pushed. The agent can
  still push them itself, for a new pull request.
- **Agents make another pull request's branch where the repository is
  checked out** (`git switch -c`), not in a worktree or another clone, as
  the system prompt tells them. fctl lists that pull request wherever it
  came from, but Changes shows only the branches checked out in the
  session's checkout and in its clones in `~/repos`.

## Consequences

- Session pages and lists name every pull request the agent opened, with
  its current state, once hosts run agents of this release. Older agents
  report only the checked-out branches' pull requests, as before.
- One more `gh` call each turn for every followed pull request that isn't
  merged, and one before pushing a branch whose pull request was open.
- Work in a worktree or a clone outside `~/repos` still doesn't show under
  Changes, though its pull request does.
- A merged branch keeps its changes on the session page, and its counts in
  the session lists.
- With a `gh` from before `baseRefOid` and `headRefOid`, fctl-agent asks
  again for the fields every version has: pull requests are still found,
  without the commits after a merge.
