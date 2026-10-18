# 0012: The sandbox image lives in its own public repository

2026-10-18 · accepted

## Context

The default sandbox image started in this repository, in `images/agent`,
and [#39](https://github.com/hyperstructure/fctl/pull/39) first published
it to GHCR from here, building each release's image from the release's tag.
That fits it badly:

- Hosts pull the image without signing in, so it is public, and the people
  who run agents in it or build images from it should be able to read what
  goes into it. This repository is private for now.
- Its cadence isn't fctl's. It is rebuilt when Claude Code or Debian move
  on and when a tool is added or upgraded; none of that needs an fctl
  release, yet a broken image built from a tag needed one to be fixed.

## Decision

- [hyperstructure/sandbox](https://github.com/hyperstructure/sandbox), a
  public repository, holds the image: `Dockerfile` and `provision.sh` at
  its root, `check.sh`, and the Image workflow that publishes
  `ghcr.io/hyperstructure/sandbox` for arm64 and amd64 from its `main`.
  This amends [0005](0005-three-repositories.md): images leave this
  repository.
- The tags stay as #39 made them: one for each fctl release, which that
  release's hosts pull, plus `X.Y` and `latest`. Each is the sandbox's
  `main` with that release of fctl and the newest Claude Code.
- The repositories connect through fctl.sh, not tokens: the Image workflow
  looks for new releases on fctl.sh every hour, as it reads Claude Code's
  latest version from downloads.claude.ai. fctl's Release workflow starts
  nothing there.
- `fctl host image build` downloads an archive of the sandbox's `main`
  instead of carrying the files in the binary, and gives Lima's base VM its
  `provision.sh`. `--source DIR` builds from a checkout.
- The layout (the `agent` user, the PATH, `/opt/fctl/bin`) and the files
  `fctl host image build` reads are an interface between the two
  repositories. `check.sh` checks every build for the layout, and
  `docs/sandbox-images.md` here describes it.

## Consequences

- Changes to the image ship when they merge in hyperstructure/sandbox,
  without an fctl release, and anyone can read how it's made.
- A release's image follows the release within the hour rather than within
  minutes. Hosts start sessions from `latest` meanwhile, as they did for
  those minutes.
- An image is no longer built from its release's tag: the newest release's
  is rebuilt from the sandbox's `main`, and older ones only when someone
  asks. Changing the layout means changing and releasing fctl first.
- No test checks any longer that the image's Rust matches
  `rust-toolchain.toml`; both repositories' instructions say to bump them
  together.
- `fctl host image build` needs to reach GitHub, as it already needed the
  package mirrors and installers its build downloads from.
- GitHub turns off scheduled workflows in public repositories after 60
  days without activity, warning by email first. Until someone turns the
  Image workflow back on, new releases get no image of their own.
