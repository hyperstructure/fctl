-- Messages an autopilot sent its sessions that rest while as many of its
-- sessions worked as its people allow: fctl holds them, rather than refuse
-- them, and sends them as one of its sessions ends its turn, stops or
-- fails, in the order it sent them (crate::autopilots::release).
CREATE TABLE IF NOT EXISTS autopilot_held (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES autopilots(project_id) ON DELETE CASCADE,
  -- The session it goes to, one the autopilot started.
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  -- The delivery it goes out as, which the API answered with.
  delivery_id TEXT NOT NULL,
  text TEXT NOT NULL,
  -- The files it sends, claimed for the session as it was held: a JSON
  -- array of fctl_proto::event::Attachment; NULL for none.
  attachments TEXT,
  -- As whom it goes: the autopilot's owner when it sent it.
  author_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  -- The request's idempotency key ('<user id>:<client id>'), which a retry
  -- of the request finds it by, and which its delivery keeps.
  client_key TEXT,
  -- Its own session that sent it.
  from_session_id TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS autopilot_held_project ON autopilot_held (project_id, created_at, id);
