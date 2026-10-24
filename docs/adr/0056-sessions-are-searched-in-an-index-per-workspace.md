# 0056: Sessions are searched in an index per workspace, in a Durable Object

2026-10-24 · accepted

## Context

What a session said scrolls away, and people can't find it again (#11).
The session page loads a session's events a page at a time as the reader
scrolls up, and folds most of what it shows: a run of tool calls into a
line, thoughts, a subagent's work, what fctl told the agent. A browser's
find sees only what has loaded and is open. Across sessions there was
nothing: ⌘K filters the titles of the 50 newest sessions.

#11 proposed SQLite's full-text search (FTS5) in D1, over a compact text
projection that SessionDO writes. D1 can't export a database that holds a
virtual table, and an FTS5 index is one: `wrangler d1 export`, which
docs/self-hosting.md gives for a copy, would fail on every instance, and
the workaround (drop the index, export, build it again) is no backup
procedure. A transcript's text also grows faster than D1's relational
state, under one 10 GB limit with it.

## Decision

- **What is searched.** A session's title and description, people's
  messages (and fctl's) and the agent's replies: what the session is
  about, without its tool calls, what they returned, its thoughts or its
  subagents' work (`fctl_domain::search`). Each is a document, at most its
  first 10,000 characters. Review agents' sessions aren't searched: their
  project's reviews list them.
- **How it matches.** As typed, but for case, accents and how much
  whitespace separates words: a search for `login redirect` finds
  "the Login  redirect", not "the redirect after login". It looks for text
  of two characters or more.
- **Where the index lives.** Each workspace has a `SearchObject`, a
  Durable Object whose SQLite holds its sessions' documents and an FTS5
  index of them with the `trigram` tokenizer (which finds any text of
  three characters or more, and with `remove_diacritics` ignores accents);
  shorter searches look through the documents. SessionObjects put new
  documents there from their alarm, five seconds after they come, and a
  session's when its title or description changes; they take theirs out
  when their project is deleted. D1 says which sessions' documents the
  index has all of (`sessions.search_indexed`); the hourly job asks the
  others, those from before first of all, to index what they hold.
- **Who sees what.** A search (`GET /api/v1/search`) asks the index of
  each workspace of the caller's for the sessions of the projects they see
  there and the sessions shared with them, then reads those sessions from
  D1 and checks the caller's role in each as lists do. A limited access
  token searches what it reads. Archived sessions are found, and say so.
- **On the session page**, finding (⌘F) runs in the browser, over all the
  transcript shows, folded or not: what search looks in, and the lines of
  the agent's work, the commands behind them, its thoughts and its
  subagents. It matches as search does, opens what folds a match, and
  loads earlier activity to look in it too. A search's result opens its
  session there, at the event it was found in.

## Consequences

- D1 stays exportable. The index holds nothing that its sessions don't:
  should it be lost, setting `search_indexed` to 0 has the hourly job
  build it again.
- Instances get a new Durable Object class (`SearchObject`, migration
  `v5`), which deploys and the standalone server pick up from the
  configuration.
- The text of messages and replies is stored twice, and a trigram index
  of it takes about three times its size again, in each workspace's
  object (at most 10 GB each).
- A search costs a request to each of the caller's workspaces' objects,
  which answer at once.
- Sessions from before are searchable once the hourly job got to them, 200
  an hour, the most recently active first; until then their titles aren't
  found either, though ⌘K still lists the newest 50 by title.
- Searching for words in any order, rather than as typed, would take
  another query on the same index.
