# Development

Everything runs on one machine, with one command, and needs no Claude
subscription, Cloudflare account or GitHub App.

## Setup

- Rust via [rustup](https://rustup.rs) — `rust-toolchain.toml` pins the
  version and adds the `wasm32-unknown-unknown` target.
- Node.js 22 or newer (`npm ci` at the root installs the web app and
  wrangler). The web app depends on
  [@hyperstructure/design](https://github.com/hyperstructure/design) through
  git, so `npm ci` needs read access to that repository while it is private.
- [worker-build](https://crates.io/crates/worker-build), which builds the
  control plane for wrangler. The first `cargo xtask dev` installs it with
  `cargo install` if it isn't on your PATH. On Linux that needs pkg-config
  and OpenSSL's headers: `sudo apt install pkg-config libssl-dev` on Debian
  and Ubuntu.
- Optional, for release builds: [zig](https://ziglang.org) and
  `cargo install cargo-zigbuild`.

## The local stack

```sh
cargo xtask dev
```

This starts:

- the control plane under `wrangler dev` at http://127.0.0.1:8787, with local
  D1 and Durable Objects (state in `crates/fctl-control/.wrangler/`);
- the web app under Vite at http://localhost:5173, with hot reload, proxying
  `/api` to the control plane;
- a host using the `process` driver whose agents run **fake-claude**: a
  scripted stand-in for Claude Code that speaks the same stream-json protocol.

The first run creates an owner (`dev@fctl.test` / `fctl-dev-password`), a
Claude credential, a runner and a project called Playground, and stores a CLI
token in `.fctl-dev/token`:

```sh
export FCTL_SERVER=http://127.0.0.1:8787 FCTL_TOKEN=$(cat .fctl-dev/token)
cargo run -p fctl -- run -p playground "write notes.md"
```

fake-claude reacts to words in the prompt: `write <name>` creates and
commits a file, `rename <title>` names the session with `fctl edit` (the
host's own fctl comes first on its agents' PATH), `slow` streams slowly,
`limit` hits a usage limit, `overage` reports paid usage (the session must
pause), `refresh` asks the host for a new OAuth token, and `crash` exits
mid-turn. Anything else gets a short answer with a tool call. A message with
attached files also gets a sentence about them: how many images it holds,
and whether each listed file is on disk. Turns other than `limit` and
`crash` report plan usage as Claude Code does (`unifiedWindows`), with
five-hour and weekly windows that follow the clock, so Settings → Claude,
the floor and `fctl claude status` show it. Its `system/init` message lists a
claude.ai connector, `claude.ai Fake Docs`, unless
`ENABLE_CLAUDEAI_MCP_SERVERS` is `false`, so sessions in a project with
claude.ai connectors turned off show none (`fctl logs --json`). Its commits
end as Claude Code's do: with Claude's trailer, or with the
`attribution.commit` fctl-agent writes into Claude's settings. To see
yours, set a commit email
(`cargo run -p fctl -- account set --commit-email 1+dev@users.noreply.github.com`);
to see Claude's go, `cargo run -p fctl -- projects set --credit-claude off`.

Notifications work locally too: `http://localhost:5173` is a secure
context, so turn them on in Settings → Notifications, follow a session and
give it a task. The dev control plane sends them through your browser's real
push service, so it needs the internet. A finished turn notifies you unless
the session's tab is visible; `limit` pauses a session.

`--no-host` skips the host; `--no-vite` serves the built web app from
wrangler instead. Delete `.fctl-dev/` and `crates/fctl-control/.wrangler/`
to start over.

The hourly housekeeping job (`crates/fctl-control/src/housekeeping.rs`:
purging deleted sessions, deleting expired sign-in records) runs on a cron
trigger. To run it locally:

```sh
curl "http://127.0.0.1:8787/cdn-cgi/handler/scheduled?cron=17+*+*+*+*"
```

## Checks

```sh
cargo xtask check          # what CI runs
cargo xtask check --quick  # without the production builds
```

That is `cargo fmt --check`, a check that `web/src/gen` matches the Rust
types, Biome and TypeScript for the web app, Clippy (native and wasm) with
warnings as errors, `cargo test`, the web app's Vitest suite, and a
production build of the web app.

## Repository map

| Path | What |
|---|---|
| `crates/fctl-proto` | Wire types for every protocol; TypeScript is generated from them |
| `crates/fctl-domain` | Pure logic: permissions, validation, tokens, naming, scheduling, event shaping |
| `crates/fctl-control` | The control plane Worker (Rust → wasm): API, auth, Durable Objects, GitHub |
| `crates/fctl-client` | HTTP and WebSocket client used by the CLI and hosts |
| `crates/fctl-host` | The host daemon: control connection, sessions, sandbox drivers, images |
| `crates/fctl-agent` | The supervisor inside each sandbox: runs Claude, journals events, prepares git |
| `crates/fctl` | The `fctl` binary: CLI plus `fctl host` |
| `crates/fake-claude` | The scripted Claude stand-in |
| `xtask` | `cargo xtask`: dev, check, gen, deploy, secrets, dist, publish |
| `web` | The web app (React, TanStack Router and Query, Tailwind, WebGL) |
| `deploy/instances` | Instance descriptions for `cargo xtask deploy` |
| `docs` | These docs; `docs/adr` records decisions |

## Changing a protocol

1. Change the types in `crates/fctl-proto` — add fields as `Option` (with
   `#[serde(default)]` where needed), never repurpose a `type` tag.
2. `cargo xtask gen` regenerates `web/src/gen`.
3. Keep older peers working: hosts and agents can lag the control plane by a
   version, and the web app talks to whatever is deployed.

## Working on the control plane

- It compiles to `wasm32-unknown-unknown` and runs on workers-rs. No
  `unwrap`, `expect` or panics (Clippy denies them): a panic kills the
  request, and in a Durable Object it can poison the instance.
- Durable Object SQL must not fail on a schema that is already migrated:
  create tables with `IF NOT EXISTS` and add columns with `add_column`,
  which checks first.
- D1 migrations are in `crates/fctl-control/migrations`; `cargo xtask dev`
  applies them locally and `cargo xtask deploy` before deploying. Never edit
  a migration that has shipped.

## Working on the web app

```sh
cd web
npm run dev        # with the control plane from `cargo xtask dev --no-vite`, or any instance via FCTL_API
npm run check      # Biome and TypeScript
npm test
```

Design tokens, fonts, the WebGL scenes, the mark and the app icons come
from [@hyperstructure/design](https://github.com/hyperstructure/design),
pinned to a tag in `web/package.json` (it has a playground showing all of
them). Use the semantic colors (`canvas`, `surface`, `ink`, `accent`,
`highlight`, `pink`, `blue`, `danger`…) rather than raw values; `danger`
is the one red, for destructive actions, errors and failures only. The
scenes render dithered to the brand palette; they pause when hidden and
render one still frame for people who prefer reduced motion.

People install the app on their phones and computers.
`public/manifest.webmanifest` describes it, the `brand-assets` plugin in
`vite.config.ts` serves the favicon and icons from the design package, and
`public/sw.js` shows `public/offline.html` when a page cannot load offline,
and shows notifications. The service worker registers by itself only in
production builds (turning notifications on registers it anywhere): to try
installing, run `cargo xtask dev --no-vite` and open http://127.0.0.1:8787
(Chrome's DevTools → Application shows the manifest and the worker). In the
installed app on a phone, the page reaches under the notch and the home
indicator, so anything fixed to an edge of the screen keeps clear of them
with `env(safe-area-inset-*)`, as the message box does.

## Other repositories

| Repository | What | How it connects |
|---|---|---|
| [hyperstructure/design](https://github.com/hyperstructure/design) | Tokens, fonts, WebGL scenes, the marks | `web/` depends on a tag; bumps arrive as pull requests |
| [hyperstructure/fctl-site](https://github.com/hyperstructure/fctl-site) | fctl.sh: landing page, these docs, `/install`, `/releases` | renders `docs/` at the commit in its `fctl-docs.ref`, which this repository's CI updates when `docs/` changes on `main`; serves the releases `cargo xtask publish` writes to the KV namespace in `deploy/releases.toml` |

## Releasing

Releases come from `main` through the **Release** workflow: Actions →
Release → Run workflow, or `gh workflow run release.yml --ref main -f
bump=auto`. It picks the version from the Conventional Commits since the
last tag, adds the notes to `CHANGELOG.md`, pushes the release commit and
tag, builds the binaries in parallel jobs (`cargo xtask dist build`: fctl
for macOS and Linux on arm64 and x86_64, and the Linux agents), makes the
archives from them (`cargo xtask dist package`), creates the GitHub release
with the notes, publishes to `https://fctl.sh/releases` (`cargo xtask
publish --latest`) and redeploys app.fctl.sh. Within the hour,
[hyperstructure/sandbox](https://github.com/hyperstructure/sandbox), where
the default sandbox image lives, publishes the release's image to
`ghcr.io/hyperstructure/sandbox`: its Image workflow looks for new releases
on fctl.sh every hour.

```sh
cargo xtask release --dry-run   # the next version and its notes
cargo xtask notes v0.1.2        # one release's notes
```

The steps and what to do when one fails are in
[.claude/skills/release/SKILL.md](../.claude/skills/release/SKILL.md);
commit and pull request conventions in
[.claude/skills/commit/SKILL.md](../.claude/skills/commit/SKILL.md).

To build locally: `cargo xtask dist` (or `--target aarch64-apple-darwin`).
It does what the Release workflow does in separate jobs: `cargo xtask dist
build <fctl|fctl-agent> --target <triple>` for each binary, into
`dist/bin/v<version>/`, then `cargo xtask dist package`.

## Deploying

`main` deploys `app.fctl.sh` from CI (fctl.sh deploys from
hyperstructure/fctl-site). By hand:

```sh
cargo xtask deploy fctl-sh
```
