//! Slack (docs/architecture.md#slack): fctl's Slack app, which workspaces
//! install in their Slack workspaces so that people there start and steer
//! agents; what agents read and post there with `fctl slack` in their
//! sandboxes ([`SlackRequest`], [`SlackAnswer`]); what a session that
//! started there keeps and shows ([`SessionSlack`], [`SlackMessage`],
//! [`SlackNotice`]); and what the web app and the CLI show and set
//! ([`WorkspaceSlack`], [`SlackSettings`], [`MySlack`], [`AdminSlack`]).
//!
//! Slack's own boundary is membership: the app reads every conversation it
//! is in. What an agent reaches is fctl's to narrow, so the agent never
//! holds a Slack token: every `fctl slack` call goes through the control
//! plane, which checks the conversation with Slack against the session and
//! its workspace's and project's choices before it reads anything.

use crate::session::UserRef;
use crate::{Millis, ProjectId, WorkspaceId};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

// ───────────────────────────── places and people ─────────────────────────────

/// What kind of conversation a Slack message is in, as fctl tells them
/// apart: what an agent may read there depends on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SlackPlace {
    /// A public channel.
    #[default]
    Channel,
    /// A private channel: agents read only their own thread there.
    Private,
    /// Someone's direct messages with fctl's app: agents read only their
    /// own thread there, and never anyone else's.
    Dm,
    /// One a newer control plane knows, which this version doesn't.
    #[serde(other)]
    Other,
}

impl SlackPlace {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Private => "private",
            Self::Dm => "dm",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "channel" => Some(Self::Channel),
            "private" => Some(Self::Private),
            "dm" => Some(Self::Dm),
            _ => None,
        }
    }
}

/// Someone in Slack, as Slack names them.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackPerson {
    /// Their name in Slack (its real name, else its display name).
    pub name: String,
    /// Slack's id for them (`U…`).
    pub user: Option<String>,
    /// The display name people mention them by, when it differs.
    pub handle: Option<String>,
    pub avatar_url: Option<String>,
    /// An app or a bot rather than a person: fctl's own, say.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub bot: bool,
}

/// A channel, as fctl names it: Slack's id and its name without `#`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SlackChannelRef {
    /// Slack's id (`C…`).
    pub id: String,
    pub name: String,
}

// ───────────────────────────── sessions ─────────────────────────────

/// Where in Slack a session started (`Session::slack`): a thread of a
/// channel, or of someone's direct messages with fctl's app. Its card,
/// replies and questions go there, and replies there steer it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SessionSlack {
    /// Slack's id of the Slack workspace (`T…`), and its name.
    pub team: String,
    pub team_name: Option<String>,
    /// The conversation (`C…`, `G…`, `D…`), and the channel's name without
    /// `#`: absent for direct messages.
    pub channel: String,
    pub channel_name: Option<String>,
    pub place: SlackPlace,
    /// The thread's first message (Slack's `ts`).
    pub thread_ts: String,
    /// The thread in Slack.
    pub url: String,
}

/// How a message from Slack came to the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SlackEvent {
    /// Someone mentioned fctl's app in a channel.
    Mentioned,
    /// Someone wrote to fctl's app in their direct messages with it.
    Direct,
    /// Someone replied in the session's thread.
    Replied,
    /// Someone clicked an answer to the agent's question.
    Answered,
    /// Someone sent a message to fctl with its shortcut (Send to fctl…).
    Shortcut,
    /// Someone started it with `/fctl`, or from fctl's Home tab.
    Command,
    /// One a newer control plane knows, which this version doesn't.
    #[serde(other)]
    Other,
}

