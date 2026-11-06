# 0076: Sessions wait for a runner whose driver is ready

2026-11-06 · accepted

## Context

A runner's sandboxes come from its host's driver: Docker, Apple's
`container`, Firecracker, Lima or the process driver. Docker that isn't
running, a Linux user who isn't in the `docker` group, or Apple's service
that stopped as the Mac restarted leave a host connected to the control
plane but unable to start a sandbox. Since fctl#411, hosts say why in
their hello and again in a heartbeat once it changes, and the runners
lists show it, but FleetDO still sent such a runner sessions, and each
failed with the reason. A person who quit Docker Desktop on their laptop,
the workspace's only runner, saw every session they started fail, and had
to resume each one once Docker ran again.

The host probes its drivers when it starts and every five minutes (every
30 seconds while the one it uses isn't ready), so what the control plane
knows can be minutes old: Docker can stop just after a probe that found it
running.

## Decision

The person who started the session that built this asked that sessions
start or show that they are queued, rather than fail, while a runner's
driver isn't ready, as they do while its disk is low
([ADR 0019](0019-hosts-make-room-when-their-disk-runs-low.md)):

- **A runner whose driver isn't ready takes no session.** FleetDO reads
  the driver the host uses from its hello and its heartbeats' drivers
  (`fctl_domain::schedule::driver_problem`), and `schedule::pick` passes
  over such a runner, also for a resume into the sandbox it has, as if it
  were offline: the session starts on another runner when one can take
  it. Unlike a drain or a low disk, a runner that can't start sandboxes
  can't resume one either.
- **A session with nowhere else to go waits and says why**, naming the
  runner and the host's reason ("Waiting for studio, whose docker driver
  isn't ready: the Docker daemon isn't running"), or each runner's when
  several could take it. It waits only for runners that would take it
  once ready: one too small for it, or that drains, doesn't count.
- **It starts once a driver is ready.** The host probes every 30 seconds
  while its driver isn't ready and sends a heartbeat as soon as that
  changes, rather than at the next beat, and FleetDO dispatches at every
  heartbeat.
- **The host checks too, as late as it can.** It declines a session while
  its last probe says the driver isn't ready. When a new sandbox can't be
  made, it probes the driver again: if that is why, it hands the session
  back (a second `StartResult`, declining) instead of failing it, with a
  heartbeat that says why, and FleetDO queues it again, as for any
  decline. Such declines say they are for the driver, so that a resume
  still goes back to its sandbox there once the driver is ready, where
  after other declines it goes to any runner. A session someone stopped meanwhile stays stopped: FleetDO
  marks an assignment it asked to stop, and a decline doesn't queue it
  again.
- **Only hosts that say they keep the control plane told.** Hosts
  announce `drivers` in their hello's features. Older hosts say their
  drivers only as they connect, so what FleetDO knows of them can be old:
  it places sessions on them as before, and their runners' lines say
  sessions that start there fail.

## Consequences

- A session started while Docker is down on its only runner stays queued
  and starts by itself within about 30 seconds of Docker answering,
  instead of failing.
- A resume whose runner can't start sandboxes starts elsewhere in a new
  sandbox, without Claude's conversation, when another runner can take
  it, as when that runner is offline.
- A driver that keeps failing still fails a session: a host's declines
  count towards the five attempts a session gets, so one that hands
  sessions back as its driver comes and goes doesn't loop.
- A probe that says a driver isn't ready when it could start sandboxes
  keeps sessions off its runner, where before they started there. The
  host probes the driver it uses with the settings it uses
  (`agent_bin` for the process driver).
