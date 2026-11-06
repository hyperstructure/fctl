# 0076: Sessions hear when their runner runs short of disk, and VM sandboxes pause before it fills

2026-11-06 · accepted

## Context

Hosts keep a floor of free disk, `min_free_disk_gb`
([ADR 0019](0019-hosts-make-room-when-their-disk-runs-low.md)). Below it
they take no new sessions and remove stopped sandboxes early, and the
runners list tells instance admins. The sessions running there heard
nothing, and nothing named a full disk when it stopped them
([#410](https://github.com/hyperstructure/fctl/issues/410)).

In session #733 a host's disk filled up. Its agent was partway through a
turn, which stopped. Its VM was restarted, and a file the agent had just
written came back as 8,253 NUL bytes, its size and modification time
kept, so that `git status` called the tree clean. `df` in the sandbox had
shown 495 GiB free all along. A Firecracker VM's disk is a sparse file on
the host's (`disk_gb` of it), so the guest can't see that the host has no
room left to back what it writes. The guest's page cache takes the write,
and the host refuses its blocks later: the file keeps its metadata and
loses its data. The session page said nothing about a disk.

## Decision

The person who started the session that built this asked for issue #410
to be fixed. The issue left two choices open, and the session made them
this way.

- **Two levels of short.** A runner is *low on disk* below its floor, and
  *nearly out of disk* below 2 GiB free, or below its floor when that is
  lower (`fctl_domain::disk::room`, `OUT_MB`). A fixed amount rather than
  a share of the floor: what a runner's sessions have left to write before
  writes fail doesn't grow with its disk, and a host that keeps a small
  floor still gets a warning before its disk is full. A host that keeps no
  floor (`min_free_disk_gb = 0`) is never short, as before.
- **The control plane tells the sessions.** FleetDO keeps what each
  runner's sessions were told about its disk and, after each heartbeat,
  tells them as `fctl_domain::disk::heard` says: notices in their
  transcripts (`runner_low_on_disk`, `warn`; `runner_out_of_disk`,
  `error`; `runner_disk_ok`, `info`), and the runner's disk on their pages
  (`Snapshot.runner_disk`, `runner_connected`'s `disk`), which SessionDO
  keeps for the pages opened later. Less room is told at once. More room
  is told once it has lasted ten minutes: the sweep removes stopped
  sandboxes only until the disk is back at its floor, which the running
  sessions may fill again a few minutes later, and each crossing would
  otherwise say so in every transcript. The pages follow each heartbeat,
  and again when the free space moves by a tenth. A session that starts on
  a runner short of disk hears so, and a new runner clears what the last
  one said. No protocol change was needed for it: heartbeats carried the
  free disk, and hellos the floor.
- **The page shows it apart from the stall line.** The stall line says
  that the agent's work isn't coming through, and stops showing work in
  progress. A runner short of disk still brings the agent's work, so the
  page says how short below it, in amber while low and in red (`danger`)
  once nearly out, when writes are about to fail. The admins' runners list
  says `out of disk`, in red, below 2 GiB.
- **VM sandboxes pause; they aren't stopped.** Below the hard threshold the
  host pauses its running Firecracker VMs (`PATCH /vm` to `Paused` on
  Firecracker's API socket), and resumes them once twice that is free (4
  GiB, or twice the floor when that is lower than 2 GiB:
  `fctl_domain::disk::holds`), so that what they write as they resume
  doesn't pause them again at once. A paused VM keeps what it holds in its
  memory, what it has yet to write to its disk too, and loses none of it.
  The other option, stopping the session with a reason, would write the
  VM's memory out to a disk that may not have room for it, end the turn,
  and leave a stopped sandbox, which the sweep for room may then remove
  with Claude's conversation and what the agent left uncommitted. A stop asked for while paused
  resumes the VM first, so that the agent stops cleanly and the VM writes
  out its disk, as far as there is room. The host measures its disk every
  5 seconds rather than every 30 while it is below its floor, so that it
  pauses before its sessions fill the rest.
- **While paused, the agent isn't missed.** The host doesn't ping or
  attach to a paused sandbox's agent, and tells its viewers that it paused
  it (`AgentLink.paused`, which the page shows as a stall, and
  `sandbox_paused` and `sandbox_resumed` notices, which a host says in the
  transcript with `UplinkUp::Notice`). A host that restarts finds a paused
  VM paused, and keeps it so until it has room.
- **Other drivers don't pause.** Docker's containers write straight to
  the host's disk, so their writes fail where the agent sees them. Apple's
  `container` and Lima also give their VMs disks that are files on the
  host's, but pausing them would need their own way to, which this leaves
  for later: their sessions are told, and their writes can still be lost
  when the disk fills.
- **A full disk is named where it is the cause** (`fctl_domain::disk::out_of_space`,
  `io::ErrorKind::StorageFull` or "No space left on device"): a session
  that couldn't start fails with "Runner X ran out of disk: …"; when the
  host starts fctl-agent again while its disk is short, or can't for want
  of room, the transcript says so (`agent_restarted`, beside fctl-agent's
  own `turn_cut_off`); fctl-agent tells the host, beside the journal it
  can't write, that the sandbox's disk is full; and Claude's
  `claude_restarted` and `claude_crashed` notices say the disk was full
  when it was.

## Consequences

- The people watching a session see when its runner runs short of disk,
  and how short, in the transcript and on the page, and agents watching
  sessions through the API see the notices' codes.
- A Firecracker session on a runner that fills up waits, paused, rather
  than losing what it writes; it carries on by itself once there is room,
  which the sweep makes when it can, and otherwise someone with access to
  the runner. A paused VM keeps its memory meanwhile.
- A session that waits paused holds its place: its turn goes on when it
  resumes, though Claude's requests in flight may need retrying. A paused
  session that waits for a message stops at its idle timeout as before,
  resumed to stop.
- Sandboxes of the other drivers aren't paused: Docker's see the full
  disk, and Apple's and Lima's can still lose writes, as before, though
  their sessions are now told.
- Older hosts pause nothing and send no notices of their own, and older
  control planes ignore the hosts' notices. Hosts older than ADR 0019
  keep no floor, so their sessions hear nothing of their disk.
