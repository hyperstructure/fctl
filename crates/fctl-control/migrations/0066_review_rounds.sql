-- How many reviews in a row a project's review agents pass the findings of
-- to the session whose agent opened the pull request while nobody messages
-- it (fctl_domain::reviews::ROUNDS): 1 to 10, as its owners chose; 5, as
-- before. fctl holds back those of the next review until a person messages
-- the agent or passes them on.
ALTER TABLE project_reviews ADD COLUMN rounds INTEGER NOT NULL DEFAULT 5;
