-- Whether a project's review agents review drafts (ADR 0068): 1, each new
-- commit of a draft starts a review, as before; 0, as its owners chose,
-- drafts wait until they are marked ready for review, which starts a
-- review of their newest commit, or until someone asks for one.
ALTER TABLE project_reviews ADD COLUMN drafts INTEGER NOT NULL DEFAULT 1;
