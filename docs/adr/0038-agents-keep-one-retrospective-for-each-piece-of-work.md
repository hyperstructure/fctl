# 0038: Agents keep one retrospective for each piece of work

2026-10-22 · accepted

## Context

Agents record a retrospective as a turn that finishes a piece of work ends
([ADR 0034](0034-agents-keep-a-project-memory-and-record-retrospectives.md)),
and `~/AGENTS.md` gave each follow-up that changes the work (review
comments, a new request) one of its own. Sessions that carry on after
their first pull request, through review rounds, failed checks or a
person's answer to a blocked agent, piled up retrospectives that each told
part of the story: a blocked one, then a completed one that said little of
what blocked it, and the project's counts kept the blocked one.

People wanted one retrospective for each piece of work, not one for each
turn, brought up to date as the work goes on, above all a partial or
blocked one once the work is done or unblocked. A session that turns to
different work should still record another. Agents could only record
retrospectives; people delete them.

## Decision

- **A retrospective is of a piece of work.** `~/AGENTS.md` asks for one
  for each piece of work, not for each turn. When a later turn carries on
  with work the session recorded one for (review comments, a failed check,
  the answer that unblocks it, the rest of a partial one), the agent
  updates that one, so that it tells the whole piece of work as it now
  stands. It records another when the session turns to different work.
- **Agents update their own session's, in place.** `fctl retro update
  rtr_…` replaces the outcome and the parts it gives (an empty one leaves
  its part out) and keeps the others. The control plane changes a
  retrospective only for the session that recorded it, under the same
  `retrospectives` setting, and keeps when (`updated_at`); updates don't
  count toward the 50 a session records. `fctl retro` prints the id it
  recorded, and `fctl retro list` in a sandbox lists the session's own, so
  that an agent that lost its conversation (a resume on another runner, a
  compacted context) finds it.
- **Unlike notes.** Agents still neither change nor delete notes: a note
  speaks to every later session, and a mistake in one stays visible until
  a person cleans it up. A retrospective is one session's account of its
  own work, and its transcript keeps each version: the `retrospective`
  notice as it was recorded, then a `retrospective_updated` notice with it
  as updated.
- **Lists keep their order.** The project's list stays in the order
  retrospectives were recorded, so its pages, after an id, stay as they
  are; each retrospective says when its agent last updated it, and the
  counts by outcome count each piece of work once, as it now stands.
- **New calls, not new fields.** Updating and listing are `AgentCall`s of
  their own (`update_retrospective`, `retrospectives`), not an option of
  `retrospective`: an fctl-agent or control plane from before them refuses
  them rather than record a second retrospective. fctl-agent asks for
  updates only where the sandbox's fctl has `fctl retro update` (it asks
  `fctl retro update --help`); elsewhere `~/AGENTS.md` still asks for one
  for each follow-up.

## Consequences

- A session that goes through review rounds, or waits on a person, ends
  with one retrospective whose outcome is the work's, and the project's
  counts of blocked and partly done work fall as agents finish it.
- An updated retrospective stays where it was first recorded in the
  project's list, saying when it was updated, rather than rising to the
  top.
- An update replaces what lists show; what it said before stays only in
  the transcript.
- Agents judge what counts as the same piece of work. One that records
  another for a follow-up, or updates one with different work, leaves
  what agents left before this, or a retrospective people can delete.
- Sessions on hosts or in sandboxes whose fctl is from before it record
  one for each follow-up, as they did.
