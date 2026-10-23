# 0051: New projects start with follow-ups on

2026-10-23 · accepted

## Context

Follow-ups bring an agent back to a pull request it opened, for comments
that address it, failed checks, conflicts with the base branch, and a
merge or close
([0043](0043-agents-come-back-to-their-pull-requests-for-comments-failed-checks-and-merges.md),
[0045](0045-merge-conflicts-bring-agents-back-too.md)). Every switch
started off, and each new kind started off in projects that used the
others, `--follow-ups all` included.

People expected agents to see their pull requests through without first
finding these switches. In session #209 a pull request began to conflict
with `main` when another merged, and its agent wasn't told: the person
found the conflict on GitHub, asked the agent to merge `main` in, and only
then found the project's **Merge conflicts** switch off. The session page
didn't say either: a session keeps its pull requests' state and checks,
but not whether they conflict, so a conflict shows only when a follow-up
passes it on ([#247](https://github.com/hyperstructure/fctl/issues/247)).

## Decision

- **New projects start with all four on.** `settings::for_new_project`
  turns on each of `FollowUps`' switches that the create request leaves
  out, as it turns claude.ai connectors off; what a request sends stays.
  The project then stores each switch. `FollowUps::ALL` names them all on,
  and `fctl projects create --connectors`, which sends whole settings,
  sends it (a CLI from before still sends them off).
- **Stored projects stay as they are.** A stored `false` doesn't say
  whether someone turned a switch off or never touched it, and turning
  them on would start turns on people's subscriptions in projects whose
  owners chose off. `FollowUps::default()` stays all off, so a project
  stored without `follow_ups`, or without one of its fields, still reads
  them as off, in serde as in the SQL of
  `crate::pull_requests::moved_onto` (`json_extract(…) = 1`, where a
  missing key is `NULL`). Projects turn them on in their Settings, or with
  `fctl projects set --follow-ups all`.

This supersedes the "all off" of 0043 and 0045 for new projects.

## Consequences

- An agent of a new project comes back, on the Claude subscription of the
  person whose session opened the pull request, unless someone turns that
  off. What bounds it: at most three failed checks, and three conflicts,
  in a row while nobody messages the agent or addresses it on GitHub;
  once for a merge or close; comments only from people who can write to
  the repository, and only those that address the agent.
- Instances that don't take GitHub's webhook pass nothing on either way.
- Projects made before keep what they store: their maintainers turn
  follow-ups on themselves.
