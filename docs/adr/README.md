# Architecture decision records

Short records of decisions that shape fctl, so nobody has to rediscover why.
One file per decision, numbered, never rewritten: a later record can
supersede an earlier one. Copy the newest record to start a new one.

| # | Decision |
|---|---|
| [0001](0001-control-plane-on-cloudflare.md) | The control plane is a Cloudflare Worker with Durable Objects, written in Rust |
| [0002](0002-subscription-only.md) | Agents only ever run on Claude subscriptions |
| [0003](0003-hosts-dial-out.md) | Hosts dial out, and every session gets its own VM |
| [0004](0004-events-are-journaled.md) | Agent events are journaled in the sandbox and acknowledged |
| [0005](0005-three-repositories.md) | fctl, fctl-site and design are separate repositories |
| [0006](0006-conventional-commits-and-workflow-releases.md) | Conventional Commits, and releases from a workflow |
| [0007](0007-agents-name-their-sessions-through-fctl-agent.md) | Agents name their sessions through fctl-agent |
| [0008](0008-attachments-in-a-durable-object-per-project.md) | Attachments live in a Durable Object per project |
| [0009](0009-projects-span-an-accounts-repositories.md) | Projects span the repositories of one GitHub account |
| [0010](0010-notifications-are-web-push-from-a-durable-object-per-person.md) | Notifications are Web Push, sent from a Durable Object per person |
| [0011](0011-commits-credit-people-with-an-address-they-choose.md) | Commits credit people with an address they choose |
| [0012](0012-the-sandbox-image-lives-in-its-own-public-repository.md) | The sandbox image lives in its own public repository |
| [0013](0013-idle-sessions-stop-and-a-message-resumes-them.md) | Idle sessions stop, and a message resumes them |
| [0014](0014-agents-publish-screenshots-at-unguessable-links.md) | Agents publish screenshots at unguessable links |
| [0015](0015-runners-drain-and-their-sessions-carry-on.md) | Runners drain, and their sessions carry on |
| [0016](0016-uninstalling-a-host-stops-its-sessions-first.md) | Uninstalling a host stops its sessions first |
| [0017](0017-hosts-choose-the-images-they-run-and-build.md) | Hosts choose the images they run and build |
| [0018](0018-background-work-keeps-a-session-up-for-an-hour.md) | Background work keeps a session up, for an hour at most |
| [0019](0019-hosts-make-room-when-their-disk-runs-low.md) | Hosts make room when their disk runs low |
| [0020](0020-sessions-carry-on-when-their-runner-comes-back.md) | Sessions carry on when their runner comes back |
| [0021](0021-rulesets-protect-main-and-release-tags.md) | Rulesets protect main and the release tags |
| [0022](0022-two-factor-sign-in-with-authenticator-apps.md) | Two-factor sign-in with authenticator apps, which admins can require |
| [0023](0023-passkeys-sign-in-on-their-own.md) | Passkeys sign in on their own, and count as a second factor where it is required |
| [0024](0024-workspaces-hold-people-projects-runners-and-github-accounts.md) | Workspaces hold people, projects, runners and GitHub accounts |
| [0025](0025-hosts-trim-apple-sandboxes-as-they-stop-them.md) | Hosts trim Apple sandboxes as they stop them |
| [0026](0026-the-control-plane-runs-without-cloudflare-on-workerd.md) | The control plane also runs without Cloudflare, on workerd |
| [0027](0027-agents-report-their-repositories-while-claude-works.md) | Agents report their repositories while Claude works |
| [0028](0028-app-fctl-sh-takes-early-access-requests.md) | app.fctl.sh takes requests for early access |
| [0029](0029-workspaces-connect-github-accounts-with-a-github-sign-in.md) | Workspaces connect GitHub accounts with a GitHub sign-in, not an instance admin |
| [0030](0030-sessions-follow-every-pull-request-their-agent-opens.md) | Sessions follow every pull request their agent opens |
| [0031](0031-admins-choose-a-github-accounts-repositories-after-signing-in.md) | Admins choose a GitHub account's repositories after signing in, and installing the app comes back to the workspace |
| [0032](0032-githubs-webhook-keeps-sessions-pull-requests-current.md) | GitHub's webhook keeps sessions' pull requests current, with their checks |
| [0033](0033-the-control-plane-looks-pull-requests-up-50-at-a-time.md) | The control plane looks pull requests up 50 at a time |
| [0034](0034-agents-keep-a-project-memory-and-record-retrospectives.md) | Agents keep a project memory and record retrospectives |
| [0035](0035-only-a-workspaces-admins-create-its-projects.md) | Only a workspace's admins create its projects |
| [0036](0036-members-run-their-sessions-on-their-own-runners.md) | Members run their sessions on their own runners |
| [0037](0037-access-tokens-and-sessions-limit-what-agents-do-on-github.md) | Access tokens and sessions limit what agents do on GitHub |
| [0038](0038-agents-keep-one-retrospective-for-each-piece-of-work.md) | Agents keep one retrospective for each piece of work, and update it |
| [0039](0039-sensitive-actions-take-a-passkey-or-a-code.md) | Sensitive actions take a passkey or a code, where the primary instance owner asks for it |
| [0040](0040-review-agents-review-pull-requests-on-an-owners-subscription.md) | Review agents review pull requests on an owner's subscription, and fctl posts for them |
| [0041](0041-review-agents-findings-go-to-the-agent-that-opened-the-pull-request.md) | Review agents' findings go to the agent that opened the pull request |
| [0042](0042-lookups-github-gives-up-on-ask-about-fewer-pull-requests.md) | Lookups GitHub gives up on ask about fewer pull requests at once |
