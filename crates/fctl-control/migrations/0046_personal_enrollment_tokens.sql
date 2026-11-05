-- Enrollment tokens for their maker's own sessions (ADR 0072): any member
-- of a workspace makes one, and the runners it enrolls belong to them
-- (`runners.created_by`) and run only their sessions there
-- (`runner_workspaces.personal`), so that they count as their own runners
-- for Only on my runners and a Claude token kept on runners. They are still
-- held to the token's workspace (`runners.workspace_id`) and run no other
-- workspace's sessions. Its maker approves the runners that wait.
ALTER TABLE enrollment_tokens ADD COLUMN personal INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS enrollment_tokens_creator ON enrollment_tokens (created_by);