/// For a message to the agent from Slack (`UserMessage::slack`): who wrote
/// what where, for the apps to show it as Slack does. The message's text
/// says the same to the agent.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackMessage {
    pub event: SlackEvent,
    pub by: Option<SlackPerson>,
    pub place: SlackPlace,
    /// The channel's name without `#`; absent for direct messages.
    pub channel_name: Option<String>,
    /// The Slack workspace's name.
    pub team_name: Option<String>,
    /// What they wrote, as text: absent where the workspace keeps no Slack
    /// messages in transcripts once the session ends, and fctl dropped it.
    pub body: Option<String>,
    /// The message in Slack.
    pub url: Option<String>,
    /// For a session that starts: how many of the thread's earlier
    /// messages came with it, as information.
    pub context: Option<u32>,
    /// For a message sent with the shortcut: who wrote the message sent.
    pub quoted: Option<SlackPerson>,
    /// fctl dropped what Slack's messages said from the transcript, as
    /// the workspace asks once a session ends.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub dropped: bool,
}

/// What a `slack_*` notice is about: what fctl posted in the session's
/// Slack thread for it (`slack_replied`: the turn's last message;
/// `slack_asked`: the agent's question, with its answers), what the agent
/// posted there itself (`slack_posted`, `fctl slack reply`), or why fctl
/// couldn't post (`slack_failed`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackNotice {
    pub place: SlackPlace,
    pub channel_name: Option<String>,
    /// What it posted, and its link.
    pub body: Option<String>,
    pub url: Option<String>,
    /// For what it read: how many messages.
    pub messages: Option<u32>,
}

/// What a session's agent is told about Slack (`SessionSpec::slack`): the
/// control plane says it, so that what agents are told grows with what it
/// lets them do. Absent for sessions whose workspace works in no Slack
/// workspace, review agents, and from older control planes; older hosts
/// leave it out, and their agents hear nothing of Slack.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackSpec {
    /// The thread the session started from.
    pub thread: Option<SessionSlack>,
    /// The channels its agent may read beyond its thread.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<SlackChannelRef>>", optional))]
    pub readable: Vec<SlackChannelRef>,
    /// It may search them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub search: bool,
    /// It may post in its own thread (`fctl slack reply`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub reply: bool,
    /// The system prompt's lines about it, for an agent whose sandbox's
    /// fctl has `fctl slack`.
    pub instructions: String,
    /// What it says instead where the sandbox's fctl has no `fctl slack`:
    /// how replies go back without it.
    pub without_cli: Option<String>,
}

impl SlackSpec {
    /// Whether the agent reaches anything with `fctl slack`.
    pub fn reaches(&self) -> bool {
        self.thread.is_some() || !self.readable.is_empty()
    }
}

// ───────────────────────────── fctl slack ─────────────────────────────

/// What an agent asks of Slack with `fctl slack` in its sandbox: an
/// [`crate::api::AgentCall::Slack`], which hosts and fctl-agent pass on as
/// it is, so that what `fctl slack` can do grows with the control plane.
/// The control plane keeps every call to what the session may reach: its
/// own thread (in any conversation), and the public channels its
/// workspace's admins made readable, where its project reads them.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SlackRequest {
    /// What the agent reaches in Slack: its thread, the channels it may
    /// read, whether it may search and post.
    About,
    /// The thread the session started from: what's new since the agent
    /// last read it, or with `all` every message.
    Thread {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
        all: bool,
    },
    /// A message and its thread, by its link, in a readable channel or the
    /// session's own thread.
    Read { link: String },
    /// A readable channel's messages, newest last: since a time, at most
    /// `limit` (default 50, at most 200).
    History {
        /// `#name`, `name` or Slack's id.
        channel: String,
        #[cfg_attr(feature = "ts", ts(optional))]
        since: Option<Millis>,
        #[cfg_attr(feature = "ts", ts(optional))]
        limit: Option<u32>,
    },
    /// Messages of the readable channels that say something, newest first.
    Search {
        query: String,
        /// In one of them only.
        #[cfg_attr(feature = "ts", ts(optional))]
        channel: Option<String>,
        #[cfg_attr(feature = "ts", ts(optional))]
        limit: Option<u32>,
    },
    /// Posts in the session's own thread, as fctl's app.
    Reply { text: String },
}

impl SlackRequest {
    /// Whether it only reads, so that asking again changes nothing.
    pub fn reads(&self) -> bool {
        !matches!(self, Self::Reply { .. })
    }
}

