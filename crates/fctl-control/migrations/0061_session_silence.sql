-- Why an autopilot's session last ended its turn without a word, where
-- fctl knows (fctl_domain::autopilot::Silence), kept by SessionDO beside
-- said and written through with its summary, as JSON: Claude Code cut off
-- twice, an interrupt and by whom, or a message that reached the agent
-- mid-turn and from whom. NULL from when a turn starts, for other
-- sessions, and for those from before it.
ALTER TABLE sessions ADD COLUMN silence TEXT;
