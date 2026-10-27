<p align="center">
  <img src="https://fctl.sh/favicon.svg" width="56" height="56" alt="">
</p>

<h1 align="center">fctl</h1>

<p align="center">
  <strong>Agents that build while you sleep.</strong><br>
  An open source software factory for Claude Code, on machines you own.
</p>

<p align="center">
  <a href="https://fctl.sh">fctl.sh</a> ·
  <a href="docs/getting-started.md">Getting started</a> ·
  <a href="docs/self-hosting.md">Self-hosting</a> ·
  <a href="docs/architecture.md">Architecture</a>
</p>

---

fctl runs Claude Code agents in sandboxes on your own Mac (or Linux box),
around the clock, on your Claude subscription — never on API billing. Start
work from the web app, the terminal or your phone; follow every session live;
message an agent to steer it, with screenshots and files; share projects and
sessions with your team.

```sh
curl -fsSL https://fctl.sh/install | sh
fctl login app.fctl.sh
fctl host install                        # this machine now runs agents
fctl run -p my-app "Add a dark mode toggle"
```

Prefer Homebrew? `brew install hyperstructure/tap/fctl` installs the same
release.

Pointing an agent at fctl? [llms.txt](docs/llms.txt), also at
https://fctl.sh/llms.txt, tells it what fctl is and how to drive it.

![app.fctl.sh](https://github.com/user-attachments/assets/7f3f3b3c-60e8-46df-b51a-0bde53b13971)

## How it works

```
 web app · fctl CLI · Claude apps
              │
   control plane                      a Rust → wasm Worker, Durable Objects, D1:
              │  (hosts dial out)     on Cloudflare, or in one container anywhere
         fctl host on your Mac        one Linux VM per session
              │
   fctl-agent + Claude Code           journaled events, branch, push, pull request
```

- **Subscriptions only.** API keys are stripped from every sandbox, sessions
  must report a subscription credential, and a session pauses before it would
  use paid extra usage.
- **Nothing is lost.** Agents journal every event until the control plane
  acknowledges it; links reconnect and replay; hosts recover running sessions
  after a restart.
- **Agents that learn.** Each project keeps a memory: every session's agent
  starts with the notes earlier agents left and adds what the next one
  should know. As they finish work, agents record what they did, how, and
  what got in the way, for the people who follow along.
- **Work that starts itself.** Schedules start sessions on their own, once
  or again and again: dependency updates every Monday, triage every
  morning. Teams that use Linear delegate tickets to fctl there, and other
  tools start a session when they have something for it.
- **Your tracker.** Each project chooses where its agents read and write
  tickets: GitHub Issues, Linear, or none at all.
- **Follow along anywhere.** With a full Claude login, sessions join Remote
  Control and show up in the Claude apps.
- **Open source and yours to run.** All of fctl is MIT licensed. Use the
  managed control plane at app.fctl.sh, or host your own: on your
  Cloudflare account with `cargo xtask deploy`, or without one, on a spare
  machine or a Kubernetes cluster, with
  `docker run ghcr.io/hyperstructure/fctl-control`
  ([self-hosting](docs/self-hosting.md)).

## Developing

```sh
cargo xtask dev     # control plane, web app and a host running fake-claude
cargo xtask check   # what CI checks
```

Start with [AGENTS.md](AGENTS.md) (written for coding agents, useful for
people) and [docs/development.md](docs/development.md). The plan is in
[ROADMAP.md](ROADMAP.md). The design system lives in
[hyperstructure/design](https://github.com/hyperstructure/design) and fctl.sh
in [hyperstructure/fctl-site](https://github.com/hyperstructure/fctl-site).

## License

[MIT](LICENSE)
