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
- [x] Web app: the floor, live sessions with streaming transcripts and
      diffs, projects, settings, admin; dithered WebGL scenes
- [x] CLI for everything, with `--json`
- [x] fctl.sh: landing page, docs, installer, release downloads
- [x] `cargo xtask dev`: the whole system locally with fake-claude

## 0.2 — run it every day

- [ ] First real-Claude runs on the hyperstructure Mac host; harden the Apple
      container driver (DNS, clock, disk, caches) from what they show
- [ ] CI on GitHub Actions: checks on pull requests, deploys from `main`,
      releases from tags
- [ ] Prebuilt sandbox images (GHCR) so `fctl host image build` is a pull
- [ ] Shared caches per project (npm, cargo, uv) mounted into sandboxes
- [ ] Notifications when a session waits for you or fails (email, web push)
- [ ] Homebrew tap: `brew install hyperstructure/tap/fctl`
- [ ] Usage view: each person's Claude plan limits as sessions report them

## 0.3 — closing the loop

- [ ] GitHub webhooks: agents react to CI failures and review comments on
      their pull requests
- [ ] Scheduled and recurring sessions (nightly dependency updates, triage)
- [ ] Preview links: share a sandbox's dev server through a tunnel
- [ ] Search across transcripts
- [ ] Session templates and a shared library of project instructions

## Later

- [ ] Agents that plan and start sub-sessions, with a review step
- [ ] Network egress policies per project
- [ ] Master key rotation
- [ ] Email through Cloudflare's email service (no Resend account)
- [ ] GitHub sign-in
- [ ] Linux hosts with Firecracker microVMs
- [ ] Windows hosts through WSL 2
