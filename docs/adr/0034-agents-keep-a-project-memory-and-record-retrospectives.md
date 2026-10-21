# 0034: Agents keep a project memory and record retrospectives

2026-10-21 · accepted

## Context

Every session starts in a new conversation, and most in a new sandbox. What
an agent learns that the repository doesn't say (that the e2e tests need
Chromium installed first, that the staging bucket has no credentials in the
project's secrets, that people decided in a message to merge migrations by
Wednesday) goes with its sandbox, and the next agent learns it again, from
the same failure or from a person repeating it. The repository's AGENTS.md
can hold it, but only through a pull request someone reviews, and much of
it is about how the project's tools and people behave, not about the code.
Claude Code's own auto memory lives in the sandbox's `~/.claude/projects/`,
so it lasts as long as the sandbox, is one person's by design, and its
system prompt calls it persistent.

People also wanted a record of each piece of work: what the agent worked
on, the approach it took, what got in the way, and what would have helped
to know, both to follow sessions and, later, to learn from many of them.

## Decision

- **A memory per project, on the control plane.** D1 keeps notes
  (`memories`) of at most 2,000 characters, at most 500 a project, each
  naming the session whose agent wrote it or the person who did. Everyone
  in the project reads them; editors add, change and delete them, which the
  workspace's audit log records.
- **Agents start with it.** Each run's spec carries the current notes,
  newest first, as many as fit in 24 KB and 200 notes, about what Claude
  Code loads of its own memory, and fctl-agent lists them in `~/AGENTS.md`
  with how to keep them. `fctl memory` lists them as they are now, others'
  new ones included. They are presented as notes, not instructions: the
  person's request and fctl's rules come first.
- **Agents mark notes stale; people delete them.** An agent adds notes, and
  marks one stale with a reason (or adds one `--replaces` it), but neither
  changes nor deletes one: a mistake stays visible, with who said so, until
  a person cleans up. Stale notes stay out of what agents start with.
- **One memory that lasts.** Where an agent keeps the project's memory,
  fctl-agent sets `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`, so that Claude Code
  offers no second memory that vanishes with the sandbox. fctl doesn't sync
  Claude Code's memory files instead: their format and their kinds (`user`,
  `feedback`) are one person's, and a project's sessions belong to many.
- **Retrospectives have parts.** `fctl retro` records how the work ended
  (`completed`, `partial` or `blocked`) and four parts: what it worked on,
  the approach, what got in the way and what a future agent should know.
  Lists count and filter them by outcome, and an insight becomes a note
  with a click. At most 50 a session; they go with it.
- **Asked for, not forced.** `~/AGENTS.md` asks for one retrospective per
  piece of work, as a turn that finishes it ends. Nothing blocks the turn
  for one (a Claude Code Stop hook could): a missing retrospective costs
  less than a turn that can't end.
- **One way for agents to ask the control plane.** `fctl memory` and `fctl
  retro` send an `AgentCall` over the session socket; fctl-agent hands it
  to its host, which sends it with its runner token (`POST
  /api/v1/runner/sessions/{id}/calls`) and hands back the `AgentAnswer`,
  as for published screenshots ([ADR 0014](0014-agents-publish-screenshots-at-unguessable-links.md)).
  Sandboxes hold no fctl credentials, and what agents ask next needs no new
  frame. The transcript shows each note and retrospective as a notice, not
  a new event kind, so older clients show the message.
- **On by default, per project.** `project_memory` and `retrospectives` are
  project settings, on in new projects and in those from before them. Off,
  the spec says nothing, agents hear nothing, and the control plane refuses
  their calls; what was kept stays for people.

## Consequences

- The memory is a channel between sessions that no review stands in: text
  in a repository or on a web page that steers one agent can leave a note
  that later agents start with. The framing in `~/AGENTS.md`, each note's
  session, the transcript's notice and people's deletions are the checks;
  projects whose agents read untrusted text can turn it off. The security
  docs say so.
- Notes are as good as agents' judgment, and the prompt budget keeps the
  newest: older notes reach agents only through `fctl memory`, so people
  prune them.
- Sessions on hosts from before it, and in sandboxes whose fctl lacks the
  commands, keep no memory and record no retrospectives.
- app.fctl.sh keeps what agents and people write in notes until someone
  deletes them or the project, and retrospectives as long as their
  sessions; fctl.sh's privacy policy says so.
- Doing more with the retrospectives is left for later: a session that
  reads a project's retrospectives and proposes notes or AGENTS.md changes,
  or trends across projects.
