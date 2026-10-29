-- The installation of the GitHub App review agents post their reviews as
-- (`GITHUB_REVIEW_APP_ID`) that a review writes its check and comments
-- with, where that app reached its repository as it started: only the app
-- that created a check may change it. NULL: the instance's GitHub App
-- writes them, through `installation_id`.
ALTER TABLE reviews ADD COLUMN review_installation_id INTEGER;
