-- Files people attach to messages. Their bytes live in a FilesObject per
-- project; a session keeps the files attached to its task (JSON, a list of
-- attachments) so hosts can fetch them before the first turn.
ALTER TABLE sessions ADD COLUMN prompt_attachments TEXT;

-- Projects whose files (stored in their FilesObject) still have to be
-- deleted because the project was. Filled in the same batch that deletes the
-- project; drained right away and by the hourly cron trigger.
CREATE TABLE IF NOT EXISTS file_purges (
  project_id TEXT PRIMARY KEY,
  queued_at INTEGER NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0
);
