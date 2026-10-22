-- Limited access tokens and what sessions' agents may do on GitHub
-- (docs/architecture.md#access-tokens, ADR 0037). A personal access token
-- may be limited (`scope`, fctl_domain::limits::Limits as JSON): in what it
-- does, the workspaces and projects it reaches, and what the agents of its
-- sessions may do on GitHub. NULL: no limits, as every token had before.
ALTER TABLE tokens ADD COLUMN scope TEXT;
-- What a session's agent may do on GitHub: 'push', 'comment' or 'read'.
-- NULL: push, as every session's agent did before.
ALTER TABLE sessions ADD COLUMN github_access TEXT;
