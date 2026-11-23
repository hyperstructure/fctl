-- Runner pools (docs/architecture.md#runner-pools, #615): the runners an
-- enrollment token enrolled are a pool. Every one of them carries the
-- token's labels, which projects choose a pool by
-- (ProjectSettings::runner_labels), beside its host.toml's; FleetDO reads
-- them from here, never from a host's hello, so that no machine joins a
-- pool it didn't enroll in.
--
-- `labels`: JSON array of lowercase labels (NULL: none).
-- `placement`: 'spread' (the least busy runner first, NULL too) or
-- 'packed' (the busiest runner with room, so that idle ones can go).
-- `shape`: what its runners announced last (JSON, fctl_proto::api::PoolShape),
-- which its demand counts with while none of them is online, so that an
-- autoscaler starts it from none.
ALTER TABLE enrollment_tokens ADD COLUMN labels TEXT;
ALTER TABLE enrollment_tokens ADD COLUMN placement TEXT;
ALTER TABLE enrollment_tokens ADD COLUMN shape TEXT;
