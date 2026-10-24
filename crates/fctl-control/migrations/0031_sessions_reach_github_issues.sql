-- Sessions whose agent reaches the GitHub issues of the repositories it
-- writes to, whatever its project's ticket tracker says, as the maintainer
-- who started it asked (ADR 0055): to move them into the tracker, say.
ALTER TABLE sessions ADD COLUMN github_issues INTEGER NOT NULL DEFAULT 0;
