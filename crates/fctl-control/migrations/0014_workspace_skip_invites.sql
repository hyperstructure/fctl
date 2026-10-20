-- A workspace's admins skipped inviting people while setting it up, as in a
-- workspace for one: the app's setup checklist leaves that step out. They
-- invite people from its People tab whenever they want.
ALTER TABLE workspaces ADD COLUMN skip_invites INTEGER NOT NULL DEFAULT 0;
