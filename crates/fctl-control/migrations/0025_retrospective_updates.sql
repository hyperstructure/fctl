-- When an agent last updated a retrospective of its session
-- (docs/architecture.md#retrospectives): it keeps one for each piece of
-- work and updates it as later turns carry on with that work (`fctl retro
-- update`), its outcome and parts in place. NULL until it does.
ALTER TABLE retrospectives ADD COLUMN updated_at INTEGER;
