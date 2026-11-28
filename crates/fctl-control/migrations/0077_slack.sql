-- Slack (docs/architecture.md#slack): fctl's Slack app, which workspaces
-- install in their Slack workspaces so that people there start and steer
-- agents.

-- Slack workspaces fctl's app is installed in, by Slack's team id: one row
-- each, whose bot token every fctl workspace connected to it shares, since
-- Slack keeps one bot token for an app in a Slack workspace and installing
-- the app again replaces it. The token is sealed with the master key (bound
-- to `slack:<team_id>`). `status` is active, or revoked once Slack says the
-- app was uninstalled or its tokens revoked.
CREATE TABLE IF NOT EXISTS slack_installations (
  team_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  domain TEXT,
  icon_url TEXT,
  app_id TEXT,
  -- fctl's app user there, whom people mention, and its bot's id.
  bot_user_id TEXT NOT NULL,
  bot_id TEXT,
  secret_enc TEXT NOT NULL,
  scopes TEXT,
  status TEXT NOT NULL DEFAULT 'active',
  installed_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- The fctl workspaces that work in a Slack workspace: an admin of the fctl
-- workspace installed fctl's app there, signed in to Slack as `slack_user`.
-- `settings` is what the workspace lets people and agents do there
-- (JSON: a SlackSettings). Once Slack revokes the app, every connection to
-- it ends (`revoked_at`) until an admin connects it again.
CREATE TABLE IF NOT EXISTS workspace_slack (
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  team_id TEXT NOT NULL REFERENCES slack_installations(team_id) ON DELETE CASCADE,
  connected_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  slack_user TEXT,
  settings TEXT NOT NULL DEFAULT '{}',
  revoked_at INTEGER,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, team_id)
);
CREATE INDEX IF NOT EXISTS workspace_slack_team ON workspace_slack (team_id);

-- Who people are in Slack workspaces: from a Sign in with Slack of theirs
-- (`via` linked), or their verified address there where the instance and
-- the workspace let fctl match it (`via` email). What they ask fctl's app
-- there runs as them. One account per person and Slack workspace, and one
-- person per Slack account. `dm_project_id` is the project their direct
-- messages with the app start sessions in, when they chose to keep it.
CREATE TABLE IF NOT EXISTS slack_accounts (
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  team_id TEXT NOT NULL,
  team_name TEXT NOT NULL,
  team_domain TEXT,
  slack_user_id TEXT NOT NULL,
  name TEXT NOT NULL,
  email TEXT,
  avatar_url TEXT,
  via TEXT NOT NULL DEFAULT 'linked',
  dm_project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
  linked_at INTEGER NOT NULL,
  PRIMARY KEY (user_id, team_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS slack_accounts_slack_user ON slack_accounts (team_id, slack_user_id);

-- Channels a workspace's admins chose something for: the project whose
-- sessions mentions of fctl's app there start (`project_id`), and whether
-- agents of its projects that read Slack read it (`readable`, public
-- channels only, which fctl's app joins).
CREATE TABLE IF NOT EXISTS slack_channels (
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  team_id TEXT NOT NULL,
  channel_id TEXT NOT NULL,
  name TEXT NOT NULL,
  private INTEGER NOT NULL DEFAULT 0,
  project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
  readable INTEGER NOT NULL DEFAULT 0,
  added_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, team_id, channel_id)
);
CREATE INDEX IF NOT EXISTS slack_channels_channel ON slack_channels (team_id, channel_id);

-- Slack threads that reach sessions, by `<team>:<channel>:<thread_ts>`: a
-- thread where someone mentioned fctl's app, a thread of someone's direct
-- messages with it, or the message `/fctl`, its shortcut or its Home tab
-- started. `state` is new, waiting (for its person to link their Slack
-- account, or to choose the project: `waiting_for`, with the event kept in
-- `payload` until `expires_at`), started (it reaches `session_id`), refused
-- or expired. `card_ts` is the message fctl keeps current with how the
-- session stands, `question_ts` the agent's open question, `read_ts` the
-- newest message its agent read, `delivered` the messages fctl passed on as
-- people's (JSON: their ts, the newest 200), `updates` whether fctl posts
-- in the thread at all (a shortcut's choice), `stream` an open stream of the
-- agent's words (JSON).
CREATE TABLE IF NOT EXISTS slack_threads (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL,
  channel_id TEXT NOT NULL,
  thread_ts TEXT NOT NULL,
  place TEXT NOT NULL,
  session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
  slack_user_id TEXT,
  state TEXT NOT NULL,
  waiting_for TEXT,
  payload TEXT,
  expires_at INTEGER,
  card_ts TEXT,
  question_ts TEXT,
  read_ts TEXT,
  delivered TEXT,
  updates INTEGER NOT NULL DEFAULT 1,
  stream TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS slack_threads_session ON slack_threads (session_id, updated_at);
CREATE INDEX IF NOT EXISTS slack_threads_waiting ON slack_threads (team_id, slack_user_id) WHERE state = 'waiting';

-- The Slack thread a session started from (JSON: a SessionSlack).
ALTER TABLE sessions ADD COLUMN slack TEXT;

-- What sessions' agents did in Slack in the last hour (`kind`: read,
-- search or post), which keeps each to so many an hour
-- (fctl_domain::slack::limit_of); the hourly job forgets the rest.
CREATE TABLE IF NOT EXISTS slack_calls (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS slack_calls_session ON slack_calls (session_id, kind, at);
