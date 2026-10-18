# 0011: Commits credit people with an address they choose

2026-10-18 · accepted

## Context

Commits from sessions are authored by the GitHub App's bot, with Claude's
`Co-authored-by` trailer, and nothing credited the person who started the
session ([#15](https://github.com/hyperstructure/fctl/issues/15)). The
control plane already sent the starter's name and sign-in email in the
spec's `co_authors`, but no agent read it.

GitHub credits a co-author only when the trailer's address belongs to their
account: a verified address, or their `ID+login@users.noreply.github.com`.
The sign-in email may be neither, and it is personal data: a trailer
publishes it to everyone who can read the repository, for good.

## Decision

- **People choose the address**, in Settings → Profile or with `fctl
  account set --commit-email`, and get no trailer without one. It is kept
  on their user row, shown only to them (in `Me`, not in `User`, which
  admins see), included in their data export, and cleared when the account
  is deleted. The sign-in email never goes into a commit.
- **Claude Code's `attribution.commit` setting adds the trailer.**
  fctl-agent writes it into Claude's user settings: Claude's trailer, then
  one per person. The text replaces Claude Code's default, which names the
  model; fctl-agent writes the one Claude Code itself uses when it can't
  tell the model's version, `Claude <noreply@anthropic.com>`. When only
  Claude is credited it leaves the default alone. Not a git hook: a global
  `core.hooksPath` turns off the repository's own hooks, and hook managers
  replace the ones fctl would install. Not rewriting commits before
  pushing them: that changes commits the agent has already seen.
- **Projects choose who their commits credit**, with two switches, both
  on by default, for repositories whose conventions or audience call for
  it: **Credit people in commits** (`credit_people`) and **Credit Claude
  in commits** (`credit_claude`), also `fctl projects set --credit-people
  off --credit-claude off`. Without people, the control plane doesn't look
  up the address, so it never reaches the sandbox. Without Claude,
  fctl-agent leaves Claude's trailer out of `attribution.commit` and sets
  `attribution.sessionUrl: false`, so Remote Control sessions add no
  claude.ai link either. An update from a client that doesn't know a
  setting keeps its stored value, as for `claude_ai_connectors`.
- **A new spec field, `co_authored_by`**, carries the people to credit.
  `co_authors` stays, empty, for older agents that require it. An older
  control plane fills `co_authors` with the sign-in email and sends no
  `co_authored_by`, so a newer agent working for it adds no trailers.
- **Pull request descriptions stay plain** (`attribution.pr` is empty): a
  squash merge carries the `Co-authored-by` trailers of the commits it
  squashes, so the credit reaches the default branch either way.

## Consequences

- Only the commits Claude makes get the trailers; a commit a script makes
  carries whatever the script writes. A repository's Claude settings, or a
  rule against attribution in its CLAUDE.md, take precedence.
- When someone is credited, Claude's trailer no longer names the model.
- Sessions take a changed address or project setting when they next
  start or resume; running ones keep what they started with.
- Agents from before `credit_claude` always credit Claude: turning it off
  needs hosts from the same release.
- Commits keep the address after it is removed or the account deleted.
  fctl.sh's privacy policy says so.
- Once people can sign in with GitHub, the noreply address can come from
  the linked account.
- People who message a session are not credited; only its starter is.
