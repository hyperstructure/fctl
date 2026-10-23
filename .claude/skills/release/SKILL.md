---
name: release
description: Release a new version of fctl through the Release workflow, which publishes the archives to GitHub and fctl.sh/releases and the standalone control plane's image to GHCR, updates the CHANGELOG and redeploys app.fctl.sh; hyperstructure/sandbox then publishes the release's sandbox image to GHCR, and hyperstructure/homebrew-tap proposes the release's Homebrew formula. Only for people and agents with release access (write access to Actions on hyperstructure/fctl). Use when asked to release, publish or ship fctl, to preview what the next release contains, or to fix a failed release.
---

# Releasing fctl

Releases are made from `main` by the **Release** workflow
(`.github/workflows/release.yml`). Never edit the version, write
CHANGELOG.md or push tags by hand: the workflow does all three, and a
ruleset lets only its deploy key create `v*` tags.

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

  or run the workflow with `dry_run` from `main` (every run starts from
  `main`: its jobs use the production environment); its summary shows the
  same. The
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

It runs in four steps, and a fifth beside them:

1. **Version and notes**: `cargo xtask release` sets the version in
   `Cargo.toml` and `Cargo.lock`, adds the notes to `CHANGELOG.md`, commits
   `chore(release): vX.Y.Z`, tags it, and pushes both to `main` at once
   with `RELEASE_DEPLOY_KEY`, the deploy key that the rulesets on `main`
   and `v*` tags let through (a `main` that moved meanwhile rejects the
   push; start it again). Like any push to `main`, that starts CI, which
   redeploys app.fctl.sh.
2. **Build**: a job for each binary, all at once (`cargo xtask dist
   build`): fctl for macOS and Linux (arm64 and x86_64) and the Linux
   agents.
3. **Publish**: makes the archives from those binaries (`cargo xtask dist
   package`), creates the GitHub release with the notes, publishes the
   files to https://fctl.sh/releases (moving `latest`), and installs it
   with the installer as a check.
4. **Homebrew tap**: starts the brew bump workflow of
   [hyperstructure/homebrew-tap](https://github.com/hyperstructure/homebrew-tap)
   with the version, using `HOMEBREW_TAP_TOKEN`. That workflow waits until
   livecheck finds the release on fctl.sh, then opens a pull request,
   `fctl X.Y.Z`, with the formula's new URLs and checksums for every
   platform.
5. **Standalone image**, beside Build and Publish: the Standalone image
   workflow (`.github/workflows/standalone-image.yml`) builds the control
   plane without Cloudflare (`deploy/standalone`) at the tag, runs a
   session through it, and only then publishes
   `ghcr.io/hyperstructure/fctl-control:X.Y.Z` for arm64 and amd64, moving
   `X.Y` and `latest` when the release is the newest they cover.

Within the hour, the Image workflow of
[hyperstructure/sandbox](https://github.com/hyperstructure/sandbox) finds
the release on fctl.sh and publishes its sandbox image,
`ghcr.io/hyperstructure/sandbox:X.Y.Z`, for arm64 and amd64, moving `X.Y`
and `latest`. Hosts of the new version pull it; until it is there, their
sessions start from `latest`. To publish it at once:
`gh workflow run image.yml -R hyperstructure/sandbox`.

Follow it with `gh run watch` or in the Actions tab. Build and Publish
take about 10 minutes; the standalone image takes a few more, since it runs
a session before it publishes.

## 3. After

- `curl -fsS https://fctl.sh/releases/latest` prints the new version, and
  `fctl self update --check` sees it.
- The GitHub release and `CHANGELOG.md` on `main` have the notes.
- `https://app.fctl.sh/api/v1/health` reports the version once CI is done.
- `docker buildx imagetools inspect ghcr.io/hyperstructure/fctl-control:X.Y.Z`
  lists linux/amd64 and linux/arm64.
- Within the hour, `docker buildx imagetools inspect
  ghcr.io/hyperstructure/sandbox:X.Y.Z` lists linux/amd64 and linux/arm64
  (`gh run list -R hyperstructure/sandbox --workflow image.yml` shows the
  runs).
- Within minutes, hyperstructure/homebrew-tap has a pull request,
  `fctl X.Y.Z` (`gh pr list -R hyperstructure/homebrew-tap`). Select
  **Approve workflows to run** on it to start its checks; once a code owner
  merges it, `brew upgrade fctl` installs the release.
- Hosts update themselves within minutes of the archives' publication,
  unless their `host.toml` turns that off: those, and Homebrew's, update
  with `fctl self update` (or `brew upgrade fctl`); either restarts the
  host service, and running sessions keep going.

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
- **The standalone image** (its Build and run a session job failed, or its
  Publish job did): nothing is published until a session has run through
  the image. If it was the infrastructure, run the failed jobs again, or
  publish the image on its own:
  `gh workflow run standalone-image.yml --ref main -f tag=vX.Y.Z`. A
  rebuild of the tag publishes it again too, under the same tags. If the
  image is broken, fix it in a pull request (one that changes
  `deploy/standalone` runs the same session) and release a patch. If the
  workflow is at fault, fix it in a pull request, then publish the image
  on its own: a re-run uses the workflows of the commit its run started
  from, so it fails the same way.
- **The sandbox image** (hyperstructure/sandbox's Image workflow failed, or
  the release has no image after an hour): its `publish` skill says what
  to do. Start it for the release with
  `gh workflow run image.yml -R hyperstructure/sandbox -f version=X.Y.Z`.
  The image is built from that repository's `main`, so fixing it takes a
  pull request there, not an fctl release.
- **The Homebrew tap** (the Homebrew tap job failed, or the tap has no
  pull request for the release): the release itself is out, and the tap's
  daily run proposes it anyway. If `HOMEBREW_TAP_TOKEN` is missing or
  expired, a person with admin access replaces it: a fine-grained token
  for hyperstructure/homebrew-tap alone, with Actions read and write. Then
  run the failed job again, or start the bump with
  `gh workflow run autobump.yml -R hyperstructure/homebrew-tap -f version=X.Y.Z`.
  When the bump itself failed, its run in the tap says why
  (`gh run list -R hyperstructure/homebrew-tap --workflow autobump.yml`).
- Never move or delete a published tag or its files on fctl.sh: installed
  CLIs and hosts download releases by version.

## Other repositories

- **hyperstructure/design** releases are tags as well; see its AGENTS.md
  and release skill.
- **hyperstructure/fctl-site** deploys from `main` and has no releases.
- **hyperstructure/sandbox** publishes the sandbox image from `main`, for
  every fctl release; see its AGENTS.md and publish skill.
- **hyperstructure/homebrew-tap** has the Homebrew formula, which its
  brew bump workflow updates in a pull request for every fctl release.
