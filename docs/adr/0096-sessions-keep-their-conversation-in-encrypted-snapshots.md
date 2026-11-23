# 0096: Sessions keep their conversation in encrypted snapshots

2026-11-23 · accepted

## Context

A stopped session keeps its sandbox on its runner, where Claude's
conversation and the work its agent hadn't pushed stay
([ADR 0013](0013-idle-sessions-stop-and-a-message-resumes-them.md)). A
resume anywhere else, because that runner drained, went away or kept the
sandbox past `keep_stopped_hours`, started a new conversation from the
session's task and branch: the agent forgot what it had been told and
what it had found out, and lost what it hadn't committed and pushed.
FleetDO made a resume wait ten minutes for a busy runner that had the
sandbox, to spare it that. Machines in pools that grow and shrink with
their work (#615) come and go, so this became the common case, and
organizations asked that sessions carry on wherever they resume (#616).

A sandbox holds much more than a session needs to carry on: installed
packages, build output, caches. Saving it whole (a container commit, a
VM's disk) would take gigabytes, depend on the driver, and not move
between drivers or architectures. Copying it from runner to runner needs
the runner that has it, which is the one that went away. Claude Code keeps
a conversation in files (`~/.claude/projects/<directory>/<session>.jsonl`
and what it keeps beside them), which `--resume` reads anywhere, and git
can carry a repository's work, uncommitted files included, as a bundle.

## Decision

- **A session's snapshot is its conversation and its unpushed work.** As
  the sandbox stops, after the run's last event, fctl-agent saves Claude
  Code's files for the session and, for each of its repositories, a commit
  of the working tree on top of the commits not pushed, as a git bundle;
  what `.gitignore` leaves out stays behind. Restoring puts the
  conversation back under the new working directory and the work back on
  the branch, uncommitted again, where the branch is where the snapshot
  left it or behind; otherwise at `refs/fctl/wip/<session>`, and Claude
  hears where.
- **The agent seals it; the control plane keeps it without opening it.**
  Each session has a random key, kept in D1 sealed with the master key
  and given in the spec only to the runner that runs it. The agent packs
  the snapshot and seals it in segments of AES-256-GCM bound to the
  session; the host hands it to the control plane in parts with its
  runner token, and SessionDO keeps the session's newest in its storage.
  So a snapshot works on app.fctl.sh and on an instance of one's own
  alike, without a bucket or keys to manage, and whoever reads the
  storage without the master key reads nothing.
- **The host uploads before it says the run ended**, so that a message
  right after a stop finds the snapshot, for ten minutes at most.
- **It is used only when it is the session as it was.** A snapshot older
  than the session's last run (that run carried on elsewhere without
  saving one), one written by a newer or another major version of Claude
  Code than the sandbox has, and one that fails its SHA-256 or doesn't
  open, are not restored: the session starts a new conversation as
  before, and the transcript says why.
- **A resume with a snapshot doesn't wait for its runner.** FleetDO still
  sends it to the runner that has the sandbox, but where that runner is
  busy or away it starts elsewhere at once, rather than after ten minutes.
- **On by default, with limits people choose.** A workspace keeps its
  sessions' snapshots for 30 days after each is saved (1 to 365), or none;
  an instance keeps them, at most 200 MiB each (up to 2,048), or none.
  Past the limit an agent leaves the conversation's images out first,
  then the work, then saves none. The hourly job deletes snapshots past
  their days, and all of a workspace or instance that turns them off.
  Review agents' sessions save none: what they review is a pull request
  they don't carry on.
- **Hosts and agents opt in.** Hosts that save and restore snapshots say
  so in their hello; FleetDO leaves the snapshot out of other hosts'
  specs, so older hosts save none and run as before.

## Consequences

- A session resumes on any runner with its conversation and its work,
  whether its runner drained, went away after it stopped, or removed its
  sandbox. A runner that goes without a word (a crash, a Spot reclaim
  without notice) saved nothing of the run it cut off: that session
  starts a new conversation, as before.
- The control plane stores what sessions' conversations hold, which
  includes what agents read and ran, for 30 days unless workspaces choose
  otherwise: app.fctl.sh's privacy policy says so. Whoever has both its
  storage and its master key can read them, as they can the instance's
  other secrets.
- Storage grows by a few MB for each session stopped in the last 30 days,
  up to the instance's limit each: on Cloudflare, Durable Object storage;
  on the standalone server, its data directory.
- What `.gitignore` leaves out (dependencies, build output, `.env` files)
  isn't restored: the agent installs or builds it again, as in any new
  sandbox.
- Stopping a session takes as long as its upload, which a drain's `--wait`
  and a cloud's notice include.
- A later change can keep snapshots in a bucket of an organization's own,
  for sessions whose snapshots outgrow the control plane's limit.
