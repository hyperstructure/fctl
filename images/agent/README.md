# Agent image

The default sandbox for fctl sessions: Debian bookworm with the toolchains
agents build and test with, Claude Code (installed for the `agent` user, who
has passwordless sudo: the sandbox is the boundary) and fctl (in
`/usr/local/bin`).

- Python 3.15.0 as `python3` and `python` for every user (uv's build of
  CPython, in `/opt/uv/python`), and uv 0.13.1.
- Rust 1.99.0 through rustup, with rustfmt, Clippy and the
  `wasm32-unknown-unknown` target: the toolchain in this repository's
  `rust-toolchain.toml`, so sessions working on fctl have nothing to
  download.
- Node.js 22 with npm, TypeScript 7.0.2 (`tsc`) and Bun 1.4.3.
- build-essential (gcc, make), cmake, pkg-config and OpenSSL's headers.
- git, the GitHub CLI, ripgrep, fd, jq, ShellCheck, sqlite3, curl, wget,
  rsync, lsof, fuser, tree, file, zip, xz and zstd.

The versions are set at the top of `provision.sh`. A test in
`crates/fctl-host/src/image.rs` fails when Rust's differs from
`rust-toolchain.toml`.

- Rust and Bun are installed for the agent user (in `~/.rustup` and
  `~/.cargo`, and `~/.bun`), so agents add toolchains, targets and global
  tools without sudo: `cargo install`, `bun add -g`, and for Python `uv
  tool install` and `uv python install`. Images built from this one run
  cargo as `agent`.
- Python is marked externally managed: packages go into a virtual
  environment (`uv venv`, `python3 -m venv`) or come with `uv run --with`.
  Debian's Python 3.11, which Node.js depends on, stays in `/usr/bin`,
  after `/usr/local/bin` on the PATH.
- `~/.local/bin`, `~/.cargo/bin`, `~/.bun/bin` and `/opt/fctl/bin` lead the
  PATH. The Dockerfile sets it, and `/etc/profile.d/fctl.sh` sets it for
  login shells (setup scripts, the Lima VM). fctl-agent passes Claude the
  PATH, HOME and locale but not the image's other environment variables, so
  every tool here works from the PATH alone.
- npm leaves out its funding and audit notices and its update check
  (`/usr/etc/npmrc`); `npm audit` still runs when asked.
- `fctl host image build` builds it locally as `fctl-agent:latest` with the
  host's driver (Apple `container build` or `docker build`), or provisions
  the `fctl-base` Lima VM with the same `provision.sh`. It installs the
  host's own version of fctl (`FCTL_VERSION`; the latest release when that
  one is not published).
- Agents use fctl to name and describe their session: `fctl edit --title …
  --description …` reaches fctl-agent over the session socket and needs no
  sign-in. At each start fctl-agent writes how to do that to `~/AGENTS.md`,
  beside the checkout in `~/workspace`, and imports it from Claude's user
  memory (`~/.claude/CLAUDE.md`), so it applies in every repository.
- Projects can use any other image (Settings → Sandbox image). It needs
  `sh`, `tail`, git and Claude Code on the PATH; fctl copies its supervisor
  into `/opt/fctl/bin` at session start. With fctl on the PATH as well,
  agents name their sessions; without it, sessions keep the title they
  start with.
