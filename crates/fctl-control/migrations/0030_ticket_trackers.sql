-- Ticket trackers (docs/architecture.md#ticket-trackers): a project's
-- tracker is in its settings (`tickets`); Linear's needs these.

-- Linear workspaces fctl's app is installed in, by Linear's organization
-- id: one row each, whose tokens every fctl workspace connected to it
-- shares, since Linear keeps one token for an app's user and installing the
-- app again replaces it. The access and refresh tokens are sealed with the
-- master key (bound to `linear:<organization_id>`); the access token runs
-- out at `expires_at`. `status` is active, or revoked once Linear says the
-- app was uninstalled or refuses to renew its token.
CREATE TABLE IF NOT EXISTS linear_installations (
  organization_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  url_key TEXT NOT NULL,
  logo_url TEXT,
  -- fctl's app user there, whom people delegate issues to, with its name
  -- and handle (which people mention), as Linear named them.
  app_user_id TEXT,
  app_user_name TEXT,
  app_user_handle TEXT,
  secret_enc TEXT NOT NULL,
  expires_at INTEGER,
  scopes TEXT,
  status TEXT NOT NULL DEFAULT 'active',
  installed_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- The fctl workspaces that work in a Linear workspace: an admin of both
-- connected them, signed in to Linear as `linear_user`.
CREATE TABLE IF NOT EXISTS workspace_linear (
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  organization_id TEXT NOT NULL REFERENCES linear_installations(organization_id) ON DELETE CASCADE,
  connected_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  linear_user TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, organization_id)
);
CREATE INDEX IF NOT EXISTS workspace_linear_organization ON workspace_linear (organization_id);

-- Who people are in Linear workspaces, from a Linear sign-in of theirs:
-- the issues they delegate to fctl there run as them, and their replies
-- reach agents as theirs. One account per person and Linear workspace, and
-- one person per Linear account.
CREATE TABLE IF NOT EXISTS linear_accounts (
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  organization_id TEXT NOT NULL,
  organization_name TEXT NOT NULL,
  url_key TEXT,
  linear_user_id TEXT NOT NULL,
  name TEXT NOT NULL,
  email TEXT,
  avatar_url TEXT,
  linked_at INTEGER NOT NULL,
  PRIMARY KEY (user_id, organization_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS linear_accounts_linear_user ON linear_accounts (organization_id, linear_user_id);

-- Linear's agent sessions (an issue's thread with fctl's agent), by
-- Linear's id, and the fctl session each reaches. `state` is waiting (for
-- its person to link their Linear account, or to choose the project;
-- `waiting_for` says which, and `payload` keeps the delivery until
-- `expires_at`), started (it reaches `session_id`) or refused (with
-- `waiting_for` retry and the delivery kept when starting failed, which a
-- reply tries again). `stepped_at` is when fctl last showed a step of the
-- agent's there.
CREATE TABLE IF NOT EXISTS linear_threads (
  id TEXT PRIMARY KEY,
  organization_id TEXT NOT NULL,
  issue_id TEXT NOT NULL,
  issue_key TEXT NOT NULL,
  session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
  linear_user_id TEXT,
  state TEXT NOT NULL,
  waiting_for TEXT,
  payload TEXT,
  expires_at INTEGER,
  stepped_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS linear_threads_session ON linear_threads (session_id, updated_at);
CREATE INDEX IF NOT EXISTS linear_threads_issue ON linear_threads (organization_id, issue_id);
CREATE INDEX IF NOT EXISTS linear_threads_waiting ON linear_threads (organization_id, linear_user_id) WHERE state = 'waiting';

-- The ticket a session started from (JSON: a SessionTicket).
ALTER TABLE sessions ADD COLUMN ticket TEXT;

-- What sessions' agents wrote in their trackers in the last hour (a
-- comment, a new ticket), which keeps each to a few
-- (fctl_domain::tickets::MAX_COMMENTS_PER_HOUR); the hourly job forgets the
-- rest.
CREATE TABLE IF NOT EXISTS ticket_writes (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ticket_writes_session ON ticket_writes (session_id, kind, at);
