-- The GitHub account a workspace's connection is to, by GitHub's id
-- (docs/architecture.md#github-accounts): a login can change hands (an
-- account renamed or deleted, and its name taken by someone else), an id
-- can't. The control plane mints tokens, links repositories and looks pull
-- requests up for a connection only while its login still belongs to that
-- account. NULL for connections from before, which take the account of the
-- installation they recorded, or else the one that has the login, the first
-- time the control plane uses them. Offers keep it until their admin
-- chooses.
ALTER TABLE workspace_github ADD COLUMN account_id INTEGER;
ALTER TABLE github_offers ADD COLUMN account_id INTEGER;
