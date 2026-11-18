# 0088: Webhooks post only while their keeper manages them, and only where they may

2026-11-18 · accepted

## Context

A webhook carries a workspace's or a project's events to an address
someone chose ([ADR 0084](0084-webhooks-post-what-happens-in-a-workspace-to-addresses-its-people-choose.md)):
session titles, the start of agents' replies, questions, pull requests
and reviews. A security pass of the features added in November 2026
found two ways for that to reach further than meant:

- A webhook outlived its maker's access. A workspace's admin, or a
  project's maintainer, who added one at an address of their own kept
  getting every event there after they left the workspace, were demoted
  or disabled, until someone noticed it among the webhooks. Service
  accounts already stop with the admin who made their secret
  ([ADR 0086](0086-service-accounts-start-sessions-for-the-people-who-allow-them.md)),
  and schedules pause when their person can no longer run them.
- On the standalone server, fctl checked a webhook's address as written:
  a public-looking host name that resolves into the server's own network
  passed, and the server posted there, its challenge and tests showing
  the answer. Miniflare's default network reaches private addresses, and
  any member of an instance can make a workspace and add webhooks to it,
  so ADR 0084's advice to keep who adds webhooks to trusted people
  couldn't be followed.

## Decision

- **A webhook has a keeper** (`webhooks.keeper_id`): whoever chose where
  it posts, by adding it, pointing it elsewhere or turning it on again.
  It posts only while they manage it: an active account that is an admin
  of its workspace, or, for a project's own, an owner or maintainer of
  the project (`crate::hooks::KEPT`). The workspace's HooksObject reads
  that with the webhooks, and turns off one whose keeper doesn't, as an
  event or a delivery for it comes, with its waiting deliveries, saying
  why on its page and in the audit log. Whoever turns it on again keeps
  it. Webhooks from before keepers are their makers'.
- **The standalone server posts webhooks through a network of their
  own** (the Worker's `FCTL_WEBHOOK_NETWORK` binding, a workerd
  `network` service): public addresses and the IP ranges
  `FCTL_WEBHOOK_PRIVATE_HOSTS` lists, checked against every address a
  host name resolves to as it connects. The host names it lists go
  through the Worker's own fetch, to where they resolve, as their
  operator trusts them. On Cloudflare, without the binding, the Worker's
  own fetch reaches no private address.

## Consequences

- An admin who leaves takes nothing of the workspace's events with them;
  webhooks they set up stop, as their service accounts do, until another
  admin checks the address and turns them on, which breaks receivers
  until then.
- A keeper who has to set up two-factor sign-in keeps their webhooks:
  they haven't lost their role, and the webhook posts what it did.
- A self-hosted instance's webhooks reach its own network only at the
  ranges and names its operator lists, whatever names resolve to, and
  their answers come back from nowhere else.
- What fctl's own requests reach otherwise (GitHub, Google, Linear,
  email, Web Push) is unchanged: only webhooks, whose addresses people
  choose, go through the narrower network.
