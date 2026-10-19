-- Workspaces (docs/architecture.md#workspaces): people, the projects they
-- work on, the runners that serve them and the GitHub accounts they work
-- in, with their own admins, two-factor requirement and audit log. People
-- have one account with any number of email addresses, each invited on its
-- own, and are a member of a workspace as one of them. Everything so far
-- moves into a first workspace.

CREATE TABLE IF NOT EXISTS workspaces (
  id TEXT PRIMARY KEY,
  slug TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  -- Its avatar is its initial on this brand color (yellow, pink, blue,
  -- ink), until an admin uploads an image: its bytes, base64, and type.
  color TEXT NOT NULL DEFAULT 'yellow',
  avatar TEXT,
  avatar_type TEXT,
  avatar_updated_at INTEGER,
  -- The role every member has in every project (none, viewer, editor,
  -- maintainer), on top of their own.
  base_role TEXT NOT NULL DEFAULT 'none' CHECK (base_role IN ('none', 'viewer', 'editor', 'maintainer')),
  -- Members need two-factor sign-in; it is closed to those without.
  require_two_factor INTEGER NOT NULL DEFAULT 0,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- Who is in a workspace, as which of their addresses: the one the
-- workspace's people see, and whose commit email credits them in its
-- projects. `owner` is the primary admin, one per workspace. Their sessions
-- there run on `credential_id`, or their default Claude connection.
CREATE TABLE IF NOT EXISTS workspace_members (
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  email TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
  credential_id TEXT REFERENCES claude_credentials(id) ON DELETE SET NULL,
  added_by TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, user_id)
);
CREATE INDEX IF NOT EXISTS workspace_members_user ON workspace_members (user_id);
CREATE UNIQUE INDEX IF NOT EXISTS workspace_members_primary ON workspace_members (workspace_id) WHERE role = 'owner';

-- Invitations to join a workspace, by address, until they are answered or
-- expire. Whoever has the address (an account's, or connected to one)
-- sees it.
CREATE TABLE IF NOT EXISTS workspace_invitations (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  email TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('admin', 'member')),
  invited_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  UNIQUE (workspace_id, email)
);
CREATE INDEX IF NOT EXISTS workspace_invitations_email ON workspace_invitations (email);

-- The GitHub accounts a workspace works in: ones the instance's GitHub App
-- is installed on. `repos` (JSON, names) narrows it to some repositories;
-- NULL is every one the app reaches there. A workspace admin's request is
-- `requested` until an instance admin approves it. The installation is
-- found by the account, and remembered.
CREATE TABLE IF NOT EXISTS workspace_github (
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  account TEXT NOT NULL COLLATE NOCASE,
  installation_id INTEGER,
  repos TEXT,
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'requested')),
  requested_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  approved_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, account)
);

-- The workspaces whose sessions a runner runs. A runner belongs to whoever
-- registered it (runners.created_by), who adds it to workspaces they are
-- an admin of; the workspaces' admins take it out.
CREATE TABLE IF NOT EXISTS runner_workspaces (
  runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  added_by TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (runner_id, workspace_id)
);
CREATE INDEX IF NOT EXISTS runner_workspaces_workspace ON runner_workspaces (workspace_id);

