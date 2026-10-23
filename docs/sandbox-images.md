# Sandbox images

With the Apple container and Docker drivers, each session runs in a
container made from an image. This page describes the default image and
how hosts get it, how a project uses an image of its own or builds one from
its repository, which images a host runs, and what an image needs.

## The default image

`ghcr.io/hyperstructure/sandbox` is built from
[hyperstructure/sandbox](https://github.com/hyperstructure/sandbox), a
public repository whose README lists the versions: Debian 13 (trixie)
with git, the GitHub CLI, build tools, Python and uv, Rust, Node.js,
TypeScript, Bun, Claude Code and fctl. It is published for arm64 (Apple
silicon) and amd64.

| Tag | Image |
|---|---|
| `0.5.0` | fctl 0.5.0's. Hosts of that version use it. |
| `0.5` | The newest 0.5 release's. |
| `latest` | The newest release's. |

The sandbox repository's Image workflow builds every tag from its `main`
branch, with that release of fctl and the newest Claude Code. It looks for
new fctl releases every hour, so a release's image follows within the hour.
fctl turns off Claude Code's auto-updater, so an image's Claude Code is as
old as the image: every day, the workflow rebuilds the latest release's
image when Claude Code has a newer release, and once the image is a week
old, for Debian's updates. A change to the image rebuilds it too. A rebuild
moves the tags; to pin one build, add its digest:
`ghcr.io/hyperstructure/sandbox:0.5.0@sha256:…`.

Its labels say what is inside:

| Label | Value |
|---|---|
| `org.opencontainers.image.version`, `sh.fctl.version` | fctl's version |
| `sh.fctl.claude-code.version` | Claude Code's version |
| `org.opencontainers.image.revision` | the commit of hyperstructure/sandbox it was built from; images built elsewhere have none |
| `org.opencontainers.image.created` | when it was built |

```sh
docker image inspect ghcr.io/hyperstructure/sandbox:0.5.0 \
  --format '{{ index .Config.Labels "sh.fctl.claude-code.version" }}'
```

## How hosts get it

A host whose `host.toml` sets no `image` uses the default image of its own
version, so a host that updates, by itself or with `fctl self update`,
moves its new sessions to the new version's image. The host pulls the image in the background when
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
  …`) sets the image for sessions whose project sets none, and a project
  can choose one for its own sessions ([Your own
  image](#your-own-image)). `fctl-agent:latest`, the name the default
  image had when hosts built it themselves, means the default. On another
  image, the host pulls it when it isn't there, only `fctl host image pull`
  pulls it again, and the host leaves the default image's tags alone.
- **Building it.** `fctl host image build` builds the default image on the
  host with its driver, from the `main` branch of hyperstructure/sandbox,
  under the same name, so sessions use the build: for a host that can't
  reach ghcr.io, for instance. `--source DIR` builds from a checkout of the
  repository instead, to try changes to the image with fctl. The host never
  replaces a build with the published image, which it tells apart by its
  `org.opencontainers.image.revision` label; `fctl host image pull` does.
- **Lima** sessions clone the `fctl-base` VM instead, which `fctl host
  image build` prepares with the image's provisioning script, on Lima's
  Debian 13 (Debian 12 with Lima older than 2.0, which lacks it). The
  process driver uses no image.
- **Disk.** The host removes the default images that sessions no longer
  start from: builds that a newer one replaced, other versions' images, and
  `latest` once the version's image is there. It does that when it starts,
  after each pull and after the sweep removes stopped sandboxes, and leaves
  images built on the host and tags it doesn't pull, like `0.4`, alone. It
  removes them without force: with Docker, an image that a sandbox uses,
  stopped ones too, stays until the sweep removes the sandbox
  (`keep_stopped_hours`, or sooner on a low disk), and a build that a
  newer one replaced keeps the name `<tag>-replaced-<the start of its ID>`
  until then. Apple's `container` gives each sandbox its own copy of the
  image's filesystem, so the host removes them right away, and stopped
  sandboxes still start.

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
  through rustup with rustfmt and Clippy, Node.js 26 with npm and corepack,
  TypeScript (`tsc`), Bun, build-essential, cmake, pkg-config, OpenSSL's
  headers, ripgrep, fd, jq, ShellCheck, sqlite3, curl, wget, rsync, zip and
  tree.
- **Rust and Bun** are `agent`'s, in `~/.rustup`, `~/.cargo` and `~/.bun`:
  run `cargo`, `rustup` and `bun` as `agent`. Python is marked externally
  managed, so packages go into a virtual environment (`uv venv`,
  `python3 -m venv`).

In projects that share package caches, the Docker and Apple container
drivers mount the project's caches at `/var/cache/fctl`, over whatever an
image has there, and fctl-agent points the package managers there, for the
setup script and for Claude: `npm_config_cache`, `pnpm_config_store_dir`
and `pnpm_config_cache_dir` (pnpm 11; pnpm 9 and 10 get theirs in
`~/.config/pnpm/rc`), `COREPACK_HOME`, `YARN_CACHE_FOLDER` and
`YARN_GLOBAL_FOLDER`, `BUN_INSTALL_CACHE_DIR`, `UV_CACHE_DIR`,
`PIP_CACHE_DIR`, `GOMODCACHE` and `PLAYWRIGHT_BROWSERS_PATH`, with
`FCTL_CACHE_DIR` for the directory itself. Cargo keeps its downloads
beside its tools, so `~/.cargo/registry`, `~/.cargo/git` and their lock
files become links into the caches. What an image sets stays: its own
variables, a `~/.cargo/registry` it comes with, and browsers in
`~/.cache/ms-playwright`. With Docker, the host gives the caches to the
image's user, `agent` or another; fctl-agent checks it can write to them,
and without that, the session's package managers keep their own.

An image that installs something as root switches back to `agent` at the
end:

```dockerfile
FROM ghcr.io/hyperstructure/sandbox:0.14
USER root
RUN apt-get update \
  && apt-get install -y --no-install-recommends postgresql-client \
  && rm -rf /var/lib/apt/lists/*
USER agent
```

## Your own image

When sessions need tools the default image lacks, such as .NET or a
company's CLIs, give the project an image that has them. A setup script
would install them in every session instead. This example adds the .NET 8
SDK. .NET needs ICU, which Debian has, and the SDK goes into `agent`'s
home, like Rust. The PATH is set twice: `ENV` for Claude and the commands
it runs, and `/etc/profile.d` for login shells, which setup scripts run in:

```dockerfile
FROM ghcr.io/hyperstructure/sandbox:0.14

USER root
RUN apt-get update \
  && apt-get install -y --no-install-recommends libicu76 \
  && rm -rf /var/lib/apt/lists/* \
  && printf '%s\n' 'export DOTNET_ROOT="$HOME/.dotnet"' \
    'PATH="$HOME/.dotnet:$HOME/.dotnet/tools:$PATH"' > /etc/profile.d/dotnet.sh
USER agent

RUN curl -fsSL https://dot.net/v1/dotnet-install.sh -o /tmp/dotnet-install.sh \
  && bash /tmp/dotnet-install.sh --channel 8.0 --install-dir /home/agent/.dotnet \
  && rm /tmp/dotnet-install.sh
ENV DOTNET_ROOT=/home/agent/.dotnet \
  DOTNET_CLI_TELEMETRY_OPTOUT=1 \
  PATH=/home/agent/.dotnet:/home/agent/.dotnet/tools:$PATH
```

1. **Build it for both architectures** and push it to a registry: Apple
   silicon hosts run arm64 images, most others amd64.

   ```sh
   docker buildx build --platform linux/arm64,linux/amd64 \
     -t ghcr.io/acme/dotnet-agent:1 --push .
   ```

2. **Choose it for the project.** In Settings → Sandbox → Image, pick
   "An image from a registry" and enter `ghcr.io/acme/dotnet-agent:1`. From
   the CLI:

   ```sh
   fctl projects set -p my-app --image ghcr.io/acme/dotnet-agent:1
   fctl projects set -p my-app --image ""     # back to each runner's own
   ```

   Maintainers and owners can change it. Sessions that start or resume
   afterwards in a new sandbox use it; a sandbox that exists keeps the
   image it was made from.

3. **Hosts run it if their owners allow it.** A host that lists
   `allowed_images` runs only the images on its list ([Which images a host
   runs](#which-images-a-host-runs)); one that lists none runs any.

4. **Private registries.** Hosts pull images with their own login to the
   registry: run `docker login ghcr.io` (with Apple container,
   `container registry login ghcr.io`) on each host, as the user its
   service runs as. fctl sends no registry credentials. For GHCR, a token
   with `read:packages` is enough.

Lima runners don't use images: their sessions run in a clone of the VM
`fctl host image build` prepared. The process driver uses none either.

## Building it from the repository

A project can build its image from a Dockerfile or a dev container
configuration in its repository instead, so that the environment lives
next to the code and changes with it. In Settings → Sandbox → Image, pick
"Built from the repository" and name the file, or from the CLI:

```sh
fctl projects set -p my-app --build Dockerfile
fctl projects set -p my-app --build docker/agent.Dockerfile --build-context app
fctl projects set -p my-app --build .devcontainer/devcontainer.json
fctl projects set -p my-app --build ""     # stop building
```

The project needs a repository, and its build replaces any image it chose.
Only hosts whose owners allow builds from the repository take its sessions
(`allowed_builds`, [below](#which-images-a-host-runs)), and only those with
the Apple container or Docker driver.

When a session starts in a new sandbox, the host:

1. Reads the file at the tip of the project's default branch, with the
   session's GitHub token, and works out what the build reads: the
   Dockerfile, and the directory it builds in (the repository's root
   unless the setting or the dev container configuration names another).
2. Uses its last build when none of that changed since. Changes outside
   the directory don't count, but with the repository's root as the
   directory, every change does. In a busy repository, give the image a
   directory of its own (`--build-context docker`, say, or the
   `.devcontainer` directory a dev container configuration builds in by
   default); the builder's cache keeps the builds that follow short
   anyway.
3. Otherwise downloads the repository at that commit (up to 2 GiB), keeps
   only the directory, and builds the image with `docker build` or
   `container build`, from the Dockerfile's text as GitHub sent it. The
   session's log shows each step. Sessions that need the same build wait
   for the one that builds it. Builds pass no secrets: a Dockerfile that
   needs a token should leave that step to the setup script.
4. Tags the image `fctl-build-<project id>:<hash>`, and removes the
   project's earlier build, unless the runtime keeps it for a sandbox made
   from it (Docker does).

When GitHub can't be reached, sessions start from the last build of the
same setting and say so; any other problem, like a file that isn't there,
fails the session with the reason. A dev container configuration's
`image`, with no build, is used as it is, from its registry.

**Limits.** A host builds one image at a time, and reads no Dockerfile or
configuration larger than 1 MiB. A build that takes longer
than `build_timeout_minutes` (60) is stopped, and an image larger than
`max_build_mb` (20 GiB) is removed, where the runtime reports image sizes
(Docker does); the session fails and says why. A build runs in the
runtime's builder, not in a sandbox, so the project's CPU and memory
settings don't apply: with Docker it gets what the Docker daemon has (on
macOS, its VM), and with Apple container what its builder VM has
(`container builder start --cpus 8 --memory 16g` sizes it).

**What fctl reads from a dev container configuration.** `image`, or
`build.dockerfile`, `build.context`, `build.args` and `build.target`
(`dockerFile` and `context` at the top level in older ones), with paths
relative to the configuration's directory. It leaves out features,
lifecycle commands like `postCreateCommand`, `containerEnv`, `mounts` and
`runArgs`, and says so in the session's log: put what they install into
the Dockerfile, and what they run into the project's setup script.
Sessions run as the image's user rather than `remoteUser`, so end the
Dockerfile with `USER vscode`, say, to run as that user. Docker Compose
configurations aren't built.

The image still needs what every image needs ([below](#what-an-image-needs)):
most dev container images lack Claude Code. Start the Dockerfile `FROM
ghcr.io/hyperstructure/sandbox:0.14`, or install Claude Code in it as the
user sessions run as (`curl -fsSL https://claude.ai/install.sh | bash`).

## Which images a host runs

A project's image and its build run on machines that belong to other
people, with the session starter's Claude token and the session's GitHub
tokens inside them ([Security](security.md#sandboxes)). So each host's
owner decides, in its `host.toml`:

```toml
# Images projects may choose; the host's own `image` always runs. Unset: any.
allowed_images = [
  "ghcr.io/hyperstructure/sandbox",                     # any tag of it
  "ghcr.io/acme/*",                                     # anything under ghcr.io/acme/
  "ghcr.io/partner/agent@sha256:4f1c…",                 # this build only
]
# Repositories whose Dockerfile or devcontainer.json it builds. Unset: none.
allowed_builds = ["acme/*", "partner/app"]
build_timeout_minutes = 60
max_build_mb = 20480
```

The same lists from the CLI, each flag repeated for more entries
(`--allow-image '*'` allows any image, `--allow-build ""` none):

```sh
fctl host install --allow-image 'ghcr.io/acme/*' --allow-image ghcr.io/hyperstructure/sandbox \
  --allow-build 'acme/*'
```

- **An entry of `allowed_images`** is a reference without a tag, which
  allows any tag or digest of that repository; with a tag, which allows
  that tag but not a reference that pins a digest to it, since a pull by
  digest ignores the tag; or pinned to a digest (`@sha256:…`), which allows
  only that build, and only when the project's reference names the digest
  too, since a tag can move.
  An entry that ends in `/*` allows everything under that registry path,
  and `*` alone allows any image. Docker Hub's names count as Docker does:
  `ubuntu` is `docker.io/library/ubuntu`.
- **An entry of `allowed_builds`** is `owner/name` or `owner/*` on GitHub.
  Whoever can push to a project's default branch decides what its builds
  contain, including the images they start `FROM`.
- **Without the lists**, a host runs any image a project chooses, as hosts
  did before them, and builds none. Hosts of older versions do the same.

The host sends the lists to the control plane when it connects, so run
`fctl host restart` after editing `host.toml`. The control plane gives a
session only to a runner that runs its project's image or builds it, and
the runner checks again. A session that no runner online allows stays
queued and says why: "No runner allows this project's image,
ghcr.io/acme/agent:1", or "No runner builds this project's image from
acme/app". Admin → Runners, `fctl admin runners` and `fctl host status`
show each runner's image and lists.

## What an image needs

An image that doesn't start from the default one needs:

- **`sh`, `tail` and git** on the PATH. The sandbox's main process is `tail
  -f /dev/null`.
- **Claude Code 2.1.101 or later**, as `claude` on the PATH. fctl relies on
  requests that Claude Code sends and answers in its stream-json mode: it
  asks fctl for a fresh OAuth access token (`oauth_token_refresh`, from
  2.1.101), and fctl turns Remote Control on (`remote_control`, from
  2.1.63). fctl-agent runs `claude --version` before it starts Claude, and
  the session fails with a message that says so when Claude Code is
  missing or older.
- **`gh`**, which fctl-agent asks for the pull request of the session's
  branch. Without it, sessions show no pull request.
- **fctl** on the PATH, for agents to name their sessions, publish
  screenshots, keep their project's memory and record retrospectives
  (`fctl memory` and `fctl retro` need an fctl that has them; agents
  update their retrospectives where it has `fctl retro update`, and
  record one for each follow-up where it doesn't). Without it, sessions
  keep the title they start with, and their agents keep no memory.
- **A non-root user with a writable home.** fctl copies its supervisor into
  `/opt/fctl/bin` as root at session start, and runs it as the image's
  user (its `USER`), in `~/workspace`.
- **arm64** builds for Apple silicon hosts, and amd64 for most others.

## What fctl writes into a sandbox

At session start, fctl copies fctl-agent into `/opt/fctl/bin` and starts
it as the image's user, which writes:

- **`~/.fctl/`**: fctl-agent's sockets, log and state, the session's GitHub
  tokens (mode 0600) and the git credential helper that hands them out.
- **git's global configuration**: the GitHub App's bot as the author, the
  credential helper for github.com, `push.autoSetupRemote`,
  `init.defaultBranch` and a few quiet defaults. `~/.config/gh/hosts.yml`
  gives `gh` the token that can write.
- **`~/.claude.json`**: Claude Code's onboarding and the bypass-permissions
  prompt marked done, the auto-updater off, the working directory trusted,
  and the Claude account of the person who started the session. The rest
  of the file stays.
- **`~/.claude/settings.json`**: the image's own settings, with fctl's over
  them. fctl decides the permission mode (bypass), `attribution` (the
  trailers the project's settings ask for), `disableClaudeAiConnectors` and
  the auto-updater, and removes `apiKeyHelper`, `ANTHROPIC_*` and provider
  variables from `env`, and `permissions.disableBypassPermissionsMode`.
  Plugins, marketplaces, hooks, permission rules and other variables stay,
  and so do skills in `~/.claude/skills`.
- **`~/.claude/CLAUDE.md`**: a line that imports `~/AGENTS.md`, fctl's
  instructions for agents, after whatever the image has there.
- **`~/workspace`**, the project's repository, on the session's branch;
  `~/repos/<owner>/<name>` for its other repositories; and
  `~/attachments/` for files people attach.

Claude's environment has the project's variables and secrets,
`CLAUDE_CODE_OAUTH_TOKEN` and its scopes, `IS_SANDBOX=1`,
`DISABLE_AUTOUPDATER=1`, `GIT_TERMINAL_PROMPT=0`, `FCTL_SESSION_ID`,
`FCTL_SESSION_URL`, `FCTL_SESSION_SOCKET`, the sandbox's size in
`FCTL_SANDBOX_CPUS` and `FCTL_SANDBOX_MEMORY_MB`, and in projects that keep
a memory `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`. `ANTHROPIC_*` variables and
the Bedrock, Vertex and Foundry switches never reach it.
