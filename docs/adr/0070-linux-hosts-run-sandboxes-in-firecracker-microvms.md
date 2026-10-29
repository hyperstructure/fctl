# 0070: Linux hosts run sandboxes in Firecracker microVMs

2026-10-29 · accepted

## Context

[0003](0003-hosts-dial-out.md) gave every session its own sandbox, a VM
where the platform has one: Apple's `container` on Apple silicon, Lima
elsewhere on macOS. On Linux, sessions ran in Docker containers, which
share the host's kernel: a bug in it, or in the container runtime, is a way
out of the sandbox for an agent that runs with its permission prompts off,
and Docker's daemon is root's. Lima runs a full VM per session on Linux
too, but it clones a base VM prepared by hand, boots for a minute or more,
runs no images, and is experimental.

The roadmap had "Linux hosts with Firecracker microVMs". Session #536 was
asked to explore everything that has to come together for Firecracker to
be a seamless driver for the Linux hosts whose owners want each agent in a
microVM of its own.

## Decision

- **A `firecracker` driver, which a host's owner chooses** (`fctl host
  install --driver firecracker`). `auto` keeps choosing among Apple's
  `container`, Docker and Lima, so that updating fctl changes no host's
  driver.
- **Nothing runs as root.** Firecracker runs as the host's user, who needs
  `/dev/kvm` (the `kvm` group), `passt` and e2fsprogs. The host downloads
  Firecracker itself, at the release fctl pins (1.17.0, checked against its
  SHA-256), unless `host.toml` names a binary. It runs without
  Firecracker's jailer, which needs root; Firecracker's own seccomp filters
  still apply.
- **The same images as Docker, pulled without Docker.** The host reads an
  image's manifest for its platform, its config and its layers from the
  registry, each checked against its digest, anonymously or with the
  credentials of `docker login`. A short-lived VM booted for it unpacks the
  layers, whiteouts and all, onto an ext4 filesystem, as root there:
  owners, modes, setuid bits and capabilities stay as the image has them,
  without root on the host, and no file of an image touches the host's
  filesystem. Projects' own images run as with Docker; building images
  (`allowed_builds`) is for later.
- **An image's filesystem is shared; each sandbox has a disk.** The
  sandboxes made from an image share its filesystem, read only, and each
  has its own sparse ext4 disk (`disk_gb`, 64 GiB unless `host.toml` says
  otherwise) that the VM lays over it with overlayfs. It takes what the
  session writes, and gives back what it deletes (discard, which
  Firecracker has from 1.17). A stopped sandbox keeps its disk, and
  resuming the session boots from it, with Claude's conversation.
- **fctl-agent is the VM's init.** The host boots each VM with an
  initramfs whose init is its own static fctl-agent. It mounts the root,
  copies itself to `/opt/fctl/bin`, sets up what an image doesn't bring
  (the kernel's filesystems, a hostname, DNS), reaps orphans, and keeps a
  server on the VM's vsock that runs what the host asks as `docker exec`
  does: as the image's user, with its environment, in its working
  directory. The host needs no SSH, and the image nothing of fctl's.
- **The network goes through `passt`, without root.** The VM's `eth0` is
  a TAP device inside the VM, whose frames fctl-agent carries over the
  vsock to a `passt` of the VM's own on the host, which makes them the
  host's own connections, as Podman's rootless networking does. The host
  gets no bridge, TAP device or firewall rule; VMs reach neither each
  other nor the host's loopback, and nothing reaches them from outside.
- **hyperstructure/sandbox publishes the kernel.** Linux's long-term 6.18
  with Firecracker's guest configuration, built for both architectures and
  published as an image, `ghcr.io/hyperstructure/sandbox-kernel:<version>`,
  whose version each fctl release pins. `host.toml` can name another image
  or a file.
- **VMs outlive the host's process.** Firecracker and `passt` run in
  process groups of their own, as the process driver's agents do, which
  the service doesn't stop with the host (`KillMode=process`), so
  restarting or updating the host leaves sessions running. The host finds
  their VMs by their sockets when it starts and attaches again.

## Consequences

- A Linux host's sessions each get a kernel of their own, from the
  default image or their project's, once its owner has added themselves to
  `kvm` and installed `passt`.
- A VM escape would run as the host's user, who can read the runner's
  token and any Claude token kept on the host, as a container escape can
  with Docker. Running Firecracker through its jailer, as another user in
  a chroot, needs a host that runs as root, and is for later.
- Firecracker's VMs share no filesystem with the host, so they get no
  package caches, as Lima's don't.
- An image's first pull takes longer than Docker's (it unpacks in a VM),
  and each image takes all its size on disk: images share no layers.
- A VM holds the memory its guest has touched, but for what its balloon
  reports free.
- The VM's clock follows the host's at boot and at each request of the
  host, not between them, after the host sleeps.
- CI has no KVM: the driver's test that boots a VM runs only by hand, with
  `--ignored`. Session #536 ran it, and sessions end to end, on Firecracker
  in a VM under QEMU's emulation of a CPU with virtualization.
