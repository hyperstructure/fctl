-- The pull requests of autopilots' sessions that merged, waiting for the
-- checks on their base branch to end, and how those ended
-- (src/merge_checks.rs, fctl_domain::merge_checks): fctl wakes the
-- autopilot once they do. A row for each session that lists the pull
-- request, which an autopilot started; housekeeping deletes it three days
-- after the merge.
CREATE TABLE IF NOT EXISTS merge_checks (
  -- The pull request: its repository (owner/name, lowercase, as
  -- pull_request_sessions has it), number, URL and title.
  repo TEXT NOT NULL,
  number INTEGER NOT NULL,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  url TEXT NOT NULL,
  title TEXT,
  -- The autopilot that started the session: its project's id.
  autopilot_id TEXT NOT NULL,
  -- The branch it merged into, and the commit that holds it there
  -- (merge_commit_sha).
  base_ref TEXT NOT NULL,
  merge_sha TEXT NOT NULL,
  merged_at INTEGER NOT NULL,
  -- Once the checks of a commit there that holds it ended: that commit,
  -- when, how (passed, failed or cancelled), how many checks did each, and
  -- the names of those that failed (a JSON array). NULL while they run.
  sha TEXT,
  ended_at INTEGER,
  outcome TEXT,
  passed INTEGER,
  failed INTEGER,
  cancelled INTEGER,
  failing TEXT,
  -- When a wake told the autopilot how they ended.
  told_at INTEGER,
  -- When the hourly job last looked the checks up itself, in case
  -- GitHub's word that they ended went missing (merge_checks::sweep).
  looked_at INTEGER,
  PRIMARY KEY (repo, number, session_id)
);
CREATE INDEX IF NOT EXISTS merge_checks_waiting ON merge_checks (repo, ended_at);
CREATE INDEX IF NOT EXISTS merge_checks_untold ON merge_checks (autopilot_id, told_at);
CREATE INDEX IF NOT EXISTS merge_checks_merged ON merge_checks (merged_at);