/// A message, as `fctl slack` shows it to the agent.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackMessageView {
    /// Slack's `ts`: its id in its conversation.
    pub ts: String,
    pub at: Millis,
    pub by: SlackPerson,
    pub text: String,
    /// The channel's name, for search results across channels.
    pub channel_name: Option<String>,
    pub url: Option<String>,
    /// Its thread, when it is a reply, or a thread's first message.
    pub thread_ts: Option<String>,
    /// How many replies its thread has, for a thread's first message.
    pub replies: Option<u32>,
    /// New since the agent last read the thread.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub new: bool,
    /// fctl passed it on to the agent as its author's message: what they
    /// asked of it. Others are information from people who may not
    /// instruct it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub delivered: bool,
    /// Its files, by name: `fctl slack` doesn't fetch them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub files: Vec<String>,
}

/// The control plane's answer to a [`SlackRequest`].
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackAnswer {
    About {
        about: SlackAbout,
    },
    /// Messages, oldest first: a thread, a channel's history, or search
    /// results (newest first).
    Messages {
        #[cfg_attr(feature = "ts", ts(optional))]
        channel: Option<SlackChannelRef>,
        messages: Vec<SlackMessageView>,
        /// More match, or came before, than it shows.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
        more: bool,
        /// What the agent should know of how it read them: Slack limits
        /// how often fctl reads, say.
        #[cfg_attr(feature = "ts", ts(optional))]
        note: Option<String>,
    },
    /// What it posted, and its link.
    Posted {
        ts: String,
        #[cfg_attr(feature = "ts", ts(optional))]
        url: Option<String>,
    },
}

/// What `fctl slack` says the agent reaches.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackAbout {
    pub team_name: Option<String>,
    /// The thread the session started from.
    pub thread: Option<SessionSlack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<SlackChannelRef>>", optional))]
    pub readable: Vec<SlackChannelRef>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub search: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub reply: bool,
}

// ───────────────────────────── workspaces ─────────────────────────────

/// What a workspace keeps of Slack in its sessions' transcripts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SlackTranscripts {
    /// What agents read, and the messages that came from Slack, stay in
    /// the transcript, as anything else there does.
    #[default]
    Keep,
    /// Once a session ends (it stopped, failed or was archived), fctl drops
    /// what Slack's messages said from its transcript, and keeps who wrote
    /// them and where.
    Drop,
}

/// What a workspace lets people and agents do with a Slack workspace it
/// works in. Its admins choose; [`SlackSettings::default`] is what a new
/// connection starts with. An update that leaves a field out keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct SlackSettings {
    /// Mentioning fctl's app in a channel starts a session, and so do
    /// `/fctl` and its shortcut, Send to fctl….
    pub channels: bool,
    /// Writing to fctl's app in its direct messages starts a session.
    pub direct_messages: bool,
    /// Agents search the readable channels (`fctl slack search`).
    pub search: bool,
    /// Agents post in their own thread (`fctl slack reply`), beside the
    /// replies fctl posts at each turn's end.
    pub agent_replies: bool,
    /// People get their notifications from fctl's app, where they choose
    /// "Tell me in Slack".
    pub notifications: bool,
    /// Projects post events (pull requests ready for review, failures) to
    /// a channel.
    pub project_events: bool,
    /// fctl knows people by their verified address in Slack, where they
    /// haven't linked their Slack account, as it is the address of one of
    /// the workspace's members: only where the instance's admins allow it
    /// ([`crate::api::InstanceSettings::slack_match_email`]).
    pub match_email: bool,
    pub transcripts: SlackTranscripts,
}

impl Default for SlackSettings {
    fn default() -> Self {
        Self {
            channels: true,
            direct_messages: true,
            search: true,
            agent_replies: true,
            notifications: true,
            project_events: true,
            match_email: false,
            transcripts: SlackTranscripts::Keep,
        }
    }
}

