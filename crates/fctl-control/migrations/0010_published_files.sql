-- Screenshots agents publish (`fctl publish`) so that pull requests can show
-- them. Their bytes live in the project's FilesObject; this index finds
-- them by link. A link holds a random token, of which only the SHA-256 is
-- kept here. Anyone with the link can open the file until `expires_at`,
-- while the instance and the project allow publishing. Rows go with their
-- session and project; the hourly cron trigger deletes expired ones, and
-- the FilesObject deletes their bytes when they expire.
CREATE TABLE IF NOT EXISTS published_files (
  id TEXT PRIMARY KEY,
  token_hash TEXT NOT NULL UNIQUE,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  media_type TEXT NOT NULL,
  size INTEGER NOT NULL,
  width INTEGER,
  height INTEGER,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS published_files_session ON published_files (session_id);
CREATE INDEX IF NOT EXISTS published_files_expiry ON published_files (expires_at);
