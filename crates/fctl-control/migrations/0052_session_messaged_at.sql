-- When a person last sent a session a message (fctl#906), in milliseconds.
-- NULL: nobody has since the column. SessionDO keeps it in its summary and
-- writes it through, so that lists tell a reply someone waits for from a
-- session that only waits until it stops.
ALTER TABLE sessions ADD COLUMN messaged_at INTEGER;
