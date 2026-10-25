# Development

Everything runs on one machine, with one command, and needs no Claude
subscription, Cloudflare account or GitHub App.

## Setup

- Rust via [rustup](https://rustup.rs) — `rust-toolchain.toml` pins the
  version and adds the `wasm32-unknown-unknown` target.
- Node.js 22 or newer (`npm ci` at the root installs the web app and
  wrangler). The web app depends on
  [@hyperstructure/design](https://github.com/hyperstructure/design) through
  git, at a tag, which `npm ci` downloads from GitHub without signing in.
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

The first run creates the primary instance owner (`dev@fctl.test` /
`fctl-dev-password`), a Claude credential, a workspace called Development,
a runner for it and a project called Playground, and stores a CLI token in
`.fctl-dev/token`:

```sh
export FCTL_SERVER=http://127.0.0.1:8787 FCTL_TOKEN=$(cat .fctl-dev/token)
cargo run -p fctl -- run -p playground "write notes.md"
```

To try a Claude token kept on the runner, keep a made-up one on the dev
host and connect it; the dev host's next heartbeat, within 15 seconds,
tells the control plane, and sessions on it then run there with that
token:

```sh
echo sk-ant-oat01-fctl-dev-kept-on-the-runner-0000000000 \
  | FCTL_CONFIG_DIR=.fctl-dev/host cargo run -p fctl -- host claude set-token
cargo run -p fctl -- claude connect --on-runners
```

More workspaces, and people in them, take a few commands: `cargo run -p
fctl -- workspaces create "Side project"`, then `workspaces add-runner
dev-host -w side-project` so that the dev runner takes its sessions too,
and `admin invite grace@example.com -w side-project`, whose link signs Grace
up in another browser. The dev instance has no GitHub App, so projects there
have no repository and the GitHub tabs say so. It takes requests for early
access, as app.fctl.sh does: signed out, the sign-in page links to
`/early-access`, and the requests show under Admin → Early access and in
`cargo run -p fctl -- admin early-access`.

fake-claude reacts to words in the prompt: `explore` looks around the
working directory first, as agents do before they change something (a
thought, whose tokens the session page counts as it comes, then
commands, searches and a read, two of them at once, with another thought
among them), which the session page folds into a line that opens
into its steps (`explore <n>` looks around that many times in one line,
more than the newest events the session page gets first, which then loads
earlier ones until it has a screenful to show), `write <name>` creates and
commits a file, `push` pushes the branch (dev projects have no repository,
so a bare one in the sandbox's `~/.fake-github` stands in for GitHub),
`watch <seconds>` watches the pull request's checks in one tool call that
long, as agents wait for CI before they end their turn (meanwhile the
session page shows the commit and the push), `rename <title>` names the
session with `fctl edit` (the host's own fctl comes first on its agents'
PATH), `screenshot` draws a small PNG and publishes it with `fctl publish`
(allow that first: `cargo run -p fctl -- admin set --publish-screenshots
on`, then `cargo run -p fctl -- projects set --publish-screenshots on`),
`remember <text>` adds the rest of its line to the project's memory with
`fctl memory add`, `recall` lists the memory with `fctl memory`, a line
that starts with `fctl ticket` runs as it is (`fctl ticket view ENG-1`), as
agents reach their project's tracker,
`printenv` shows the `CLAUDE_CODE_…` variables it got and says the token
back, streamed in pieces (the host redacts the Claude token in both),
`retro`
records a retrospective with `fctl retro` (completed, or `partial` or
`blocked` when the message says so; the project's Memory and
Retrospectives tabs, and the session page, show them), `revise` updates
the session's newest retrospective to completed with `fctl retro list` and
`fctl retro update`, as agents do when a later turn carries on with the
same work,
`slow` streams slowly, `background <seconds>` starts a shell command in
the background that ends after that many seconds and takes its result in
a turn of its own (the session waits for it, then runs again, as with
Claude Code), `delegate` hands a look around the sandbox to a subagent,
whose steps the session page shows under its Delegated line as they come
(`delegate in the background` runs it in the background: the turn ends,
the subagent works on, and a turn of its own takes its report, which
starts without a `running` of its own, as with Claude Code), `limit`
hits a usage limit that resets 20 seconds later (the session pauses and
continues half a minute after the reset; a message that also says
`reset`, like the one fctl-agent continues it with, doesn't count),
`overage` reports paid usage (the session must pause), `warning`
reports plan usage with Claude Code's warning that the weekly limit goes
faster than the week passes (`allowed_warning`: the apps go by the share
used instead), `refresh` asks the host for a new OAuth token,
`effort <level>` (or Claude Code's `/effort <level>`) changes the effort
level it works at (`low` … `max`, or `none` as a model that takes none),
`context <tokens>` fills its context to that many tokens before it answers
(`context 155k`, which the session page shows in amber, nine tenths of the
way to where it compacts), `compact` (or Claude Code's `/compact`)
compacts the conversation,
a review session's task (`Review acme/web#12, … at commit …`) reviews as a
review agent does: it reads the pull request's commits and diff and fctl's
earlier findings (`fctl review threads`), then posts with `fctl review
submit`, marking earlier findings still open fixed, and without any,
finding an issue on the first line the pull request adds (the dev
instance has no GitHub App, so no review starts there by itself),
fctl's message about a review of the agent's own pull request (`fctl's
review agent reviewed commit …`) gets a look at the branch's commits and
an answer that leaves each finding as it is, without calling GitHub (no
other word in it counts), so does fctl's message about what happened to
its pull request on GitHub (`On GitHub, …`: a comment, failed checks,
conflicts, a merge), with a line for each part (the dev instance has no
GitHub App, so none comes there by itself),
and `crash` exits mid-turn. Anything
else gets a short answer with a tool call. It answers `get_settings` with
its model and effort level (`--effort`, else `medium`), which the session
page and `fctl show` show. Its context grows with each call, from the 18K
tokens of its system prompt and tools; like Claude Code with a 200K
window, it compacts the conversation on its own once its context reaches
167K tokens, and it answers `get_context_usage` with that window and where
it compacts, which the Work section of the session page and `fctl show`
show with the context. A message sent during a turn
waits for it and gets a turn of its own; each `result` says how many
messages still wait (`queued_turn_count`), as Claude Code's does, so the
session stays running until fake-claude has answered the last one. Like
Claude Code, it starts each turn with `system/init`, and reports its state
(`session_state_changed`) when `CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS` is
set, as fctl-agent sets it: `running` as a turn starts and `idle` once it
has ended, but from a turn that starts a subagent in the background,
`running` until the turn that takes its report has ended, as Claude Code
2.1.308 does. A message with
attached files also gets a sentence about them: how many images it holds,
and whether each listed file is on disk. Turns other than `limit` and
`crash` report plan usage as Claude Code does (`unifiedWindows`), with
five-hour and weekly windows that follow the clock, so Settings → Claude,
the Agent Floor and `fctl claude status` show it. Each of its API calls reports
tokens as Claude Code's do, its input as it starts and with its assistant
messages, and its output in the `message_delta` that ends it, so the
session page's Work counters move during a turn. Its `system/init` message lists a
claude.ai connector, `claude.ai Fake Docs`, unless
`ENABLE_CLAUDEAI_MCP_SERVERS` is `false`, so sessions in a project with
claude.ai connectors turned off show none (`fctl logs --json`). Its commits
end as Claude Code's do: with Claude's trailer, or with the
`attribution.commit` fctl-agent writes into Claude's settings. To see
yours, set a commit email
(`cargo run -p fctl -- account set --commit-email 1+dev@users.noreply.github.com`);
to see Claude's go, `cargo run -p fctl -- projects set --credit-claude off`.

The dev instance has no Linear app either. To try Linear's side, point
the control plane at a stand-in for Linear in `crates/fctl-control/.dev.vars`
(read only as wrangler starts): `LINEAR_CLIENT_ID`, `LINEAR_CLIENT_SECRET`
and `LINEAR_WEBHOOK_SECRET` with made-up values, and `LINEAR_API_URL` and
`LINEAR_AUTHORIZE_URL` with the stand-in's address. It must answer
`POST /oauth/token` and `/oauth/revoke`, the GraphQL queries and mutations
in `crates/fctl-domain/src/linear.rs` at `POST /graphql`, and its authorize
page by sending the browser back to `/api/v1/linear/installed` with a code
and the `state` it got. Connect it on the workspace's Linear tab, choose a
team in Playground's Tickets tab and link your account in Settings; then
deliveries it signs with the webhook secret (`Linear-Signature`, the
body's HMAC-SHA256 in hex, with a `webhookTimestamp` of now) at
`/api/v1/linear/webhook` start and message sessions as Linear's do, and
what fctl posts back arrives as its GraphQL mutations.

Notifications work locally too: `http://localhost:5173` is a secure
context, so turn them on in Settings → Notifications, follow a session and
give it a task. The dev control plane sends them through your browser's real
push service, so it needs the internet. A finished turn notifies you unless
the session's tab is visible; `limit` pauses a session.

`--no-host` skips the host; `--no-vite` serves the built web app from
wrangler instead; `--smoke` runs one session and stops ([Checks](#checks)).
Delete `.fctl-dev/` and `crates/fctl-control/.wrangler/` to start over.

The hourly housekeeping job (`crates/fctl-control/src/housekeeping.rs`:
purging deleted sessions, deleting expired sign-in records, indexing
sessions for search that the index lacks) runs on a cron trigger. To run
it locally:

```sh
curl "http://127.0.0.1:8787/cdn-cgi/handler/scheduled?cron=17+*+*+*+*"
```

A session is searchable (⌘K, `fctl search`) five seconds after its
messages and replies come, when its alarm puts them in its workspace's
SearchObject. The job indexes again, from their first events, the sessions
D1 says the index lacks something of, as from before search; to see it do
so, say that of them all, then run it:

```sh
cd crates/fctl-control && npx wrangler d1 execute DB --local --command "UPDATE sessions SET search_indexed = 0"
```

Schedules (`crates/fctl-control/src/schedules.rs`) start their sessions on
ScheduleObject's alarm, which rings under `wrangler dev` too: one that runs
in a minute shows a run start on time, as fake-claude takes its task.

```sh
cargo run -p fctl -- schedules add "Check" -p playground --at 1m -m "write check.md"
cargo run -p fctl -- schedules add "Busy" -p playground --cron "*/15 * * * *" -m "watch 90"
cargo run -p fctl -- schedules run Busy -p playground   # its task now, with --context for a tool's text
cargo run -p fctl -- schedules runs Busy -p playground  # what each run started, or why it started none
```

To make a schedule due at once, set its next run in the local D1 and wake
the alarm with the hourly job (`npx wrangler d1 execute DB --local
--command "UPDATE schedules SET next_run_at = 0 WHERE name = 'Busy'"` in
`crates/fctl-control`, then the `curl` above): a run while its last session
still works (`watch 90`) is skipped. A row in `claude_usage` with a
`utilization` of 1 and a `resets_at` ahead puts the dev subscription's plan
at its limit, which skips the runs of a schedule until then. The dev
`.fctl-dev/token` is an access token, so its runs count as a tool's.

## The demo stack

```sh
cargo xtask demo                         # the demo cast, lived in, at http://localhost:5173
node xtask/demo/shoot.mjs --out shots    # from where Playwright is installed: its screenshots
```

`cargo xtask demo` runs the local stack as `cargo xtask dev` does, with
its state in `.fctl-demo/` (made afresh each time), and loads the made-up
world of [the demo skill](../.claude/skills/demo/SKILL.md) into it: the
people, workspaces, runners, projects, schedules, memory and sessions in
`xtask/demo/cast.toml`. Its people make it through the API, with a host for
each runner and a stand-in GitHub (bare repositories in `.fctl-demo/github`
that git reaches in place of github.com, holding the files in
`xtask/demo/repos`, where dependency manifests are kept as
`<name>.stand-in` so that GitHub doesn't count them as fctl's); each
session plays its transcript in
`xtask/demo/transcripts`, forty times faster than written. Then, with the
control plane stopped, it moves everything to the story's times in D1 and
the Durable Objects' SQLite, and fills in what a dev instance has no GitHub
App or Linear for: pull requests' checks, reviews, tickets, older sessions
for the projects' counts. The sessions still at work in the story work on
once it is back, and the people watching them watch. Sign in as any of
them with the cast's password, which it prints with Noor's address.

The story happens on a Tuesday at 09:10. The seed sets that moment at a
minute when some time zone's clocks read 09:10, within the hour before it
finishes, and moves the schedules to that zone and to the day of the week
it is there; `.fctl-demo/shots.json` says which. `xtask/demo/shoot.mjs`
takes the browser there, signs in as Noor and shoots the pages the skill
lists, in both themes and at a phone's width (`--only`, `--page`,
`--theme`), and `--open` opens a browser at that moment to look around or
record in. Any other browser shows the times as they are where it is. With
`--features <dir>`, it takes the screenshots of fctl.sh's feature pages
instead, as WebP, into hyperstructure/fctl-site's `src/features/shots`
([the demo skill](../.claude/skills/demo/SKILL.md) says what they need).

fake-claude plays the transcripts with `--replays <dir>`: each `*.toml`
there is one session's turns, which it streams as Claude Code would, its
tool calls with their outputs, and makes their edits, writes, commits and
pushes for real in the session's working directory. The format is in
`crates/fake-claude/src/replay.rs`. `cargo test -p xtask
the_cast_holds_together` checks that every session has a transcript that
answers its messages and changes only what its repository holds.

## The standalone control plane

Outside Cloudflare, the control plane runs as `deploy/standalone/server.mjs`
runs it: the Worker and the web app built for production, on workerd
through Miniflare, as `wrangler dev` runs them, with what Cloudflare does
around the Worker done by the server: migrations, the client's address,
the hourly job ([Self-hosting](self-hosting.md#on-your-own-machine-or-cluster),
[ADR 0026](adr/0026-the-control-plane-runs-without-cloudflare-on-workerd.md)).
Its container image is `deploy/standalone/Dockerfile`. To run it from your
checkout, without Docker:

```sh
FCTL_SETUP_TOKEN=dev-setup-token cargo xtask serve      # http://localhost:8787, state in .fctl-serve/
cargo xtask dev --smoke --server http://localhost:8787  # a host and a session through it
```

`--server` points `cargo xtask dev` at a control plane that already runs,
instead of starting `wrangler dev`: it bootstraps the dev owner there with
the dev setup token, and runs the host (and with `--smoke`, the session)
against it. Give it the server's `FCTL_PUBLIC_URL`, the origin the server
accepts requests from. Its state goes in `.fctl-dev/server/`. Delete
`.fctl-serve/` to start the server over.

The image runs the same session with Docker, as the Standalone image
workflow does ([Checks](#checks)):

```sh
docker build -f deploy/standalone/Dockerfile -t fctl-control .
docker run --rm -d --name fctl -p 127.0.0.1:8787:8787 \
  -e FCTL_PUBLIC_URL=http://127.0.0.1:8787 -e FCTL_SETUP_TOKEN=dev-setup-token fctl-control
cargo xtask dev --smoke --server http://127.0.0.1:8787
docker stop fctl
```

## Checks

```sh
cargo xtask check          # what CI runs
cargo xtask check --quick  # without the production builds
cargo xtask check test     # one part: lint, test or web
cargo xtask dev --smoke    # and CI's other check: a session through the local stack
```

That is `cargo fmt --check`, a check that `web/src/gen` matches the Rust
types, Biome and TypeScript for the web app, Biome for the standalone
server, Clippy (native and wasm) with warnings as errors, `cargo test`, the
web app's Vitest suite, the standalone server's tests (`node --test`), and
a production build of the web app. It comes in three parts: `lint`
(formatting, the generated types and Clippy), `test` (`cargo test`) and
`web` (the rest), and runs all three, in the order that fails fastest,
when you name none. CI runs each part in a job of its own, at once, and
its `cargo xtask check` check passes once all three have; a part that
fails there fails the same way locally with `cargo xtask check <part>`.

`cargo xtask dev --smoke` starts the local stack as `cargo xtask dev` does
and runs `fctl run -p playground --model haiku` with a task that has
fake-claude commit `notes.md`, push it, watch its checks for 12 seconds,
keep a note in the project's memory, record a partial retrospective and
update it, and leave a command and a subagent working in the background.
It waits until the session has shown the commit pushed before the turn
ended, the transcript has shown the note and the retrospective recorded
and updated, the turn has ended, the session has waited for the command
and the subagent and has run again while the agent took each one's result
in a turn of its own, checks that the project's
memory has the note and the session its one retrospective, completed and
updated,
then stops the session, checks the model and effort level it reports and
that the project remembers the model it asked for, and stops everything
else. It fails when any step does. CI runs it from a fresh
checkout, so its migrations, Worker, bootstrap, host and agent all run
before a change can deploy. Locally it uses your `.fctl-dev/` state and
ports 8787 and 5173: stop `cargo xtask dev` first.

`cargo xtask dev`, with `--smoke` or without, builds the CLI, the host
and the agent, the Worker (`worker-build --dev`, in `target/worker`, so
that the two cargo builds don't wait for each other), and the web app and
the local D1's migrations at once; the lines of the last two come after
`worker │` and `web │`.

CI caches the dependencies, not the workspace's own crates
([Swatinem/rust-cache](https://github.com/Swatinem/rust-cache), set up by
`.github/actions/rust` with the toolchain), and builds without debug info.
Two builds of the workspace are cached by the files they are built from
instead: xtask's binary (`.github/actions/xtask`), which the jobs run as
`xtask check lint` and the like, and on `main` the Worker's release build,
which runs beside the checks for the deploy. Only `main` saves caches;
pull requests use `main`'s.

The Standalone image workflow (`.github/workflows/standalone-image.yml`)
builds the standalone control plane's image, starts it and runs `cargo
xtask dev --smoke --server` against it, so the container, its migrations
and its proxy run a session too. The build takes several minutes, so it
runs on the pull requests that change the image, its server or the
bindings it reads (`deploy/standalone`, `.dockerignore`,
`crates/fctl-control/wrangler.toml`), and for each release before it
publishes the image; other pull requests don't run it. To check the image
of another change, run the same session through it yourself
([The standalone control plane](#the-standalone-control-plane)).

Pull requests that change the agent skills in `.claude/skills` also run
the Skills workflow, which validates them all in one job against the
[Agent Skills specification](https://agentskills.io/specification), with
[agent-skills-validator](https://github.com/coolapso/agent-skills-validator).

Dependabot opens security updates, and no version updates
(`.github/dependabot.yml`). It opens them from the root, where
`package-lock.json` and `Cargo.lock` are, and titles them as Conventional
Commits (`build(deps): bump …`, `ci(deps): bump …` for the workflows'
actions), so they pass these checks like any other pull request. A title
that a long package name and its versions take past 72 characters
(`build(deps): bump @cloudflare/workerd-linux-64 from …`) fails the
Conventional Commits check: shorten it on the pull request, which reruns
the check. GitHub also raises an alert for `web/package.json`,
`deploy/standalone/package.json` and the crates' own `Cargo.toml` files,
but Dependabot would update those without the lockfile, so their
directories open none.

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
| `xtask` | `cargo xtask`: dev, check, gen, serve, deploy, secrets, dist, publish |
| `web` | The web app (React, TanStack Router and Query, Tailwind, WebGL) |
| `deploy/instances` | Instance descriptions for `cargo xtask deploy` |
| `deploy/standalone` | The control plane without Cloudflare: its server, container image, Compose and Kubernetes examples |
| `docs` | These docs; `docs/adr` records decisions, and `docs/llms.txt` sums fctl up for agents |

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
  applies them locally (in CI, to a fresh database), `cargo xtask deploy`
  before deploying, and the standalone server as it starts. Never edit a
  migration that has shipped.
- It runs outside Cloudflare too, on workerd through Miniflare
  (`deploy/standalone`), with the bindings in `wrangler.toml`: a binding
  added there and in `xtask/src/deploy.rs` reaches both. Workers APIs that
  Miniflare lacks don't work there. Self-hosted instances keep their state
  where Miniflare does (`state/v3` in their data directory): when you
  upgrade wrangler, check that its Miniflare still reads it there.

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
is the one red, for destructive actions, errors and failures only. Hovered
rows and buttons fill with `hover` (`hover:bg-hover`, fainter on large
rows), not `raised`, which is white in the light theme, as `surface` is.
The scenes render dithered to the brand palette; they pause when hidden
and render one still frame for people who prefer reduced motion.

An address the app doesn't know shows `src/routes/not-found.tsx`, and a
request that fails before a page can show (the control plane's 5xx, no
connection) or a page that crashes shows `src/routes/error.tsx`, the
router's error page. Both are the design package's stream, with the code
as glyphs people can throw (`src/components/stream-screen.tsx`); on the
error page the power goes out and "Try again" turns it back on, asking
`/api/v1/health` before it loads the page again. To see it, block
`/api/v1/auth/me` in DevTools (Network → Block request URL) and reload.
fctl.sh has its own 404 and 5xx pages, in hyperstructure/fctl-site.

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

On a phone the app keeps the screen's size. Zooming is off: the viewport
tag in `index.html` asks for none (which also keeps Safari from zooming
into a text field), and `src/lib/zoom.ts` cancels the pinches Safari on
iOS allows anyway. The image viewer zooms its picture by itself instead:
`src/components/ui/lightbox.tsx` follows the fingers, and `Zoom` in
`src/lib/zoom.ts` works out where they take the picture. No page is wider
than the screen: what can be wider than a phone, like the Agent Floor's
workspace chips or a page's tabs, scrolls in a row of its own
(`overflow-x-auto`), long names, addresses and links wrap
(`[overflow-wrap:anywhere]`) or truncate, and the signed-in layout clips
whatever still sticks out. A card whose buttons go under its text when it
is narrow, like Home's invitations, goes by its own width rather than the
screen's, with a container query (`@container` on the card, `@xl:` and
`@max-xl:` inside): from 768 px the sidebar takes 244 px of the screen, so
a card is narrower at 768 px than at 767. When you change a page, look at
it 320 to 430 px wide, with long names in its data, and beside the sidebar
at 768 px.

## Other repositories

| Repository | What | How it connects |
|---|---|---|
| [hyperstructure/design](https://github.com/hyperstructure/design) | Tokens, fonts, WebGL scenes, the marks | `web/` depends on a tag; bumps arrive as pull requests |
| [hyperstructure/fctl-site](https://github.com/hyperstructure/fctl-site) | fctl.sh: landing page, these docs, `/install`, `/releases` | renders `docs/` at the commit in its `fctl-docs.ref`, which this repository's CI updates when `docs/` changes on `main`; serves the releases `cargo xtask publish` writes to the KV namespace in `deploy/releases.toml` |
| [hyperstructure/sandbox](https://github.com/hyperstructure/sandbox) | The default sandbox image, `ghcr.io/hyperstructure/sandbox` | its Image workflow publishes an image for each fctl release it finds on fctl.sh; hosts pull it, and `fctl host image build` builds its `main` ([Sandbox images](sandbox-images.md)) |

## Releasing

Releases come from `main` through the **Release** workflow: Actions →
Release → Run workflow, or `gh workflow run release.yml --ref main -f
bump=auto`. It picks the version from the Conventional Commits since the
last tag, adds the notes to `CHANGELOG.md`, pushes the release commit and
tag with a deploy key (which starts CI on `main`, so app.fctl.sh
redeploys), builds the binaries in parallel jobs (`cargo xtask dist build`:
fctl for macOS and Linux on arm64 and x86_64, and the Linux agents), makes
the archives from them (`cargo xtask dist package`), creates the GitHub
release with the notes and publishes to `https://fctl.sh/releases`
(`cargo xtask publish --latest`). Beside those jobs, the Standalone image
workflow runs a session through the release's standalone control plane and
then publishes its image to `ghcr.io/hyperstructure/fctl-control`;
`gh workflow run standalone-image.yml --ref main -f tag=v0.1.2` publishes
a release's image again. Within the hour,
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
hyperstructure/fctl-site). The jobs that deploy, make and publish releases
or pin fctl.sh's docs run in the `production` environment, which holds
their secrets (the Cloudflare token, the deploy keys that push release
commits here and docs pins to fctl-site, and the token that starts the
Homebrew tap's bump) and which only `main` may use, so workflows on other
branches cannot read them. Rulesets keep everything else off `main` and
off the `v*` tags
([ADR 0021](adr/0021-rulesets-protect-main-and-release-tags.md)). By hand:

```sh
cargo xtask deploy fctl-sh
```

Cloudflare's API throttles a token, an account or an address that has made
too many requests: it answers 429 with a Retry-After, for up to five
minutes, even to a deploy's first request. Wrangler gives up at once in its
D1, KV and secret commands, so `cargo xtask deploy`, `secrets` and
`publish` run a throttled wrangler command again once the wait is over, up
to four times (`util::retry_throttled`).
