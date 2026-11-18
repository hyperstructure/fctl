-- A webhook's keeper (docs/architecture.md#webhooks, ADR 0087): whoever
-- chose where it posts, by adding it, pointing it elsewhere or turning it
-- on again. It posts only while they manage it (an admin of its workspace,
-- or, for a project's, an owner or maintainer of the project, with an
-- active account): someone who left, was demoted or disabled stops getting
-- the workspace's events at an address they chose, and fctl turns the
-- webhook off and says why. Webhooks from before keepers are their makers'.

ALTER TABLE webhooks ADD COLUMN keeper_id TEXT REFERENCES users(id) ON DELETE SET NULL;
UPDATE webhooks SET keeper_id = created_by WHERE keeper_id IS NULL;
