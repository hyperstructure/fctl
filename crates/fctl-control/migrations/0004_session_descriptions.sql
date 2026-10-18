-- What a session is about, in a sentence or two. Agents write it with
-- `fctl edit` in their sandbox (with the title), and so can its owner.
ALTER TABLE sessions ADD COLUMN description TEXT;
