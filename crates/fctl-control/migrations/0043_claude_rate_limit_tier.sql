-- The rate limit tier of a Claude login (`rateLimitTier`), which tells
-- Claude's Ultra plan (Max 20x before) from Max 5x: both have the
-- subscription type `max`. Logins kept before this have none.
ALTER TABLE claude_credentials ADD COLUMN rate_limit_tier TEXT;
