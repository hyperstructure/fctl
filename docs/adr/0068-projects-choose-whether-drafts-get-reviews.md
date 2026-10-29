# 0068: Projects choose whether drafts get reviews

2026-10-29 · accepted

## Context

Since [0040](0040-review-agents-review-pull-requests-on-an-owners-subscription.md)
each new commit of a pull request in the repositories a project's review
agents review starts a review, drafts too. A draft says its author isn't
done: they push work in progress, often many times, and may not want
findings yet. Each of those commits cost the operator's subscription a
review whose findings the next commit made outdated. Owners could only
skip a whole repository
([0062](0062-projects-choose-which-of-their-repositories-review-agents-review.md))
or turn review agents off.

Session #528 was asked for a setting that decides whether drafts get
reviews, for the whole project rather than for each repository.

## Decision

- **One switch for the project** (`project_reviews.drafts`,
  `ReviewAgents.drafts`), which its owners change with the rest of its
  review agents' settings: **Drafts** on the Reviews tab, `fctl review
  settings --drafts on|off`. It holds for every repository they review,
  those linked later too.
- **On by default**, for new projects and those that had review agents
  before: drafts get reviews, as they did, until the owners turn it off.
- **Off, a draft waits until it's ready.** Deliveries about a draft start
  no review in the project (`crate::reviews::on_delivery`,
  `fctl_domain::reviews::waits_until_ready`). Marking it ready for review
  starts a review of its newest commit, unless someone asked for one of
  that commit already; later commits get reviews as other pull requests' do.
  Other projects that review the repository decide for themselves.
- **People may still ask.** Someone who starts sessions in the project
  asks for a review of a draft (the Reviews tab's **Review a pull
  request**, `fctl review start`, a session page's review again): they
  looked at it and want one now. Unlike a skipped repository, a draft is
  only early, not someone else's to review.

## Consequences

- With drafts off, nothing on GitHub says a review will come: a draft
  gets no check and no reaction until it's marked ready. A skipped check
  would count as passed where `fctl review` is required, and would be
  noise on each push.
- A review under way when a pull request turns into a draft finishes,
  even when a new commit follows: only a review that starts takes the
  place of one under way.
- An fctl agent whose project asks it to open drafts gets no findings on
  them until someone marks them ready, where the owners turn drafts off.
