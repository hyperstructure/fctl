# 0072: Members enroll runners for their own sessions

2026-11-05 · accepted

## Context

Enrollment tokens ([0071](0071-runners-enroll-themselves-with-a-workspaces-enrollment-token.md))
let machines that autoscaling creates enroll as a workspace's runners
without anyone signing in on them. Those runners belong to the workspace,
run everyone's sessions there, and nobody owns them, so a session whose
person keeps their sessions on their own runners (**Only on my runners**,
[0036](0036-members-run-their-sessions-on-their-own-runners.md)) or keeps
their Claude token on them
([0050](0050-claude-tokens-can-stay-on-their-owners-runners.md)) never runs
on them. Only admins made the tokens.

People who run their own runners, say in their own Kubernetes cluster for
their own private projects, want the same autoscaling for runners that are
theirs: so that their sessions stay on machines they control and their
Claude token stays off the control plane. A person's runner could only be
registered signed in, or with a runner token made beforehand for one
machine.

## Decision

- **An enrollment token for its maker's own sessions**
  (`enrollment_tokens.personal`; `personal` in the API, **Only my
  sessions** on the Runners tab, `fctl runners enrollment-tokens create
  --personal`). Any member of a workspace makes one; an admin chooses
  between everyone's sessions and their own. As with adding a runner, a
  member's token is always for their own sessions, and absent a choice an
  admin's is for everyone's (`fctl_domain::workspace::enrollment_for_own`).
  Making one is a sensitive action and needs the workspace open to them, as
  a workspace token does.
- **The runners it enrolls are its maker's.** `runners.created_by` is the
  maker and `runner_workspaces.personal` is set, so that
  `fctl_domain::schedule::runs_for` gives them only the maker's sessions,
  and they count as the maker's runners for **Only on my runners** and for
  a Claude token kept on runners, with no change to scheduling. They keep
  `runners.workspace_id`, which now means the workspace a runner enrolled
  in: `serves!` holds them to it, nobody adds them to another workspace or
  changes whose sessions they run, and deleting the workspace removes them.
  Everything else enrollment does applies as before: their own runner
  tokens, the limit, offline removal, and removal at `fctl host uninstall`.
- **Its maker approves them.** Where the token asks for approval, the
  runner waits for its owner, not the workspace's admins: only they see it
  waiting, approve or reject it, and get the notification and email, and
  the host's welcome names them (`PendingApproval.owner`). The workspace's
  wait reason for runners that admins approve leaves these out.
- **Its maker manages them**, as their other runners: rename, drain,
  disable, replace the token, remove. The workspace's admins see them as
  any member's runner and take them out of the workspace, which removes
  them, since they serve no other. Their maker and admins see the token,
  and either revokes it; other members see their own alone.
- **They go with their maker.** A token for one's own sessions enrolls only
  while its maker is an active member of its workspace (checked in the
  statement that inserts the runner). When they leave, or their account is
  deleted, the runners it enrolled there are removed and their tokens
  revoked.
- **The Claude token reaches such machines from a secret store.** `fctl
  host claude set-token` reads it from stdin, so a boot script or a
  container's entrypoint keeps it on the machine without fctl seeing it
  (`fctl host claude set-token < /run/secrets/claude-token`). The app says
  so when a person who keeps their token on their runners makes a token.

## Consequences

- A person runs an autoscaled fleet of their own with one secret, as a
  workspace does, and their sessions and Claude token stay on it.
- `runners.workspace_id` no longer implies that nobody owns a runner: code
  that asks who manages a runner checks `created_by` first. `Runner.workspace`
  in the API is set for both kinds; `created_by` tells them apart. CLIs and
  web apps from before show a member's enrolled runner as the workspace's.
- A host from before reads its welcome without `owner`, and says it waits
  for an admin. A CLI from before that enrolls with such a token says the
  runner runs everyone's sessions, though it runs only its owner's.
- A member who leaves loses the runners they enrolled there, which would
  otherwise serve nothing; they enroll again with a new token if they come
  back.
