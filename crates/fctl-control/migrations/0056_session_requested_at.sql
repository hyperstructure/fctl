-- When a person last sent a session a message that didn't answer its
-- agent's question (fctl ask), in milliseconds. NULL: nobody has. SessionDO
-- keeps it in its summary and writes it through, so that lists tell a
-- session someone asked for more after its pull requests merged from one
-- that only heard back on what it asked. Sessions from before it take
-- messaged_at, as nobody could tell their answers apart; SessionDO's write
-- keeps that until a message sets its own.
ALTER TABLE sessions ADD COLUMN requested_at INTEGER;
UPDATE sessions SET requested_at = messaged_at WHERE messaged_at IS NOT NULL;
