-- Plan usage (docs/architecture.md#plan-usage): how much of each Claude
-- plan limit is used, as the sessions on a credential last reported it
-- (rate_limit_event), so people see what is left before they start work.

-- One row per credential and limit. window is Claude's name for the limit:
-- five_hour, seven_day (every model), or a model's weekly limit
-- (seven_day_opus, seven_day_sonnet, …). A newer reading replaces the row.
CREATE TABLE IF NOT EXISTS claude_usage (
  credential_id TEXT NOT NULL REFERENCES claude_credentials(id) ON DELETE CASCADE,
  window TEXT NOT NULL,
  -- The share used, from 0 to 1 (a little more past the limit); NULL when
  -- Claude did not say.
  utilization REAL,
  resets_at INTEGER,
  -- allowed, allowed_warning or rejected, when Claude reported this limit
  -- as the one closest to stopping requests.
  status TEXT,
  observed_at INTEGER NOT NULL,
  PRIMARY KEY (credential_id, window)
);
