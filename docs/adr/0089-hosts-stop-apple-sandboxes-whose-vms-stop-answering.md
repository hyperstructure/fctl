# 0089: Hosts stop Apple sandboxes whose VMs stop answering

2026-11-19 · accepted

## Context

On 2026-11-19 a runner on the apple driver started nothing for hours. A
session's sandbox had filled its memory overnight (fctl-agent had warned
at 87%), and its VM stopped answering. The host's link to fctl-agent went
quiet, and the host attached again and again, as a runtime that hangs may
come back ([Sandboxes and drivers](../architecture.md#sandboxes-and-drivers)):
each `container exec` in the sandbox ran out of time after a minute, and
so did the `container stop` when the session was stopped. After that, every new
session's `container run` waited 30 minutes and failed, one after
another, while `container list` and `container system status` answered,
so the driver looked ready, `fctl host doctor` said so, and the control
plane kept sending sessions. Killing `container-apiserver` and running
`container system start` brought it back.

Apple's `container` (1.0 to 1.7.1, read at its source):

- Each sandbox's VM runs from a helper process of its own,
  `container-runtime-linux start --root … --uuid NAME`, a launchd job
  whose calls (`startProcess`, `stop`, `kill`, `statistics`) take one
  lock each: one that waits on the VM holds the others.
- The API server's `ContainersService` takes one lock for every
  `create`, `bootstrap` (the start of `run` and `start`), `startProcess`
  (every `exec`, in any container) and the end of every stop and delete,
  and holds it while it waits on a helper, with no time limit. `list`,
  `inspect`, `stats` and `system status` don't take it.
- Killing the `container` command that waits frees nothing: the API
  server goes on waiting. When a helper goes, though, the API server's
  calls to it fail at once, which frees its lock, and it takes the
  sandbox for stopped, as Apple's code expects of a helper killed from
  outside.
- [apple/container#1916](https://github.com/apple/container/issues/1916)
  reports the same: an `exec` in a VM that doesn't answer wedges the
  service until `container-apiserver` is killed. A fix proposed in
  [apple/container#1926](https://github.com/apple/container/pull/1926)
  is open, not merged.
- Restarting the API server leaves the helper and its VM running, and
  the new one takes the sandbox for stopped: starting it again reaches
  the old helper, which still waits, and holds the new one's lock.

## Decision

- **The host looks for stuck VMs when commands wait.** When a
  `container` command that waits on VMs (`exec`, `run`, `create`,
  `start`, `stop`, `kill`, `delete`, `cp`, `clean`) runs out of time, or
  a `run` has taken three minutes (and every three minutes after), the
  host lists the running containers and asks each for its stats
  (`container stats --no-stream`), all at once. Those wait on the VM's
  helper and the VM, not on the API server's lock. One look runs at a
  time; commands that run out of time meanwhile wait for its answer,
  which stands for 30 seconds.
- **It stops its own sandboxes that don't answer within two minutes**,
  killing their `container-runtime-linux` (found by its `--uuid` in
  `ps`). A VM answers stats within a second however busy, and one that
  takes two minutes holds up the whole runner. The host waits until the
  API server takes the sandbox for stopped, and logs which it stopped and
  why. Commands that waited on its lock go through; one that ran out of
  time while another sandbox held it up (the `true` before a session
  resumes in its sandbox) is tried once more.
- **The session fails, and resumes in its sandbox.** A session whose
  sandbox the host stopped so ends at the next second, failed, saying
  why and that resuming it starts the sandbox again with its files. The
  host doesn't start the sandbox again by itself: what ran in it is gone,
  and starting the agent afresh in the middle of its turn would start its
  task over.
- **It leaves what isn't its own.** A stuck container that isn't fctl's
  (no `fctl-` name), or a sandbox whose helper the host can't find,
  makes the apple driver not ready, with what stops it (`kill -9` of its
  helper's pid, or the restart in [Hosts](../cli.md#hosts)). The host
  looks again at each probe and is ready once none is stuck.
- **`fctl host doctor` names stuck VMs**: every running container whose
  VM doesn't answer stats within 10 seconds, with the command that stops
  it.
- **Not restarting the API server.** It stops nothing that waits, and
  leaves the stuck helper for the sandbox's next start.

## Consequences

- One frozen sandbox costs its session, and holds up the runner for
  about four minutes (a minute for the link to go quiet, half a minute
  for a hello, a minute for an `exec`, two for its stats) rather than
  until someone restarts Apple's `container`.
- What a frozen sandbox had in memory is gone, as after a restart of the
  Mac; its files stay, and the session resumes in it.
- A VM that is only slow, not stuck, is stopped once it takes two
  minutes to answer stats while a command waits on it.
- The host kills processes that Apple's `container` started, by their
  command line; a version of Apple's `container` that runs VMs otherwise
  leaves the driver not ready, saying so, rather than wedged unseen.
- Docker, Lima and Firecracker don't change: their sandboxes don't hold
  each other up this way.
