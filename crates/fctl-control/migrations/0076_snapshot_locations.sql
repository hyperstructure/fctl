-- Snapshots that a runner kept in a bucket of its organization's own
-- (`[snapshots] url` in its host.toml, docs/architecture.md#session-snapshots):
-- where the object is (`s3://bucket/key`). NULL where the session's
-- SessionDO keeps its parts.
ALTER TABLE session_snapshots ADD COLUMN location TEXT;