-- An account's email addresses, its primary one (users.email) included.
-- Each signs in to it, and has its own commit email, which replaces
-- users.commit_email.
CREATE TABLE IF NOT EXISTS user_emails (
  email TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  commit_email TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS user_emails_user ON user_emails (user_id);

ALTER TABLE projects ADD COLUMN workspace_id TEXT REFERENCES workspaces(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS projects_workspace ON projects (workspace_id);

-- What happened in a workspace goes in its audit log; the rest is the
-- instance's.
ALTER TABLE audit_log ADD COLUMN workspace_id TEXT;
CREATE INDEX IF NOT EXISTS audit_log_workspace ON audit_log (workspace_id, ts DESC);

-- ───────────────────────────── what exists so far ─────────────────────────────

INSERT OR IGNORE INTO user_emails (email, user_id, commit_email, created_at)
  SELECT email, id, commit_email, created_at FROM users
  WHERE email NOT LIKE 'deleted-%@deleted.invalid';

-- One primary instance admin: the first owner. Any other becomes an admin.
UPDATE users SET role = 'admin'
  WHERE role = 'owner' AND id != (
    SELECT id FROM users WHERE role = 'owner' ORDER BY status = 'active' DESC, created_at, id LIMIT 1
  );

-- The first workspace, named after the GitHub account most projects'
-- repositories belong to, or the instance. Its admins can rename it.
WITH first AS (
  SELECT COALESCE(
    (SELECT repo_owner FROM projects WHERE repo_owner IS NOT NULL
      GROUP BY lower(repo_owner) ORDER BY COUNT(*) DESC, MIN(created_at) LIMIT 1),
    (SELECT value FROM settings WHERE key = 'instance_name'),
    'Main'
  ) AS name
)
INSERT INTO workspaces (id, slug, name, color, base_role, require_two_factor, created_by, created_at, updated_at)
  SELECT
    'wsp_' || upper(hex(randomblob(13))),
    CASE
      WHEN length(lower(name)) BETWEEN 2 AND 48
        AND lower(name) NOT GLOB '*[^a-z0-9-]*'
        AND lower(name) NOT GLOB '-*' AND lower(name) NOT GLOB '*-' AND lower(name) NOT GLOB '*--*'
        AND lower(name) NOT IN ('api', 'admin', 'new', 'settings', 'login', 'logout', 'signup', 'setup',
          'device', 'invite', 's', 'p', 'projects', 'sessions', 'runners', 'assets', 'static', 'app',
          'help', 'docs')
      THEN lower(name)
      ELSE 'main'
    END,
    name,
    'yellow',
    'none',
    0,
    (SELECT id FROM users WHERE role = 'owner' LIMIT 1),
    COALESCE((SELECT MIN(created_at) FROM users), 0),
    COALESCE((SELECT MIN(created_at) FROM users), 0)
  FROM first
  WHERE EXISTS (SELECT 1 FROM users WHERE email NOT LIKE 'deleted-%@deleted.invalid');

-- Everyone is in it, as their address, with the role they had: the
-- primary instance admin is its primary admin, the instance's admins its
-- admins. Its base role is none, so members see the projects they are
-- members of, as before.
INSERT OR IGNORE INTO workspace_members (workspace_id, user_id, email, role, added_by, created_at)
  SELECT w.id, u.id, u.email,
    CASE u.role WHEN 'owner' THEN 'owner' WHEN 'admin' THEN 'admin' ELSE 'member' END,
    NULL, u.created_at
  FROM users u, (SELECT id FROM workspaces ORDER BY created_at LIMIT 1) w
  WHERE u.email NOT LIKE 'deleted-%@deleted.invalid';

UPDATE projects SET workspace_id = (SELECT id FROM workspaces ORDER BY created_at LIMIT 1)
  WHERE workspace_id IS NULL;

-- Every runner runs its sessions.
INSERT OR IGNORE INTO runner_workspaces (runner_id, workspace_id, added_by, created_at)
  SELECT r.id, w.id, r.created_by, r.created_at
  FROM runners r, (SELECT id FROM workspaces ORDER BY created_at LIMIT 1) w;

-- It works in the GitHub accounts its projects use.
INSERT OR IGNORE INTO workspace_github (workspace_id, account, installation_id, repos, status, created_at, updated_at)
  SELECT w.id, o.owner, NULL, NULL, 'active',
    CAST(strftime('%s', 'now') AS INTEGER) * 1000, CAST(strftime('%s', 'now') AS INTEGER) * 1000
  FROM (
    SELECT repo_owner AS owner FROM projects WHERE repo_owner IS NOT NULL
    UNION SELECT owner FROM project_repos
  ) o, (SELECT id FROM workspaces ORDER BY created_at LIMIT 1) w;

-- What happened to its projects and sessions goes in its audit log.
UPDATE audit_log SET workspace_id = (SELECT id FROM workspaces ORDER BY created_at LIMIT 1)
  WHERE workspace_id IS NULL AND (action LIKE 'project.%' OR action LIKE 'session.%');
