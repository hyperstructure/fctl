-- People without an account who asked for early access on the sign-in page,
-- where the deployment turns it on (`FCTL_EARLY_ACCESS`, as app.fctl.sh's
-- does), for the instance's admins to invite (docs/architecture.md#early-access).
-- One row per address: asking again moves `asked_at`. Inviting the address,
-- from the list or any other way, sets `invited_at` and `invited_by`.
-- `withdraw_hash` is the SHA-256 of the token the request's answer gave the
-- browser that asked first, with which it withdraws the request. A row goes
-- once its address has an account, when an admin removes it or that browser
-- withdraws it, or a year after it last asked (src/housekeeping.rs).
CREATE TABLE IF NOT EXISTS access_requests (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL,
  asked_at INTEGER NOT NULL,
  invited_at INTEGER,
  invited_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  withdraw_hash TEXT UNIQUE
);
CREATE INDEX IF NOT EXISTS access_requests_asked ON access_requests (asked_at);
