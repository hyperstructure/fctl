-- Review agents (docs/architecture.md#review-agents): where a project turns
-- them on, an agent reviews each new commit of the pull requests in the
-- repositories its agents may write to, on the Claude subscription of the
-- person who turned them on, and fctl posts what it found on GitHub.

-- A project's review agents: on or off, whose subscription runs them (who
-- turned them on, or took them over since), and how they run.
CREATE TABLE IF NOT EXISTS project_reviews (
  project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
  enabled INTEGER NOT NULL DEFAULT 0,
  operator_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  -- NULL: the project's model and effort level.
  model TEXT,
  effort TEXT,
  -- What review agents are told besides fctl's review instructions.
  instructions TEXT,
  updated_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  updated_at INTEGER NOT NULL
);

-- Each review: a review agent's session, the pull request it reviews at
-- one commit, its check on GitHub and what came of it.
CREATE TABLE IF NOT EXISTS reviews (
  session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  -- owner/name, as GitHub spells it, and in lowercase.
  repo TEXT NOT NULL,
  repo_key TEXT NOT NULL,
  number INTEGER NOT NULL,
  head_sha TEXT NOT NULL,
  -- 0 for the review a delivery started; people who ask for another
  -- review of the same commit get the next.
  attempt INTEGER NOT NULL DEFAULT 0,
  url TEXT NOT NULL,
  title TEXT,
  author TEXT,
  draft INTEGER NOT NULL DEFAULT 0,
  -- From a fork; its author may write to the repository, so that the agent
  -- may run its code (fctl_domain::reviews::trusted).
  fork INTEGER NOT NULL DEFAULT 0,
  trusted INTEGER NOT NULL DEFAULT 0,
  -- The commit the last finished review of the pull request reviewed.
  previous_sha TEXT,
  -- When GitHub last changed the pull request, as of what started the
  -- review: a delivery about an older head, which comes late, starts none.
  pr_updated_at INTEGER,
  -- The GitHub App's installation it works through, and its check run.
  installation_id INTEGER,
  check_run_id INTEGER,
  -- Why GitHub shows no check for it.
  check_error TEXT,
  -- running, passed, commented, superseded, stopped or failed.
  state TEXT NOT NULL DEFAULT 'running',
  reason TEXT,
  findings INTEGER NOT NULL DEFAULT 0,
  resolved INTEGER NOT NULL DEFAULT 0,
  still_open INTEGER NOT NULL DEFAULT 0,
  review_url TEXT,
  created_at INTEGER NOT NULL,
  finished_at INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS reviews_commit ON reviews (project_id, repo_key, number, head_sha, attempt);
CREATE INDEX IF NOT EXISTS reviews_project ON reviews (project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS reviews_pull ON reviews (repo_key, number, state);
CREATE INDEX IF NOT EXISTS reviews_running ON reviews (state, created_at);
