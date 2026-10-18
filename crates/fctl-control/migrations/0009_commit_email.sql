-- The address the commits of sessions someone starts credit them with, in a
-- `Co-authored-by` trailer. They choose it in Settings; without one,
-- commits don't name them. Never the sign-in email: commits are public
-- wherever their repository is.
ALTER TABLE users ADD COLUMN commit_email TEXT;
