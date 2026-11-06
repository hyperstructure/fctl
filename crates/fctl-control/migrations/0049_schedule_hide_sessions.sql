-- Schedules whose sessions stay off session lists (fctl#788): an hourly
-- triage's runs would fill them. 1: GET /api/v1/sessions leaves out the
-- sessions they started, unless it is asked for them (`?hidden=1`); their
-- runs and search still find them.
ALTER TABLE schedules ADD COLUMN hide_sessions INTEGER NOT NULL DEFAULT 0;
