# 0007: Attachments live in a Durable Object per project

2026-10-18 · accepted

## Context

People want to show agents what they mean: a screenshot of a bug, a design,
a log, a PDF. Files have to reach the sandbox, where Claude can read them,
and stay viewable in the transcript. They can be large (screenshots from a
Retina display are several megabytes), and a session's first message is
written before the session exists.

Messages travel as deliveries: stored in the session's Durable Object until
the agent confirms them, and sent over WebSockets. Rows there hold at most
2 MB, so files cannot ride inside deliveries. R2 is built for files but
would add a bucket to every instance and an R2 permission to the deploy
token. D1 would mix large blobs into the relational database and its
10 GB limit.

## Decision

Each project has a `FilesObject`, a Durable Object that keeps attached files
in its SQLite storage in 1 MiB chunks. A file is uploaded on its own (the
request body passes through the Worker unread) and a message, a new session
or a resume sends it by id; that claims it for the session. Uploads no
message claims are deleted after a day; deleting a project purges its
files like its sessions' event logs.

The host fetches a message's files with its runner token before it forwards
the message, in order, and hands them to fctl-agent as `file` frames. The
agent saves them in the sandbox, outside the repository, and lists them in
Claude's turn; images Claude accepts as they are also go into the turn.

## Consequences

- Instances need nothing new: Durable Objects are already deployed, and the
  class arrives with a Durable Object migration.
- A project holds at most 10 GB of files (the per-object limit). Files are
  addressed by id through the control plane, so moving them to R2 later
  changes no client.
- Viewers of a session read its files; nobody reads another session's
  files through it.
- Hosts and agents announce an `attachments` feature. Older ones keep
  working; their agents get messages without the files, and the transcript
  says so.
