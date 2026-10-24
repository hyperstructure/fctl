# 0054: Review agents review Dependabot's pull requests by themselves, and run none of them

2026-10-24 · accepted · projects choose which apps' pull requests start reviews, Dependabot until they choose otherwise, since [0057](0057-projects-choose-which-apps-pull-requests-start-reviews.md)

## Context

Since [0047](0047-review-agents-review-outsiders-pull-requests-when-asked.md)
a `pull_request` delivery starts a review by itself only when the pull
request's author may write to the repository, or is fctl's own app:
anyone else's waits until someone in the project asks for it, since its
text reaches an agent that has the operator's Claude token. GitHub says
`NONE` of Dependabot, which is no collaborator, so its pull requests waited
too: fctl's first, a security update of happy-dom (#252), got no review.
People asked for Dependabot's pull requests to be reviewed by themselves.

Nobody outside the project opens one. Dependabot opens them where the
repository's admins turned it on (security updates in its settings,
version updates in `.github/dependabot.yml`), from branches of the
repository itself, for dependencies it already has, and GitHub writes
their titles and descriptions. Two things in them come from outside the
project all the same:

- Their descriptions quote the dependencies' release notes, changelogs and
  commits, which their maintainers and contributors wrote. Text there
  reaches the agent as an outsider's would, though only someone who gets
  it into a release of a dependency, one Dependabot proposes, puts it
  there.
- Their code is the dependencies' new versions. Installing them runs
  their install scripts, with the session's access.

## Decision

- **Dependabot's pull requests start reviews by themselves.** A delivery
  about a pull request whose author is `dependabot[bot]`, from a branch of
  the repository, starts its review as one of the repository's people's
  does (`fctl_domain::reviews::starts_by_itself`, `by_dependabot`).
- **Their agent runs none of them.** `trusted` stays as it was, so the
  agent is told what it is told of an outsider's pull request someone
  asked for: no installs, builds, tests or scripts. It is told what
  matters in a dependency update too: lockfiles changed to match their
  manifests, new versions from where the old ones came, and what they
  bring that the code would notice (breaking changes, a new major version,
  install scripts); and that the description quotes the dependencies'
  authors.
- **Other apps' pull requests wait**, as do those of people who can't
  write to the repository, until someone asks for their review.

## Consequences

- Each pull request Dependabot opens, and each commit it pushes to one,
  costs the operator a review: a few a month where it opens security
  updates alone, many more with version updates.
- Text a dependency's authors wrote reaches the agent before anyone in
  the project has looked at the pull request. The prompt says it is no
  instruction to the agent, and, as 0047 says, a prompt is not a boundary.
  Whoever publishes a dependency already runs code wherever it is
  installed; to reach the agent, their text has to be in a version
  Dependabot proposes.
- Only Dependabot's own login counts: people can't take a name that ends
  in `[bot]`, and an account named `dependabot` is someone's.
