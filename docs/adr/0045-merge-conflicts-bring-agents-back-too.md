# 0045: Merge conflicts bring agents back too

2026-10-23 · accepted · new projects start with them on since [0051](0051-new-projects-start-with-follow-ups-on.md)

## Context

Follow-ups bring an agent back to a pull request it opened for comments,
failed checks and merges
([0043](0043-agents-come-back-to-their-pull-requests-for-comments-failed-checks-and-merges.md)),
but not when the pull request conflicts with its base branch, which happens
whenever another pull request that touched the same lines merges first. A
pull request with conflicts can't merge, and GitHub runs none of its
`pull_request` workflows, so its checks wait for good. Agents didn't know:
one that waited for its checks saw none come, and a person had to go to
the session and tell the agent to merge the base branch in. People asked
for agents to pick conflicts up and resolve them on their own.

0043 left conflicts out because no delivery says when a pull request starts
to conflict. GitHub works out whether a pull request merges (`mergeable`)
only when someone asks after either branch moved, in the background, and
says `UNKNOWN` until it has: of 30 of a busy repository's open pull
requests, it knew about none at first, and about all 30 twenty seconds
later.

## Decision

- **A fourth switch, off by default.** `FollowUps::conflicts`, in the
  project's Settings and `fctl projects set --follow-ups conflicts`. An
  update that leaves it out keeps what was stored, as for the others.
- **fctl asks.** The lookups that keep sessions' pull requests current ask
  for `mergeable`: after any delivery about the pull request, and when a
  session finds it, since one opened with conflicts gets no check
  deliveries. A delivery that says a pull request merged into a repository
  moved its base branch: the control plane then looks up the other pull
  requests sessions list there, neither merged nor closed, whose projects
  follow up on conflicts, without their checks, which take GitHub the
  longest (one point of the rate limit for up to 50).
- **While GitHub doesn't know, the session asks again.** The session that
  opened the pull request keeps when to ask, from its own alarm: 15
  seconds later, then 30, 60, 120 and 240, and after that it leaves it to
  the next delivery. The question of the first lookup is what starts
  GitHub's work, so the second ask usually has the answer.
- **Once for each commit.** Conflicts reach the session that opened the
  pull request, as other follow-ups do, once for its head commit: when the
  agent merges the base branch in and pushes, conflicts after that come
  again only with a later commit. They wait while a turn runs and go only
  while GitHub says the pull request's head is still that commit.
- **Merge, don't rebase.** The message tells the agent to merge the
  updated base branch in, resolve each conflict so that both sides'
  changes still do what they were for, run the tests and push, without
  rebasing or force-pushing, which rewrites commits people may have
  reviewed; and to ask on the pull request when a conflict takes a
  decision the code and the pull request don't answer.
- **Three in a row at most**, counted apart from failed checks: in a busy
  repository, a pull request nobody merges would otherwise take in its
  base branch after every merge into it, each time on someone's
  subscription. A person's message, or their comment that addresses the
  agent, starts a new count.

## Consequences

- A merge into a repository costs a GitHub lookup, and a few while GitHub
  works out its answer, for each account whose pull requests may conflict
  with it, where projects follow up on conflicts.
- A push straight to a base branch, such as a release's commit, sends no
  delivery that names the pull requests it affects: their conflicts show at
  the next merge into the repository, or the next delivery about them.
- The hourly sweep still wakes nobody, so turning conflicts on wakes no
  session for a pull request that conflicted long ago until something
  happens to it, or another pull request merges into its repository. Then
  its agent takes the base branch in, and does again after later merges
  while the pull request stays open, three times in a row at most.
- Hosts and agents need nothing new; clients from before show conflicts as
  any message from fctl.
