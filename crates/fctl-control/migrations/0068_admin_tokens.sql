-- Instance admin tokens (docs/architecture.md#admin-tokens, ADR 0085):
-- what programs provision people with, where the deployment turns them on
-- (FCTL_ADMIN_TOKENS; never app.fctl.sh). Each acts as the admin who made
-- it, while they are an active admin of the instance, and only on the
-- admin API's people endpoints. Only its SHA-256 is kept.

CREATE TABLE IF NOT EXISTS admin_tokens (
  id TEXT PRIMARY KEY,
  hash TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  last_used_at INTEGER,
  -- NULL: it doesn't expire.
  expires_at INTEGER
);
CREATE INDEX IF NOT EXISTS admin_tokens_user ON admin_tokens (user_id);

-- The Claude token an admin token set for someone (PUT
-- /api/v1/admin/users/{id}/claude): one per person, which the next one
-- replaces in place. They remove it as they remove their own connections,
-- which admin tokens never touch.
ALTER TABLE claude_credentials ADD COLUMN provisioned INTEGER NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX IF NOT EXISTS claude_credentials_provisioned
  ON claude_credentials (user_id) WHERE provisioned = 1;
