# 0006: Conventional Commits, and releases from a workflow

2026-10-18 · accepted

## Context

Releases were cut by bumping the version and pushing a tag by hand, with
GitHub's generated notes. More people and agents now merge into `main`,
and anyone who could push a tag could publish a release, including agents
working in fctl sessions.

## Decision

- Changes reach `main` as squash-merged pull requests whose titles are
  [Conventional Commits](https://www.conventionalcommits.org); CI checks
  them (`cargo xtask lint-commit`).
- Releases are made only by the Release workflow, started by hand. It
  picks the next version from the commits since the last tag (a breaking
  change bumps the minor version before 1.0, a `feat` the minor, anything
  else the patch), writes `CHANGELOG.md`, pushes the release commit and
  tag, and builds and publishes from them (`cargo xtask release`,
  `cargo xtask notes`). Tags pushed by hand publish nothing.
- How to commit and how to release are written down for agents as skills
  (`.claude/skills/commit`, `.claude/skills/release`) and in AGENTS.md.

## Consequences

- Release notes write themselves, and the version says what kind of
  change a release is.
- Publishing needs write access to Actions, which fctl session tokens do
  not have.
- `main` is protected only by convention while the repositories are
  private (GitHub Free has no branch rules for private repositories):
  [#22](https://github.com/hyperstructure/fctl/issues/22) protects it once
  they are public.
