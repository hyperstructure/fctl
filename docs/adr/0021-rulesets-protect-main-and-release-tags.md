# 0021: Rulesets protect main and the release tags

2026-10-18 · accepted

## Context

fctl, design and fctl-site became public on 2026-10-18; sandbox already
was. Until then GitHub Free gave their `main` branches no rules, so `main`
was protected only by convention
([0006](0006-conventional-commits-and-workflow-releases.md)). Anyone can now
write the issues and comments that agents read, and agents in fctl
sessions can push branches and change workflows (their tokens have
`contents`, `pull_requests` and `workflows` write).

What reaches `main` runs with the production secrets: CI deploys
app.fctl.sh from it, and sandbox's Image workflow publishes the image every
host pulls. Those secrets are in a `production` environment that only
`main` may use, so the question is who can get code onto `main`.

[#22](https://github.com/hyperstructure/fctl/issues/22) planned rulesets
with no required approvals, and with GitHub Actions on the bypass list so
that the Release workflows could push their release commits. Either would
leave a way in: with no approval, an agent can merge its own pull request;
with GitHub Actions bypassing, any workflow can push to `main`, including
one an agent writes on a branch.

## Decision

- **A `main` ruleset in each repository** (`.github/rulesets/main.json`):
  - changes arrive as squash-merged pull requests, with linear history;
  - no force pushes, and `main` can't be deleted;
  - the repository's CI checks must pass, coming from GitHub Actions;
  - a code owner must approve, and that code owner must not have pushed
    the last commit. Agents push and review as fctl-sh[bot], which can't
    be a code owner (`.github/CODEOWNERS`).
- **A `v*` tag ruleset in fctl and design**
  (`.github/rulesets/release-tags.json`): only bypass actors create, move
  or delete release tags.
- **What bypasses them:**
  - Deploy keys, always. These are the write keys that the Release workflows
    push release commits and tags with (`RELEASE_DEPLOY_KEY`), and the key
    that fctl's CI pins fctl.sh's docs with (`SITE_DEPLOY_KEY`). They are
    secrets of the `production` environment.
  - Organization owners, on pull requests only: they can merge one without
    an approval, but can't push to `main`.
- **GitHub Actions is not on the bypass list.** Releases push with the
  deploy key. Such a push starts workflows, as a GITHUB_TOKEN push does not,
  so fctl's release commit starts CI, which redeploys app.fctl.sh. The
  dispatch at the end of Publish is gone.
- **Workflows can't approve pull requests.** Each repository turns off
  "Allow GitHub Actions to create and approve pull requests".

The rulesets live in GitHub's settings. The JSON files are what was
applied, with `gh api -X POST repos/hyperstructure/<repo>/rulesets --input
<file>`. Change both together.

## Consequences

- Agents can't push to `main` or merge their own pull requests. A code
  owner approves every change before it merges, or an owner merges it
  without one (`gh pr merge --admin`).
- Every Release run starts from `main`, a dry run too, because its jobs use
  the `production` environment.
- app.fctl.sh redeploys when fctl's release commit lands, a few minutes
  before the archives are on fctl.sh. Only its reported version changes,
  and hosts and the CLI update from fctl.sh.
- In design, the release commit and tag now start CI (the tag check runs),
  and start the Release workflow again, which finds nothing due.
- What a stranger might steer an agent into no longer reaches `main`, the
  release tags or the production secrets. Interaction limits on the
  organization keep strangers out of issues and pull requests for now;
  lifting them is a separate decision, since an agent still acts with its
  session's own secrets and token.
