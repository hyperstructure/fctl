-- The session a question of an autopilot's is about (fctl autopilot ask
-- --session): one in its projects that needs files only people have, say.
-- People open it from the ask, and the files they attach to their answer
-- go to it in a message from them.
-- files: those files, as a JSON array of fctl_proto::event::Attachment, so
-- that the autopilot hears which went; NULL when the answer sent none.
ALTER TABLE autopilot_asks ADD COLUMN about_id TEXT;
ALTER TABLE autopilot_asks ADD COLUMN files TEXT;
