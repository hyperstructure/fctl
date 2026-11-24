-- Claude oversight (docs/architecture.md#claude-oversight, #627): whether
-- each Claude connection works, when a long-lived token runs out, how often
-- sessions on it paused on usage limits, and what a workspace's admins see
-- of their members' connections.

-- When Claude last answered it: a usage reading, or a session's first
-- answer from Claude, either of which means Claude took it.
ALTER TABLE claude_credentials ADD COLUMN answered_at INTEGER;
-- When `claude setup-token` made a long-lived token, as its person said:
-- it runs out a year on (expires_at). NULL: nobody said, and expires_at is
-- a year after it was connected, an estimate.
ALTER TABLE claude_credentials ADD COLUMN made_at INTEGER;
-- When the hourly job last warned its person that it runs out (30, then 7
-- days before): a warning counts for the date it was about only while it
-- came after that date's warning day.
ALTER TABLE claude_credentials ADD COLUMN warned_at INTEGER;
-- When fctl marked it ended (status 'invalid'): Claude refused the token
-- in sessions, or the login in renewing it.
ALTER TABLE claude_credentials ADD COLUMN ended_at INTEGER;

-- Sessions in which Claude refused a connection: once as many as the
-- instance says (InstanceSettings.claude_refusals) did since Claude last
-- answered it, fctl marks it ended. Kept 30 days.
CREATE TABLE IF NOT EXISTS claude_refusals (
  credential_id TEXT NOT NULL REFERENCES claude_credentials(id) ON DELETE CASCADE,
  session_id TEXT NOT NULL,
  at INTEGER NOT NULL,
  PRIMARY KEY (credential_id, session_id)
);

-- Each time a session paused on a usage limit, with the connection it ran
-- on: one row per session, limit and reset, so that a report replayed adds
-- none. Kept 30 days.
CREATE TABLE IF NOT EXISTS claude_limit_pauses (
  credential_id TEXT NOT NULL REFERENCES claude_credentials(id) ON DELETE CASCADE,
  session_id TEXT NOT NULL,
  window TEXT NOT NULL DEFAULT '',
  resets_at INTEGER NOT NULL DEFAULT 0,
  at INTEGER NOT NULL,
  PRIMARY KEY (session_id, window, resets_at)
);
CREATE INDEX IF NOT EXISTS claude_limit_pauses_credential ON claude_limit_pauses (credential_id, at);
CREATE INDEX IF NOT EXISTS claude_limit_pauses_at ON claude_limit_pauses (at);

-- What a workspace's admins see of their members' Claude connections
-- (NULL: the most the instance lets them, InstanceSettings.claude_oversight),
-- whether they see limit pauses, how they hear by email (off, each or
-- weekly), and when the last weekly email went.
ALTER TABLE workspaces ADD COLUMN claude_oversight TEXT;
ALTER TABLE workspaces ADD COLUMN claude_limit_pauses INTEGER NOT NULL DEFAULT 1;
ALTER TABLE workspaces ADD COLUMN claude_admin_email TEXT NOT NULL DEFAULT 'weekly';
ALTER TABLE workspaces ADD COLUMN claude_digest_at INTEGER;
