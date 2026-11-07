-- What a session's agent asked the people who follow it (fctl#850, `fctl
-- ask`), as JSON (`SessionQuestion`), until a person's message answers it.
-- NULL: it asks nothing. SessionDO keeps it and writes it through, so lists
-- show it without opening the session.
ALTER TABLE sessions ADD COLUMN question TEXT;
