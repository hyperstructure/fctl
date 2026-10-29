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
| [0043](0043-agents-come-back-to-their-pull-requests-for-comments-failed-checks-and-merges.md) | Agents come back to their pull requests for comments, failed checks and merges |
| [0044](0044-schedules-start-sessions-on-their-owners-subscription.md) | Schedules start sessions on their owner's subscription, and other tools run them when asked |
| [0045](0045-merge-conflicts-bring-agents-back-too.md) | Merge conflicts bring agents back too |
| [0046](0046-registering-a-runner-takes-confirming-its-you.md) | Registering a runner, or replacing its token, takes confirming it's you |
| [0047](0047-review-agents-review-outsiders-pull-requests-when-asked.md) | Review agents review outsiders' pull requests when someone asks |
| [0048](0048-hosts-keep-package-caches-for-projects-that-share-them.md) | Hosts keep package caches for projects that share them |
| [0049](0049-agents-write-balanced-pull-request-descriptions.md) | Agents write balanced pull request descriptions, unless their project chooses otherwise |
| [0050](0050-claude-tokens-can-stay-on-their-owners-runners.md) | Claude tokens can stay on their owner's runners |
| [0051](0051-new-projects-start-with-follow-ups-on.md) | New projects start with follow-ups on |
| [0052](0052-projects-choose-a-ticket-tracker-and-linear-delegates-tickets-to-fctl.md) | Projects choose a ticket tracker, and Linear delegates tickets to fctl |
| [0053](0053-hosts-update-themselves-to-the-control-planes-version.md) | Hosts update themselves to the control plane's version |
| [0054](0054-review-agents-review-dependabots-pull-requests-by-themselves.md) | Review agents review Dependabot's pull requests by themselves, and run none of them |
| [0055](0055-maintainers-start-sessions-that-reach-github-issues-beyond-the-tracker.md) | Maintainers start sessions that reach GitHub issues beyond the tracker |
| [0056](0056-sessions-are-searched-in-an-index-per-workspace.md) | Sessions are searched in an index per workspace, in a Durable Object |
| [0057](0057-projects-choose-which-apps-pull-requests-start-reviews.md) | Projects choose which apps' pull requests start reviews by themselves |
| [0058](0058-the-primary-instance-owner-can-be-in-any-number-of-workspaces.md) | The primary instance owner can be in any number of workspaces |
| [0059](0059-sessions-keep-the-images-their-agents-read-for-a-week.md) | Sessions keep the images their agents read for a week |
| [0060](0060-agents-follow-rules-for-pull-requests-and-issues-that-projects-can-change.md) | Agents follow rules for pull requests and issues that projects can change |
| [0061](0061-taken-and-reserved-slugs-get-a-random-ending.md) | Taken and reserved slugs get a random ending, and instance admins reserve slugs |
| [0062](0062-projects-choose-which-of-their-repositories-review-agents-review.md) | Projects choose which of their repositories review agents review |
| [0063](0063-fctl-moves-linear-tickets-through-their-teams-workflow-as-sessions-work.md) | fctl moves Linear tickets through their team's workflow as sessions work |
| [0064](0064-agents-call-people-what-they-choose-or-all-their-first-names.md) | Agents call people what they choose, or all their first names |
| [0065](0065-projects-take-tickets-from-several-linear-teams.md) | Projects take tickets from several Linear teams |
| [0066](0066-review-agents-post-as-a-github-app-of-their-own.md) | Review agents post as a GitHub App of their own |
| [0067](0067-review-agents-sum-pull-requests-up-in-their-descriptions.md) | Review agents sum pull requests up in their descriptions |
