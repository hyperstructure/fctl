# 0025: Hosts trim Apple sandboxes as they stop them

2026-10-20 · accepted

## Context

With Apple's `container`, each sandbox's root filesystem is a sparse
512 GiB ext4 disk image on the Mac, cloned from the image's. It grows as
the session writes, and deleting files didn't shrink it: the guest mounts
it without `discard`, the runtime never trimmed it, and the container
can't run `fstrim` (`FITRIM ioctl failed: Operation not permitted`): it
has the OCI default capabilities, which leave out `CAP_SYS_ADMIN`. So a
stopped sandbox held the most its session ever stored: one
`cargo xtask check --quick` of fctl leaves 5.5 GB. A host low on disk
removes stopped sandboxes early
([ADR 0019](0019-hosts-make-room-when-their-disk-runs-low.md)), and those
sessions resume without Claude's conversation or what the agent left
uncommitted.

ADR 0019 left open whether to give Apple sandboxes `CAP_SYS_ADMIN`, so
that fctl-agent could run `fstrim` after each turn. The capability widens
what root can do in the container (mounts, namespaces), and whether
Virtualization.framework frees space on the Mac for a discard needed
checking on a Mac
([#92](https://github.com/hyperstructure/fctl/issues/92)).

Since then:

- The sandbox's disk takes discards: in a sandbox on the hyperstructure
  Mac, `lsblk -D` shows a granularity of 512 B and up to 513G a request.
- Apple's `container` 1.4 added `container clean`
  ([apple/container#1949](https://github.com/apple/container/pull/1949)).
  The VM's init, vminitd, enters the container's mount namespace and runs
  `FITRIM` on `/` and on writable volumes, with privileges the container
  doesn't have. It works only while the container runs.
- Apple's integration tests (`TestCLIClean`), which its CI runs on Macs
  with macOS 26, write 10 MiB in a container, delete it, `sync` and clean.
  They check that the allocated size of the container's `rootfs.ext4` on
  the Mac fell by at least 80% of what the write took.
- The same check by hand, on a Mac: with a 1 GiB file written in an
  Alpine container, its `rootfs.ext4` had 2,190,692 KiB allocated (`du`).
  After `rm`, `sync` and `container clean`, it had 1,109,288 KiB: the Mac
  got back 1,081,404 KiB.

## Decision

- **The host trims, through the runtime.** Before the host stops a
  running Apple sandbox, it runs `sync` in it, then `container clean`.
  ext4 trims only the blocks whose release its journal has committed,
  hence the `sync`.
- **No `CAP_SYS_ADMIN`.** Sandboxes keep the capabilities they had: the
  trim needs none of theirs, and nothing in the sandbox decides when it
  happens. fctl-agent doesn't change.
- **When sandboxes stop, not after each turn,** as the person who started
  the session that built this chose. Idle sessions stop ten minutes after
  their last turn
  ([ADR 0013](0013-idle-sessions-stop-and-a-message-resumes-them.md)), and
  stopped sandboxes are what a host keeps for days and removes when its
  disk runs low. Trimming as they stop covers them with one trim each.
- **Not `discard`.** `container run` can't mount the root filesystem
  with it, and a guest that discards on every delete pays for it on every
  delete.
- **Best effort.** `container` before 1.4 has no `clean`, and a sandbox
  made by `container` 1.0 or earlier has an init without the trim. Then,
  and whenever the trim fails, the host stops the sandbox as before, and
  its log says that the sandbox keeps the space. A `sync` that doesn't
  finish within a minute skips the trim.

## Consequences

- Stopped Apple sandboxes no longer hold the space of the files their
  sessions deleted, so a host low on disk removes fewer of them, and
  fewer resumes lose their conversation. What a trim can't free stays:
  the files, and the filesystem's own blocks, like its journal (1 GiB at
  most).
- Stopping an Apple sandbox takes a `sync` and a trim longer. Each may run
  for a minute at most, like any driver command.
- Running sandboxes still hold the space of what their sessions deleted,
  until they stop. A host low on disk could trim them before it removes
  stopped ones.
- Hosts with an older `container` keep stopped sandboxes as before, and
  log it at each stop; upgrading `container` is enough. Older hosts never
  trim. The change is the host's alone: no protocol, control plane or
  sandbox image change.
- The Docker, Lima and process drivers don't change.
