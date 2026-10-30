-- When Linear revokes fctl's app in a Linear workspace (it was uninstalled
-- there, or its token no longer renews), every fctl workspace's connection
-- to it ends: installing the app again, which takes one of the Linear
-- workspace's admins, connects only the fctl workspace it is installed
-- from, so that the Linear workspace's admins can end what they let fctl
-- workspaces reach. The others keep their row, revoked since `revoked_at`,
-- which their Linear tab shows until their admins connect it again or
-- disconnect it.
ALTER TABLE workspace_linear ADD COLUMN revoked_at INTEGER;
