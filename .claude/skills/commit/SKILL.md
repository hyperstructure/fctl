---
name: commit
description: Commit and open pull requests in hyperstructure/fctl. Covers Conventional Commit subjects (they become the release notes and pick the version), what goes in the body, branch names, and how a change reaches main. Use when committing, naming a branch, writing a pull request title or description, or getting work merged.
---

# Commits and pull requests

Every change reaches `main` as one squash-merged pull request, and the
pull request's **title becomes the commit subject**; its description
becomes the commit body. `cargo xtask release` makes the version number
and the release notes from those subjects, so they follow
[Conventional Commits](https://www.conventionalcommits.org).

## The title

`type(scope): what changed`, at most 72 characters, no full stop.

| Type | For | Next release |
|---|---|---|
| `feat` | something people can do that they couldn't before | minor |
| `fix` | a bug fix | patch |
| `perf` | faster or lighter, same behavior | patch |
| `docs` | documentation only | patch |
| `refactor` | restructuring that changes no behavior | patch |
| `test`, `build`, `ci`, `chore`, `revert` | tests; builds and dependencies; CI; upkeep; reverts | patch |

The scope is optional and names the part of fctl: `control`, `host`,
`agent`, `cli`, `web`, `proto`, `domain`, `xtask`, `deps`, `docs`,
`release`.

The description is what the release notes say, so name the change people
notice, in the imperative, lowercase unless it starts with a name:

- `feat(cli): add fctl self update`
- `fix(agent): start session branches untracked`
- `perf(control): page transcripts out of the session object`
- `docs: explain sandbox expiry`

A **breaking change** (people or older hosts must adapt: a protocol,
config, CLI or API change) gets a `!` and a `BREAKING CHANGE:` paragraph
in the description that says what to do: `feat(proto)!: drop the v0
uplink`. Before 1.0 it bumps the minor version. Remember hosts and agents
may run one version behind the control plane (AGENTS.md).

Check a title before you use it:

```sh
cargo xtask lint-commit "feat(cli): add fctl self update"
```

CI checks every pull request title, and again when you edit it.

## The description

Plain prose for someone reading `git log` in a year: what was wrong or
missing, what changed and why this way, and how you verified it (tests,
`cargo xtask dev`, screenshots of UI in both themes). Link issues with
`Closes #12`. End with the attribution lines your environment asks for.

## The flow

1. Branch from an up-to-date `main`: `<type>/<topic>` (`fix/plain-push`),
   or `fctl/<number>-<topic>` when an fctl session makes the branch.
2. Commit as you go. Commits on the branch are squashed, so their
   messages are yours; the title is what lasts.
3. Run `cargo xtask check` before you push.
4. Open the pull request: a conventional title, a description, one
   change. Split unrelated work into separate pull requests.
5. A person merges it (squash) when CI is green, unless they asked you to
   merge it yourself.

Never push to `main` directly: it is what releases are made from. The
only pushes to `main` are the Release workflow's release commits and
fctl.sh's docs pins (in hyperstructure/fctl-site).

Never commit secrets: keys, tokens, `.dev.vars`, `*.pem`, Cloudflare
credentials. Check `git status` before committing.
