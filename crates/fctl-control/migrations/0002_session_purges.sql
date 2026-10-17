-- Sessions whose event log (stored in their SessionObject) still has to be
-- deleted because their project was. Filled in the same batch that deletes
-- the project; drained right away and by the hourly cron trigger.
CREATE TABLE IF NOT EXISTS session_purges (
  session_id TEXT PRIMARY KEY,
  queued_at INTEGER NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0
);
