-- What each person asked to be called ("Carl Oscar", "Bob"), set in
-- Settings → Profile: agents call them this, and greetings use it. NULL:
-- their first names (given_name), as before.
ALTER TABLE users ADD COLUMN preferred_name TEXT;
