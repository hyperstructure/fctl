# 0097: Hosts restart Apple's `container` when its sandboxes lose the network

2026-11-23 · accepted

## Context

On 2026-11-23 sessions on a Mac on the apple driver failed one after
another as they started, `cloning … failed: fatal: unable to access
'https://github.com/…': Could not resolve host: github.com`, while
running sessions' Claude got `Can't reach the API server … (EAI_AGAIN)`.
An autopilot whose sessions failed so paused. The Mac's own network
worked. `container system stop` and `container system start` brought the
sandboxes' network back, as they had a few times before.

Apple's `container` (1.7.1, read at its source):

- It gives each container its network's gateway as nameserver, unless
  told otherwise (`RuntimeService.getDefaultNameservers`): `192.168.64.1`
  on the default network. Its API server's own nameservers listen on
  `127.0.0.1`, for the names the Mac looks up. The gateway's answers, and
  the containers' traffic out, are macOS's vmnet's, in shared (NAT) mode,
  on the network `container-network-vmnet` makes with
  `vmnet_network_create` as it starts.
- That network can stop carrying lookups or traffic for every container
  while the Mac's own network works on, and doesn't come back by itself:
  when a network's bridge went with its last container and came back
  without its way out
  ([apple/container#2051](https://github.com/apple/container/issues/2051)),
  after the Mac's network changed, or a VPN or an exit node came or went
  ([#1881](https://github.com/apple/container/issues/1881),
  [#1986](https://github.com/apple/container/issues/1986)).
  Restarting Apple's `container` makes the network again: `container
  system stop` stops every container, giving each 5 seconds and waiting
  up to 20 more, then unloads the API server and its plugins, the network
  helper among them; `container system start` loads them again, and the
  helper makes a new network. Containers' files stay. Where the Mac's own
  routes or packet filter keep containers off the network, restarting
  Apple's `container` doesn't help, and in some reports only restarting
  the Mac did.

## Decision

- **The host asks sandboxes whether they reach the network**: before
  fctl-agent starts in a sandbox, and every minute while sessions'
  sandboxes run, `fctl-agent reach github.com:443` in the sandbox looks
  GitHub up as the sandbox's programs do and opens a TCP connection to
  it, within 10 seconds, and prints what it found
  (`fctl_domain::reach`). An fctl-agent from before `reach` says nothing,
  and the sandbox is taken as it is.
- **It asks a fresh sandbox, and the Mac**: a sandbox whose root changed
  its `/etc/hosts`, say, keeps itself off the network, so the host asks a
  fresh, short-lived sandbox too, from the session's image or the host's,
  which runs only the host's own fctl-agent, mounted read-only; twice, 5
  seconds apart, as a network whose bridge has just come may carry
  nothing for a moment. A sandbox that keeps itself off is left as it is,
  its session's log says so, and the host doesn't ask it again for half
  an hour. When the Mac doesn't reach GitHub either, restarting Apple's
  `container` wouldn't help, and the host leaves it.
- **When fresh sandboxes don't reach GitHub and the Mac does, the host
  restarts Apple's `container`**: `container system stop`, `container
  system start`, then `container start` for each container that ran,
  fctl's sandboxes and any other. What ran in them is gone, as after the
  Mac restarts: a session at work has its turn cut off, and fctl-agent
  carries it on. What starts a sandbox, or fctl-agent in one, waits for
  the restart. The host's log says so, and the log of the session whose
  start found it.
- **At most once every 30 minutes**: a restart stops every sandbox on
  the Mac, and one that didn't bring the network back may not the next
  time either. Until a fresh sandbox reaches GitHub again, the apple
  driver isn't ready, saying why: sessions wait for a runner rather than
  fail, and one whose start found it goes back to the queue. The host
  asks a fresh sandbox every minute meanwhile, and restarts Apple's
  `container` again once 30 minutes have passed.
- **Not other nameservers** (`container run --dns`, or `[container.dns]`
  in Apple's `config.toml`): they don't help where the network carries
  nothing, and nameservers chosen on one network (a public one, the
  Mac's at home) may not answer on the next.

## Consequences

- Sandboxes that lost the network get it back within a minute or two of
  a session starting, or of the host's next look while sessions run,
  without anyone restarting Apple's `container` by hand; the session that
  found it starts rather than fail.
- A restart costs every running sandbox what ran in it, and stops and
  starts again the Mac owner's own containers too.
- A network that a restart doesn't bring back (a VPN that keeps
  containers off it, a program that holds port 53) holds the runner's
  sessions until it is back, rather than fail them; the Runners tab,
  `fctl runners` and `fctl host status` say why.
- Each session's start on the apple driver runs one more `container
  exec`, and each minute while sessions run one more; a fresh sandbox,
  which boots a VM, runs only when a sandbox doesn't reach the network.
- Docker, Lima and Firecracker don't change.
