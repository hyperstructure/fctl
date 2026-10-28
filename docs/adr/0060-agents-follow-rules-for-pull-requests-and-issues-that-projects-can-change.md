# 0060: Agents follow rules for pull requests and issues that projects can change

2026-10-28 · accepted

## Context

fctl told agents how to open a pull request, how to describe it
([ADR 0049](0049-agents-write-balanced-pull-request-descriptions.md)) and,
through the tracker's lines ([ADR 0052](0052-projects-choose-a-ticket-tracker-and-linear-delegates-tickets-to-fctl.md)),
to write `Fixes #12` when a pull request does what an issue asks. Nothing
more about handling them: not to write `Fixes` for part of an issue,
which closes it with work left, nor to wait for checks, answer review
comments where they were made, or leave merging to people. hyperstructure's
own repositories have those rules in their AGENTS.md and skills; agents in
every other project improvised, as fctl's own did before it had them
([#28](https://github.com/hyperstructure/fctl/issues/28)).

The person who decided wanted brief default instructions for every
project, which each project can change.

## Decision

- **A section of the system prompt.** fctl-agent tells agents that may
  push and open pull requests how to handle them and the issues they work
  on, under "Pull requests and issues", after the section on descriptions
  and before the project's instructions. The tracker's lines follow it, in
  the same section. Like descriptions, it comes whether or not the project
  asks agents to open a pull request when they finish; agents that may
  only comment or read, and review agents, don't get it.
- **fctl's defaults are short.** The repository's own instructions come
  first where they say otherwise; then a rule each: read an issue with its
  comments before starting, and check no open pull request does it
  already; `Fixes #12` only for everything an issue asks, otherwise
  `Refs #12` and a comment on the issue; wait for the checks after a push
  (`gh pr checks --watch --fail-fast`) and fix what the changes broke;
  answer each review comment in its thread; merge only when asked, and
  push nothing to a merged pull request's branch. They name GitHub's
  syntax as examples; the tracker's lines name the tracker's own.
- **A setting per project.** `pull_request_instructions` in a project's
  settings: absent for fctl's defaults, the project's own text in their
  place, or empty for none. Settings → Agents shows the text agents get,
  fctl's to start from, and the CLI changes it with `fctl projects set
  --pr-instructions` (or `--pr-instructions-file`) and prints it with
  `fctl projects show --pr-instructions`. It is always sent, as `null`
  for the defaults, and an update that leaves it out keeps what was
  stored, so that an older client can't reset it.
- **Defaults stay fctl's.** A project that keeps the defaults stores
  nothing, and the control plane stores a text that says what they say as
  nothing, so that the defaults reach it as fctl improves them. The text
  lives once, in `fctl_proto::session`; `cargo xtask gen` writes it for
  the web app, so that the settings show what agents are told.

## Consequences

- Agents in every project wait for checks, reference issues by how much
  they did, and answer reviews in their threads, unless the project or the
  repository says otherwise. Repositories with rules of their own keep
  them.
- The defaults' wording reaches agents with the agent's version, as the
  rest of the system prompt does: hosts one version behind tell the
  previous wording, and hosts from before the setting tell none of it.
- A project that copied the defaults and changed them keeps its copy when
  fctl's change; one that only reads them gets the new ones.
