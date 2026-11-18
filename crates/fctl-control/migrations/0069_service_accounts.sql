-- Service accounts (docs/architecture.md#service-accounts, ADR 0086): a
-- workspace's credentials for programs of its own (a portal where people
-- start agents, a chat bot, a script), which start sessions for the
-- workspace's people who allow them, or for everyone where the instance
-- doesn't ask, run its schedules and read its sessions. Only the secret's
-- SHA-256 is kept, with its last four characters to tell secrets apart.

CREATE TABLE IF NOT EXISTS service_accounts (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  -- What it is, for the people it asks.
  description TEXT,
  hash TEXT NOT NULL UNIQUE,
  hint TEXT NOT NULL,
  -- The admin who made its secret: it works while they are an active admin
  -- of the workspace, and a new secret makes whoever makes it its keeper.
  keeper_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  secret_made_at INTEGER NOT NULL,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  last_used_at INTEGER
);
CREATE INDEX IF NOT EXISTS service_accounts_workspace ON service_accounts (workspace_id, created_at);

-- Whether a service account may start sessions for someone, as they
-- answered (no row: they haven't). Leaving the workspace forgets theirs.
CREATE TABLE IF NOT EXISTS service_account_answers (
  service_account_id TEXT NOT NULL REFERENCES service_accounts(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  allowed INTEGER NOT NULL,
  answered_at INTEGER NOT NULL,
  PRIMARY KEY (service_account_id, user_id)
);
CREATE INDEX IF NOT EXISTS service_account_answers_user ON service_account_answers (user_id);

-- The service account that started a session for its person; it stays
-- once the service account is deleted, which then has no name to show.
ALTER TABLE sessions ADD COLUMN service_account_id TEXT;
CREATE INDEX IF NOT EXISTS sessions_service_account ON sessions (service_account_id, created_at)
  WHERE service_account_id IS NOT NULL;
