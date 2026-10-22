-- Whose sessions a runner runs in a workspace, and where a member's
-- sessions run (docs/architecture.md#runners, ADR 0036). A runner a member
-- adds for their own sessions (`personal`) runs only theirs there; one its
-- admins add runs everyone's. A member who keeps their sessions in a
-- workspace on their own runners (`own_runners`) has them run only on
-- runners they registered.
ALTER TABLE runner_workspaces ADD COLUMN personal INTEGER NOT NULL DEFAULT 0;
ALTER TABLE workspace_members ADD COLUMN own_runners INTEGER NOT NULL DEFAULT 0;
