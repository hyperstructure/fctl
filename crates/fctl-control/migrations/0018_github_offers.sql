-- What a GitHub sign-in showed a workspace's admin they may connect of an
-- account, while they choose what the workspace works in
-- (docs/architecture.md#github-accounts): the repositories the app reaches
-- there that they are an admin of, and whether every one, also those it
-- reaches later, is theirs to give. One per admin, workspace and account,
-- until they choose, or for ten minutes.
CREATE TABLE IF NOT EXISTS github_offers (
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  account TEXT NOT NULL COLLATE NOCASE,
  installation_id INTEGER NOT NULL,
  -- Organization or User.
  account_type TEXT,
  -- Who they signed in to GitHub as.
  github_user TEXT NOT NULL,
  all_repos INTEGER NOT NULL DEFAULT 0,
  -- JSON: the names, as GitHub spells them.
  repos TEXT NOT NULL,
  -- How many more the app reaches there that they see but don't administer.
  others INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, user_id, account)
);
