-- A project's memory (docs/architecture.md#project-memory): notes agents
-- and people keep for the agents of later sessions, which each session's
-- agent starts with. A note an agent writes names its session, and the
-- person who started it; one a person writes names them alone. Stale notes
-- stay, with when, why and who said so, until someone deletes them. They go
-- with the project; a person's account going leaves their notes nameless.
CREATE TABLE IF NOT EXISTS memories (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  text TEXT NOT NULL,
  session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  edited_at INTEGER,
  edited_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  stale_at INTEGER,
  stale_reason TEXT,
  stale_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  stale_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  replaced_by TEXT REFERENCES memories(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS memories_project ON memories (project_id, created_at);

-- What agents record as they finish a piece of their session's work
-- (docs/architecture.md#retrospectives): the outcome (completed, partial or
-- blocked), what they worked on, their approach, what got in the way and
-- what a future agent should know. They go with their session.
CREATE TABLE IF NOT EXISTS retrospectives (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  outcome TEXT NOT NULL,
  worked_on TEXT NOT NULL,
  approach TEXT,
  challenges TEXT,
  insights TEXT
);
CREATE INDEX IF NOT EXISTS retrospectives_project ON retrospectives (project_id, created_at);
CREATE INDEX IF NOT EXISTS retrospectives_session ON retrospectives (session_id, created_at);
