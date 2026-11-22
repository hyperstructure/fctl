# 0092: Projects choose how long stale memory notes stay

2026-11-22 · accepted

## Context

A project keeps at most 1,000 memory notes, stale ones included, and no
agent starts with a stale note. Since
[ADR 0091](0091-autopilots-delete-stale-memory-notes-once-they-crowd-the-memory.md)
an autopilot deletes the notes stale for over a day once a memory holds
900 notes, and people delete every stale note at once; a project without
an autopilot still fills up unless someone remembers to. That record left
fctl deleting stale notes itself for later.

Carl Oscar asked whether ordinary agents could delete stale notes, so that
a weekly schedule would clean up, and chose instead that fctl do it, after
a number of days each project chooses: some want a stale note gone after
a day, others after a week.

## Decision

- **A project setting, `stale_note_days`**, 1 to 365: fctl deletes a note
  once it has been stale that many days. Absent, the default for new
  projects and those from before it, stale notes stay until someone
  deletes them, as before. Maintainers and owners set it (Settings →
  Agents → Memory, `fctl projects set --stale-note-days 7|never`), as
  they do the project's other settings, and an autopilot with the
  settings power may too.
- **The hourly job deletes them** (`memories::expire_stale`), with the
  other housekeeping: no agent, no Claude run, no schedule. Deleting a
  stale note needs no judgment, as an agent or a person already judged it
  stale, and a session that only runs one statement would spend a
  subscription's usage on it.
- **The audit log says so, as fctl's own doing**: one entry per project and
  run with how many notes went and after how many days, with no person as
  its actor.
- **Ordinary agents still delete nothing.** No sandbox gains a way to
  delete notes, so an agent that text steers can't make one go; the
  setting is people's.

## Consequences

- Turning it on in a project with many old stale notes deletes them within
  the hour. A note marked stale by mistake goes once the days pass, unless
  someone makes it current again first; one day gives the least time to
  notice.
- How long app.fctl.sh keeps a stale note now depends on the project, which
  fctl.sh's privacy policy says.
- Hosts, agents and CLIs from before it don't know the setting, and a
  client that leaves it out of an update keeps the stored value
  (`fctl_domain::settings::keep_unsent`); the API always sends it, as
  `null` for never, so that a client sending back what it read keeps that.
