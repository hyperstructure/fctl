-- The pictures people upload for their avatars (Settings → Profile), or
-- that linking Google copies when they have none: its bytes, base64, and
-- type. users.avatar_url then holds its path, /api/v1/users/{id}/avatar?v=
-- with updated_at, which changes with it. Before, avatar_url was an image
-- address on another site that people typed in; those stay until replaced.
CREATE TABLE IF NOT EXISTS user_avatars (
  user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  image TEXT NOT NULL,
  media_type TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
