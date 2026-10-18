---
name: release
description: Release a new version of fctl through the Release workflow, which publishes the archives to GitHub and fctl.sh/releases and the sandbox image to GHCR, updates the CHANGELOG and redeploys app.fctl.sh. Only for people and agents with release access (write access to Actions on hyperstructure/fctl). Use when asked to release, publish or ship fctl, to preview what the next release contains, or to fix a failed release.
---

# Releasing fctl

Releases are made from `main` by the **Release** workflow
(`.github/workflows/release.yml`). Never edit the version, write
CHANGELOG.md or push tags by hand: the workflow does all three, and tags
pushed by hand publish nothing.

Starting the workflow needs write access to Actions. Agents in fctl
sessions have read access only, so if you are one, ask a person to start
it (or to give you access) and stop there.

## 1. Before

- CI passed on the newest commit of `main`, and everything meant for the
  release is merged.
- Preview the version and notes:

  ```sh
  git switch main && git pull
  cargo xtask release --dry-run
  ```

  or run the workflow with `dry_run`; its summary shows the same. The
  version follows the commits since the last tag: a breaking change bumps
  the minor version (the major one from 1.0), a `feat` the minor version,
  anything else the patch. Override it only for a reason, with
  `bump=patch|minor|major`.
- Read the notes as a user would. A badly worded title cannot be changed
  on `main`, but you can edit the GitHub release text and CHANGELOG.md
  afterwards (a `docs:` pull request).

## 2. Release

```sh
gh workflow run release.yml --ref main -f bump=auto
```

or GitHub → Actions → Release → Run workflow. With the GitHub App API:
`POST /repos/hyperstructure/fctl/actions/workflows/release.yml/dispatches`
with `{"ref": "main", "inputs": {"bump": "auto"}}`.

It runs in three steps:

1. **Version and notes**: `cargo xtask release` sets the version in
   `Cargo.toml` and `Cargo.lock`, adds the notes to `CHANGELOG.md`, commits
   `chore(release): vX.Y.Z`, tags it, and pushes both to `main` at once (a
   `main` that moved meanwhile rejects the push; start it again).
2. **Build**: a job for each binary, all at once (`cargo xtask dist
   build`): fctl for macOS and Linux (arm64 and x86_64) and the Linux
   agents.
3. **Publish**: makes the archives from those binaries (`cargo xtask dist
   package`), creates the GitHub release with the notes, publishes the
   files to https://fctl.sh/releases (moving `latest`), installs it with
   the installer as a check, starts CI on `main`, which redeploys
   app.fctl.sh, and starts the **Image** workflow.

The Image workflow (`.github/workflows/image.yml`) builds the release's
sandbox image from its tag, for arm64 and amd64, and publishes it as
`ghcr.io/hyperstructure/fctl-agent:X.Y.Z`, moving `X.Y` and `latest`.
Hosts of the new version pull it; until it is there, their sessions start
from `latest`.

Follow it with `gh run watch` or in the Actions tab; it takes about 10
minutes.

## 3. After

- `curl -fsS https://fctl.sh/releases/latest` prints the new version, and
  `fctl self update --check` sees it.
- The GitHub release and `CHANGELOG.md` on `main` have the notes.
- `https://app.fctl.sh/api/v1/health` reports the version once CI is done.
- The Image workflow's run for the tag passed (`gh run list --workflow
  image.yml`), and `docker buildx imagetools inspect
  ghcr.io/hyperstructure/fctl-agent:X.Y.Z` lists linux/amd64 and
  linux/arm64.
- Hosts update with `fctl self update` (it restarts the host service;
  running sessions keep going).

## When something fails

- **Before the release commit** (version and notes failed): fix the cause
  in a pull request, merge it, start the workflow again.
- **After the tag was pushed** (a build or publish failed): if it was the
  infrastructure, run the failed jobs again (`gh run rerun <run-id>
  --failed`, or "Re-run failed jobs" on the run's page); Publish uses the
  binaries the Build jobs made, which are kept for 7 days. Or rebuild the
  tag from the start with
  `gh workflow run release.yml --ref main -f tag=vX.Y.Z`. If the code is
  broken, fix it and release a patch.
- **The image** (the Image workflow failed): fix the cause in a pull
  request if it is the workflow, then start it again for the tag:
  `gh workflow run image.yml --ref main -f tag=vX.Y.Z`. It builds from the
  tag, so a broken `images/agent` needs a patch release.
- Never move or delete a published tag or its files on fctl.sh: installed
  CLIs and hosts download releases by version.

## Other repositories

- **hyperstructure/design** releases are tags as well; see its AGENTS.md
  and release skill.
- **hyperstructure/fctl-site** deploys from `main` and has no releases.
