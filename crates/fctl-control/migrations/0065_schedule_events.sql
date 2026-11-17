-- Schedules that run on events (docs/architecture.md#schedules,
-- fctl_domain::triggers): each time something happens on GitHub in the
-- project's repositories, such as an issue opened or a pull request
-- merged, GitHub's webhook starts a run of their own.

-- What a schedule waits for: the kind of event (issue_opened,
-- issue_labeled, pull_request_opened, pull_request_merged,
-- release_published, checks_failed), and what narrows it down (JSON:
-- repos, labels, branches, by). NULL for schedules that run at their time
-- or when asked.
ALTER TABLE schedules ADD COLUMN event_kind TEXT;
ALTER TABLE schedules ADD COLUMN event_filter TEXT;
CREATE INDEX IF NOT EXISTS schedules_event ON schedules (event_kind) WHERE event_kind IS NOT NULL;

-- For a run an event started (trigger 'event'): what happened (JSON, the
-- API's ScheduleRunEvent); the key that keeps a delivery GitHub sends
-- again, or one thing, from starting two runs (fctl_domain::triggers::key);
-- and what it works on (an issue, a pull request, a release, a branch's
-- checks: fctl_domain::triggers::subject), which starts no other run while
-- this one's session still works.
ALTER TABLE schedule_runs ADD COLUMN event TEXT;
ALTER TABLE schedule_runs ADD COLUMN event_key TEXT;
ALTER TABLE schedule_runs ADD COLUMN subject TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS schedule_runs_event_key ON schedule_runs (schedule_id, event_key)
  WHERE event_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS schedule_runs_subject ON schedule_runs (schedule_id, subject, at)
  WHERE subject IS NOT NULL;
