-- Two-factor sign-in (docs/architecture.md#two-factor-sign-in): after the
-- password, and after Google when the person or the instance asks for it,
-- a code from an authenticator app or a recovery code.

-- An authenticator app (TOTP, RFC 6238), at most one per person. The secret
-- is sealed with the master key and bound to `totp:<user_id>`. Codes of time
-- steps up to `last_step` are refused, so that each code works once.
-- `failures` counts wrong codes in a row; after five, sign-ins wait until
-- `locked_until`, longer each time.
CREATE TABLE IF NOT EXISTS authenticator_apps (
  user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  secret_enc TEXT NOT NULL,
  last_step INTEGER NOT NULL DEFAULT 0,
  failures INTEGER NOT NULL DEFAULT 0,
  locked_until INTEGER,
  created_at INTEGER NOT NULL,
  last_used_at INTEGER
);

-- One-time codes for when the authenticator app is gone, ten at a time.
-- Only the SHA-256 of each (without its dash, in capitals) is kept.
CREATE TABLE IF NOT EXISTS recovery_codes (
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  code_hash TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  used_at INTEGER,
  PRIMARY KEY (user_id, code_hash)
);

-- Sign-ins that wait for the second factor, for ten minutes: after the
-- password, after Google (`return_to`: where the app goes next) or after a
-- reset link chose a new password (`reset_hash` names the link, and
-- `password_hash` holds the new password until the code checks out). The
-- browser holds the challenge in the `__Host-fctl-2fa` cookie; only its
-- SHA-256 is kept here.
CREATE TABLE IF NOT EXISTS sign_in_challenges (
  hash TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  reason TEXT NOT NULL CHECK (reason IN ('password', 'google', 'password_reset')),
  return_to TEXT,
  reset_hash TEXT,
  password_hash TEXT,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sign_in_challenges_user ON sign_in_challenges (user_id);

-- Google sign-ins ask this person for the second factor too.
ALTER TABLE users ADD COLUMN two_factor_google INTEGER NOT NULL DEFAULT 0;
