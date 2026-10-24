-- The GitHub Apps whose pull requests start reviews by themselves in a
-- project (ADR 0057), as pull requests name their bots: a JSON array such
-- as ["dependabot[bot]", "renovate[bot]"]. Dependabot's did in every
-- project before its owners could choose (ADR 0054), and still do until
-- they choose otherwise.
ALTER TABLE project_reviews ADD COLUMN bots TEXT NOT NULL DEFAULT '["dependabot[bot]"]';