/// `PATCH /api/v1/workspaces/{id}/slack/{team}` (admins): what it names
/// changes, the rest stays.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateSlackRequest {
    pub channels: Option<bool>,
    pub direct_messages: Option<bool>,
    pub search: Option<bool>,
    pub agent_replies: Option<bool>,
    pub notifications: Option<bool>,
    pub project_events: Option<bool>,
    pub match_email: Option<bool>,
    pub transcripts: Option<SlackTranscripts>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SlackStatus {
    Active,
    /// Slack revoked fctl's access: the app was uninstalled there, or its
    /// token revoked. An admin connects it again.
    Revoked,
}

/// A project, as Slack's pages name it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SlackProjectRef {
    pub id: ProjectId,
    pub slug: String,
    pub name: String,
}

/// A channel a workspace's admins chose something for: the project its
/// mentions start sessions in, and whether agents read it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackChannel {
    pub id: String,
    /// Its name without `#`, as fctl last saw it.
    pub name: String,
    /// Mentions of fctl's app here start sessions in this project.
    pub project: Option<SlackProjectRef>,
    /// Agents of the workspace's projects that read Slack read it
    /// (`fctl slack read`, `history`, `search`); fctl's app joined it, which
    /// shows there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub readable: bool,
    /// A private channel: mentions there start sessions, but agents never
    /// read more of it than their own thread.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub private: bool,
    pub updated_at: Millis,
}

/// What one of the workspace's projects does with Slack.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackProjectUse {
    pub id: ProjectId,
    pub slug: String,
    pub name: String,
    /// Its agents read the readable channels.
    pub read_channels: bool,
    /// The channel its events go to, and which events.
    pub events: Option<SlackEvents>,
    /// The channels whose mentions start its sessions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<SlackChannelRef>>", optional))]
    pub mapped: Vec<SlackChannelRef>,
}

/// A Slack workspace an fctl workspace works in: fctl's app is installed
/// there, so people start and steer agents from it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackConnection {
    /// Slack's team id (`T…`).
    pub team: String,
    pub name: String,
    /// Its address: `<domain>.slack.com`.
    pub domain: Option<String>,
    pub icon_url: Option<String>,
    /// fctl's app user there, which people mention (`<@U…>`), and its
    /// name.
    pub bot_user: Option<String>,
    pub app_name: Option<String>,
    pub status: SlackStatus,
    /// Who connected it, and as whom they signed in to Slack.
    pub connected_by: Option<UserRef>,
    pub slack_user: Option<String>,
    pub created_at: Millis,
    pub settings: SlackSettings,
    /// The channels its admins chose something for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<SlackChannel>>", optional))]
    pub channels: Vec<SlackChannel>,
    /// The workspace's projects that the caller sees, and what each does
    /// with Slack.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<SlackProjectUse>>", optional))]
    pub projects: Vec<SlackProjectUse>,
    /// What agents can read there, in a sentence: "Agents read the thread
    /// they started from, and #incidents and #deploys."
    pub reach: String,
    /// The workspace's members who linked their Slack account there.
    pub linked: u32,
}

/// `GET /api/v1/workspaces/{id}/slack`: the Slack workspaces it works in.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WorkspaceSlack {
    /// The instance has a Slack app (`SLACK_CLIENT_ID`, and its secret).
    pub configured: bool,
    /// Slack's events reach it: over HTTP (`SLACK_SIGNING_SECRET`) or Socket
    /// Mode (`SLACK_APP_TOKEN`). Without either, nothing in Slack reaches
    /// fctl.
    pub events: bool,
    /// They come over Socket Mode, and whether it is connected now.
    pub socket_mode: bool,
    pub socket_connected: Option<bool>,
    /// The instance lets workspaces know people by their verified address
    /// in Slack.
    pub match_email: bool,
    /// The caller is one of the instance's admins, who set the Slack app up.
    pub instance_admin: bool,
    pub connections: Vec<SlackConnection>,
    /// Whether the caller linked their own Slack account in each
    /// connection's Slack workspace, by team.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub linked_teams: Vec<String>,
}

/// A link that signs in to Slack, for the browser to open: to install
/// fctl's app in a Slack workspace, or to link your Slack account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SlackLink {
    pub url: String,
    pub expires_at: Millis,
}

