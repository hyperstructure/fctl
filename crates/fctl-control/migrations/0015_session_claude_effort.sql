-- The effort level Claude reports a session's agent works at (`low` …
-- `max`; fctl-agent asks it with `get_settings`), which the session page
-- and `fctl show` show. `effort` stays the level the session asked for,
-- which its runs pass to Claude. NULL until Claude reports one, when its
-- model takes none, and for sessions from before it.
ALTER TABLE sessions ADD COLUMN claude_effort TEXT;
