-- Which sessions list which pull request, so that what GitHub's webhooks
-- say of one reaches them (src/pull_requests.rs): a row for each pull
-- request in a session's `pull_requests`, by its repository (`owner/name`,
-- lowercase) and number, with what GitHub last said of it.
CREATE TABLE IF NOT EXISTS pull_request_sessions (
  repo TEXT NOT NULL,
  number INTEGER NOT NULL,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  -- Its head commit, which commit statuses name rather than the pull request.
  head_sha TEXT,
  -- open, draft, queued, merged or closed.
  state TEXT,
  -- When the control plane last looked it up on GitHub.
  checked_at INTEGER,
  PRIMARY KEY (repo, number, session_id)
);
CREATE INDEX IF NOT EXISTS pull_request_sessions_head ON pull_request_sessions (repo, head_sha);
CREATE INDEX IF NOT EXISTS pull_request_sessions_session ON pull_request_sessions (session_id);
CREATE INDEX IF NOT EXISTS pull_request_sessions_checked ON pull_request_sessions (checked_at);

-- The pull requests sessions listed before: those in `pull_requests`, and
-- the one sessions from before it have.
INSERT OR IGNORE INTO pull_request_sessions (repo, number, session_id, state)
SELECT lower(substr(url, 20, instr(url, '/pull/') - 20)), number, session_id, state FROM (
  SELECT s.id AS session_id,
         json_extract(j.value, '$.url') AS url,
         json_extract(j.value, '$.number') AS number,
         json_extract(j.value, '$.state') AS state
  FROM sessions s, json_each(s.pull_requests) j
  WHERE s.pull_requests IS NOT NULL
  UNION ALL
  SELECT id, pr_url, pr_number, pr_state FROM sessions
  WHERE pull_requests IS NULL AND pr_url IS NOT NULL AND pr_number IS NOT NULL
)
WHERE url LIKE 'https://github.com/%/%/pull/%' AND number > 0;
