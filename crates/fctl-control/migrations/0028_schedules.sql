-- Schedules (docs/architecture.md#schedules): tasks a project's agents start
-- on their own, once at a set time or again and again, each run a session
-- of its own on the Claude subscription of the person the schedule runs for.

CREATE TABLE IF NOT EXISTS schedules (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  -- The task each run's agent gets.
  prompt TEXT NOT NULL,
  -- When it runs: a cron expression in `timezone`, or once, at `at`.
  cron TEXT,
  at INTEGER,
  timezone TEXT NOT NULL DEFAULT 'UTC',
  -- What its sessions start with; NULL: the project's.
  model TEXT,
  effort TEXT,
  base_branch TEXT,
  github_access TEXT,
  -- Whose Claude subscription its runs start on: who set it up, or took it
  -- over since.
  owner_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  -- 0 while it is paused; `paused_reason` says why, when fctl paused it.
  enabled INTEGER NOT NULL DEFAULT 1,
  paused_reason TEXT,
  -- When it starts its next session (NULL: paused, or done), and when it
  -- last ran.
  next_run_at INTEGER,
  last_run_at INTEGER,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  updated_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS schedules_project ON schedules (project_id, created_at);
CREATE INDEX IF NOT EXISTS schedules_due ON schedules (next_run_at) WHERE next_run_at IS NOT NULL;

-- Each time a schedule ran: the session it started, or why it started none.
CREATE TABLE IF NOT EXISTS schedule_runs (
  id TEXT PRIMARY KEY,
  schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
  -- When it was due (the schedule's time, or when someone asked), and when
  -- fctl started it or decided not to.
  due_at INTEGER NOT NULL,
  at INTEGER NOT NULL,
  -- started, skipped or failed.
  outcome TEXT NOT NULL,
  reason TEXT,
  -- time, person or token.
  trigger TEXT NOT NULL,
  -- Who it ran for: the owner at its time, whoever asked otherwise.
  user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS schedule_runs_schedule ON schedule_runs (schedule_id, at DESC);
CREATE INDEX IF NOT EXISTS schedule_runs_session ON schedule_runs (session_id);

-- The schedule a session started from. Not a foreign key: the session
-- keeps it once the schedule is deleted, and says it came from one.
ALTER TABLE sessions ADD COLUMN schedule_id TEXT;
