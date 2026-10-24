# 0047: Review agents review outsiders' pull requests when someone asks

2026-10-23 · accepted · Dependabot's pull requests start reviews by themselves since [0054](0054-review-agents-review-dependabots-pull-requests-by-themselves.md), and those of the apps each project chooses since [0057](0057-projects-choose-which-apps-pull-requests-start-reviews.md)

## Context

[0040](0040-review-agents-review-pull-requests-on-an-owners-subscription.md)
has every new commit of every pull request in a project's writable
repositories start a review, drafts and forks too, on the Claude
subscription of the review agents' operator. Of pull requests from people
who can't write to the repository it said: "their agent runs none of their
code and has nothing worth taking, but their text reaches the operator's
subscription".

The agent has something worth taking: the operator's Claude access token
is in its sandbox, as every session's is, since Claude Code needs it.
Claude runs there without asking before it acts, and only the prompt
tells the agent not to run the pull request's code or follow what it
says. Text in a pull request can steer an agent; that is what the prompt
guards against, and a prompt is not a boundary. Anyone who can open a pull
request on a public repository could have the review agent read the token
from its environment and send it out, or put it in a finding that fctl
then posts on the pull request, without anyone in the project having
looked at the pull request first. A full Claude login's token lasts
hours; one from `claude setup-token` lasts a year.

GitHub Actions faces the same problem with workflows on outsiders' pull
requests, and asks someone who may write to approve them first.

## Decision

- **Only trusted authors' pull requests start reviews by themselves.** A
  `pull_request` delivery starts a review when the pull request's author
  may write to the repository, as GitHub says of its owners, the members
  of its organization and its collaborators, or it is fctl's own app,
  whose pull requests come from the workspace's sessions
  (`fctl_domain::reviews::trusted`, which already decided whether the agent
  may run the pull request's code).
- **Anyone else's waits for someone to ask.** People who start sessions in
  the project ask for a review of any pull request, outsiders' included
  (the Reviews tab's **Review a pull request**, `fctl review start`), once
  they have looked at what it says. Each new commit waits for them again.

## Consequences

- Projects that review contributions from outside lose the automatic
  review of those; their people ask for it, as GitHub has them approve
  outsiders' workflow runs.
- What a review that someone asked for reads can still steer its agent:
  asking takes looking at the pull request first.
- Nothing changes for pull requests of the repository's own people and of
  fctl's agents, which are most of what review agents review.
