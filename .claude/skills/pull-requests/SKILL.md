---
name: pull-requests
description: What to do once a pull request is open in hyperstructure/fctl. Covers waiting for CI in the same turn and fixing failures, the title check, answering review comments in their threads, keeping the branch current, and what to do after the merge (no pushes to the merged branch, ticking the issue, saying whether the change is released or deployed). Use right after `gh pr create`, when asked about a pull request's CI, reviews or state, and after a merge.
---

# After opening a pull request

Opening the pull request is not the end of the work: it is done when CI is
green, the review is answered and the merge is recorded in the issue. How
to write the title and description is in the `commit` skill.

## Wait for CI in the same turn

```sh
gh pr checks N --watch --fail-fast
```

or, for one run, `gh run watch ID --exit-status`. Do it before you end your
turn. An fctl session goes idle when the turn ends, and nothing wakes it
when CI finishes or someone reviews (#8 will), so a failure you don't watch
for stays unnoticed.

The checks: **Commits** checks the title, and **CI** runs
`cargo xtask check` (formatting, generated types, Biome and TypeScript,
Clippy, tests and builds).

## When a check fails

1. Find the run and read the failure:

   ```sh
   gh run list --branch "$(git branch --show-current)" --limit 5
   gh run view ID --log-failed
   ```

2. Reproduce it locally (`cargo xtask check`, or the one command that
   failed), fix it, commit, push, and watch again.
3. The title check fails on the title, not the code:
   `gh pr edit N --title "fix(cli): …"`. Only the Commits workflow reruns.
4. If it looks like infrastructure (a timeout, a registry or network error),
   rerun it once with `gh run rerun ID --failed`. If it fails again, treat
   it as real.
5. Don't ask anyone to merge a red pull request, and don't skip or weaken a
   check to make it pass.

## Review comments

Read the conversation, and the comments on lines of code:

```sh
gh pr view N --comments
gh api repos/{owner}/{repo}/pulls/N/comments \
  --jq '.[] | {id, path, line, user: .user.login, body}'
```

Answer **every** comment in its own thread. Either fix it, push, and reply
with what changed (and the commit), or say plainly why not:

```sh
gh api repos/{owner}/{repo}/pulls/N/comments/ID/replies -f body="Fixed in abc1234: …"
gh pr comment N --body "…"   # for comments on the conversation
```

Leave resolving threads to the reviewer. If a comment asks for something
outside the pull request, say so and offer an issue (the `issues` skill).
After changes, update the description so it still says what the pull
request does: it becomes the commit message.

## Keeping the branch current

When `main` moves on and the pull request conflicts, or CI needs the newer
base, update your own branch:

```sh
git fetch origin
git rebase origin/main         # resolve, rerun the checks
git push --force-with-lease
```

Only ever force-push your own pull request branch, never `main`.

## After the merge

Check it: `gh pr view N --json state,mergeCommit,mergedAt`.

- **Don't push to the merged branch.** GitHub deletes it. Follow-ups go
  on a new branch from the updated `main`, in a new pull request that
  links the first.
- **Record it in the issue.** Tick what the merge completed and link the
  pull request. If it had `Refs`, comment what is left (the `issues` skill).
- **Say where it is live.** On merge, CI deploys the control plane and the
  web app to app.fctl.sh, and docs reach fctl.sh. Changes to fctl-agent,
  the host or the CLI reach people only with a release:

  ```sh
  git fetch --tags origin
  git tag --contains MERGE_SHA    # empty: not released yet
  ```

  Then say "deployed", "released in vX.Y.Z" or "ships with the next
  release". Releases are the `release` skill's job, for those with access.
