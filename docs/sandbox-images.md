# Sandbox images

With the Apple container and Docker drivers, each session runs in a
container made from an image. This page describes the default image, how
hosts get it, and what images built from it can rely on.

## The default image

`ghcr.io/hyperstructure/sandbox` is built from
[hyperstructure/sandbox](https://github.com/hyperstructure/sandbox), a
public repository whose README lists the versions: Debian bookworm with
git, the GitHub CLI, build tools, Python and uv, Rust, Node.js, TypeScript,
Bun, Claude Code and fctl. It is published for arm64 (Apple silicon) and
amd64.

| Tag | Image |
|---|---|
| `0.4.0` | fctl 0.4.0's. Hosts of that version use it. |
| `0.4` | The newest 0.4 release's. |
| `latest` | The newest release's. |

The sandbox repository's Image workflow builds every tag from its `main`
branch, with that release of fctl and the newest Claude Code. It looks for
new fctl releases every hour, so a release's image follows within the hour.
fctl turns off Claude Code's auto-updater, so an image's Claude Code is as
old as the image: every day, the workflow rebuilds the latest release's
image when Claude Code has a newer release, and once the image is a week
old, for Debian's updates. A change to the image rebuilds it too. A rebuild
moves the tags; to pin one build, add its digest:
`ghcr.io/hyperstructure/sandbox:0.4.0@sha256:…`.

Its labels say what is inside:

| Label | Value |
|---|---|
| `org.opencontainers.image.version`, `sh.fctl.version` | fctl's version |
| `sh.fctl.claude-code.version` | Claude Code's version |
| `org.opencontainers.image.revision` | the commit of hyperstructure/sandbox it was built from; images built elsewhere have none |
| `org.opencontainers.image.created` | when it was built |

```sh
docker image inspect ghcr.io/hyperstructure/sandbox:0.4.0 \
  --format '{{ index .Config.Labels "sh.fctl.claude-code.version" }}'
```

## How hosts get it

A host whose `host.toml` sets no `image` uses the default image of its own
version, so upgrading the host (`fctl self update`) moves its new sessions
to the new version's image. The host pulls the image in the background when
it starts, and a session that starts before it has the image pulls it
itself. When the image can't be pulled, as in the hour after a release
before its image is published, the host pulls `latest` instead, and
sessions start from it and say so in their log. The first session after the
version's image is published pulls it.

The host pulls its tag again when it starts and every 6 hours, so new
sessions start from the newest build of it, with the newest Claude Code.
Sessions don't wait for that: they start from the build that is there, and
a tag that hasn't changed costs one request to ghcr.io. `fctl host image
pull` pulls it now. While the version's image can't be pulled, it pulls
`latest` instead, and says so.

- **Another image.** `image` in `host.toml` (or `fctl host install --image
  …`) sets the image for sessions whose project sets none, and a project's
  Settings → Sandbox → Image sets one for its own sessions.
  `fctl-agent:latest`, the name the default image had when hosts built it
  themselves, means the default. On another image, the host pulls it when
  it isn't there, only `fctl host image pull` pulls it again, and the host
  removes no images.
- **Building it.** `fctl host image build` builds the default image on the
  host with its driver, from the `main` branch of hyperstructure/sandbox,
  under the same name, so sessions use the build: for a host that can't
  reach ghcr.io, for instance. `--source DIR` builds from a checkout of the
  repository instead, to try changes to the image with fctl. The host never
  replaces a build with the published image, which it tells apart by its
  `org.opencontainers.image.revision` label; `fctl host image pull` does.
- **Lima** sessions clone the `fctl-base` VM instead, which `fctl host
  image build` prepares with the image's provisioning script. The process
  driver uses no image.
- **Disk.** The host removes the default images that sessions no longer
  start from: builds that a newer one replaced, other versions' images, and
  `latest` once the version's image is there. It does that when it starts,
  after each pull and after the sweep removes stopped sandboxes, and leaves
  images built on the host and tags it doesn't pull, like `0.4`, alone. It
  removes them without force: with Docker, an image that a sandbox uses,
  stopped ones too, stays until the sweep removes the sandbox
  (`keep_stopped_hours`), and a build that a newer one replaced keeps the
  name `<tag>-replaced-<the start of its ID>` until then. Apple's
  `container` gives each sandbox its own copy of the image's filesystem,
  so the host removes them right away, and stopped sandboxes still start.

## Its layout

Images built `FROM` the default image can rely on its layout, which stays
the same from version to version. The sandbox repository's `check.sh`
checks every build for it:

- **The user** is `agent`, uid 1000, with the home directory `/home/agent`
  and sudo without a password: the sandbox is the boundary. Sessions run as
  `agent`, in `~/workspace`.
- **The PATH** starts with `~/.local/bin`, where Claude Code (the native
  build) and uv's tools are, `~/.cargo/bin` and `~/.bun/bin`, then
  `/opt/fctl/bin`, where fctl copies its supervisor, fctl-agent, at session
  start. The image sets it, and `/etc/profile.d/fctl.sh` sets it for login
  shells, which setup scripts run in. fctl itself is in `/usr/local/bin`.
- **The tools**: git, gh, Python 3 (`python3` and `python`) with uv, Rust
  through rustup with rustfmt and Clippy, Node.js 22 with npm and corepack,
  TypeScript (`tsc`), Bun, build-essential, cmake, pkg-config, OpenSSL's
  headers, ripgrep, fd, jq, ShellCheck, sqlite3, curl, wget, rsync, zip and
  tree.
- **Rust and Bun** are `agent`'s, in `~/.rustup`, `~/.cargo` and `~/.bun`:
  run `cargo`, `rustup` and `bun` as `agent`. Python is marked externally
  managed, so packages go into a virtual environment (`uv venv`,
  `python3 -m venv`).

An image that installs something as root switches back to `agent` at the
end:

```dockerfile
FROM ghcr.io/hyperstructure/sandbox:0.4
USER root
RUN apt-get update \
  && apt-get install -y --no-install-recommends postgresql-client \
  && rm -rf /var/lib/apt/lists/*
USER agent
```

## Other images

An image that doesn't start from the default one needs `sh`, `tail`, git
and Claude Code on the PATH, and a user with a writable home. fctl copies
its supervisor into `/opt/fctl/bin` at session start and runs it as the
image's user, in `~/workspace`. With fctl on the PATH as well, agents name
their sessions; without it, sessions keep the title they start with. Apple
silicon hosts need arm64 images.
