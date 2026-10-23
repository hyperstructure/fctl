# 0042: Lookups GitHub gives up on ask about fewer pull requests at once

2026-10-23 · accepted

## Context

[ADR 0033](0033-the-control-plane-looks-pull-requests-up-50-at-a-time.md)
has the hourly job look pull requests up in queries of 50 of an account,
and leave the rest of an account's to its next run when GitHub doesn't
answer. GitHub gives up on a query that takes more than ten seconds and
answers 502 or 504. The more checks the pull requests' head commits have,
the longer it takes. This repository's pull requests have about 10 checks
each, and GitHub answers a query of 50 of them in two to three seconds.
vercel/next.js's have 100 to 240 each
([#219](https://github.com/hyperstructure/fctl/issues/219)). Measured from
an fctl session on 2026-10-23, GitHub answered a query of one of them in
1.4 seconds, 5 in 2.3, 10 in 4.2 to 7.2 and 25 in 10.7, and gave up on 50
after 10.8 seconds with nginx's 502 page. For such an account every run
failed at its first query and left everything to the next run, which
failed the same way. Its pull requests changed only when a webhook
delivery about them came, and a missed delivery was never made up for.

## Decision

- **Fewer at once after GitHub gives up.** When GitHub answers 502 or 504
  to a lookup, the job asks again about the same pull requests 10 at a
  time, as a webhook delivery's lookups do. If GitHub gives up on 10, it
  asks one at a time (`fctl_domain::webhooks::fewer`). It keeps to the
  smaller number for the rest of the account's pull requests in that run,
  since they are likely to have as many checks. Other failures stop the
  account as before: asking about fewer wouldn't help them.
- **One alone is the end.** When GitHub gives up on one pull request
  alone, the job leaves the rest of the account's to the next run, as it
  did after any failure. That pull request waits its six hours, as every
  one it asks about does, so it can't hold up the others.
- **Five minutes a run.** The job stops looking pull requests up after
  five minutes, and the next run carries on with what is left. Each query
  can take up to ten seconds, and at one pull request a query, an
  account's 500 could take longer than the 15 minutes the Worker's cron
  trigger may run.
- Webhook deliveries don't ask again: GitHub waits only ten seconds for
  the control plane to answer a delivery.

This amends ADR 0033: a query GitHub gives up on no longer ends an
account's run.

## Consequences

- Accounts whose pull requests have many checks get their lookups. For 50
  of next.js's, that is one query GitHub gives up on and five queries of
  10, each costing a point of the installation's rate limit, about 40
  seconds in all.
- A query GitHub gives up on takes ten seconds of the run, and GitHub
  takes extra points of the installation's rate limit for it. An account
  has at most three a run, one of each size. They come mostly from large
  backlogs: in steady state the job asks only about pull requests nobody
  asked about for six hours, a few at a time.
- When GitHub can look an account's pull requests up only one at a time,
  each costs a point, and a run gets through as many as GitHub answers in
  five minutes: about 200 at the 1.4 seconds one of next.js's took.
