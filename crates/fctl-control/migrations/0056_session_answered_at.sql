-- When a person's message last answered a session's question (fctl ask), in
-- milliseconds. NULL: nobody has since the column. SessionDO keeps it in its
-- summary and writes it through, so that lists tell a message that asked
-- the agent for more (messaged_at) from one that only answered it.
ALTER TABLE sessions ADD COLUMN answered_at INTEGER;
