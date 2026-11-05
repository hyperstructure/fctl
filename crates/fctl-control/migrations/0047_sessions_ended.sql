-- The sessions that ended since a time, by when they ended, without reading
-- the others: `GET /api/v1/sessions?active_since=`, which the web app's
-- sidebar asks for on every page, lists them with those that haven't ended.
CREATE INDEX IF NOT EXISTS sessions_ended ON sessions (ended_at);
