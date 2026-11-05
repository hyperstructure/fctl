-- Runners that enroll themselves in a workspace with one of its enrollment
-- tokens (`fctl host install --enroll`, docs/architecture.md#enrollment-tokens,
-- ADR 0071). Its admins make the tokens; a host that enrolls gets a runner
-- token of its own, and the enrollment token is used for nothing else.
CREATE TABLE IF NOT EXISTS enrollment_tokens (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  -- The runners it enrolls wait for an admin to approve them.
  approval INTEGER NOT NULL DEFAULT 0,
  -- The most runners enrolled with it that may be there at once (NULL: no
  -- limit). Rejected, expired and removed ones don't count.
  max_runners INTEGER,
  -- The runners it enrolls are removed once offline this many minutes
  -- (NULL: kept).
  remove_offline_after_minutes INTEGER,
  expires_at INTEGER,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  last_used_at INTEGER,
  revoked_at INTEGER,
  revoked_by TEXT REFERENCES users(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS enrollment_tokens_workspace ON enrollment_tokens (workspace_id);

-- A runner that enrolled belongs to its workspace, not to a person
-- (`created_by` is NULL): its admins manage it, and it runs everyone's
-- sessions there and no other workspace's. Deleting the workspace removes
-- it.
ALTER TABLE runners ADD COLUMN workspace_id TEXT REFERENCES workspaces(id) ON DELETE CASCADE;
-- The token it enrolled with, and where from.
ALTER TABLE runners ADD COLUMN enrollment_id TEXT REFERENCES enrollment_tokens(id) ON DELETE SET NULL;
ALTER TABLE runners ADD COLUMN enrolled_ip TEXT;
ALTER TABLE runners ADD COLUMN enrolled_location TEXT;
-- What its host said of the machine as it enrolled (JSON,
-- `fctl_proto::api::RunnerFacts`), until its first hello says more.
ALTER TABLE runners ADD COLUMN facts TEXT;
-- Set while it waits for an admin of its workspace to approve it: when it
-- is removed unless one does. It has no runner_workspaces row until then,
-- so it runs nothing and gets nothing for any session.
ALTER TABLE runners ADD COLUMN pending_until INTEGER;
-- Removed once offline this many minutes, as its enrollment token said
-- (NULL: kept): FleetDO removes it as it watches it, the hourly job one
-- that never connected.
ALTER TABLE runners ADD COLUMN remove_offline_after_minutes INTEGER;
CREATE INDEX IF NOT EXISTS runners_workspace ON runners (workspace_id);
CREATE INDEX IF NOT EXISTS runners_enrollment ON runners (enrollment_id);
