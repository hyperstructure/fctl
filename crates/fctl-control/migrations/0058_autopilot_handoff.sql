-- The context size, in tokens, past which a wake goes to a fresh session of
-- the autopilot's own, briefed from its plan and log, rather than to its
-- session as it is: NULL, as before, hands off only after 12 hours.
ALTER TABLE autopilots ADD COLUMN handoff_at INTEGER;
