-- The repositories a project's agents may write to whose pull requests its
-- review agents don't review (ADR 0062), as its owners chose: another
-- project's review agents review them, say. A JSON array of owner/name,
-- such as ["acme/shared-lib"]. The others are reviewed, those linked later
-- too, and a repository that stops being one its agents may write to
-- leaves the list.
ALTER TABLE project_reviews ADD COLUMN skipped_repos TEXT NOT NULL DEFAULT '[]';
