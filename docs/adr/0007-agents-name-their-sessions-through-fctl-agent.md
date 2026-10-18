# 0007: Agents name their sessions through fctl-agent

2026-10-18 · accepted

## Context

A session's title is the first line of its task, so the session lists
showed prompts ("Work on issue https://…") instead of the work. The agent
knows best what it is working on, but it runs in a sandbox that holds no
fctl credentials, and the people following a session may rename it too.

## Decision

- fctl is installed in the default sandbox image, and `fctl edit --title …
  --description …` run in a sandbox without naming a session edits the one
  the sandbox runs. It talks to fctl-agent over a second unix socket beside
  the host's (the session socket, `FCTL_SESSION_SOCKET`), not to the
  control plane: no fctl token enters the sandbox, and a sandbox can only
  ever reach its own session. A session-scoped API token was the
  alternative; it would have added a credential to every sandbox and a
  restricted kind of caller to every API endpoint.
- fctl-agent keeps the title and description in the session state it
  reports (`SessionState.title`, `.description`), which the host passes on
  and sends again after every reconnect. A new event kind would have put
  the change in the transcript, but older CLIs cannot read event kinds
  they do not know, and the agent's `fctl edit` call is in the transcript
  anyway.
- SessionDO applies an agent's title or description only when it differs
  from the agent's previous report, validates it again, and writes it
  through to D1. A person's rename stands until the agent renames the
  session again.
- fctl-agent tells Claude when to do it in `~/AGENTS.md`, beside the
  checkout, imported from Claude's user memory (`~/.claude/CLAUDE.md`).
  User memory loads in every repository, also when fctl ignores a
  repository's settings, and unlike a CLAUDE.md above the checkout it does
  not make Claude skip a repository's own AGENTS.md. fctl-agent writes the
  file only when the sandbox's fctl can edit sessions.

## Consequences

- Session lists show what agents work on, and change when the work does.
- Sessions have a description, which their owners can edit as well
  (`fctl edit my-app#12 --description …`, `PATCH /sessions/{id}`).
- Hosts need `fctl host image build` once to get fctl into the default
  image; custom images need fctl on the PATH, or their sessions keep their
  first titles.
- The session socket is the first channel from programs in a sandbox to
  fctl-agent. Anything the agent runs can use it, so what it accepts stays
  narrow: today, the session's title and description.
