# 0093: Runner pools scale with the sessions that wait for them

2026-11-23 · accepted

## Context

Organizations that run fctl's runners in their own cloud enroll machines
automatically with a workspace's enrollment token
([ADR 0071](0071-runners-enroll-themselves-with-a-workspaces-enrollment-token.md)),
but couldn't scale them on demand (#615): nothing told an autoscaler how
many sessions wait for a runner, every runner took every project's
sessions, and FleetDO spread sessions over the least-loaded runners, so
that machines rarely emptied out to be scaled in. Hosts already sent
`host.toml`'s labels, and `fctl_domain::schedule::pick` could require
labels, but nothing set any. Carl Oscar asked for pools that grow and
shrink, configurable where it makes sense, with sensible defaults.

## Decision

- **A pool is an enrollment token's runners.** A token gets labels, which
  each runner it enrolled carries beside its `host.toml` labels, and a
  placement. FleetDO reads both from D1 at each dispatch, never from a
  host's hello, so that no machine claims a pool it didn't enroll in, and
  changing a token's labels moves its runners at once, older hosts' too.
  No new object: tokens already have a name, a limit, approval and
  removal, and the people who manage them.
- **Projects choose pools by labels** (`runner_labels`), which a runner
  must have all of, and wait for them unless they choose a fallback
  (`runner_fallback_minutes`): then any runner takes a session that has
  waited that long. Requiring is the default, since labels usually say
  what a session needs (a GPU, a region); the fallback makes them a
  preference for those who'd rather run elsewhere than wait.
- **Placement is a pool's choice: `spread` or `packed`.** Packed takes the
  busiest runner with room, so that idle runners empty out and can go.
  A token that removes offline runners packs by default, since its
  machines come and go; others spread, as runners always did. Runners
  that spread take a session before runners that pack it, so that a
  fixed fleet runs what it can before an elastic pool adds machines.
- **Demand is counted by FleetDO, from what it already has**, and read
  with the enrollment token (one secret for the machines and their
  autoscaler, which learns only what running the pool would show), or by
  the workspace's admins and service accounts, for those who'd rather not
  hand the enrollment token to their autoscaler. `wanted` is the runners
  that would run everything running and waiting, never fewer than those
  running a session (scaling down to it never takes a busy one) nor more
  than the token's limit. A waiting session counts towards one pool, so
  that two pools don't both grow for it, and one that waits for its home
  runner counts nowhere until it would start elsewhere.
- **A pool remembers its shape** (capacity, sandbox limits, images) from
  its runners' hellos, so that demand counts with it once none is online
  and an autoscaler starts it from none.
- **Exporters reuse what organizations run**: the JSON as KEDA's
  metrics-api scaler reads it, `fctl runners demand --cloudwatch` through
  the AWS CLI rather than an AWS SDK in fctl, and Prometheus at
  `/api/v1/metrics` for self-hosted control planes, behind a token of its
  own that the deployment sets, so that no instance publishes metrics
  unless asked.

## Consequences

- A session whose project asks for labels no runner has waits, and says
  which pool, or which labels, it waits for. Members' own pools serve only
  their sessions, as their runners do.
- Labels compare as written: `host.toml` labels must be lowercase to match
  a project's, which the control plane lowercases.
- Older hosts carry their pool's labels and placement without an update;
  demand from zero needs one of its runners to have connected since this
  release, to know its shape.
- Retiring idle hosts, draining when a cloud takes a machine, pod deletion
  costs and the reference deployments follow in their own changes; demand
  and placement are what they build on.
