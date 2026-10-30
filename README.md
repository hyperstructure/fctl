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
  <a href="https://fctl.sh/features/">Features</a> ·
  <a href="https://fctl.sh/docs/">Docs</a> ·
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

## Features

### [Runners](https://fctl.sh/features/runners/)

A runner is a machine someone on the team connected with one command: the
Mac mini in the cupboard, the Linux box under the desk, your own laptop.
`fctl host install` registers it and starts a service that survives a
reboot and updates itself. It takes the sessions of the workspaces it
serves, so your code is built and tested on hardware you own.

### [Sandboxes](https://fctl.sh/features/sandboxes/)

Each session gets a fresh Linux sandbox of its own: a lightweight VM on a
Mac, a Firecracker microVM or a container on Linux. fctl clones the
repository, runs the project's setup script and lets Claude Code work
without stopping to ask for permission. Start from fctl's image, one from a
registry, or a Dockerfile in the repository.

### [The CLI](https://fctl.sh/features/cli/)

Everything the app does, the CLI does too: start a session and watch it
stream, steer it, search every transcript, manage projects, runners and
schedules. Add `--json` to any command, for scripts and for other agents
that hand work to fctl.

### [Workspaces and projects](https://fctl.sh/features/workspaces/)

A workspace is the people who work together, with their projects, runners
and GitHub accounts; roles go from viewer to owner. A project is a
repository and everything its agents need: the model they start with,
instructions, secrets, and the most they may do on GitHub.

### [Review agents](https://fctl.sh/features/review-agents/)

Turn them on, and an agent reviews each new commit of every pull request in
a project's repositories, whoever wrote it, with findings on the lines they
are about. Findings on an fctl agent's pull request go back to that agent,
which fixes what's real and answers in the thread.

### [Linear](https://fctl.sh/features/linear/)

Delegate a Linear issue to fctl, or mention `@fctl` in a comment, and a
session starts in the project that takes the issue's team, as you, on your
Claude plan. The agent answers in the ticket's thread, and the pull request
comes back linked. Projects can take GitHub issues the same way.

### [Memory and retrospectives](https://fctl.sh/features/memory/)

Agents leave notes for the agents after them: the command that works, the
trap they fell into, how the tests really behave. Every session starts with
the project's notes, and when an agent finishes a piece of work it records
how it went, for the people who follow along.

### [Schedules](https://fctl.sh/features/schedules/)

Give an agent a task and a time, once or again and again: dependency
updates every Monday, triage every morning, benchmarks every night. Each
run is a session of its own, on the subscription of whoever set it up, and
other tools can start one with an access token.

Read about [more features](https://fctl.sh/features/) on fctl.sh, and
everything else in [the documentation](https://fctl.sh/docs/), whose
source is in [docs/](docs/).

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
