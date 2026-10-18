-- Repositories a project's agents may use besides its primary one
-- (projects.repo_owner, projects.repo_name). name '*' is an owner-wide
-- grant: every repository of owner that the GitHub App can reach, read-only.
-- All of a project's repositories belong to one account, so one installation
-- of the GitHub App mints their tokens.
CREATE TABLE IF NOT EXISTS project_repos (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  owner TEXT NOT NULL COLLATE NOCASE,
  name TEXT NOT NULL COLLATE NOCASE,
  access TEXT NOT NULL CHECK (access IN ('read', 'write')),
  added_by TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (project_id, owner, name)
);
