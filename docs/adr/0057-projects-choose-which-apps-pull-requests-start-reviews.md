# 0057: Projects choose which apps' pull requests start reviews by themselves

2026-10-24 · accepted

## Context

Since [0047](0047-review-agents-review-outsiders-pull-requests-when-asked.md)
a `pull_request` delivery starts a review by itself only when the pull
request's author may write to the repository or is fctl's own app, and
since [0054](0054-review-agents-review-dependabots-pull-requests-by-themselves.md)
when Dependabot opened it, in every project. Teams use other apps that
open pull requests: Renovate, which does what Dependabot does; workflows
that open them as GitHub Actions (generated code, releases);
pre-commit.ci; Copilot's coding agent; and apps of their own. Their pull
requests waited for someone to ask, and Dependabot's started reviews
wherever review agents were on, wanted or not.

What 0054 found of Dependabot holds for apps in general:

- Nobody outside the project opens a pull request as an app. An app opens
  them where an account's admins installed it, people can't take a login
  that has `[bot]` in it, and a pull request from a branch of the
  repository came from something that may push there.
- What their pull requests say can come from outside all the same: the
  release notes dependency updaters quote, text a workflow took from an
  issue, what an agent wrote from its task. It reaches the review agent,
  which has the operator's Claude token, before anyone in the project
  has looked.

How much of that a project takes on differs by app and by project, and
the project's owners, who turn review agents on, run them on their
subscription and know their apps, are the ones to decide.

## Decision

- **Each project keeps the apps whose pull requests start reviews by
  themselves** (`project_reviews.bots`, `ReviewAgents.bots`), which its
  owners choose on the Reviews tab or with `fctl review settings --bots`.
  A delivery about a pull request that one of them opened from a branch
  of the repository starts a review in that project as one of the
  repository's people's does (`fctl_domain::reviews::starts_by_itself`,
  `by_bot`); other apps' wait, as outsiders' do, until someone asks for
  their review.
- **Dependabot stays on the list until the owners take it off**: projects
  from before keep 0054's behavior, and new ones start with it.
- **Only GitHub Apps go on the list**, as their bots are named,
  `<slug>[bot]`. A login without `[bot]` is someone's account, which the
  list doesn't take: people who may write to the repository, machine
  accounts among them, start reviews already. Pull requests name an
  app's bot by its page on GitHub, `https://github.com/apps/<slug>`, which
  deliveries carry: Copilot's coding agent, whose bot GitHub calls
  `Copilot`, is `copilot-swe-agent[bot]`, as GraphQL names it.
- **Their agent runs none of their code**: `trusted` is unchanged.
  Dependabot's and Renovate's agents check them as dependency updates, as
  0054 has Dependabot's; the agents of other apps' pull requests are told
  that an app opened them, and run nothing of them.
- **Owners see which apps they might add**: the apps that opened the
  newest pull requests (100 of each repository) from branches of the
  project's repositories, with how many and the newest
  (`GET /api/v1/projects/{id}/review-agents/bots`, `fctl review bots`), and
  any app GitHub shows by its name (`GET …/review-agents/bots/{name}`),
  through the installation the project's workspace connected. fctl asks
  GitHub when the owners look, and keeps none of it.

## Consequences

- Each pull request an app on the list opens, and each commit it pushes
  to one, costs the operator a review.
- Text in a listed app's pull requests reaches the agent before anyone in
  the project has looked at them, as 0054 says of Dependabot's. The owners
  decide that for each app, and the Reviews tab says so where they do.
- Owners can stop Dependabot's pull requests from starting reviews, which
  0054 didn't let them.
- GitHub shows no other account's private apps, so the lookup finds none
  of them; owners add one by its name all the same.
- An app renamed on GitHub gets a new slug, and its pull requests stop
  starting reviews until the owners add it again. An app that takes the
  old slug opens no pull request from the repository's branches unless
  its admins install it.
