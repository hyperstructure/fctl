# 0067: Review agents sum pull requests up in their descriptions

2026-10-29 · accepted

## Context

A review takes minutes, and what it found comes at its end. People who
open a pull request meanwhile, or who choose which pull request to look at
first, had nothing from fctl to go by.

Session #507 was asked for a setting that has the review agent add a short
summary at the end of the pull request's description before it reviews in
depth: how risky what the pull request attempts looks (in a GitHub note, a
warning when the risk is high, a caution for a critical security problem
seen at first glance), what it brings, and what is worth pointing out,
updated by later reviews, without the agent losing anything people wrote
in the description. It was also asked to give the summary a look of its
own rather than copy another review agent's.

## Decision

- **A setting of the project's review agents, off until its owners turn
  it on** (`project_reviews.description_summary`, the Reviews tab, `fctl
  review settings --description-summary`): it writes in descriptions that
  other people own.
- **The agent sums up, and fctl writes.** The review agent's token still
  only reads ([0040](0040-review-agents-review-pull-requests-on-an-owners-subscription.md)).
  It hands its summary to the control plane (`fctl review summarize`),
  which checks it and writes the description with the app the review
  writes with ([0066](0066-review-agents-post-as-a-github-app-of-their-own.md)).
  The agent never sends a description, only its summary, so it can't drop
  or rewrite what people wrote, whatever a pull request steers it to.
- **First, from what the pull request sets out to do.** The agent writes
  the summary before it reviews in depth, from the description and the
  diff: the risk of what it changes and where (low, medium, high
  sparingly, critical only for a serious security problem seen at first
  sight), not of bugs the review may find later, which go in the review.
- **Fields from the agent, the layout from fctl.** The agent gives a
  risk, why, what the pull request does in a paragraph or two, and
  optionally what deserves a closer look, in plain sentences. fctl lays
  them out under a rule as a GitHub alert, each part led in bold as fctl's
  findings are (**Medium risk.**, **What it does.**), with a line of what
  fctl knows for sure: the commit it sums up, the pull request's size and
  links to the review and its settings. Lists, headings, code blocks and
  HTML are refused: they would break out of the alert, or could pass for
  fctl's marks.
- **Only between fctl's marks.** `<!-- FCTL_REVIEW_SUMMARY -->` and
  `<!-- /FCTL_REVIEW_SUMMARY -->`, each on a line of its own, bound
  fctl's part. fctl reads the description just before it writes it, and
  replaces its part where it is, or adds it at the end after a blank
  line. Marks in fenced code blocks, within a line or without their pair
  are text, which stays; a code block the description leaves open is
  closed first, or it would take the summary in.
- **Each review rewrites it, and puts it back as it posts.** The summary
  describes the pull request at the commit reviewed, and the next review
  replaces it. `reviews.pull_request_summary` keeps it, and as the review
  posts, fctl puts it back where something took it out or changed it
  since: fctl's own agents rewrite their descriptions as they push
  ([0049](0049-agents-write-balanced-pull-request-descriptions.md)), from
  what they read before, and they're told to keep the part between the
  marks.

## Alternatives

- **The agent edits the description itself** (`gh pr edit`): it would
  need a token that writes pull requests, which what a pull request says
  could then steer to change anything on it, and an agent that rewrites a
  whole description can drop what people wrote.
- **A comment instead**: fctl's summary comment comes as the review ends
  ([0040](0040-review-agents-review-pull-requests-on-an-owners-subscription.md));
  the description is what people read first, and it stays in one place
  across commits.
- **Markdown as the agent writes it**: simpler, but nothing would keep it
  in its alert, out of fctl's marks or short.

## Consequences

- Where it's on, a pull request's description changes as each review
  starts, and GitHub shows that the review's app edited it; its history
  keeps every version.
- GitHub takes no condition on an edit of a description, so a person who
  changes the description in the instant between fctl's read and its
  write loses that change, which the history keeps.
- A summary removed by hand comes back as the review posts, and with the
  next commit's review: owners who don't want it turn the setting off.
- A description at GitHub's limit (65,536 characters) gets no summary,
  and the review carries on; so do reviews whose host is older than
  `fctl review summarize`.
