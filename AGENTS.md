# AGENTS.md

You are working on **fctl**, a software factory: a control plane on Cloudflare
(Rust compiled to a wasm Worker, Durable Objects, D1), a host daemon that runs
Claude Code agents in sandboxes on people's machines, a CLI, and a web app.
Read [docs/architecture.md](docs/architecture.md) before changing anything
that crosses a component boundary.

## Commands

```sh
cargo xtask dev            # the whole system locally; agents run fake-claude
cargo xtask check          # everything CI checks; run it before you push
cargo xtask check --quick  # the same without production builds
cargo xtask gen            # regenerate web/src/gen after changing crates/fctl-proto
cargo test -p fctl-domain  # one crate's tests
cd web && npm run check    # Biome + TypeScript for the web app
```

`cargo xtask dev` signs you in as `dev@fctl.test` / `fctl-dev-password` at
http://localhost:5173 and writes a CLI token to `.fctl-dev/token`
(`export FCTL_SERVER=http://127.0.0.1:8787 FCTL_TOKEN=$(cat .fctl-dev/token)`).
fake-claude's behaviors are listed in [docs/development.md](docs/development.md).

## Rules that are not negotiable

- **Never commit or print secrets**: keys, tokens, `.dev.vars`, `*.pem`,
  Cloudflare credentials. `.gitignore` covers the usual files; check
  `git status` before committing anyway.
- **Subscriptions only.** Agents must never use Anthropic API billing. Do not
  add code paths that pass `ANTHROPIC_API_KEY` (or any `ANTHROPIC_*`
  variable, `apiKeyHelper`, Bedrock/Vertex/Foundry switches) to Claude, and
  keep the guards in `crates/fctl-agent` and `crates/fctl-domain/src/events.rs`
  (billing source check, overage pause) intact.
- **Protocols stay compatible.** Hosts and agents may run one version behind
  the control plane. Add fields as optional; never rename or repurpose a
  serde tag. Change `crates/fctl-proto` first, then `cargo xtask gen`.
- **The control plane never panics.** No `unwrap`/`expect`/`panic!` in
  `crates/fctl-control` (Clippy denies them). Durable Object SQL must succeed
  on an already-migrated schema (`IF NOT EXISTS`, `add_column`).
- **Shipped migrations are immutable.** Add a new file in
  `crates/fctl-control/migrations`.

## Conventions

- Rust 2024, the toolchain in `rust-toolchain.toml`, `cargo fmt` (max width
  100). `anyhow` in binaries, typed errors at API boundaries
  (`fctl_proto::error::ApiError`), `tracing` for logs. Pure logic goes in
  `fctl-domain` with unit tests; I/O stays at the edges.
- The web app: React 19, TanStack Router (routes in `web/src/router.tsx`) and
  Query (keys and hooks in `web/src/lib/queries.ts`), Tailwind 4 with the
  tokens from `@hyperstructure/design`, Biome formatting. Live session state
  goes through `web/src/lib/live.ts`; do not open extra WebSockets.
- Design: the brand palette is yellow `#ffff5d` (lime `#eaff81` on light
  backgrounds), hot pink `#ff00d8`, blue `#007eff`, blackish `#171717`,
  whiteish `#fafafa`; use the semantic tokens, not raw colors. Inter (with
  optical sizes) for text and display, JetBrains Mono for code and small
  labels. Sentence case everywhere, no decorative uppercase eyebrows above
  headings, no card grids for prose. Motion is quick and purposeful and
  respects `prefers-reduced-motion`. The tokens, fonts, WebGL scenes and the
  mark live in [hyperstructure/design](https://github.com/hyperstructure/design)
  (`@hyperstructure/design`, pinned to a tag in `web/package.json`): change
  them there, tag a release, then bump the tag here.
- Copy is plain and specific: say what happens ("The agent reads your message
  at its next step"), not what is possible.
- Every capability is reachable from the CLI with `--json`. When you add an
  API endpoint, add the client method in `crates/fctl-client` and, if people
  need it, a command in `crates/fctl`.

## Verifying your change

- Logic: unit tests next to the code (`#[cfg(test)]` in Rust, `*.test.ts` in
  the web app).
- End to end: `cargo xtask dev`, then drive it with the CLI (`fctl run`,
  `fctl attach`, `fctl logs --json`) or the browser. fake-claude makes runs
  deterministic and free.
- UI: look at it in both themes (Settings → Appearance) and at a phone width.

## Other repositories

- [hyperstructure/design](https://github.com/hyperstructure/design): the
  design system the web app imports.
- [hyperstructure/fctl-site](https://github.com/hyperstructure/fctl-site):
  fctl.sh. It renders this repository's `docs/` at a pinned commit; CI here
  pins it when `docs/` changes on `main`, so write docs in this repository.
  `cargo xtask publish` writes releases to the KV namespace in
  `deploy/releases.toml`, which the site's Worker serves.

Both are private for now: `npm ci` reaches the design package over git, so
it needs GitHub access (CI uses a read-only deploy key).

## Git

Small, focused commits with an imperative subject line ("Add session
resume to the CLI"). Update docs in the same change when behavior changes.
Record significant decisions in `docs/adr/` (copy the newest one's format).
The roadmap is in [ROADMAP.md](ROADMAP.md) and GitHub issues.
