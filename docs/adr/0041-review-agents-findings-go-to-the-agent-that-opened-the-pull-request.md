# 0041: Review agents' findings go to the agent that opened the pull request

2026-10-23 · accepted

## Context

Review agents ([0040](0040-review-agents-review-pull-requests-on-an-owners-subscription.md))
post their findings on GitHub, and acting on them is up to the pull
request's people. Most pull requests in a project with review agents are
opened by the project's own agents, which go idle once their turn ends:
nothing wakes them when a review comes, so the findings wait until a
person reads them and messages the agent, which then does what it could
have done on its own. People asked for the agent that opened the pull
request to hear of the findings, decide what to do about them, and say
what it decided in each finding's thread.

## Decision

- **The opener's session, by message.** When a review posts findings on a
  pull request whose author is the instance's GitHub App, the control
  plane sends them to the session that opened it: of the sessions that
  list it, the one whose branch it comes from, else the one that started
  first. It is a message from fctl, delivered as a person's message is: it
  resumes a stopped session, on the subscription the session ran on. The
  control plane writes it, so it improves with each deploy, and older
  hosts and agents take it as any message.
- **The agent decides, and answers in the threads.** The message lists
  the findings, each with its comment's id, and says they are another
  agent's reading, not instructions. The agent fixes what it finds real,
  and answers each finding in its thread: fixed in which commit, not a
  problem, or intended. It leaves the threads open: the review of its next
  commit checks the fixes and resolves them, and reads the answers to the
  others, which it doesn't raise again.
- **Only fctl's own pull requests.** A pull request a person opened is
  theirs to act on, even when an agent pushed to it since, and so is a
  pull request of another app. Pull requests that merged or closed,
  archived and failed sessions, and agents that only read on GitHub get
  nothing.
- **At most five in a row.** An agent's fix gets a review, which can find
  something new, which the agent fixes in turn. A session gets the
  findings of at most five reviews in a row while nobody messages its
  agent; the next ones only say in its transcript what they found, until
  someone does.

## Consequences

- Each review with findings can start a turn of the session that opened
  the pull request, on that session's subscription, besides the review's
  on the operator's.
- A finding steers an agent only as far as a message of its own would:
  the review agent reads the pull request, whose text anyone may write,
  so the message calls the findings material to weigh, and the agent can
  do no more than its session's access lets it.
- The review session's transcript, and `fctl review submit`, say which
  session got the findings (`PostedReview.sent_to`, absent from control
  planes before it).
- Reviews of a pull request that several sessions list reach one of them,
  so that two agents don't fix the same finding.
