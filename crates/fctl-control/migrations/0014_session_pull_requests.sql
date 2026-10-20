-- Every pull request a session's agent found, in all of its project's
-- repositories, with the repository of each (JSON, `[PullRequestRef]`;
-- `fctl_domain::pulls`). The pr_* columns keep the one clients from before
-- it show. NULL for sessions from before it, which have those only.
ALTER TABLE sessions ADD COLUMN pull_requests TEXT;
