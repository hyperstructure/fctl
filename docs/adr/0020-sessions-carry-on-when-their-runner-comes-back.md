# 0020: Sessions carry on when their runner comes back

2026-10-18 · accepted

## Context

A host can stop answering while its agents keep working: a laptop goes to
sleep, Docker hangs, the network drops, or the daemon is stopped. Agents
are detached from the host on purpose, and journal what they do. In
[#46](https://github.com/hyperstructure/fctl/issues/46), runner `now`
went quiet for half an hour while its agent opened pull requests in three
repositories.

A runner away for ten minutes loses its sessions: FleetDO marks them
stopped, so that a message can resume them on another runner. When the
runner came back with the agent still at work, FleetDO found the session
stopped and told the host to stop it as if it had been deleted. The agent
was interrupted, and what it did meanwhile stayed in its sandbox's journal:
the runner was no longer the session's, so its uplink was refused.

#46 left three questions for a person: whether such a session carries on
or stays stopped and says why, whether ten minutes is the right grace
period, and what hosts do about disk.

## Decision

The person who started the session that built this asked for what was left
of #46 to be done. This follows the answers suggested in the issue:

- **A session stopped only because its runner was away carries on when the
  runner comes back with it**, as long as no other run of it has started.
  FleetDO remembers the sessions it stops for their runner's absence
  (`stopped_away`, in its storage). When that runner's hello reports one
  running, FleetDO assigns it to the runner again and restores the
  session's runner, so that the host's uplink gets in and replays the
  agent's journal, and the transcript says the runner is back. Since
  [0013](0013-idle-sessions-stop-and-a-message-resumes-them.md) a session
  stays with its sandbox where it can; this keeps it there when the
  runner was only away.
- **So does a session resumed meanwhile that no runner took.** The agent
  that is still running gets the messages, rather than being stopped for a
  new run in the same sandbox.
- **Otherwise the old run stops, with a reason that says why**: `Shutdown`
  when the session runs on another runner, `Deleted` when its project was
  deleted, and `User` when someone stopped or archived it, or it failed.
- **The grace period stays at ten minutes.** The session page says at once
  that a runner is away (#69), and a session stopped for it comes back with
  its runner, so waiting longer would only delay a resume elsewhere.
- **Disk is [0019](0019-hosts-make-room-when-their-disk-runs-low.md)'s**:
  hosts keep a floor of free disk, measured where their sandboxes are (in
  Docker's VM on macOS), remove stopped sandboxes early to stay above it,
  and take no new sessions below it. Quotas per sandbox, the other option
  #46 named, are left out: they depend on the driver.

## Consequences

- A runner that sleeps, hangs or loses its network for more than ten
  minutes no longer costs its sessions their work: the agents carry on,
  and the transcript fills in what they did.
- A session can show as stopped and then carry on. The stopped status says
  that it may, and the transcript says when the runner is back.
- A session resumed on another runner meanwhile keeps running there; its
  old run stops when its runner comes back, and its agent pushes what it
  committed.
- A session someone resumed with **Resume** while its runner was away,
  and that carries on, runs on the resumer's Claude subscription from its
  agent's next credential refresh.
- It needs no host or agent release: hosts already report their sessions
  in every hello. No protocol change and no migration; FleetDO's new table
  is created with `IF NOT EXISTS`.
