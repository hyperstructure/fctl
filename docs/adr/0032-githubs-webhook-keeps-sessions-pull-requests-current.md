# 0032: GitHub's webhook keeps sessions' pull requests current

2026-10-21 · accepted

## Context

Sessions learned what became of their pull requests only from their
agents, which look each one up with `gh pr view` when a turn ends, until it
merges ([ADR 0030](0030-sessions-follow-every-pull-request-their-agent-opens.md)).
A pull request merged or closed after the session's last turn, which is
how most of them end, stayed open on the session page and in the lists for
good, and nothing said whether its checks passed. The GitHub App had its
webhook off.

## Decision

- **GitHub tells the control plane** when pull requests and the checks of
  commits change: the app's webhook, at `POST /api/v1/github/webhook`,
  with the events Pull request, Check run, Check suite and Status. A
  delivery counts only when it is signed with the webhook's secret
  (`GITHUB_WEBHOOK_SECRET`); without the secret the instance takes none,
  and sessions show what their agents last looked up, as before.
- **A delivery is a cue; a lookup is the answer.** For each pull request a
  session lists that a delivery concerns (by number, or by its head commit
  for commit statuses), the control plane asks GitHub with one GraphQL
  query: its title, state, merge queue entry, times, and the checks of its
  head commit counted by state. Deliveries come out of order and some never
  come, and counting checks from them would miss those that ran before the
  session listed the pull request; a lookup says how things are now. When
  the lookup fails, a `pull_request` delivery's own payload gives the state.
- **D1 keeps which sessions list which pull request**
  (`pull_request_sessions`), with its head commit and state. SessionDO still
  owns each session's list: the control plane sends it what GitHub says,
  and it keeps what is newer (`fctl_domain::pulls::update`), and passes it
  on to its viewers, the dashboards and D1 as it does what agents report.
- **The newer lookup wins.** GitHub's `updatedAt` orders what agents and
  GitHub say of a pull request's title and state, so an agent's stale state
  file doesn't reopen a merged one; checks go by when the control plane
  asked. `gh` can't see merge queues: an agent's `open` leaves a queued one
  queued. A pull request is queued when the delivery's action is `enqueued`
  (or `queued`), GitHub lists its merge queue entry, or its merge state
  status (else `mergeable_state`) is `queued` or `in_merge_queue`.
- **Each pull request says when its state changed** (`state_changed_at`):
  when GitHub says it merged, closed or joined the queue, or else when fctl
  saw it change. Agents report GitHub's times too.
- **Lookups without a delivery:** the control plane looks up the pull
  requests a session finds as it finds them, which gives them their checks
  at once, and the hourly job up to 30 that aren't merged or closed and that
  nobody asked about for six hours, which catches what missing deliveries
  missed and brings sessions from before the webhook up to date.
- **The web app shows them as GitHub does:** GitHub's octicon for each state
  and its colors, the same in both themes, and a mark for the checks while
  the pull request is open (`@hyperstructure/design`'s `pr-*` and
  `checks-*` tokens).

## Consequences

- Session lists and pages show each pull request's state and checks as
  they change, also while their sessions are stopped, and say when one
  merged or closed.
- One GraphQL query for each delivery that concerns a listed pull request,
  a few dozen for a push to a pull request whose CI runs many jobs, on the
  installation's rate limit (5,000 an hour); the others cost one D1 query.
  The control plane keeps each installation's token for its lookups while
  it is valid.
- Instances turn the webhook on in their GitHub App and set its secret;
  until then nothing changes. The app needs no new permissions: it reads
  pull requests, checks and commit statuses already.
- app.fctl.sh receives GitHub's events about every pull request and check
  in the repositories its app is installed on, and keeps only what concerns
  the pull requests sessions list.
- The same deliveries can wake agents when their CI fails or a review comes
  in ([#8](https://github.com/hyperstructure/fctl/issues/8)).
