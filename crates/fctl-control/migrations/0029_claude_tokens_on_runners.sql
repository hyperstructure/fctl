-- Claude connections whose token their owner keeps on their runners
-- (docs/architecture.md#claude-credentials, ADR 0049): the control plane
-- never has their token (`secret_enc` is empty), and their sessions run
-- only on those of their owner's runners that keep one. One per person.
ALTER TABLE claude_credentials ADD COLUMN on_runners INTEGER NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX IF NOT EXISTS claude_credentials_on_runners
  ON claude_credentials (user_id) WHERE on_runners = 1;
