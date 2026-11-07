-- When each person last exported their data (GET /api/v1/me/export), in
-- milliseconds: the instance allows one export in each of its
-- export_interval_hours (fctl#920). NULL: never, or the last one failed.
ALTER TABLE users ADD COLUMN exported_at INTEGER;
