-- Confirming it's you before sensitive actions
-- (docs/architecture.md#confirming-its-you): where the instance asks for
-- it, deleting a project, making an access token and the other actions
-- in fctl_domain::verify::Sensitive take a passkey or a code from an
-- authenticator app first, and a confirmation lasts 15 minutes for the
-- browser, CLI or token that made it.

-- When the token last confirmed it's its person: with a passkey or a code
-- (POST /api/v1/auth/verify), or, for a browser, by signing in with a
-- passkey or a second factor. NULL: never.
ALTER TABLE tokens ADD COLUMN verified_at INTEGER;

-- A CLI that is signed in already asks its person to confirm it's them in
-- the browser (POST /api/v1/auth/verify/device), where passkeys work:
-- approving the code confirms that token instead of signing a new one in.
ALTER TABLE device_codes ADD COLUMN token_id TEXT REFERENCES tokens(id) ON DELETE CASCADE;

-- Passkeys confirm it too, with a challenge of their own (`verify`, for
-- `user_id`). SQLite can't change a CHECK constraint, so the table is made
-- again with the new purpose, and the challenges waiting now move over.
CREATE TABLE passkey_challenges_new (
  hash TEXT PRIMARY KEY,
  purpose TEXT NOT NULL CHECK (purpose IN ('register', 'sign_in', 'second_factor', 'verify')),
  user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
  sign_in_hash TEXT,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
INSERT INTO passkey_challenges_new (hash, purpose, user_id, sign_in_hash, created_at, expires_at)
  SELECT hash, purpose, user_id, sign_in_hash, created_at, expires_at FROM passkey_challenges;
DROP TABLE passkey_challenges;
ALTER TABLE passkey_challenges_new RENAME TO passkey_challenges;
CREATE INDEX IF NOT EXISTS passkey_challenges_expiry ON passkey_challenges (expires_at);
