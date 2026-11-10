-- An autopilot's owner keeps it on their Claude subscription when others
-- change or resume it, while they can run it (1), rather than have whoever
-- changes how much it works, or resumes it, take it over (0, as before).
-- Its owner alone sets it; a new owner starts without it.
ALTER TABLE autopilots ADD COLUMN keep_owner INTEGER NOT NULL DEFAULT 0;
