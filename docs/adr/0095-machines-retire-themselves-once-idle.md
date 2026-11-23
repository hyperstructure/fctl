# 0095: Machines retire themselves once idle

2026-11-23 · accepted

## Context

A pool that grows and shrinks (#615) needs machines to go once there is
less to do. Left to an autoscaler, scaling in picks a machine by its own
rules (the oldest, the one in the busiest zone) and takes it with whatever
runs on it: sessions stop mid-turn and resume elsewhere ten minutes later.
`fctl host drain --wait` and `fctl host uninstall` let a machine go cleanly,
but nothing ran them when a cloud scaled in, and nothing knew which
machines were idle but fctl itself.

## Decision

- **Machines take themselves away.** With `retire_idle_minutes` in
  `host.toml` (`fctl host install --retire-when-idle 20m`), a host whose
  runner has run nothing for that long drains, checks that nothing came
  meanwhile, and runs `retire_command`: the cloud's call that terminates
  this machine and lowers the group's size, or `systemctl poweroff`. Every
  machine is protected from scale-in, so the autoscaler only adds machines,
  and only idle ones go.
- **The command runs before the runner leaves.** A command that fails (the
  group is at its minimum size, a missing permission) leaves the machine a
  runner that takes sessions again and tries after another idle while.
  Removing the runner first would leave, on any such failure, a machine
  that costs money and runs nothing. Once it worked, the host uninstalls
  its service, so that no service manager starts it again on a machine
  that is going.
- **A retirement that stops short undoes its drain**, from a file kept
  until the control plane hears it, so that neither a lost answer nor a
  restart leaves a runner drained for good, a restart while the command
  runs included; and while it retires, the host declines the resumes the
  control plane still sends it.
- **Being idle is the host's to judge**, from what it runs and is about to
  (sessions, starts that wait, sign-ins, chores on sandboxes), not the
  control plane's: no new protocol, and older control planes work.
- **A deadline drains as far as it can.** `fctl host drain --timeout`
  waits for the turns under way as long as the cloud's notice allows, then
  stops what still runs as `fctl host uninstall --force` does, so that
  every agent pushes its work.
- **In Kubernetes, the pod's deletion cost follows its sessions**, which a
  ReplicaSet reads when it scales down: a Deployment scaled to a pool's
  demand removes empty pods first. The host patches its own pod with its
  service account's token, when it may, and says once when it may not.

## Consequences

- Retiring loses what any machine that goes loses: sessions whose
  sandboxes it kept resume elsewhere without their conversation, until
  session snapshots carry it (#616).
- A pool's demand never counts fewer runners than run a session (its
  `wanted`, from the change that adds runner pools, #634): a scaler that
  raises the group's size to it adds machines for the sessions that wait,
  and the machines that retire take it down again.
- `retire_command` runs as the host's user: it needs the permissions the
  cloud's call takes (an instance role that may terminate its own instance,
  `sudo` for a poweroff).
