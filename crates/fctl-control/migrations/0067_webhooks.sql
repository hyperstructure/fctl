-- Webhooks (docs/architecture.md#webhooks, ADR 0084): addresses fctl
-- posts a workspace's events to as they happen, signed with each one's
-- secret. A workspace's (project_id NULL: all its projects, or those in
-- `projects`) or a project's own. What went out to each, and how, lives in
-- the workspace's HooksObject, not here.

CREATE TABLE IF NOT EXISTS webhooks (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  -- A project's own webhook; NULL: one of the workspace's.
  project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT,
  url TEXT NOT NULL,
  -- JSON arrays: the events it wants (empty: every one), a workspace
  -- webhook's projects (empty: every one), and how the sessions it hears of
  -- started (empty: however they did).
  events TEXT NOT NULL DEFAULT '[]',
  projects TEXT NOT NULL DEFAULT '[]',
  started_by TEXT NOT NULL DEFAULT '[]',
  -- Sealed with the master key (`webhook:<id>`); after a rotation, the one
  -- before it signs too until `previous_until`.
  secret TEXT NOT NULL,
  previous_secret TEXT,
  previous_until INTEGER,
  -- 0: it gets nothing. `disabled_reason` says why when fctl turned it off.
  enabled INTEGER NOT NULL DEFAULT 1,
  disabled_reason TEXT,
  disabled_at INTEGER,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS webhooks_workspace ON webhooks (workspace_id, created_at);
CREATE INDEX IF NOT EXISTS webhooks_project ON webhooks (project_id) WHERE project_id IS NOT NULL;
