-- Autopilots (experimental; docs/architecture.md#autopilots): an agent that
-- runs a project, or its whole workspace, on its own, in sessions of its
-- own in that project (its home), on the Claude subscription of the person
-- who put it on, as whom it acts through the API.

-- Its workspace's admins let projects have them.
ALTER TABLE workspaces ADD COLUMN autopilots INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS autopilots (
  -- Its home: one autopilot per project.
  project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
  workspace_id TEXT NOT NULL,
  -- project, or workspace: its workspace's projects too, and new ones.
  scope TEXT NOT NULL DEFAULT 'project',
  -- on, paused or off.
  state TEXT NOT NULL DEFAULT 'on',
  paused_reason TEXT,
  paused_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  paused_at INTEGER,
  -- Whose subscription it runs on, and as whom it acts.
  owner_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  -- Where people want it to go (Markdown), and its own plan.
  direction TEXT NOT NULL,
  direction_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  direction_at INTEGER NOT NULL,
  -- Its direction changed since it last woke: the next wake says so.
  direction_new INTEGER NOT NULL DEFAULT 0,
  plan TEXT,
  plan_at INTEGER,
  -- What it may do besides sessions: settings,secrets,repos,schedules,projects.
  powers TEXT NOT NULL DEFAULT '',
  check_in_minutes INTEGER NOT NULL DEFAULT 60,
  crew_max INTEGER NOT NULL DEFAULT 3,
  -- What its own sessions start with; NULL: the project's.
  model TEXT,
  effort TEXT,
  -- Its own session now, the one wakes go to.
  session_id TEXT,
  -- When fctl wakes it next (NULL while paused or off), and why: the
  -- reasons gathered since it last woke, comma-separated.
  wake_at INTEGER,
  wake_reasons TEXT NOT NULL DEFAULT '',
  -- The time it asked to wake at, if it did.
  asked_wake_at INTEGER,
  woken_at INTEGER,
  check_ins INTEGER NOT NULL DEFAULT 0,
  -- Its own sessions that failed in a row.
  failures INTEGER NOT NULL DEFAULT 0,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS autopilots_due ON autopilots (wake_at) WHERE wake_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS autopilots_workspace ON autopilots (workspace_id, scope);
-- One workspace's autopilot at a time, on or paused, whoever puts it on.
CREATE UNIQUE INDEX IF NOT EXISTS autopilots_one_per_workspace ON autopilots (workspace_id)
  WHERE scope = 'workspace' AND state != 'off';

-- What people tell it to steer it; each stays until someone removes it.
CREATE TABLE IF NOT EXISTS autopilot_notes (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES autopilots(project_id) ON DELETE CASCADE,
  text TEXT NOT NULL,
  author_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  -- When it first got it (a wake, or its brief).
  seen_at INTEGER
);
CREATE INDEX IF NOT EXISTS autopilot_notes_project ON autopilot_notes (project_id, created_at DESC);

-- What it asks people: a question, or a secret for one of its projects.
CREATE TABLE IF NOT EXISTS autopilot_asks (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES autopilots(project_id) ON DELETE CASCADE,
  -- question or secret.
  kind TEXT NOT NULL,
  text TEXT NOT NULL,
  -- What helps people answer (Markdown): links, screenshots of the options.
  detail TEXT,
  -- Answers to click, JSON.
  options TEXT,
  -- The secret it needs, and the project it goes into.
  secret_name TEXT,
  secret_project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
  -- open, answered, declined or withdrawn.
  status TEXT NOT NULL DEFAULT 'open',
  -- The answer, or why someone declined: never a secret's value.
  answer TEXT,
  answered_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  answered_at INTEGER,
  -- When it heard the answer.
  seen_at INTEGER,
  -- Its own session that asked.
  session_id TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS autopilot_asks_project ON autopilot_asks (project_id, status, created_at DESC);

-- What it did, wrote down, or what happened to it.
CREATE TABLE IF NOT EXISTS autopilot_log (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES autopilots(project_id) ON DELETE CASCADE,
  at INTEGER NOT NULL,
  kind TEXT NOT NULL,
  text TEXT NOT NULL,
  -- The person who did it; NULL when it or fctl did.
  user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  -- The session it concerns, and the project when another than its home.
  session_id TEXT,
  other_project_id TEXT,
  -- Its own session that did it.
  from_session_id TEXT
);
CREATE INDEX IF NOT EXISTS autopilot_log_project ON autopilot_log (project_id, at DESC, id DESC);

-- Secrets an autopilot set itself, which it may replace and remove; those
-- people set, it may not.
ALTER TABLE project_secrets ADD COLUMN autopilot INTEGER NOT NULL DEFAULT 0;

-- The autopilot a session belongs to (its home project's id), and what it
-- is to it: pilot (its own) or crew (one it started). Not foreign keys:
-- sessions keep them once the autopilot is gone.
ALTER TABLE sessions ADD COLUMN autopilot_id TEXT;
ALTER TABLE sessions ADD COLUMN autopilot_role TEXT;
CREATE INDEX IF NOT EXISTS sessions_autopilot ON sessions (autopilot_id, created_at DESC) WHERE autopilot_id IS NOT NULL;