/// A public channel of a connected Slack workspace, for its admins to
/// choose (`GET /api/v1/workspaces/{id}/slack/{team}/channels`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackChannelChoice {
    pub id: String,
    pub name: String,
    pub topic: Option<String>,
    pub members: Option<u32>,
    /// fctl's app is in it already.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub member: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub private: bool,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackChannelList {
    pub channels: Vec<SlackChannelChoice>,
    /// More than it shows: a query narrows them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub more: bool,
}

/// `PUT /api/v1/workspaces/{id}/slack/{team}/channels/{channel}` (admins):
/// what the workspace does with a channel. What it leaves out stays; a
/// channel with neither a project nor readable goes from the list.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SetSlackChannelRequest {
    /// Its name, as the list of channels gave it.
    pub name: Option<String>,
    /// The project whose sessions its mentions start, by id or slug; an
    /// empty one for none.
    pub project: Option<String>,
    pub readable: Option<bool>,
}

// ───────────────────────────── projects ─────────────────────────────

/// A channel a project posts chosen events to, through fctl's app
/// (`ProjectSettings::slack`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackEvents {
    pub team: String,
    pub channel: String,
    /// Its name without `#`, as it was chosen.
    pub channel_name: String,
    /// Webhooks' event types (`session.ready_for_review`,
    /// `session.failed`…); empty: those [`SlackEvents::DEFAULT`] names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub kinds: Vec<String>,
}

impl SlackEvents {
    /// What a channel hears when nobody chose: pull requests ready for
    /// review, and failures.
    pub const DEFAULT: [&'static str; 2] = ["session.ready_for_review", "session.failed"];
}

/// What a project does with Slack (`ProjectSettings::slack`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct SlackProject {
    /// Its agents read the channels its workspace's admins made readable.
    /// On by default: those admins chose them for agents to read.
    pub read_channels: bool,
    /// A channel it posts chosen events to.
    pub events: Option<SlackEvents>,
}

impl Default for SlackProject {
    fn default() -> Self {
        Self {
            read_channels: true,
            events: None,
        }
    }
}

// ───────────────────────────── yours ─────────────────────────────

/// How fctl knows you in a Slack workspace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SlackVia {
    /// You signed in to Slack from fctl.
    #[default]
    Linked,
    /// Your verified address there is one of your account's, where the
    /// instance and your workspace let fctl match them.
    Email,
}

/// One of your Slack accounts, which fctl knows you by in that Slack
/// workspace: what you ask fctl's app there runs as you.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackAccount {
    pub team: String,
    pub team_name: String,
    /// `<domain>.slack.com`.
    pub team_domain: Option<String>,
    /// Slack's id for you there.
    pub slack_user: String,
    pub name: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub via: SlackVia,
    /// The project your direct messages with fctl's app start sessions in,
    /// when you chose to keep it.
    pub dm_project: Option<SlackProjectRef>,
    pub linked_at: Millis,
}

/// A Slack workspace one of your workspaces works in, which you could
/// link your account in.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackTeamRef {
    pub team: String,
    pub name: String,
    pub domain: Option<String>,
    pub icon_url: Option<String>,
    /// The fctl workspaces of yours that work in it.
    pub workspaces: Vec<SlackWorkspaceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SlackWorkspaceRef {
    pub id: WorkspaceId,
    pub slug: String,
    pub name: String,
}

/// `GET /api/v1/me/slack`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct MySlack {
    /// The instance has a Slack app.
    pub configured: bool,
    pub accounts: Vec<SlackAccount>,
    /// The Slack workspaces your workspaces work in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<SlackTeamRef>>", optional))]
    pub teams: Vec<SlackTeamRef>,
}

/// A Slack workspace where "Tell me in Slack" sends your notifications
/// about its fctl workspaces' sessions (`api::Notifications::slack`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SlackNotifyPlace {
    pub team: String,
    pub team_name: String,
    /// The fctl workspaces whose sessions it is for.
    pub workspaces: Vec<SlackWorkspaceRef>,
    /// Your account there, once linked: until then nothing goes.
    pub account: Option<SlackPerson>,
    /// Those workspaces' admins let notifications go to Slack.
    pub allowed: bool,
}

