-- Session snapshots (docs/architecture.md#session-snapshots, #616): a
-- session's conversation and uncommitted work, saved by its agent as its
-- sandbox stops and kept in its SessionDO, so that a resume on another
-- runner picks up where it left off.
--
-- `key_enc`: the session's key, which its agent seals its snapshots with,
-- sealed with the master key for `snapshot:<session>`: whoever stores a
-- snapshot reads nothing of it. One row per session, made the first time
-- a run of it may save one; `snapshot_id` and what follows say which
-- snapshot is the session's now, NULL until one is saved, and again once
-- it expires.
CREATE TABLE IF NOT EXISTS session_snapshots (
  session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  key_enc TEXT NOT NULL,
  snapshot_id TEXT,
  size INTEGER,
  sha256 TEXT,
  parts INTEGER,
  claude_version TEXT,
  agent_seq INTEGER,
  conversation INTEGER,
  changes INTEGER,
  -- What it left out to stay within the limit (JSON array).
  left_out TEXT,
  saved_at INTEGER,
  runner_id TEXT,
  runner_name TEXT,
  -- When the hourly job last failed to delete it once it expired, which
  -- puts it after the others the next time.
  expire_tried_at INTEGER,
  -- When its runner started uploading a snapshot that hasn't committed:
  -- the hourly job lets go of its parts a day on.
  uploading_since INTEGER
);
CREATE INDEX IF NOT EXISTS session_snapshots_saved ON session_snapshots (saved_at);

-- Whether a workspace's sessions save snapshots (on by default), and the
-- days one stays after it was saved (NULL: 30).
ALTER TABLE workspaces ADD COLUMN snapshots INTEGER NOT NULL DEFAULT 1;
ALTER TABLE workspaces ADD COLUMN snapshot_days INTEGER;
