# 0066: Review agents post as a GitHub App of their own

2026-10-29 · accepted

## Context

Review agents' findings and the answers of the agent that opened the pull
request came from the same bot, the instance's GitHub App (`fctl-sh[bot]`
on app.fctl.sh): a pull request's conversation showed one bot reviewing,
then the same bot replying "Fixed in ab5ef67", then the same bot resolving
the thread. People couldn't tell at a glance which agent said what.

Session #490 was asked to post reviews as a separate GitHub App,
`fctl-review` (its own lime logo), where a workspace has it installed on
the repository, and as the GitHub App otherwise, as before, so that
nothing breaks.

## Decision

- **A second, optional app.** `GITHUB_REVIEW_APP_ID`,
  `GITHUB_REVIEW_APP_SLUG` and `GITHUB_REVIEW_APP_PRIVATE_KEY`
  (`[github.review]` in an instance file) name the review app. It has no
  webhook, OAuth client or callback URL: the GitHub App's webhook tells the
  control plane everything, and nothing signs in with it. Its setup URL
  (`/api/v1/github-review/installed`) only brings people back to the
  workspace they installed it from.
- **Installing it is all it takes.** fctl uses the review app only on
  repositories a workspace connected through the GitHub App, whose
  connection already took a GitHub sign-in to show what the workspace may
  use (ADR 0029), and only where its installation is on the same account,
  by GitHub's id, and reaches the repository. So a workspace connects
  nothing for it, and nobody gets more than the GitHub App's connection
  gives them: the review app's tokens never leave the control plane and
  only post reviews.
- **A review keeps the app it started with.** As a review starts, the
  control plane picks the review app where it reaches the repository, else
  the GitHub App, writes the check and the reaction with it, and keeps
  which (`reviews.review_installation_id`): GitHub lets only the app that
  created a check change it. The summary, the review, the fixed marks, the
  resolved threads and the check's end all go through the same app. A
  review whose review app was uninstalled meanwhile posts as the GitHub
  App, and its check stays as the review app left it.
- **fctl's earlier findings are told by their author.** fctl's threads and
  summaries were the ones "the token's app wrote" (`viewerDidAuthor`), which
  would have orphaned every finding the GitHub App posted before the review
  app was installed. Now they are the comments with fctl's mark that one of
  the instance's apps wrote (a `Bot` by its slug), and each is edited by
  the app that wrote it, which GitHub is sure to let; the review app
  resolves threads, as anyone who may write the repository's contents can,
  else the GitHub App does. Lookups count the `fctl review` check of
  either app as the review.
- **The UI makes it one click.** The workspace's GitHub tab says, on each
  of its accounts, which app posts the reviews, and gives its admins a link
  that installs the review app on the account (the account and the
  workspace's repositories preselected on GitHub) or adds the repositories
  it doesn't reach; a project's Reviews tab says which app its reviews post
  as, with the same link. `fctl workspaces review-app` says it too.

## Alternatives

- **Connecting the review app like the GitHub App**, with a GitHub sign-in
  per account: twice the steps for every workspace, for nothing it would
  protect, since the review app reaches nothing the GitHub App's
  connection doesn't already give the workspace.
- **A setting per workspace or project to use it or not**: whoever
  installed it on the account chose it; a setting would only let the two
  disagree.
- **Re-creating checks with the review app as it's installed mid-review**:
  the old check would stay queued or in progress for good, since only the
  app that created it can end it.

## Consequences

- On accounts with the review app, a pull request's conversation shows
  `fctl-review` reviewing and `fctl-sh` answering, and the review agent's
  check runs as `fctl-review`.
- Reviews of a pull request that straddle the installation mix both apps;
  fctl still marks, resolves and sums up all of its findings.
- An instance without a review app works as before; self-hosters add one
  when they like ([Self-hosting](../self-hosting.md#a-review-app-optional)).
