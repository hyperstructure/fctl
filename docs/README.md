# fctl documentation

fctl is a software factory for Claude Code. You describe work; Claude agents
do it in sandboxes on machines you own, around the clock, on your Claude
subscription. You follow along live from the web app, your terminal or the
Claude apps, and anyone you share a project with can steer the agents too.

Three pieces make up a factory:

- **The control plane** — a Worker with Durable Objects and D1
  that serves the web app and the API, keeps every session's event log, and
  hands work to hosts. One per team (yours might be `app.fctl.sh`). It runs
  on Cloudflare, or in one container on any machine or Kubernetes cluster.
- **Hosts** — your machines (a spare Mac is ideal). `fctl host` runs as a
  service, connects out to the control plane, and runs each session in its
  own Linux VM.
- **Clients** — the web app, the `fctl` CLI, and the Claude apps on your
  phone and desktop through Remote Control.

## Where to go next

| If you want to… | Read |
|---|---|
| Start using fctl on an instance someone set up | [Getting started](getting-started.md) |
| Drive everything from a terminal or a script | [The CLI](cli.md) |
| Run your own instance, on a machine of yours or on Cloudflare | [Self-hosting](self-hosting.md) |
| Know what sessions run in, give a project its own image, or choose the images a host runs | [Sandbox images](sandbox-images.md) |
| Understand how the pieces talk to each other | [Architecture](architecture.md) |
| Know what fctl does with your credentials | [Security](security.md) |
| Work on fctl itself | [Development](development.md) |

## Words used in these docs

- **Project** — a repository plus what agents need to know to work in it
  (setup script, instructions, model, secrets). Projects have members.
- **Session** — one agent working on one task: one Claude conversation in
  one sandbox, on its own branch. Sessions are numbered per project
  (`my-app#12`).
- **Runner** — a host registered with the control plane.
- **Home page** — where the web app opens: the box for your next task, and
  every active session as a thread of light.
