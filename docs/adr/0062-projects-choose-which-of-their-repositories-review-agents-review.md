# 0062: Projects choose which of their repositories review agents review

2026-10-28 · accepted

## Context

Since [0040](0040-review-agents-review-pull-requests-on-an-owners-subscription.md)
a project's review agents review the pull requests of every repository
its agents may write to: its primary repository and those it lists as
read and write. Projects share repositories: a library that two products
use, a monorepo that several teams work in, a repository of shared
infrastructure. Where two such projects had review agents on, each
started a review of every new commit of the shared repository's pull
requests: two sessions on their operators' subscriptions, two summaries
and two sets of findings on the pull request, and two messages to the
agent that opened it. The owners of one project could only leave the
repository to the other by turning their review agents off for all of
their repositories.

Session #410 was asked to let a project choose which of its repositories
its review agents review, with all of them reviewed by default when the
review agents are turned on, and to let owners turn repositories off.

## Decision

- **Each project keeps the repositories its review agents skip**
  (`project_reviews.skipped_repos`, `ReviewAgents.skipped_repos`), which
  its owners, who choose everything else of its review agents, change on
  the Reviews tab, where each repository its agents may write to has a
  switch, or with `fctl review settings --skip-repos`. A list of those
  skipped, not of those reviewed: every repository is reviewed until the
  owners skip it, those linked later too.
- **A skipped repository's pull requests get no review in the project.**
  Deliveries start none there (`crate::reviews::on_delivery`), and nobody
  asks for one there either (the Reviews tab's **Review a pull request**,
  `fctl review start`): the owners chose not to spend the operator's
  subscription on it. Other projects that review it go on as before, and
  reviews under way finish.
- **The choice lasts while the project's agents may write to the
  repository.** One removed from the project, made read only or no longer
  its primary repository leaves the list (`crate::reviews::unskip`), so
  that linked again it is reviewed, as a new one is. The list also stays
  while the review agents are off: turned on again, they skip what they
  skipped.
- **Owners choose; fctl decides nothing by itself.** It doesn't pick one
  of the projects that review a repository, nor say which other projects
  do: they may be in workspaces the owners can't see.

## Consequences

- A repository that several projects share gets one review per commit
  where all of them but one skip it. Its findings still reach the agent
  that opened the pull request, in whichever project
  ([0041](0041-review-agents-findings-go-to-the-agent-that-opened-the-pull-request.md)),
  since findings follow the pull request, not the project.
- A project whose owners skip every repository gets no review; its
  Reviews tab and `fctl review settings` say so (`problem`), as they do of
  a project with no repository to review.
- Maintainers link and unlink repositories, but don't choose review
  agents' settings: unlinking a skipped repository and linking it again
  has it reviewed, as linking a new one does.
- Owners of projects that share a repository have to agree among
  themselves which one reviews it; skipping it in all of them leaves its
  pull requests unreviewed.
