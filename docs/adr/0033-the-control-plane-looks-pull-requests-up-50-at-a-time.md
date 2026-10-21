# 0033: The control plane looks pull requests up 50 at a time

2026-10-21 · accepted

## Context

[ADR 0032](0032-githubs-webhook-keeps-sessions-pull-requests-current.md)
has the control plane look pull requests up on GitHub one GraphQL query
each, and an hourly job look up 30 that aren't merged or closed and that
nobody asked about for six hours, which was also to bring the sessions
from before the webhook up to date. app.fctl.sh had about 200 pull
requests from those sessions, most of them merged long ago and shown as
open: at 30 an hour, the last would turn right seven hours after the
deploy, and a larger instance would wait days. Each lookup cost a point of
the rate limit of the app's installation (5,000 an hour), which the agents
of its sessions share: `gh` asks GitHub's GraphQL API too, and
`gh pr checks --watch` every ten seconds.

## Decision

- **One query asks about up to 50 pull requests** of an account
  (`fctl_domain::webhooks::query`), each under its own alias, and how many
  points the installation has left. GitHub counts a query by the pages of
  connections it could fetch, a hundred to a point, so it costs one point,
  as a query about one does; it answers one about 50 in about two seconds
  (and gives up after ten). A pull request or repository it doesn't have
  comes back empty and leaves the others be.
- **The hourly job looks up 500 a run**, those asked about longest ago
  first, in queries of 50 of an account. It stops for an account when
  GitHub doesn't answer or the installation has fewer than 1,000 points
  left, and the next run carries on. Those it asked about wait six hours,
  answered or not, so that one that always fails can't hold up the others.
- **Each session hears once** of all its pull requests a query found
  (SessionDO's `/pull-requests` takes a list). `pull_request_sessions`
  keeps a pull request's old state until every session that lists it
  heard, so that the job asks again for one that didn't.
- A webhook delivery asks about its pull requests (up to 10, those a commit
  heads) in one query too.

## Consequences

- An instance brings all its sessions' pull requests up to date in the
  first hourly run after it takes the webhook, or after it upgrades to
  this: app.fctl.sh's 200 or so for about four points. Then the job costs
  about a point an hour for each account with open pull requests.
- A run of 500 makes 10 requests to GitHub, about 20 to D1 and one to
  each session it tells, within a Worker's limits.
- When agents have nearly used up an installation's rate limit, the job
  waits for them, and pull requests that GitHub's webhook missed stay as
  they were for another hour.
