-- Passkeys (docs/architecture.md#passkeys): people sign in with their
-- fingerprint, face or screen lock, and where two-factor sign-in is
-- required a passkey is also a second factor.

-- A passkey: a WebAuthn credential its authenticator finds by itself.
-- `public_key` is its COSE key (base64url), which checks each sign-in's
-- signature; `sign_count` is the authenticator's counter, which must move on
-- unless it stays 0, as synced passkeys' does. `aaguid` names the
-- authenticator's model, and so the provider that keeps the passkey;
-- `backed_up` says whether it syncs between devices.
CREATE TABLE IF NOT EXISTS passkeys (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  credential_id TEXT NOT NULL UNIQUE,
  public_key TEXT NOT NULL,
  algorithm INTEGER NOT NULL,
  sign_count INTEGER NOT NULL DEFAULT 0,
  transports TEXT,
  aaguid TEXT,
  backed_up INTEGER NOT NULL DEFAULT 0,
  name TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  last_used_at INTEGER
);
CREATE INDEX IF NOT EXISTS passkeys_user ON passkeys (user_id);

-- The challenges passkeys sign, each good once for ten minutes, by its
-- SHA-256: to make a passkey (`register`, for `user_id`), to sign in with one
-- (`sign_in`), or to use one as the second factor of the waiting sign-in
-- whose hash is `sign_in_hash` (`second_factor`).
CREATE TABLE IF NOT EXISTS passkey_challenges (
  hash TEXT PRIMARY KEY,
  purpose TEXT NOT NULL CHECK (purpose IN ('register', 'sign_in', 'second_factor')),
  user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
  sign_in_hash TEXT,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS passkey_challenges_expiry ON passkey_challenges (expires_at);
