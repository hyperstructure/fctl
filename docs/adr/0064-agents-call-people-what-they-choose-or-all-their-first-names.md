# 0064: Agents call people what they choose, or all their first names

2026-10-28 · accepted

## Context

Agents never learned what to call people. The system prompt didn't name
the session's starter, and a message from anyone else began with their
full name (`Carl Oscar Aaro wrote:`), as do the tasks that Linear starts
(`Carl Oscar Aaro delegated ENG-1 to fctl in Linear`). Agents took the
first word for the first name and wrote "Carl" to someone who goes by both
of their first names, Carl Oscar. fctl's own greeting had stopped doing
that when profiles got first names, all of them, apart from the last name
(migration 0003), but agents never saw those.

Session #452 was asked to let people tell agents what to call them, so
that agents don't take one of several names for the whole.

## Decision

- **People choose a name for agents.** `preferred_name` holds it ("What
  agents call you" in Settings → Profile, `fctl account set
  --preferred-name`): a name like the others, at most 80 characters on one
  line, which an empty one clears. Without one, agents call people by their
  first names, all of them (`given_name`). An account an older client made
  with only a full name takes its words but the last for those, as
  Settings → Profile shows them and the greeting took them already
  (`fctl_domain::validate::called`); a deleted account gets no name.
- **The system prompt names the starter.** FleetDO puts what to call them
  in the spec (`author_called`), and the system prompt says who started
  the session and what to call them: "the whole name rather than a part of
  it" when it has several words.
- **Messages from others say it too.** A person's message to a session
  someone else started carries what to call them in its delivery
  (`author_called`), and fctl-agent writes it after their name:
  `Carl Oscar Aaro (call them Carl Oscar) wrote:`. The starter's own
  messages need no header, as before; fctl-agent now tells them apart by
  id rather than by name, so a starter who renames themselves stays the
  starter.
- **fctl uses it as well.** The Agent Floor greets people with it, and
  fctl's replies to people in Linear ("Carl Oscar, connect your Claude
  subscription in fctl first…") address them with it.

## Alternatives

- **First names alone, without a setting.** That calls Carl Oscar by both
  names, since their profile has both, but not someone who goes by a
  nickname, a middle name or one of their first names.
- **Free-text instructions about oneself** ("call me Carl Oscar, and reply
  in Swedish"): what goes in a message's header, or in the system prompt
  of someone's sessions, would then be instructions from one person to
  another's agent. A name is one short line, and says only what it says.
- **A field on `UserRef`**, which names people across the API: every list
  of sessions, notes and members would carry it, though only agents and
  greetings use it.

## Consequences

- People with several first names are called by all of them without doing
  anything.
- What agents call someone goes to the Claude account of each session they
  write in, as their full name did already.
- Sessions that started earlier, and those whose host or agent is older,
  see full names alone until they next start or resume on a newer one.
- Agents still meet people's names elsewhere, as GitHub, Linear's prompt
  context and commit trailers give them; the system prompt and the
  headers say what to call the people fctl knows.
