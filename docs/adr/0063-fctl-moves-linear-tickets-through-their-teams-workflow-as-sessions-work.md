# 0063: fctl moves Linear tickets through their team's workflow as sessions work

2026-10-28 · accepted

## Context

[0052](0052-projects-choose-a-ticket-tracker-and-linear-delegates-tickets-to-fctl.md)
had a delegated ticket move to its team's first started state as its
session starts, as Linear asks of agents, and left the rest to Linear's
GitHub integration, which links a session's pull requests by its branch
and moves tickets on as they open and merge; agents left states alone.

The person who started session #448 found tickets left in Backlog or Todo
while agents worked on them, and asked for a ticket to move to In
Progress as an agent starts on it, to In Review once its pull requests
are up, and to Done once it is complete.

Linear starts the thread of every delegation's agent session with a
comment of its own, "This thread is for an agent session with fctl.",
which the delivery carries as the session's comment, as other agents
built on Linear found. fctl took any comment for a mention, so no
delegated ticket ever moved, and agents heard that the person mentioned
fctl with those words. The stand-in for Linear sent no comment, so the
smoke test passed. And Linear's GitHub integration, where a workspace
has it, moves a linked issue to In Progress as a pull request opens and
to Done as it merges, unless a team configures otherwise: never to In
Review by default.

## Decision

- **Delegations and mentions are told apart by Linear's words.** A
  session whose comment is the one Linear starts a delegation's thread
  with is a delegation; one whose comment is a person's, or made from a
  comment in another thread (`sourceCommentId`), is a mention
  (`fctl_domain::linear::AgentSession::asking`). The stand-in starts a
  delegation's thread as Linear does.
- **The control plane moves the ticket of a session that started from
  it, as fctl's app, at three moments**, each to a state of the ticket's
  team (`fctl_domain::tickets::next_state`), only forward in its workflow
  and never out of done or canceled:
  - its first started state (In Progress): a delegated ticket as its
    session starts, also when the delegation reaches a session that works
    on it already; a mentioned one once its agent changes something,
    since a mention may only ask a question;
  - its review state, the first started one whose name says review (In
    Review), or else its first started one: when a turn ends with a pull
    request of the session open for review, not a draft;
  - its first completed state (Done): once the session's pull requests
    merged and none is open any more, when one of them completes the
    ticket as Linear's GitHub integration reads it: words that close it
    before its key in its title or description (`Fixes ENG-123`), not
    others (`Part of ENG-123`, `Related to ENG-123`), and without any,
    its key in its title or branch, which fctl names after the ticket
    (`fctl_domain::tickets::completes`). The control plane reads their
    titles, descriptions and branches from GitHub as they merge, of those
    the session may look up.
- **Each move has a project switch** (`LinearTickets::move_started`,
  `move_review`, `move_done`; `fctl projects set --linear-start`,
  `--linear-review`, `--linear-done`; the Tickets tab, which names the
  states fctl found in the team's workflow), on by default.
- **Agents do what fctl can't see.** A session's agent is told what fctl
  moves and to leave the state alone otherwise, and to move a ticket that
  needs no pull request to Done itself once it has done all it asks.
  Sessions that didn't start from a ticket don't tell fctl which ticket
  they work on, so their agents are told to move the tickets whose work
  their task is themselves, with `fctl ticket update`, as the project's
  switches allow.
- **The transcript says each move** (`ticket_moved`).

## Alternatives

- **Agents move every state themselves, as told.** No new machinery, but
  a pull request merges when no agent runs, and an agent forgets.
- **In Review as the pull request opens.** Agents open it and then wait
  for its checks in the same turn; and Linear's GitHub integration moves
  a ticket to In Progress as it opens, which a move at the same time
  races. The turn's end comes after both.
- **Done as any pull request merges.** A pull request that does part of a
  ticket (`Part of ENG-123`) would close a ticket with work left.
- **Reading the team's own automation in Linear to pick states, or to
  stand back where Linear moves tickets.** Linear's schema gives the
  team's automation states a type they don't have, and the older fields
  are deprecated; the names fctl finds, and a switch, are plainer.
- **Choosing the states in the project's settings.** More to configure
  for every project; a new Linear team's workflow has In Progress, In
  Review and Done, and the switches show what fctl picked.

## Consequences

- Delegated tickets move to In Progress again, and their agents hear that
  the ticket was delegated rather than that someone mentioned fctl.
- Where Linear's GitHub integration moves tickets too, both act: on
  Linear's defaults they agree on Done, and In Review comes from fctl
  after Linear's In Progress. A team whose automation moves tickets
  elsewhere as pull requests merge, to a QA state say, turns fctl's
  switch off.
- A merge of a ticket session's pull request costs one lookup on GitHub;
  each move one read and one write on the Linear installation, and a
  turn's end of a session that came further one read. fctl stores
  nothing new.
- A mention that only asks moves nothing; a ticket in a state further
  along stays there.
