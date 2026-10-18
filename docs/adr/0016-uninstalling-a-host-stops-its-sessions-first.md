# 0016: Uninstalling a host stops its sessions first

2026-10-18 · accepted

## Context

`fctl host uninstall` stopped the host's service and said "Configuration
and sandboxes are kept", with nothing about the sessions that ran there.
Agents are detached from the host on purpose, so that restarting it
doesn't stop them, so uninstalling left them working in their sandboxes
without reaching fctl. Ten minutes later the control plane marked their
sessions stopped, and a message
([0013](0013-idle-sessions-stop-and-a-message-resumes-them.md)) then
resumed one on another runner while its old agent might still be finishing
its turn: two agents on one branch. It is how a session was lost in
[#46](https://github.com/hyperstructure/fctl/issues/46), when someone
uninstalled a host so that it would take no more sessions.

Draining ([0015](0015-runners-drain-and-their-sessions-carry-on.md)) now
covers "no more sessions here". Uninstalling remains for removing a host.

## Decision

The person who started the session that built this decided it, in
[#56](https://github.com/hyperstructure/fctl/issues/56):

- With no sessions on the host, `fctl host uninstall` works as before.
- With sessions, it lists them, says what happens to them, and asks before
  stopping them. Without a terminal it refuses unless given `--wait` (drain
  the runner, wait until none runs, then uninstall) or `--force` (stop them
  now, then uninstall).
- Stopping goes through the control plane, with the runner token
  (`POST /api/v1/runner/drain` with `stop_sessions`), not through the
  daemon's signals: the host stops each session as for any stop, so the
  agent pushes its work and the session shows why it stopped right away.
  FleetDO also forgets the runner as the home of every session whose
  sandbox it has, so that a message resumes them elsewhere instead of on a
  host that is going away.
- When the host isn't connected, the control plane stops its sessions at
  once, and the CLI stops the sandboxes on the machine with the driver; when
  the control plane can't be reached, the CLI refuses while sandboxes of
  sessions run, unless given `--force`.

## Consequences

- Uninstalling no longer leaves agents working where nobody sees them.
- `--force` costs the sessions' conversations: they resume in new sandboxes
  elsewhere. `--wait` keeps them until they finish, which can take as long
  as people keep following up, since a draining runner still resumes its
  own sessions.
- The runner stays drained after an uninstall; installing the host again
  with the same token needs `fctl host undrain` before it takes sessions.
- Scripts that ran `fctl host uninstall` while sessions ran now fail until
  they choose `--wait` or `--force`.
