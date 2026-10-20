-- Workspaces connect GitHub accounts themselves, through a GitHub sign-in
-- that shows what their admin may connect (docs/architecture.md#github-accounts,
-- ADR 0029): nobody approves requests any more, so the ones waiting go, and
-- their admins connect the accounts again. `github_user` is the GitHub
-- account the admin who connected one signed in as (NULL for connections
-- from before).
ALTER TABLE workspace_github ADD COLUMN github_user TEXT;
DELETE FROM workspace_github WHERE status = 'requested';
