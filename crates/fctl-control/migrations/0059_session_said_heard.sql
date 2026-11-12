-- What fctl tells an autopilot of the sessions it started
-- (fctl_domain::autopilot::CrewView), kept by SessionDO and written through
-- with its summary; NULL for other sessions, and for those from before it.
-- said: the start of what its agent said as its last turn ended, on one
-- line; empty from when its agent takes a message until that turn ends
-- with words.
-- heard: people's latest messages to it, rather than the autopilot's, as a
-- JSON array newest first (fctl_domain::autopilot::Heard): who, when, and
-- the start of what they wrote.
ALTER TABLE sessions ADD COLUMN said TEXT;
ALTER TABLE sessions ADD COLUMN heard TEXT;
