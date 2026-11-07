-- Keeping one's Claude token out of sandboxes (Settings → Claude): the
-- sessions that run on the person's Claude connections give Claude a
-- stand-in, and the host of the runner puts the token in as Claude's
-- requests leave for the Anthropic API (ClaudeCredential.stand_in). Off
-- unless they turn it on.
ALTER TABLE users ADD COLUMN claude_stand_in INTEGER NOT NULL DEFAULT 0;
