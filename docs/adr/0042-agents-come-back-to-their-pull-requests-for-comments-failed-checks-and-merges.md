# 0042: Agents come back to their pull requests for comments, failed checks and merges

2026-10-23 · accepted

## Context

Agents open pull requests, and their sessions go idle once the turn ends,
then stop. Nothing brought them back
([#8](https://github.com/hyperstructure/fctl/issues/8)): people comment
on GitHub, CI fails after the turn, a merge leaves work (the issue it
fixes, the release that ships it), and each waits until a person goes to
the session and messages the agent with what GitHub already said. Only
review agents' findings came back, to the session that opened the pull
request ([0041](0041-review-agents-findings-go-to-the-agent-that-opened-the-pull-request.md)).

People asked for projects to bring their agents back for comments on the
pull requests they opened, failed checks and changes of state: for
comments only from people the repository trusts, so that nobody else can
steer an agent through a public repository, only from people rather than
apps, and only for comments that address the agent: `@fctl` or `@agent`
anywhere, or `fctl:` or `agent:` (or a comma) at the start.

## Decision

- **A project setting with three switches, all off.**
  `ProjectSettings::follow_ups`: comments that address the agent, failed
  checks, and merged or closed. Maintainers change it as the project's
  other settings, in its Settings or with `fctl projects set
  --follow-ups`. An update that leaves it out keeps what was stored.
- **The session that opened the pull request, by message.** As for
  review agents' findings: only pull requests whose author is the
  instance's GitHub App, and of the sessions that list one, the one on its
  branch, else the one that started first. What happened reaches it in a
  message from fctl, delivered as a person's message is: it resumes a
  stopped session on the subscription it ran on. Archived and failed
  sessions, archived projects and agents that only read on GitHub get
  nothing. The control plane writes the message, so it improves with each
  deploy, and older hosts and agents take it as any message.
- **Trust is write access, as GitHub says.** A comment counts when a
  person wrote it (not an app's bot, fctl's own review agents included),
  its `author_association` is `OWNER`, `MEMBER` or `COLLABORATOR`, and
  GitHub's permission endpoint then says they may write to the repository.
  A member of an organization may only read a public repository of it,
  and making an agent that pushes do something takes the access to push
  it yourself. The association alone costs nothing to check and keeps
  everyone else from costing a call.
- **Addressing is deliberate.** `@fctl` and `@agent` count as GitHub's
  mentions do (not in an address or after a login, as `@fctl-sh`), and
  `fctl:`, `agent:`, `fctl,` and `agent,` at the start of the first line
  that says something, in any case. Quoted lines, code and HTML comments
  don't count: people quote a request, and write `@fctl` in backticks to
  say how to make one. Edited comments don't count either. A review that
  addresses the agent brings its comments on lines.
- **Gathered, then passed on once.** The session keeps what comes for ten
  seconds, so that a review and its comments, which come as deliveries of
  their own, or checks that fail together, reach the agent in one message.
  Each comment, failed commit and merge reaches it once.
- **Failed checks are the newest commit's, three times in a row at most.**
  The lookup after a delivery about checks names the newest commit's
  failed checks (failed, timed out or couldn't start; cancelled runs and
  those waiting for approval are no failure of the code). They wait while
  a turn runs, since the agent may be at them, and are dropped when GitHub
  says a newer commit came, by anyone, or a re-run passed. After three in
  a row while nobody messaged the agent, or wrote to it on GitHub, the
  transcript says what failed instead, so that a fix that keeps failing
  doesn't spend a subscription for good.
- **People see what happened as GitHub has it.** The message carries
  what it is about (`UserMessage.follow_up`), which the transcript shows
  with people's avatars and the checks by name, with the message itself a
  click away. An `eyes` reaction on the comment tells its author that the
  agent will hear of it.

## Consequences

- A comment from someone who may write to the repository can start a
  turn on the subscription of the person whose session opened the pull
  request; they could push to its branch themselves.
- The GitHub App subscribes to three more events: **Issue comment**,
  **Pull request review** and **Pull request review comment**. It needs no
  new permission: it reads and writes issues and pull requests already.
  app.fctl.sh's app needs the events added.
- Each comment that passes costs a few GitHub calls: the permission check,
  a lookup for a comment on the conversation (which doesn't name the pull
  request's branches), and a token and a reaction once the session has it.
  The lookups after check deliveries ask for three more fields, at no cost
  to the rate limit.
- Merge conflicts bring no agent back: no delivery says when a pull
  request starts to conflict with its base.
- Only deliveries bring agents back. The hourly sweep that catches missed
  deliveries corrects pull requests' state without waking anyone, so
  turning follow-ups on, or a delivery that went missing, never wakes
  sessions for what happened long ago.
