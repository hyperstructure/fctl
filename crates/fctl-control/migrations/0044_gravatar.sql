-- Gravatar: people who have no picture of their own show their Gravatar,
-- which the control plane looks up by their primary address and copies
-- into user_avatars (api::avatars::gravatar).
--
-- users.gravatar: whether they let it (Settings → Profile); on unless they
-- turn it off or remove their picture. users.gravatar_checked_at: when fctl
-- last looked, so that it looks once a day at most.
ALTER TABLE users ADD COLUMN gravatar INTEGER NOT NULL DEFAULT 1;
ALTER TABLE users ADD COLUMN gravatar_checked_at INTEGER;
-- Where each picture came from: upload, google or gravatar. NULL for those
-- from before, uploaded or Google's.
ALTER TABLE user_avatars ADD COLUMN source TEXT;
