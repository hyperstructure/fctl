# 0049: Agents write balanced pull request descriptions, unless their project chooses otherwise

2026-10-23 · accepted

## Context

fctl told agents that open a pull request to "describe what changed and
why, how you verified it, and link the session", and nothing more. Claude
filled that in at length: sections for what changes and how it was
verified, tables, the story of how it found the problem. People reviewing
the pull requests found much of it noise and hard to read. Descriptions
also went stale: later commits, for review comments or failed checks,
changed a pull request without its description.

The person who asked for this wanted a balanced level of detail by
default, said what it should hold (a short summary, then what is worth
confirming), and wanted brief descriptions, and full ones like before, to
choose from in each project.

## Decision

- **A setting per project.** `pull_request_descriptions` in a project's
  settings: `brief`, `balanced` or `full`. Balanced is the default, for
  projects from before it too. Settings → Agents in the web app,
  `fctl projects set --pr-descriptions` in the CLI. An update that leaves
  it out keeps the stored value, and a value a version doesn't know reads
  as balanced, so that an older client or control plane can't fail or
  reset it.
- **In the system prompt.** fctl-agent gives Claude the level's guidance in
  a section of its system prompt, not in `~/AGENTS.md`, which it writes only
  where the sandbox's fctl can rename the session. It comes after the rest
  of what fctl says and before the project's instructions, which can still
  ask for something else, as can the people in the session. Agents that may
  push get it whether or not the project asks them to open a pull request
  when they finish, since people ask for them too; agents that may only
  comment or read open none and don't get it.
- **The levels.** Balanced follows the person's outline: a "Summary" of
  one or two short paragraphs, other sections only for what a reviewer
  must have, and a "Worth confirming" section only when it has items, each
  a **Decision** or an **Assumption** about behavior that the task didn't
  settle. It names the filler to leave out, Claude Code's own "Test plan"
  among it. Brief is a few sentences without headings. Full asks for the
  whole story, as agents told it before.
- **What holds at every level.** A paragraph or list item on one line,
  since GitHub shows a line break inside a paragraph; fenced code; readable
  link text, and related pull requests as their URLs alone in list items,
  which GitHub shows with their titles and states; the repository's pull
  request template, where it has one; and the session's link at the end,
  as before.
- **Kept up to date.** When later commits change what a pull request does,
  how it was verified or what is worth confirming, the agent rewrites its
  description to describe the pull request as it stands, rather than adding
  notes on what changed since. fctl doesn't check that it does.

## Consequences

- Pull requests that agents open read shorter by default, also in projects
  that preferred the long ones; those choose full to get them back.
- A pull request's description changes after it opens, which GitHub shows
  in its edit history. Where a description becomes a squash commit's
  message, the message describes the pull request as merged.
- Hosts from before this release run agents that describe pull requests as
  before, whatever the setting says, until they upgrade; older control
  planes don't keep the setting.
