# 0017: Hosts choose the images they run and build

2026-10-18 · accepted

## Context

Projects could choose a sandbox image since the first release (Settings →
Sandbox → Image), and every host pulled and ran whatever a project named.
That image runs on machines that belong to other people, with the Claude
access token of the person who started the session and the session's
GitHub tokens inside it. A token from `claude setup-token` is valid for a
year. Host owners had no say.

Getting tools into sessions meant a setup script that installs them in
every session, or an image built and published by hand. Many repositories
already describe their environment in a Dockerfile or a dev container
configuration. And an image's own Claude settings (plugins, marketplaces,
hooks) were lost, because fctl-agent overwrote `~/.claude/settings.json`.

[#16](https://github.com/hyperstructure/fctl/issues/16) asked for all of
this. This record covers the choices made in building it.

## Decision

- **Host owners list what runs.** `allowed_images` in `host.toml` holds
  references (no tag: any tag; a tag; or a digest, which the project's
  reference must name too) and prefixes ending in `/*`. `allowed_builds`
  holds GitHub repositories, `owner/name` or `owner/*`. The host's own
  image always runs. Unset, `allowed_images` allows any image, as hosts did
  before it, and `allowed_builds` allows no builds.
- **The fleet schedules by them.** Hosts send both lists as optional
  fields of their hello; hosts of older versions send neither, and run any
  image and build none. FleetDO gives a session only to a runner whose
  lists take its project's image or build, and a session that no runner
  online takes stays queued with the reason. The host checks again before
  it accepts a session. Lima and the process driver use no images and send
  no `allowed_builds`.
- **Projects build from their default branch.** `image_build` names a
  Dockerfile (with a context) or a dev container configuration. Hosts
  build from the tip of the project's default branch, never from a
  session's base branch, so that what a build runs is what reached that
  branch. fctl reads a dev container configuration's `image` and `build`
  and leaves out features and lifecycle commands, saying so in the
  session's log.
- **Builds are keyed by what they read.** At each session start in a new
  sandbox, the host asks GitHub's API for the file, the Dockerfile and the
  git tree of the context, and builds only when no image has their hash.
  It builds one image at a time, within `build_timeout_minutes`, removes
  an image over `max_build_mb`, keeps each project's newest build, and
  starts from the last build while GitHub can't be reached. Builds run in
  the container runtime's builder with no secrets. The session's GitHub
  token reads the repository on the host and never enters the build.
- **Registries use the host's own login.** fctl stores and sends no
  registry credentials. Passing them from project secrets is left for a
  later decision, if hosts' own logins prove too much work.
- **The image's Claude settings stay.** fctl-agent writes its keys
  (permission mode, `attribution`, `disableClaudeAiConnectors`, the
  auto-updater) over the image's settings. It drops `apiKeyHelper`,
  `ANTHROPIC_*` and provider variables, and
  `permissions.disableBypassPermissionsMode`, and keeps everything else.
- **Claude Code 2.1.101 or later.** It is the first release that asks its
  host for fresh OAuth tokens (`oauth_token_refresh`). fctl-agent runs
  `claude --version` before it starts Claude and fails the session, saying
  why, when Claude Code is older or missing.

## Consequences

- A host owner who wants to limit what runs must set `allowed_images`: the
  default stays as permissive as before, so that upgrading no host stops
  projects' sessions. Making new hosts restrictive by default is a
  decision for later.
- A session that starts from another base branch still gets the default
  branch's image.
- A change to a build's files makes the next session that starts in a new
  sandbox wait for the build. Each host builds its own copy, and caches
  only on its own disk.
- fctl doesn't limit a build's CPUs and memory: it gets what the runtime's
  builder has. Docker's builder has the daemon's resources, and Apple
  container's builder VM is sized with `container builder start`.
- Hosts that build reach `api.github.com` and `codeload.github.com`, as
  their sessions already reach GitHub.
- Images whose Claude Code is older than 2.1.101 stop working, with a
  message that says what to update, instead of failing once a token
  expires.
