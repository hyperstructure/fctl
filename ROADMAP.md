# Roadmap

What fctl does today and what comes next. Larger items have GitHub issues;
this file is the overview. Order within a section is priority.

## 0.1 — the factory runs (done)

- [x] Control plane on Cloudflare: Rust → wasm Worker, a Durable Object per
      session (event log, viewers, deliveries) and one for the fleet
      (runners, scheduling, live feed), D1 for relational state
- [x] Email and password accounts, an allowlist, invites, password resets,
      Google sign-in and linking, device-flow CLI sign-in, access tokens
- [x] Projects with members and roles, encrypted secrets, a setup script and
      agent settings; GitHub App integration (link or create repositories,
      one-repository tokens for agents)
- [x] Sessions: queue, schedule, provision, run, idle, pause, resume, stop;
      branch per session, push, pull request; sharing; presence
- [x] Claude subscriptions only: long-lived tokens or a full login through a
      host, token refresh through the control plane, Remote Control for the
      Claude apps, API-key and overage guards
- [x] Hosts: Apple container, Lima, Docker and process drivers; launchd and
      systemd services; recovery of running sessions after a restart
- [x] fctl-agent: journaled, acknowledged events; at-least-once deliveries;
      crash restarts; idle timeouts
- [x] Web app: the home page, live sessions with streaming transcripts and
      diffs, projects, settings, admin; dithered WebGL scenes
- [x] CLI for everything, with `--json`
- [x] fctl.sh: landing page, docs, installer, release downloads
- [x] `cargo xtask dev`: the whole system locally with fake-claude

## Three repositories (done)

- [x] **hyperstructure/fctl** keeps the Rust crates, `web/` (the app),
      `docs/`, `xtask`, `images/` and `deploy/`.
- [x] **hyperstructure/fctl-site** serves fctl.sh: the landing page, these
      docs (rendered from `docs/` at a pinned commit; fctl's CI pins it when
      `docs/` changes), `/install` and `/releases` (from the KV namespace
      `cargo xtask publish` writes). It deploys from its own CI.
- [x] **hyperstructure/design** publishes `@hyperstructure/design`: the
      tokens, fonts, WebGL scenes, fctl's mark and hyperstructure's symbol,
      with a playground. `web/` and fctl-site pin a tag.
- [x] Make the repositories public, and drop the deploy keys that read
      design and fctl: `npm ci` downloads `@hyperstructure/design` from
      GitHub without them

## 0.2 — run it every day

- [x] First real-Claude runs on the hyperstructure Mac host with the Apple
      container driver: provisioning, a full login, host restarts, DNS,
      clock and disk (#1)
- [x] The same with a long-lived token, an OAuth refresh and a usage-limit
      pause (#79)
- [x] Hosts make room when their disk runs low, starting with stopped
      sandboxes (#76)
- [x] CI on GitHub Actions: checks on pull requests, deploys from `main`,
      releases from tags
- [x] Prebuilt sandbox images (GHCR) so `fctl host image build` is a pull (#2)
- [x] Shared caches per project (npm, cargo, uv) mounted into sandboxes (#3)
- [x] Notifications when a session waits for you or fails: web push, with
      followers, per-person settings and digests (#4)
- [ ] The same notifications by email (#49)
- [x] Homebrew tap: `brew install hyperstructure/tap/fctl` (#5)
- [x] Usage view: each person's Claude plan limits as sessions report them,
      in Settings, on the home page and in `fctl claude status` (#6)
- [x] Projects that span several repositories, or read every repository of
      an organization (#17)
- [x] macOS release binaries that run on macOS 12 and later, not only 15+ (#7)
- [x] The control plane without Cloudflare: the same Worker on workerd, in
      one container (`ghcr.io/hyperstructure/fctl-control`) for a spare
      machine, a homelab or a Kubernetes cluster
- [x] Workspaces: their own people, projects, runners, GitHub accounts,
      two-factor requirement and audit log, with one primary admin each and
      a primary owner for the instance; one account with several email
      addresses, each with its own commit email

## 0.3 — closing the loop

- [x] GitHub's webhook keeps sessions' pull requests current: their state,
      merge queues included, and their checks, in session lists and pages
- [x] Project memory: each session's agent starts with the notes earlier
      agents and people left, adds what the next one should know and marks
      wrong ones stale; people read, change and delete them
- [x] Retrospectives: agents record what they worked on, how, what got in
      the way and what a future agent should know, for each piece of work
- [x] Review agents: each new commit of a project's pull requests gets a
      review, posted in one pass with a check, on an owner's subscription
- [x] Review agents' findings go to the agent session that opened the pull
      request, which acts on them and answers in their threads
- [x] Review agents: projects choose which apps' pull requests start reviews
      by themselves (Dependabot, Renovate, their own apps)
- [x] Review agents: choose the repositories they review, and whether
      drafts get reviews
- [ ] Learning from retrospectives: a session that reads a project's and
      proposes notes and AGENTS.md changes
- [x] Follow-ups: comments that mention `@fctl` or `@agent` from people who
      can write to the repository, failed checks and merges bring agents
      back to the pull requests they opened (#8)
- [x] Follow-ups: merge conflicts with the base branch bring agents back
- [x] Scheduled and recurring sessions (nightly dependency updates, triage):
      schedules start sessions once or again and again, on their owner's
      subscription, and other tools run them when asked (#9)
- [x] Ticket trackers: each project chooses GitHub Issues, a Linear team or
      none, which its agents use with `fctl ticket`
- [x] Linear: people delegate tickets to fctl or mention it there, and the
      session reports back in the ticket's agent session
- [ ] More ticket trackers (Jira, Shortcut, ClickUp), on the same `Ticket`
      model and `fctl ticket`
- [x] Search across transcripts: ⌘K and `fctl search` find what sessions
      said, and ⌘F finds it in a session's transcript (#11)
- [ ] Session templates and a shared library of project instructions

## Later

- [ ] Agents that plan and start sub-sessions, with a review step
- [ ] Network egress policies per project
- [ ] Preview links: share a sandbox's dev server through a tunnel, when a
      real need arises
- [ ] Master key rotation
- [ ] Email through Cloudflare's email service (no Resend account)
- [x] Workspace admins connect the GitHub accounts they manage themselves,
      verified with a GitHub sign-in, without an instance admin's approval
- [ ] GitHub sign-in as a way to sign in to fctl
- [ ] Merging two accounts into one with both their addresses
- [ ] Linux hosts with Firecracker microVMs
- [ ] Windows hosts through WSL 2
