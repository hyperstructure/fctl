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
