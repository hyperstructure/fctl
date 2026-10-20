-- The order each person puts their workspaces in, and each one's projects
-- (docs/architecture.md#workspaces): lists and the app's sidebar show them
-- so. `position` places the workspace among the member's own (NULL: after
-- the placed ones, in the order they joined); `project_order` is the ids
-- of its projects in their order (JSON; projects it leaves out follow,
-- oldest first). Both go with the membership.
ALTER TABLE workspace_members ADD COLUMN position INTEGER;
ALTER TABLE workspace_members ADD COLUMN project_order TEXT;
