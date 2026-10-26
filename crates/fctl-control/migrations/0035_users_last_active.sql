-- When each person was last active: they signed in in a browser, or
-- started a session themselves (on the web, with the CLI or by handing a
-- ticket to fctl), not one a review agent, a schedule or a limited token
-- started for them. The instance admins' list of people shows it.
ALTER TABLE users ADD COLUMN last_active_at INTEGER;

-- From what came before: their last sign-in, or the last session they
-- started that no review agent or schedule did, whichever is later.
UPDATE users SET last_active_at = last_login_at;
UPDATE users SET last_active_at = started.at
FROM (
  SELECT s.created_by AS user_id, MAX(s.created_at) AS at FROM sessions s
  WHERE s.schedule_id IS NULL
    AND NOT EXISTS (SELECT 1 FROM reviews r WHERE r.session_id = s.id)
  GROUP BY s.created_by
) AS started
WHERE started.user_id = users.id
  AND (users.last_active_at IS NULL OR users.last_active_at < started.at);
