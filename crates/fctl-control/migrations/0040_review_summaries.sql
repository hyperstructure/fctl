-- Review agents that sum pull requests up in their descriptions (ADR 0067):
-- before each review in depth, the agent says how risky the pull request
-- looks, what it does and what deserves a closer look, which fctl puts at
-- the end of its description, between its marks, in place of the summary
-- an earlier review left there. Off until a project's owners turn it on.
ALTER TABLE project_reviews ADD COLUMN description_summary INTEGER NOT NULL DEFAULT 0;
-- The summary a review put in its pull request's description, as JSON
-- (fctl_proto::api::PullRequestSummary): fctl puts it back as the review
-- posts, where something took it out meanwhile. NULL: it summed nothing up.
ALTER TABLE reviews ADD COLUMN pull_request_summary TEXT;
