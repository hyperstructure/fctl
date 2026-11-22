# 0091: Autopilots delete stale memory notes once they crowd the memory

2026-11-22 · accepted

## Context

A project keeps at most 1,000 memory notes, stale ones included
([ADR 0034](0034-agents-keep-a-project-memory-and-record-retrospectives.md)
said 500; fctl#399 made it 1,000). Agents mark notes stale rather than
delete them, and a session that tidies the memory, as autopilots have
sessions do, merges overlapping notes into one and marks the rest stale:
no agent starts with a stale note, but each still takes a place. ADR 0034
left the deleting to people, so that an agent's mistake stays visible, with
who made it, until a person cleans up.

People rarely do. One autopilot's project reached 1,000 notes, about 940
of them stale, and from then on `fctl memory add` refused every session's
note, its own tidy session's too, which was left with notes it could no
longer shorten. Deleting being for people, the autopilot asked them
whether it might delete the stale ones, and waited. Carl Oscar asked that
an autopilot delete stale notes itself when they are too many, without
asking.

## Decision

- **An autopilot deletes its projects' stale notes, never a current
  one.** Through its session's relay, `DELETE
  /api/v1/projects/{id}/memories/stale` (`fctl memory delete --stale`)
  deletes them all at once, as one request, one line in its log and one
  entry in the audit log, rather than a thousand. It deletes no single
  note (`DELETE /api/v1/memories/{id}` stays refused): which stale note
  goes doesn't matter, as no agent reads them.
- **Only those stale for over a day.** People who follow along then have a
  day to see a note an agent marked stale by mistake, under Stale or in
  the transcript's notice, and make it current again; and no note goes in
  the same breath as it goes stale, which marking a current note stale
  and deleting it at once would otherwise allow. The rule is the control
  plane's (`memory::stale_deleted_until`), not the autopilot's to follow,
  and the answer says how many stale notes stay.
- **Its brief says when.** Once a memory of its projects holds 900 notes
  (`memory::CROWDED`), its brief says so whatever parts it shows, with how
  many are stale and how many of those it may delete, and tells it to
  delete them without asking; when the current notes crowd the memory
  alone, to have a session tidy them. Its briefing says the same as a
  rule. Below 900 it hears nothing, and stale notes stay for people, as
  before.
- **People delete stale notes at once too.** The same request from a
  person who can edit the project deletes every stale note, without the
  day: they decide, and one note at a time was the only way before.

## Consequences

- What ADR 0034 kept visible until a person cleaned up now lasts at least
  a day in a crowded memory. The text of a deleted note stays in the
  transcripts that showed it, and the audit log keeps how many went and
  when, as the person the autopilot acts for.
- An autopilot, or a prompt injection that steers it, can delete what
  people meant to keep for the record by marking it stale. It could
  already mark every note stale; now, a day later, those can go for good.
  Projects whose autopilot reads untrusted text weigh that, as they do
  its other powers.
- Projects without an autopilot still fill up: their people delete stale
  notes, now all at once. fctl deleting the oldest stale notes itself as
  the memory fills would spare them that, and is left for later.
- A memory whose current notes alone crowd it still fills; the brief has
  the autopilot get them tidied, which marks notes stale that it then
  deletes a day later.
