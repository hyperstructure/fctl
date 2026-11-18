# 0084: Webhooks post what happens in a workspace to addresses its people choose

2026-11-17 · accepted

## Context

People build on what their agents do: a message in Slack when an agent
asks something or has a pull request ready for review, a deploy when a
schedule's session finishes, a dashboard of sessions and reviews, an
alert when a review fails or a schedule stops starting sessions. fctl
told people through Web Push alone ([ADR 0010](0010-notifications-are-web-push-from-a-durable-object-per-person.md)),
and tools could only poll the API, or read `fctl logs --json` of each
session, to find out.

Most of what they'd want to hear of is known in one place already:
SessionDO sees every status change, question and follow-up of its
session; the reviews module sees each review queued, started, posted or
ended; the schedules module sees each run that starts nothing. Sending
from there would put other people's slow or broken servers in the path
of sessions, though, and a webhook carries a project's work to an
address outside fctl.

## Decision

- **A webhook is an address, the events it wants and a secret**
  (`webhooks` in D1). A workspace's admins add webhooks for all its
  projects, or some (the workspace's **Webhooks** tab, `POST
  /api/v1/workspaces/{id}/webhooks`); a project's owners and maintainers
  add its own (its Settings → **Webhooks**, `POST
  /api/v1/projects/{id}/webhooks`). Each chooses its events (all of them,
  or some), and for sessions' events whom they're of: sessions people
  and tools start, schedules', an autopilot's or those a ticket started
  (`started_by`). A workspace has at most 20.
- **Nineteen events, named for what happened** (`fctl_domain::hooks::Kind`,
  `GET /api/v1/webhook-events`): a session started, changed status,
  finished a turn with no background work running (once that lasted five
  seconds, as background work's own turn comes within them), has its pull requests
  ready for a person's review (open, not drafts, every check passed and
  no review agent at work, for two minutes), asked something (`fctl
  ask`), was brought back by a follow-up, paused, stopped or failed; an
  agent's pull request opened, merged or closed; a review agent's review
  queued, started, posted (with how many new findings it reported, and
  fixes of earlier ones), failed or was cancelled; a schedule's run that
  started no session, and a schedule fctl paused. Each payload carries
  the workspace and the project with their ids, the session as the API
  shows it, its schedule's, autopilot's or ticket's name and id where it
  started from one, and what the event is about. The kind is a string,
  so that a later version's events need no new shape.
- **Signed as Standard Webhooks sign them**: `webhook-id` (the event's
  id, the same in every attempt and resend), `webhook-timestamp` and
  `webhook-signature` (`v1,` and an HMAC-SHA256 of `id.timestamp.body`
  with the webhook's `whsec_` secret), which libraries in most languages
  check, plus `fctl-event`, `fctl-workspace-id` and `fctl-project-id`
  for routing. The secret is made by fctl, sealed with the master key,
  shown once as the webhook is added, and later to those who manage it;
  rotating it keeps the old one signing too for a day, so that a
  receiver switches without missing a delivery. Adding a webhook,
  pointing one elsewhere, and reading or rotating its secret are sensitive actions
  ([ADR 0039](0039-sensitive-actions-take-a-passkey-or-a-code.md)).
- **An address answers a challenge before a webhook posts there**, as
  Slack's Events API checks an app's: adding a webhook, or pointing one
  elsewhere, first posts the address a signed `url_verification` request
  whose answer gives its `challenge` back, so that a webhook's events go
  only where someone set up a receiver for them, and fctl can't be
  pointed at another's server. A new webhook's secret reaches its
  receiver only once the webhook exists, so receivers answer the
  challenge before checking signatures; it asks for nothing else. Tools
  that only take requests, without answering them as asked, need a
  receiver of one's own in front.
- **A Durable Object per workspace sends them** (`HooksObject`), from a
  queue in its storage, so that nothing a receiver does slows a session
  down: sources hand it an event after their answer (`wait_until`), only
  when one of the workspace's webhooks wants it, which they look up in D1
  at most every 30 seconds per isolate. Its alarm posts each delivery
  with a 10-second timeout and no redirects, at most 120 a minute to each
  webhook and 1,000 waiting, and tries a failed one again 7 times over
  about 21 hours (10 seconds, then a minute, 5, 30, 2 hours, 6 and 12),
  sooner where the receiver's `Retry-After` asks for later. A webhook
  whose deliveries all failed for 3 days turns itself off as one fails
  for good, and says so.
- **Its people see what went out.** Those who manage a webhook see its
  deliveries of the last 30 days (the newest 1,000), each with the
  request as it was sent (headers and payload), every attempt and the
  start of each answer; they resend one (five times each, at most 30
  manual sends an hour per webhook), resend all that failed since a
  time, and send a test of any event, a sample marked `test`, whose
  answer shows at once.
- **Only public addresses**: `https://`, no credentials in the address,
  and no host that is an IP address of a private network, loopback or
  link-local, nor `localhost` or `.local`/`.internal` names, so that a
  webhook can't reach the control plane's own network. A self-hosted
  instance, whose receivers may live on its own network, lets webhooks
  reach the IP ranges and host names its operator lists
  (`FCTL_WEBHOOK_PRIVATE_HOSTS`) and post over `http://`
  (`FCTL_WEBHOOK_HTTP`): settings of the deployment, not of the app, so
  that its admins can't widen them from a browser. app.fctl.sh sets
  neither. Each address is checked again before each post. Development
  instances (`FCTL_DEV`) take `http://` and local addresses.

## Consequences

- Slack, a deploy script or a service of one's own hears of sessions,
  pull requests, reviews and schedules as they happen, with the context
  to act, and checks that a request came from fctl with any Standard
  Webhooks library and the ids it expects.
- Payloads leave fctl: session titles, the start of an agent's reply,
  questions and comments go to the addresses a workspace's admins and a
  project's maintainers choose, and the delivery log keeps them in the
  workspace's Durable Object for 30 days. The privacy policy says so.
- Each event costs a D1 read per workspace and isolate at most every 30
  seconds, and a Durable Object request when a webhook wants it;
  delivery logs grow the Durable Object's storage, not D1.
- A self-hosted control plane can't check where a host name resolves to:
  one that resolves to a private address of its network is reached. Such
  instances keep webhooks' hosts to ones they trust. One that lists
  private hosts lets whoever manages a webhook reach them, so it lists
  receivers, not whole networks.
- A change to a webhook takes effect for the events of sessions within
  half a minute; tests and resends at once.
