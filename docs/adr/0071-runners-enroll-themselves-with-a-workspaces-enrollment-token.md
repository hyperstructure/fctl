# 0071: Runners enroll themselves with a workspace's enrollment token

2026-11-05 · accepted

## Context

Each runner authenticates with a runner token of its own (`fctl_rt_`),
which belongs to the person who registered it
([0036](0036-members-run-their-sessions-on-their-own-runners.md)): the
workspace's Runners tab makes one per machine, and `fctl host install`
makes one with the CLI's sign-in. Neither suits machines that autoscaling
creates and destroys. A token made beforehand serves one machine; a
personal access token in the image or the provisioning system is unlimited,
can't confirm it's its person where the instance asks for that
([0046](0046-registering-a-runner-takes-confirming-its-you.md)), and makes
every runner that person's: it goes on running the workspace's sessions
after they leave it, and only they replace its token.

The self-hosted control plane runs on workerd without Cloudflare
([0026](0026-the-control-plane-runs-without-cloudflare-on-workerd.md)), so
whatever identifies a runner has to work there too, with the storage it
has and nothing in front of it.

## Decision

- **Enrollment tokens** (`fctl_et_`, `enrollment_tokens`), made by a
  workspace's admins on its Runners tab or with `fctl runners
  enrollment-tokens create`: a name, an expiry (an hour to a year, or
  never), the most runners it may keep enrolled at once, whether the
  runners it enrolls wait for approval, and whether they are removed once
  offline for a while. Making one is a sensitive action
  (`Sensitive::NewEnrollmentToken`) and needs the workspace open to them
  (its two-factor requirement), as adding a runner for everyone's sessions
  does: it trusts whatever enrolls with it with everyone's code, secrets
  and tokens there. D1 keeps its SHA-256 alone. Revoking one stops
  enrollments and leaves the runners it enrolled, waiting ones too.
- **`fctl host install --enroll fctl_et_…`** (or `FCTL_ENROLL_TOKEN`) asks
  what the token enrolls in (`GET /api/v1/runner/enroll`) before anything
  else, so that an expired, revoked or full token fails first; then, once
  the driver is checked, enrolls (`POST /api/v1/runner/enroll`) with what
  it knows of the machine (hostname, OS, driver, version, size), and gets a
  runner token of its own. The enrollment token goes no further: the
  connection, uplinks and calls are a runner token's, as before. Run again
  on a machine whose runner already belongs to the workspace, it keeps that
  runner, so that boot scripts can run it every time.
- **A runner proves it is itself with a runner token of its own**, issued at
  enrollment and kept in `host-token` as every runner's is. The other
  options don't earn their cost. A key pair would need a signed challenge
  on the control connection, each session's uplink and every call a host
  makes, for no gain over a 256-bit secret of which the control plane keeps
  only a hash: nothing it stores can be replayed either way, and a machine
  taken over has the private key as it has the token. Client certificates
  are checked at Cloudflare's edge, which a self-hosted instance must not
  depend on. Approving shows where and what enrolled instead: the address,
  hostname, OS, driver and version, and the token used.
- **Enrolled runners belong to the workspace.** `runners.workspace_id`
  names it and `created_by` stays empty: its admins rename, drain, disable,
  remove them and replace their tokens (`can_manage_runner`,
  `check_replace_runner_token`), instance admins manage them as they manage
  any runner, and nobody adds one to another workspace or takes it out of
  its own but by removing it. Nobody leaving the workspace or losing a role
  changes them. They run everyone's sessions there and no other
  workspace's: `serves!` holds a runner to its own workspace.
- **A runner with no owner** runs nobody's sessions that keep to their own
  runners: `fctl_domain::schedule::runs_for` already gives a runner nobody
  owns the sessions of those who take anyone's, so **Only on my runners**
  and Claude tokens kept on runners keep their sessions off it, as off
  other people's. It keeps nobody's Claude token.
- **Approval, when the token asks for it.** A runner it enrolls waits
  (`runners.pending_until`) with no `runner_workspaces` row, so that it
  serves no workspace, and every check that gives a runner sessions,
  credentials, uplinks, files or sign-ins already refuses it. It connects
  as any runner does; FleetDO's welcome tells its host it waits
  (`FleetToHost::Welcome.pending`, which older hosts ignore), and gives it
  no work. The workspace's admins see it on the Runners tab with what it
  said of itself, get a notification in each browser that shows theirs and,
  where the instance sends email, an email for the first of a burst. They
  approve (`Sensitive::ApproveRunner`) or reject it in the app or with
  `fctl runners approve|reject`. Approving gives it the row, and FleetDO a
  second welcome, after which it takes sessions without a restart.
  Rejecting removes it. One nobody approves within a week is removed.
- **Ephemeral runners don't pile up.** `fctl host uninstall` removes an
  enrolled runner (`DELETE /api/v1/runner`, with its own token) and the
  token file. A token can also have its runners removed once they have
  been offline for an hour to 30 days, which the hourly job does from the
  last heartbeat D1 has, never for a runner FleetDO has connected. The
  limit counts the runners a token enrolled that are still there, waiting
  ones too, so removed, rejected and expired ones free their place, and
  an autoscaling group keeps replacing machines within it.
- **Removing a runner stops its sessions at once** (FleetDO's
  `/runner-removed`): its connection and uplinks close, the sessions it
  ran stop and resume elsewhere, FleetDO forgets it, and feeds drop it
  (`FeedServer::RunnerRemoved`). Before, a removed runner's sessions
  waited ten minutes, as an offline runner's do.

## Consequences

- Machines that autoscaling makes enroll with one secret in the image or
  launch template, and nobody signs in on them; a leaked token enrolls
  machines until it expires, is revoked or is full, or, with approval, only
  machines an admin lets in.
- An enrolled runner keeps its token as any runner does, in a file only its
  user reads; whoever takes that file has the runner.
- The workspace that owns a runner takes it along when it is deleted.
- Hosts one version behind run with their runner tokens as before; one
  that enrolls (only a host of this version can) and waits for approval
  stays connected, idle, until it is approved. Older CLIs and web apps see
  runners as before, with the new fields left out.
- The audit log records enrollments, approvals, rejections, removals and
  the tokens made and revoked, in the workspace's log.
