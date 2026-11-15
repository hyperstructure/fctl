-- The files an autopilot keeps across its sessions (fctl autopilot files):
-- text its own session writes, such as the scripts it relies on and drafts
-- of what it will send, which its next session gets back in a new sandbox
-- and people read on its tab. At most 100, of 256 KB each and 4 MB in all
-- (fctl_domain::autopilot::MAX_FILES_BYTES); kept while the autopilot is,
-- off too, until it removes them, and deleted with its project.
CREATE TABLE IF NOT EXISTS autopilot_files (
  project_id TEXT NOT NULL REFERENCES autopilots(project_id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  content TEXT NOT NULL,
  -- In bytes, as UTF-8.
  size INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  -- Its own session that last wrote it.
  session_id TEXT,
  PRIMARY KEY (project_id, name)
);
