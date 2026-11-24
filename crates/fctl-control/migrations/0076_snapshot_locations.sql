-- Snapshots that a runner kept in a bucket of its organization's own
-- (`[snapshots] url` in its host.toml, docs/architecture.md#session-snapshots):
-- where the object is (`s3://bucket/key`). NULL where the session's
-- SessionDO keeps its parts.
ALTER TABLE session_snapshots ADD COLUMN location TEXT;

-- Where the snapshot it replaced was, when that one was in a bucket too:
-- a commit of this one that comes again, its first answer lost, says it
-- again, so that its host deletes it there. NULL once this one goes.
ALTER TABLE session_snapshots ADD COLUMN replaced TEXT;
