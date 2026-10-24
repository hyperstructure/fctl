-- Whether the search index of its workspace (`SearchObject`, see
-- docs/architecture.md#search) has all of a session's messages and
-- replies. Its SessionObject indexes them as they come and says so here;
-- the hourly job asks the others, the sessions from before search first
-- of all, to index what they hold.
ALTER TABLE sessions ADD COLUMN search_indexed INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS sessions_to_index ON sessions (last_activity_at) WHERE search_indexed = 0;
