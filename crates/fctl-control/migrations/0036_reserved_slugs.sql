-- Slugs instance admins reserve for later (Admin → Settings, fctl admin
-- reserve): no new workspace or project takes one as its address. One
-- whose name makes it gets a random ending instead (example-829d34), and
-- nobody chooses it; those that have it already keep it. fctl's own
-- (fctl_domain::validate::RESERVED_SLUGS) aren't here. A new instance has
-- none.
CREATE TABLE IF NOT EXISTS reserved_slugs (
  slug TEXT PRIMARY KEY,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL
);