/// `PATCH /api/v1/me/slack/{team}`: your choices there.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateMySlackRequest {
    /// The project your direct messages start sessions in, by id or slug;
    /// an empty one to choose each time.
    pub dm_project: Option<String>,
}

// ───────────────────────────── the instance's ─────────────────────────────

/// `GET /api/v1/admin/slack` (instance admins): the instance's Slack app,
/// as it is set up, and the manifest that creates one.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AdminSlack {
    /// `SLACK_CLIENT_ID` and `SLACK_CLIENT_SECRET` are set.
    pub configured: bool,
    /// `SLACK_SIGNING_SECRET` is set: HTTP events and interactions count.
    pub signing_secret: bool,
    /// `SLACK_APP_TOKEN` is set: events come over Socket Mode.
    pub socket_mode: bool,
    /// Whether Socket Mode is connected now, and since when; what went
    /// wrong last.
    pub socket_connected: Option<bool>,
    pub socket_since: Option<Millis>,
    pub socket_error: Option<String>,
    /// Slack can reach the instance (an `https://` address that isn't a
    /// private one): without that, only Socket Mode brings its events.
    pub reachable: bool,
    /// The manifest that creates the app, with the instance's addresses:
    /// as JSON, and as YAML for Slack's form.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub manifest: serde_json::Value,
    pub manifest_yaml: String,
    /// Opens Slack's form to create an app from the manifest.
    pub create_url: String,
    /// The addresses Slack calls.
    pub events_url: String,
    pub interactivity_url: String,
    pub commands_url: String,
    pub redirect_urls: Vec<String>,
    /// The Slack workspaces the app is installed in, and how many fctl
    /// workspaces work in each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<AdminSlackTeam>>", optional))]
    pub teams: Vec<AdminSlackTeam>,
    /// Workspaces may know people by their verified address in Slack
    /// ([`crate::api::InstanceSettings::slack_match_email`]).
    pub match_email: bool,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AdminSlackTeam {
    pub team: String,
    pub name: String,
    pub domain: Option<String>,
    pub status: SlackStatus,
    pub workspaces: u32,
    pub installed_at: Millis,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a test expects to be there: it fails the test where it isn't.
    trait Must<T> {
        fn must(self) -> T;
    }

    impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
        fn must(self) -> T {
            match self {
                Ok(v) => v,
                Err(e) => panic!("{e:?}"),
            }
        }
    }

    impl<T> Must<T> for Option<T> {
        fn must(self) -> T {
            match self {
                Some(v) => v,
                None => panic!("nothing there"),
            }
        }
    }

    #[test]
    fn requests_read_as_agents_send_them() {
        let r: SlackRequest = serde_json::from_str(r#"{"op":"thread"}"#).must();
        assert_eq!(r, SlackRequest::Thread { all: false });
        assert!(r.reads());
        let r: SlackRequest = serde_json::from_str(r#"{"op":"reply","text":"On it"}"#).must();
        assert!(!r.reads());
        let r: SlackRequest =
            serde_json::from_str(r##"{"op":"history","channel":"#deploys","since":1}"##).must();
        assert_eq!(
            r,
            SlackRequest::History {
                channel: "#deploys".into(),
                since: Some(1),
                limit: None
            }
        );
    }

    #[test]
    fn settings_keep_their_defaults_for_what_they_leave_out() {
        let s: SlackSettings = serde_json::from_str(r#"{"channels":false}"#).must();
        assert!(!s.channels);
        assert!(s.direct_messages && s.agent_replies && s.search && s.notifications);
        assert!(!s.match_email);
        assert_eq!(s.transcripts, SlackTranscripts::Keep);
        let p: SlackProject = serde_json::from_str("{}").must();
        assert!(p.read_channels);
    }

    #[test]
    fn unknown_places_read_as_other() {
        let p: SlackPlace = serde_json::from_str(r#""huddle""#).must();
        assert_eq!(p, SlackPlace::Other);
        let e: SlackEvent = serde_json::from_str(r#""reacted""#).must();
        assert_eq!(e, SlackEvent::Other);
    }
}
