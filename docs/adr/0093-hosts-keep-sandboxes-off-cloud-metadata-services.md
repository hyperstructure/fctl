# 0093: Hosts keep sandboxes off cloud metadata services

2026-11-23 · accepted

## Context

Organizations run runners on cloud machines, which enroll themselves from a
boot script ([ADR 0071](0071-runners-enroll-themselves-with-a-workspaces-enrollment-token.md)).
A cloud machine's instance metadata service, at `169.254.169.254` on most
clouds, hands out the machine's own credentials and its user data, where
the boot script keeps the enrollment token, to whatever asks from the
machine. Sandboxes reached it like any other address, so an agent that
text steers could read them.

A sandbox's user has `sudo`, so whatever keeps a sandbox off the service
must hold against the sandbox's root. In a Docker container root lacks
`NET_ADMIN`, so routes set in its network namespace hold, but it has
`NET_RAW`, whose packet sockets send frames around them. In a VM (Apple's
`container`, Lima, Firecracker) root has every capability and can change
anything set inside. The host doesn't run as root, so it can't change the
machine's firewall either. Running a program in the sandbox with
`docker exec --privileged` would give it every capability without
seccomp or AppArmor, and since the sandbox's root can replace any file in
it, a privileged exec of a file there could be the sandbox's own program:
a way out to the host.

## Decision

- **Sandboxes don't reach cloud metadata services** unless the host's
  owner lets them (`allow_cloud_metadata = true` in `host.toml`, for
  sessions meant to use the machine's own role). The addresses are
  `fctl_domain::metadata::ADDRESSES`: the one most clouds use, ECS's task
  credentials, EKS Pod Identity's, Alibaba Cloud's and Tencent Cloud's,
  and AWS's over IPv6.
- **Docker sandboxes run without `NET_RAW`** and get routes that make those
  addresses unreachable in their network namespace. A helper container
  joins that namespace alone, with `NET_ADMIN` alone, from the sandbox's
  image by its ID, and runs a copy of the host's own fctl-agent
  (`fctl-agent keep-off-metadata`), which adds the routes over rtnetlink,
  then goes. Nothing of the sandbox's runs with a capability it lacks, and
  no image needs a tool for it. The host does it before fctl-agent starts
  in a sandbox, and again as it starts it in one that runs already.
- **A sandbox made with raw sockets starts anew.** Docker can't take a
  capability from a container it made, so a sandbox made before this, or
  while its host let sandboxes reach metadata, is removed as its session
  next starts, and the session goes on in a new one, as when a stopped
  sandbox expires: Claude starts a new conversation on the session's
  branch. One that runs as its host updates gets the routes, which stop
  ordinary connections, and its session hears that its root could still
  go around them until then.
- **Firecracker VMs' frames to those addresses are dropped on the host**,
  by `fctl-agent net-filter`, which the host starts between the VM's link
  and its `passt`, out of the VM's reach. It answers IPv4 ones with an
  ICMP "administratively prohibited", so that a connection there fails at
  once with "No route to host" rather than hanging; IPv6 ones are dropped.
  DNS goes through (TCP or UDP to port 53): Google Cloud's metadata server
  is also its machines' nameserver, which VMs ask as the host does, and it
  hands out nothing else on that port. Frames whose Ethernet type says IP
  but that don't read as IP, and tagged ones, go nowhere, so that nothing
  past the filter can read a destination in them that it didn't. It runs
  detached, as passt does, so the network outlives the host's restarts. A
  VM that a host from before it linked gets it as a session next starts
  in it, which links its network again.
- **Apple container and Lima VMs, and the process driver, aren't kept off
  them**: what the host could set inside a VM, the VM's root takes out. The
  docs say to use Docker or Firecracker on cloud machines.
- **A run that can't be kept off them doesn't start.** Should the helper
  fail (a runtime that refuses it), the run fails, saying why and that
  `allow_cloud_metadata = true` lets sandboxes reach them, as a
  Firecracker VM whose network can't be linked through the filter does. A
  run under way as its host restarts carries on, and its session hears
  that it may reach them until its next run.

## Consequences

- Agents on cloud machines lose the machine's own credentials, which they
  shouldn't have had; those that need a cloud get credentials of their own
  through identity tokens
  ([ADR 0090](0090-agents-prove-who-they-are-with-tokens-their-workspaces-issuer-signs.md)),
  or their runner's owner turns the setting on.
- Docker sandboxes have no raw sockets: `ping` still works, through the
  ICMP sockets Docker gives containers, but `tcpdump` and the like don't.
- Sessions in Docker sandboxes from before this start their next run in a
  new sandbox, once, without their conversation or what their agent left
  uncommitted (it pushed after each turn).
- A runner whose Docker refuses the helper starts no session until its
  owner turns `allow_cloud_metadata` on, which its sessions' failures say.
- On a cloud whose nameserver is the metadata address (Google Cloud's),
  sandboxes on Docker's default network, where Docker leaves them only
  when it can't make fctl's own, can't resolve names: routes can't let
  port 53 through. Those on fctl's network ask through Docker's own
  resolver, from the host, and Firecracker's VMs through the filter's
  exception for DNS.
- Each Docker sandbox start takes a few more `docker` commands, about a
  second.
- Image builds (`allowed_builds`) aren't covered; hosts build only the
  repositories their owners list.
