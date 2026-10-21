-- The model and effort level each person asked for when they last started
-- a session in a project (docs/architecture.md#model-and-effort): the web
-- app's composer starts on them. NULL where they took the project's
-- setting. A session's start writes the row; it goes with the project, and
-- with the person when they leave its workspace or delete their account.
CREATE TABLE IF NOT EXISTS project_choices (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  model TEXT,
  effort TEXT,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (project_id, user_id)
);
CREATE INDEX IF NOT EXISTS project_choices_user ON project_choices (user_id);
