-- fctl control plane schema. Times are milliseconds since the Unix epoch.
-- Secrets are stored as SHA-256 hashes (tokens) or AES-256-GCM ciphertext
-- (credentials, project secrets); see crates/fctl-control/src/crypto.rs.

CREATE TABLE users (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  avatar_url TEXT,
  password_hash TEXT,
  role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member')),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
  default_credential_id TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  last_login_at INTEGER
);

-- Linked external accounts (Google).
CREATE TABLE identities (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  provider TEXT NOT NULL,
  subject TEXT NOT NULL,
  email TEXT,
  name TEXT,
  avatar_url TEXT,
  created_at INTEGER NOT NULL,
  UNIQUE (provider, subject)
);
CREATE INDEX identities_user ON identities (user_id);

-- Who may sign up: an exact email or '*@domain'.
CREATE TABLE allowlist (
  id TEXT PRIMARY KEY,
  pattern TEXT NOT NULL UNIQUE,
  role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('admin', 'member')),
  note TEXT,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE invites (
  id TEXT PRIMARY KEY,
  token_hash TEXT NOT NULL UNIQUE,
  email TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('admin', 'member')),
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  accepted_at INTEGER
);
CREATE INDEX invites_email ON invites (email);

-- Web sessions, CLI tokens and personal access tokens.
CREATE TABLE tokens (
  id TEXT PRIMARY KEY,
  hash TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL CHECK (kind IN ('web', 'cli', 'pat')),
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT,
  user_agent TEXT,
  ip TEXT,
  created_at INTEGER NOT NULL,
  last_used_at INTEGER,
  expires_at INTEGER
);
CREATE INDEX tokens_user ON tokens (user_id);

CREATE TABLE password_resets (
  token_hash TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_by TEXT,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  used_at INTEGER
);

-- CLI sign-in (OAuth 2.0 device authorization grant, RFC 8628 style).
CREATE TABLE device_codes (
  device_hash TEXT PRIMARY KEY,
  user_code TEXT NOT NULL UNIQUE,
  client_name TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'denied', 'consumed')),
  user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
  ip TEXT,
  location TEXT,
  user_agent TEXT,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  last_poll_at INTEGER
);

CREATE TABLE runners (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  enabled INTEGER NOT NULL DEFAULT 1,
  info TEXT,
  last_seen_at INTEGER,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  slug TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  description TEXT,
  repo_owner TEXT,
  repo_name TEXT,
  default_branch TEXT NOT NULL DEFAULT 'main',
  settings TEXT NOT NULL DEFAULT '{}',
  session_counter INTEGER NOT NULL DEFAULT 0,
  created_by TEXT NOT NULL REFERENCES users(id),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  archived_at INTEGER
);

CREATE TABLE project_members (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('owner', 'maintainer', 'editor', 'viewer')),
  added_by TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (project_id, user_id)
);
CREATE INDEX project_members_user ON project_members (user_id);

CREATE TABLE project_secrets (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  value_enc TEXT NOT NULL,
  updated_by TEXT,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (project_id, name)
);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  number INTEGER NOT NULL,
  title TEXT NOT NULL,
  prompt TEXT NOT NULL,
  status TEXT NOT NULL,
  status_reason TEXT,
  resume_at INTEGER,
  created_by TEXT NOT NULL REFERENCES users(id),
  credential_id TEXT,
  runner_id TEXT,
  branch TEXT,
  base_branch TEXT,
  model TEXT,
  effort TEXT,
  claude_session_id TEXT NOT NULL,
  pr_number INTEGER,
  pr_url TEXT,
  pr_title TEXT,
  pr_state TEXT,
  remote_control TEXT,
  stats TEXT NOT NULL DEFAULT '{}',
  archived INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  started_at INTEGER,
  ended_at INTEGER,
  last_activity_at INTEGER,
  UNIQUE (project_id, number)
);
CREATE INDEX sessions_project ON sessions (project_id, created_at DESC);
CREATE INDEX sessions_status ON sessions (status);
CREATE INDEX sessions_creator ON sessions (created_by, created_at DESC);
CREATE INDEX sessions_created ON sessions (created_at DESC);

CREATE TABLE session_shares (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('viewer', 'editor', 'owner')),
  created_by TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (session_id, user_id)
);
CREATE INDEX session_shares_user ON session_shares (user_id);

CREATE TABLE claude_credentials (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('setup_token', 'oauth')),
  label TEXT,
  secret_enc TEXT NOT NULL,
  scopes TEXT NOT NULL DEFAULT '',
  account_email TEXT,
  subscription_type TEXT,
  account TEXT,
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'invalid')),
  expires_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  last_used_at INTEGER
);
CREATE INDEX claude_credentials_user ON claude_credentials (user_id);

CREATE TABLE claude_login_jobs (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  state TEXT NOT NULL,
  runner_id TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX claude_login_jobs_user ON claude_login_jobs (user_id, created_at DESC);

CREATE TABLE audit_log (
  id TEXT PRIMARY KEY,
  ts INTEGER NOT NULL,
  actor_id TEXT,
  action TEXT NOT NULL,
  target TEXT,
  data TEXT,
  ip TEXT
);
CREATE INDEX audit_log_ts ON audit_log (ts DESC);

-- Instance settings (name, defaults).
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
