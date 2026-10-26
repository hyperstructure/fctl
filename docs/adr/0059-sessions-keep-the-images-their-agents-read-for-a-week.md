# 0059: Sessions keep the images their agents read for a week

2026-10-26 · accepted

## Context

Agents look at images: a screenshot they took of the app they changed,
which Claude Code's `Read` passes to Claude, or a browser tool's
screenshot. The image travels in the tool's result, in base64, and the
control plane stored Claude's messages without it, since nothing showed
it and cut short it was useless. So the session page said "Read
after.png · 1 lines read", and people following the session couldn't see
what the agent saw, as Claude's own apps show them. Only the screenshots
an agent published for a pull request
([ADR 0014](0014-agents-publish-screenshots-at-unguessable-links.md))
showed, to anyone with their link.

The person who asked wanted the images shown up to a reasonable size, kept
for a limited time, about a week, and only to people who can see the
session.

## Decision

- **SessionDO keeps them.** As it stores a Claude message, it keeps each
  image in the results of the agent's tools, its subagents' too, in its own
  SQLite storage, in 1 MiB chunks: the images belong to the session, its
  access is theirs, and deleting the session's storage deletes them. The
  message arrives with the image already, so nothing new travels from the
  sandbox, and SessionDO writes it synchronously before the event, which a
  call to the project's FilesObject would not.
- **Only what Claude takes.** PNG, JPEG, GIF and WebP images, as their bytes
  say, of up to 5 MB: about the largest Claude takes. Images people send
  are attachments ([ADR 0008](0008-attachments-in-a-durable-object-per-project.md)),
  kept as such; PDFs and other documents aren't kept.
- **The transcript says where.** The stored event goes without the image's
  bytes, as before, and with a `KeptImage` in the image's `source`, as
  `kept`: its id, type, size, pixel size and when it expires. Claude's
  messages are stored as Claude Code wrote them, so this needs no new event
  kind and no protocol change: hosts and agents of any version send what
  it needs. What runs in the sandbox may pose as the agent, so SessionDO
  drops any `kept` an event comes with.
- **For the people who can see the session.**
  `GET /api/v1/sessions/{id}/images/{image}` checks that the caller can,
  as the transcript does, and answers with the image, `nosniff`, sandboxed
  by its Content-Security-Policy and cached privately until it expires.
- **For a week, and a bounded amount.** SessionDO's alarm deletes each
  image 7 days after the agent read it. A session keeps at most 1,000
  images and 200 MB at once, its oldest going first, so that what anything
  in a sandbox sends stays bounded.
- **Small, where it was read.** The web app shows each image under the line
  of the call that read it, and a folded run shows the images its calls
  read under its own line; a click opens them in the transcript's
  lightbox, with its attachments and published screenshots. Once an image
  is gone, the call says what it read.

## Consequences

- Transcripts show what agents looked at for a week. Older events, and
  those of images no longer kept, say what the call read without showing
  it.
- The control plane stores more: the images agents read, for 7 days, in
  each session's Durable Object. fctl.sh's privacy policy says so.
- Stored events lost Claude Code's own copy of what `Read` read
  (`tool_use_result.file.base64`) too, which was cut to 128 KB of noise.
- Anyone who can see a session can see what its agent read, as they can
  read the rest of its transcript: tool output can hold anything the
  sandbox has. Sharing a session shares its images.
- Published screenshots are unchanged: they stay the way to show an image
  outside fctl, in a pull request.
