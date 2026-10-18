# 0018: Hosts make room when their disk runs low

2026-10-18 · accepted

## Context

A stopped session keeps its sandbox for `keep_stopped_hours` (72 by
default), so that a message resumes it with Claude's conversation and what
the agent left uncommitted. Hosts removed sandboxes by age only, and
nothing acted on the free disk that heartbeats reported
([#76](https://github.com/hyperstructure/fctl/issues/76)).

With Apple's `container`, each sandbox's root filesystem is a sparse
512 GiB disk image that never gives freed space back to the Mac: the guest
mounts it without `discard`, the runtime never trims it, and the container
can't run `fstrim`. So a stopped sandbox holds the most its session ever
stored. One `cargo xtask check --quick` of fctl left 5.5 GB. A busy Mac
could fill its disk with sandboxes nobody resumes, and nothing warned
before sessions started failing. `df` in a sandbox shows about 500 GB free
whatever the Mac has.

## Decision

The person who started the session that built this asked for hosts whose
disk needs no watching. #76 left three questions open, and the session
answered them this way:

- **A fixed floor.** `min_free_disk_gb` in `host.toml`, 20 GiB unless it
  says otherwise, 0 for none. A share of the disk would keep 200 GB free
  on a 2 TB disk, where no session needs that much, and would be harder to
  predict. A host with a small disk, like a 64 GB Docker VM, lowers it.
  Hosts that upgrade get the floor without editing anything.
- **Measured where the sandboxes are.** That is Apple's `container` app
  root, from `container system status`, Lima's home, Docker's root
  directory, or on macOS the disk in Docker's VM. The process driver uses
  the data directory. Measuring in the VM runs `df` in a short-lived
  container of the host's image, so it happens every five minutes rather
  than every 30 seconds.
- **Stopped sandboxes go early, oldest first.** Below the floor, the sweep
  removes the stopped sandboxes that `keep_stopped_hours` would keep, those
  stopped longest first. It removes one at a time and measures again after
  each, so that it stops once there is room. A resume of one of those
  sessions starts in a new sandbox on its branch, without Claude's
  conversation and without what the agent left uncommitted, as after
  `keep_stopped_hours`. Running sessions are never stopped for disk.
- **Below the floor, no new sessions, without a drain.** A drain is a
  person's decision: FleetDO keeps who started it, and only a person ends
  it. A low disk comes and goes with the host's free space, and must not
  end a drain someone started, or be ended by someone who stops one. So the
  host sends its floor in its hello (`min_free_disk_mb`), and FleetDO
  compares it with the free disk in each heartbeat. A runner below it gets
  only resumes of sessions whose sandboxes it has, as while it drains, but
  its idle sessions don't stop early. The host refuses other sessions
  itself, for the few seconds before its heartbeat arrives. The runners
  list, `fctl admin runners`, `fctl host status` and `fctl host doctor`
  say how much is free, and queued sessions say they wait for a runner
  with room.
- **No `CAP_SYS_ADMIN` for Apple sandboxes.** With it, fctl-agent could
  run `fstrim` after each turn, so that stopped sandboxes give their freed
  space back. It would widen what root can do in the VM, and whether
  Virtualization.framework frees space on the Mac for a discard needs
  checking there first. That is left for later.

## Consequences

- A busy host no longer fills its disk with stopped sandboxes. A host with
  no room left says so, and the sessions that wait for it say why.
- On a host short of disk, a follow-up to a session that stopped a while
  ago more often starts a new conversation.
- A host with less than 20 GiB free takes no new sessions after it
  upgrades, until it has room or its `min_free_disk_gb` is lowered.
- What the sessions that run write isn't limited: they may still fill the
  disk. Docker's build cache, which builds of projects' images leave, isn't
  pruned either.
- Older hosts send no floor and remove sandboxes by age only, and FleetDO
  places sessions on them as before. A control plane older than its hosts
  ignores the floor, and a host below it refuses the sessions that need a
  new sandbox; those retry, and then fail with its reason.
