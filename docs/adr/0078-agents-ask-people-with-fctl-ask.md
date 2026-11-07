# 0078: Agents ask people with `fctl ask`

2026-11-07 · accepted

## Context

An agent that needs a person to go on (a decision only they can make,
access, an answer) was told to say so plainly and stop. Its question then
sat at the end of its last message, and the session looked like any other
that waits for a reply: the Agent Floor, the sidebar and notifications
couldn't tell a session that finished its work from one that can't go on
without someone, and the person had to read the transcript to find what
was asked, then type an answer the agent had usually already spelled out
as choices.

The person who started the session that built this asked for a way for
agents to say that they need input, shown where people look first.

## Decision

- **`fctl ask "…" -o "…" -o "…"`** in a sandbox asks the people who follow
  the session: a question of at most 500 characters, and up to six answers
  to click, of at most 120 characters each. It goes through fctl-agent and
  the host to the control plane as `AgentCall::Ask`, like `fctl memory`,
  so no credential enters the sandbox ([ADR 0007](0007-agents-name-their-sessions-through-fctl-agent.md)).
  `fctl ask --withdraw` takes it back. A review agent asks nobody.
- **SessionDO keeps one open question** (`Session.question`), which it
  writes through to D1's `sessions.question`, so session lists carry it,
  and broadcasts it like any change to the session's summary. A newer one
  takes its place. **A person's next message answers it**: SessionDO drops
  it when it delivers a message with an author, whichever way the message
  came (the page, the CLI, Linear). An answer to click is that message,
  sent as it is.
- **Followers hear of it as the turn ends**, as a notification of its own
  kind (`notify::Kind::Asking`): "shop#231 asks you" with the question and
  its answers, in place of the last words of an ordinary finished turn. A
  question asked after the turn ended notifies at once. People who turned
  off notifications for sessions that wait for them don't get these
  either: it's the same news, told better.
- **fctl-agent writes a `question_asked` notice** into the transcript with
  the question, so that it stays there after it's answered, and tells
  Claude about `fctl ask` in its instructions where the sandbox's fctl has
  it, in place of "say so plainly and stop".

## Consequences

- The session page shows the question above the composer, with its
  answers as buttons; the Agent Floor, project pages and the sidebar show
  sessions that ask in blue, with the question; `fctl show` and `fctl
  sessions` print it.
- The question is plain text: it shows in notifications and lists, where
  Markdown wouldn't render. Details go in the agent's message.
- Hosts and fctl-agents from before it refuse the call, and `fctl ask`
  says to update the runner's fctl. Their agents keep the old instruction,
  since fctl-agent asks the sandbox's fctl whether it has `fctl ask`.
- One question at a time: an agent that asks again replaces its question,
  so people answer what it asks now.
