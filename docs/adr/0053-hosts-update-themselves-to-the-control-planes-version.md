# 0053: Hosts update themselves to the control plane's version

2026-10-23 · accepted

## Context

A host ran the fctl its operator installed until they ran `fctl self
update` on it. The agents it puts in sandboxes and its default sandbox
image follow the host's version, so a runner nobody updated kept running
older agents in an older image. The Agent Floor said which of someone's
runners were behind and showed the command, but each release still took a
person at every machine.

Hosts and agents may run a version behind the control plane, not ahead of
it ([Protocols](../architecture.md#protocols)). On app.fctl.sh the control
plane runs the latest release, since each release redeploys it; a control
plane of one's own runs whatever version its operator deployed.

The control plane can run a release before its archives can be
downloaded: CI deploys the release commit to app.fctl.sh while the Release
workflow still builds the archives and publishes them to fctl.sh, which
takes some ten minutes, and either can finish first.

Some operators want nothing on their machines to update itself.

## Decision

- **Hosts update themselves, unless their `host.toml` says not to.**
  `auto_update` is on unless set to `false`, also in `host.toml` files from
  before it: `fctl host auto-update off`, or `fctl host install
  --auto-update off`, turns it off, and `fctl self update` updates such a
  host as before.
- **To the version the control plane runs.** FleetDO's welcome says it
  (`FleetToHost::Welcome.version`) each time a host connects, which hosts
  do again after every deploy. A host updates only to a newer version than
  its own: never back, and never past its control plane. Older control
  planes send none, and their hosts stay as they are.
- **From the host's download server, never from where the control plane
  says.** The release comes from `download_base` (fctl.sh/releases unless
  `host.toml` says otherwise), and the control plane names only the
  version, three numbers. The host checks the archive as `fctl self update`
  does, with the same code (`crates/fctl-host/src/update.rs`): its SHA-256,
  its files, and that the new fctl runs on the machine and says the
  version.
- **Until it can be downloaded.** While the release is missing, or the
  server or the network fails, the host tries again every minute for an
  hour, then every half hour. A download that is no use waits ten minutes,
  twice as long each time after, up to 12 hours
  (`fctl_domain::updates::retry_in`). Hosts hear of a release together, so
  each waits up to half a minute before its first try, and a tenth either
  way between tries.
- **When restarting interrupts nothing.** The host installs the release,
  replacing fctl and the Linux agent next to it, once no sandbox is being
  made, stopped or removed there, no session waits to start, no package
  caches are being measured or emptied and no Claude sign-in runs. As the
  host service's process it then exits, and launchd or systemd, which keep
  it running, start the new fctl; run otherwise, it runs the new fctl in
  its own place. Sessions carry on in their sandboxes, as over any restart.
- **Not where Homebrew installed fctl** (Homebrew owns those files: `brew
  upgrade fctl`), nor where the host can't write to fctl's directory.
  Hosts say in their hello whether they update themselves
  (`HostHello.auto_update`), and the Agent Floor's notice about runners
  behind the control plane leaves out those that do.

## Consequences

- Runners run a release within minutes of its archives' publication, and
  their new sessions get its agent and, once it is published within the
  hour, its sandbox image (`latest` until then). The Agent Floor's notice
  now concerns runners whose operators turned updates off, Homebrew's, and
  hosts from before this.
- An update restarts the host: sessions keep running, and their uplinks
  and the control connection reconnect within seconds, as after `fctl self
  update`.
- Whoever leaves updates on trusts their download server with what runs on
  their machines, as they did each time they ran `fctl self update`, but
  no longer chooses when: whatever fctl.sh publishes as a release reaches
  their runners within minutes. The checksum comes from the same server, so
  it catches broken downloads, not a server taken over. Operators who want
  to look at a release first turn updates off; signing releases would let
  hosts check where one comes from.
- A control plane taken over can't choose what its hosts install: only a
  newer version that their download server has. One that runs a version
  the server doesn't have, such as a build of its own, leaves its hosts
  trying every half hour; their logs say so.
- A host that updates itself in the foreground, in its own place, leaves
  what it had started (its links to agents) unreaped until it exits.
