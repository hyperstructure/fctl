-- Notifications (docs/architecture.md#notifications): a session that waits
-- for a reply, pauses on a usage limit or fails notifies the people who
-- follow it, in the browsers where they turned notifications on.

-- Who follows which session. Starting or resuming a session follows it,
-- unless the person turned that off; anyone who can see one can follow it.
CREATE TABLE IF NOT EXISTS session_followers (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (session_id, user_id)
);
CREATE INDEX IF NOT EXISTS session_followers_user ON session_followers (user_id);

-- People follow the sessions they started so far.
INSERT OR IGNORE INTO session_followers (session_id, user_id, created_at)
  SELECT s.id, s.created_by, s.created_at FROM sessions s
  JOIN users u ON u.id = s.created_by WHERE u.status = 'active';

-- What notifies each person (JSON, NotificationSettings). No row: the
-- defaults, everything.
CREATE TABLE IF NOT EXISTS notification_settings (
  user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  settings TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

-- Browsers that show someone's notifications: Web Push subscriptions
-- (RFC 8030). The endpoint is an address at the browser's push service;
-- p256dh and auth are the browser's keys that messages are encrypted for
-- (RFC 8291, base64url). A subscription belongs to the sign-in that made it:
-- signing out of that browser, or revoking it, deletes it.
CREATE TABLE IF NOT EXISTS push_subscriptions (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_id TEXT REFERENCES tokens(id) ON DELETE CASCADE,
  endpoint TEXT NOT NULL UNIQUE,
  p256dh TEXT NOT NULL,
  auth TEXT NOT NULL,
  name TEXT,
  created_at INTEGER NOT NULL,
  last_sent_at INTEGER
);
CREATE INDEX IF NOT EXISTS push_subscriptions_user ON push_subscriptions (user_id);
CREATE INDEX IF NOT EXISTS push_subscriptions_token ON push_subscriptions (token_id);
