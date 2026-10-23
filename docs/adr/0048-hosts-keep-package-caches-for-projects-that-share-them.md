# 0048: Hosts keep package caches for projects that share them

2026-10-23 · accepted

## Context

Every session starts in a new sandbox, so its setup script downloads
everything again: `npm ci`, `cargo fetch`, `uv sync`, Playwright's
browsers. For this repository that is the web app's packages, the
workspace's crates and Chromium, minutes and hundreds of megabytes each
time, and the same for every session on the same runner.

[#3](https://github.com/hyperstructure/fctl/issues/3) asked for a cache
volume per project, mounted into its sandboxes, for npm, pnpm, Cargo, uv
and pip, bounded in size, cleaned by the host, and optional. The person
who started the session that built it asked for it to suit the projects,
and the runners, that want it.

## Decision

- **Projects opt in.** `package_caches` in a project's settings, off by
  default. A session on a runner with caches shares them with the
  project's other sessions there; projects never share caches with each
  other. Review sessions never get them: the control plane turns the
  setting off in their spec and the host checks again, since a pull
  request's text could steer their agent into leaving something for the
  project's other sessions.
- **Runners bound them.** `max_cache_gb` (10 GiB by default) and
  `keep_caches_days` (14) in `host.toml`: past the size the host empties a
  project's caches, and after the days without a session it removes them.
  `max_cache_gb = 0` keeps none, so a runner's owner can refuse them.
  Hosts that upgrade keep caches for the projects that turn them on,
  within those defaults, without editing anything. A session that runs on
  a host without caches runs as before, saying why in its log; the
  control plane doesn't schedule by caches.
- **Mounted where the runtime shares them safely.** With Docker, a named
  volume per project, on the daemon's disk like its sandboxes: inside the
  VM on macOS, so the free disk the host measures there counts it. With
  Apple's `container`, a directory on the Mac that each sandbox
  bind-mounts: its named volumes are ext4 images, which nothing stops two
  running VMs from mounting at once, and which would corrupt. The process
  driver's agents use the directory directly. Lima's VMs get none for
  now: their isolation removes every mount, and adding one is for later.
  Sandboxes see the caches at `/var/cache/fctl`.
- **fctl-agent points the tools there.** Variables for npm, pnpm 11,
  Corepack, Yarn, Bun, uv, pip, Go and Playwright, set before the
  project's own, so a project's environment wins; `~/.config/pnpm/rc` for
  pnpm 9 and 10, which take no variable npm doesn't warn about; and links
  for Cargo's registry, git checkouts and lock files, since `CARGO_HOME`
  holds Cargo's tools too. Sharing the lock files keeps two cargo runs, in
  two sandboxes, from writing the registry at once. What the image sets
  stays. fctl-agent first checks it can write to the caches, so a mount
  that doesn't work leaves the session as it was, never broken.
- **Emptied whole, and before sandboxes.** The sweep measures a project's
  caches only when a session used them since their last measurement, and
  empties them whole past the limit, rather than trimming the least used
  files: package managers' caches tolerate that, and a 10 GiB limit is
  rarely reached. On a low disk, caches go first, least recently used
  first, before stopped sandboxes: an empty cache costs the next session
  downloads, a removed sandbox Claude's conversation. The sweep never
  touches caches that a session here uses, and a session that starts while
  its project's caches are measured or emptied waits for that.

## Consequences

- Sessions of a project that shares caches start faster on a runner that
  ran one before, and the first one there fills them.
- Anything a session runs can change what the caches hold, and later
  sessions of the project on that runner build with it; package managers
  check downloads against lockfiles' hashes, but mostly not what is in
  their cache already. The security page says so, and projects whose
  agents run code from people they don't trust leave caches off.
- A busy project whose sessions always overlap on a runner may pass its
  limit until they don't; the disk floor still empties its caches when
  none runs. Docker keeps an emptied volume while a stopped sandbox mounts
  it, until the sandbox goes.
- Projects get caches once their control plane and their runners' hosts
  run the release with this change, which brings its own fctl-agent into
  each sandbox. Older hosts ignore the setting, and older control planes
  don't keep it.
- A sandbox made before its project turned caches on has no mount, so
  that session gets them only in its next sandbox.
