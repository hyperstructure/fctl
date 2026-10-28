//! Tickets in a project's ticket tracker ([`crate::session::Tracker`]), as
//! fctl shows them whatever the tracker: what agents read and write with
//! `fctl ticket` in their sandboxes ([`TicketRequest`], [`TicketAnswer`]),
//! what a session that started from one is told and shows
//! ([`TicketsSpec`], [`TicketMessage`], [`TicketNotice`]), and the Linear
//! workspaces that people connect.
//!
//! Trackers differ: Linear has teams, workflow states, priorities,
//! sub-issues and projects; GitHub has repositories, open and closed, and
//! sub-issues and issue types of late; Jira has epics and fields of its
//! own. A [`Ticket`] holds what they share (a key, a title, a description,
//! a state of a [`StateKind`], labels, people, a parent and children) and
//! the rest as named [`TicketField`]s, so that a tracker added later fits
//! without new types.

use crate::session::{RepoRef, SessionTicket, Tracker, UserRef};
use crate::{Millis, WorkspaceId};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

// ───────────────────────────── tickets ─────────────────────────────

/// What kind of state a ticket is in, whatever its tracker calls the state:
/// Linear's workflow state types, and GitHub's open (`Unstarted`) and
/// closed (`Completed`, or `Canceled` when not planned).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StateKind {
    /// Waiting to be looked at (Linear's triage).
    Triage,
    Backlog,
    /// To do.
    Unstarted,
    /// In progress, in review.
    Started,
    /// Done.
    Completed,
    /// Canceled, a duplicate, not planned.
    Canceled,
}

impl StateKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Triage => "triage",
            Self::Backlog => "backlog",
            Self::Unstarted => "unstarted",
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "triage" => Some(Self::Triage),
            "backlog" => Some(Self::Backlog),
            "unstarted" => Some(Self::Unstarted),
            "started" => Some(Self::Started),
            "completed" => Some(Self::Completed),
            "canceled" | "cancelled" => Some(Self::Canceled),
            _ => None,
        }
    }

    /// Whether a ticket in it is still to do or under way.
    pub fn is_open(self) -> bool {
        !matches!(self, Self::Completed | Self::Canceled)
    }
}

/// A ticket's state: the tracker's name for it (`In Review`, `closed`),
/// and its kind.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TicketState {
    pub name: String,
    pub kind: StateKind,
    /// The tracker's id for it, where it has one (Linear's).
    pub id: Option<String>,
}

/// A person in a tracker, as the tracker names them.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TicketPerson {
    pub name: String,
    /// Their handle there (`ada` on GitHub, Linear's display name).
    pub handle: Option<String>,
    pub avatar_url: Option<String>,
    /// An app rather than a person (fctl's own agent, say).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub app: bool,
}

/// Another ticket, as a ticket names it: its parent, a child.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TicketLink {
    pub key: String,
    pub title: String,
    pub url: String,
    pub state: Option<TicketState>,
}

/// A field a tracker has that [`Ticket`] has no place for: an estimate, a
/// due date, a cycle, a milestone, an issue type's custom field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TicketField {
    pub name: String,
    pub value: String,
}

/// A comment on a ticket.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TicketComment {
    /// The tracker's id, which a reply names.
    pub id: String,
    pub author: Option<TicketPerson>,
    /// Markdown.
    pub body: String,
    pub url: Option<String>,
    pub created_at: Option<Millis>,
    /// The comment it answers, in trackers that thread them.
    pub parent: Option<String>,
}

/// A ticket, as `fctl ticket` shows it whatever its tracker.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Ticket {
    pub tracker: Tracker,
    /// The tracker's own id: Linear's issue id; `owner/name#12` on GitHub.
    pub id: String,
    /// What people call it: `ENG-123`, `acme/web#12`.
    pub key: String,
    pub title: String,
    pub url: String,
    /// Markdown.
    pub description: Option<String>,
    pub state: Option<TicketState>,
    /// What kind of ticket: an issue type (bug, feature, epic), where the
    /// tracker has them.
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub labels: Vec<String>,
    /// Its priority, in the tracker's words (`Urgent`, `High`…).
    pub priority: Option<String>,
    pub assignee: Option<TicketPerson>,
    /// Whom its assignee delegated it to (Linear's agents).
    pub delegate: Option<TicketPerson>,
    /// Where it is filed: Linear's team and project, GitHub's repository.
    pub place: Option<String>,
    pub parent: Option<TicketLink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<TicketLink>>", optional))]
    pub children: Vec<TicketLink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<TicketField>>", optional))]
    pub fields: Vec<TicketField>,
    /// Its comments, oldest first, when asked for one ticket.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<TicketComment>>", optional))]
    pub comments: Vec<TicketComment>,
    /// How many earlier comments `comments` leaves out.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<u32>", optional))]
    pub earlier_comments: u32,
    pub created_at: Option<Millis>,
    pub updated_at: Option<Millis>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Ticket {
    /// The ticket as other tickets and sessions name it.
    pub fn link(&self) -> TicketLink {
        TicketLink {
            key: self.key.clone(),
            title: self.title.clone(),
            url: self.url.clone(),
            state: self.state.clone(),
        }
    }

    /// The ticket as a session that works on it names it.
    pub fn session_ticket(&self) -> SessionTicket {
        SessionTicket {
            tracker: self.tracker,
            id: self.id.clone(),
            key: self.key.clone(),
            title: Some(self.title.clone()),
            url: self.url.clone(),
        }
    }
}

// ───────────────────────────── what agents ask ─────────────────────────────

/// Which tickets a list shows, by their state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StateFilter {
    /// To do or under way.
    #[default]
    Open,
    /// Done or canceled.
    Closed,
    All,
}

/// What an agent asks of its project's ticket tracker, with `fctl ticket`
/// in its sandbox: an [`crate::api::AgentCall::Ticket`], which hosts and
/// fctl-agent pass on as it is, so that what `fctl ticket` can do grows
/// with the control plane. Tickets are named by their key (`ENG-123`;
/// `#12` or `acme/web#12` on GitHub). The control plane keeps every call to
/// the tickets the project chose (its Linear team, its repositories), and
/// writes in the name of fctl's own app.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TicketRequest {
    /// The project's tracker, what of it agents reach, and the tickets the
    /// session works on.
    About,
    /// Tickets, newest first.
    List {
        #[serde(default)]
        state: StateFilter,
        /// Words their title or description has.
        #[cfg_attr(feature = "ts", ts(optional))]
        query: Option<String>,
        #[cfg_attr(feature = "ts", ts(optional))]
        label: Option<String>,
        /// At most this many (default 20, at most 50).
        #[cfg_attr(feature = "ts", ts(optional))]
        limit: Option<u32>,
    },
    /// One ticket, with its comments.
    View { key: String },
    /// A new ticket. Without `place`, where the project files them (its
    /// Linear team, its GitHub repository); `place` names a repository
    /// for GitHub Issues of several.
    Create {
        title: String,
        #[cfg_attr(feature = "ts", ts(optional))]
        description: Option<String>,
        /// Its parent, by key: it becomes a sub-ticket.
        #[cfg_attr(feature = "ts", ts(optional))]
        parent: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
        labels: Vec<String>,
        /// A state by its name, for trackers whose new tickets may start
        /// in one (Linear).
        #[cfg_attr(feature = "ts", ts(optional))]
        state: Option<String>,
        /// A priority by its name (`Urgent`, `High`, `Medium`, `Low`).
        #[cfg_attr(feature = "ts", ts(optional))]
        priority: Option<String>,
        #[cfg_attr(feature = "ts", ts(optional))]
        place: Option<String>,
    },
    /// A comment on a ticket, or with `reply_to` an answer to one of its
    /// comments, in trackers that thread them.
    Comment {
        key: String,
        body: String,
        #[cfg_attr(feature = "ts", ts(optional))]
        reply_to: Option<String>,
    },
    /// Changes a ticket: what it names changes, the rest stays.
    Update {
        key: String,
        #[cfg_attr(feature = "ts", ts(optional))]
        title: Option<String>,
        #[cfg_attr(feature = "ts", ts(optional))]
        description: Option<String>,
        /// A state by its name (`In Review`; `closed` on GitHub).
        #[cfg_attr(feature = "ts", ts(optional))]
        state: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
        add_labels: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
        remove_labels: Vec<String>,
        /// Its parent by key, or `none` for none.
        #[cfg_attr(feature = "ts", ts(optional))]
        parent: Option<String>,
        #[cfg_attr(feature = "ts", ts(optional))]
        priority: Option<String>,
    },
    /// The states tickets can be in, in their order: Linear's team's
    /// workflow; open and closed on GitHub.
    States,
    /// The labels tickets can have.
    Labels,
}

impl TicketRequest {
    /// Whether it only reads, so that asking again changes nothing.
    pub fn reads(&self) -> bool {
        matches!(
            self,
            Self::About | Self::List { .. } | Self::View { .. } | Self::States | Self::Labels
        )
    }
}

/// The control plane's answer to a [`TicketRequest`].
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TicketAnswer {
    About {
        about: TrackerAbout,
    },
    Tickets {
        tickets: Vec<Ticket>,
        /// More match than it shows.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
        more: bool,
    },
    /// The ticket as it is now: viewed, created or changed.
    Ticket {
        ticket: Ticket,
    },
    /// The comment posted, on its ticket.
    Comment {
        ticket: TicketLink,
        comment: TicketComment,
    },
    States {
        states: Vec<TicketState>,
    },
    Labels {
        labels: Vec<String>,
    },
}

/// What `fctl ticket` says of the project's tracker.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TrackerAbout {
    pub tracker: Tracker,
    /// What of it agents reach, in words: "Linear team ENG (Acme)",
    /// "GitHub issues of acme/web".
    pub scope: String,
    /// Where new tickets go, when they go somewhere by default.
    pub files_in: Option<String>,
    /// The tickets the session works on: the one it started from first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<SessionTicket>>", optional))]
    pub tickets: Vec<SessionTicket>,
}

// ───────────────────────────── sessions ─────────────────────────────

/// How a session's agent reaches its project's tracker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum TicketsVia {
    /// It reaches none.
    Nothing,
    /// `gh issue`, whose token may.
    Gh,
    /// `fctl ticket`, through the control plane.
    Fctl,
}

/// What a session's agent is told about its project's ticket tracker
/// (`crate::runner::SessionSpec::tickets`): the control plane says it, so
/// that what agents are told grows with the trackers it knows. Absent for
/// review agents, from older control planes, and through older hosts.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TicketsSpec {
    pub tracker: Tracker,
    pub via: TicketsVia,
    /// The ticket the session started from.
    pub ticket: Option<SessionTicket>,
    /// The system prompt's lines about it, for an agent whose sandbox's
    /// fctl has `fctl ticket` where it needs it.
    pub instructions: String,
}

/// How a message from a tracker came to the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum TicketEvent {
    /// Someone delegated the ticket to fctl (assigned it to fctl's agent).
    Delegated,
    /// Someone mentioned fctl in a comment on it.
    Mentioned,
    /// Someone replied to the agent there.
    Replied,
}

/// For a message to the agent from a ticket tracker (`UserMessage::ticket`):
/// the ticket, what happened, and who did it there, for the apps to show it
/// as the tracker does. The message's text says the same to the agent.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TicketMessage {
    pub ticket: SessionTicket,
    pub event: TicketEvent,
    /// Who, as the tracker names them.
    pub by: Option<TicketPerson>,
    /// The ticket's description, for a ticket delegated to fctl.
    pub description: Option<String>,
    /// What they wrote: the comment that mentioned fctl, the reply.
    pub body: Option<String>,
    /// The comment's link.
    pub url: Option<String>,
    /// The ticket's state when it came.
    pub state: Option<TicketState>,
    /// Its labels then.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub labels: Vec<String>,
    /// The agent was asked to explore it: to find out and reply with a
    /// plan, without changing code (`LinearTickets::explore_label`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub explore: bool,
}

/// What a `ticket_*` notice is about: what the agent, or fctl for it, did
/// in the tracker (`ticket_created`, `ticket_commented`, `ticket_updated`,
/// `ticket_replied`, `ticket_moved`), or why fctl couldn't
/// (`ticket_failed`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TicketNotice {
    pub tracker: Tracker,
    pub ticket: TicketLink,
    /// The comment or reply it posted, and its link.
    pub body: Option<String>,
    pub url: Option<String>,
    /// For a ticket it moved or changed: its state now.
    pub state: Option<TicketState>,
}

// ───────────────────────────── Linear ─────────────────────────────

/// A Linear workspace connected to an fctl workspace: fctl's app is
/// installed there, so its agents read and write the issues of the teams
/// that projects choose, and people delegate issues to it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct LinearConnection {
    /// Linear's organization id.
    pub organization: String,
    pub name: String,
    /// Its address on Linear: `linear.app/<url_key>`.
    pub url_key: String,
    pub logo_url: Option<String>,
    /// fctl's app user there, which people delegate issues to.
    pub app_user: Option<String>,
    /// Its name and handle there, as people find it to assign issues to
    /// and mention it (`fctl`, `@fctl`): those of the instance's Linear
    /// app. Absent for installations from before fctl kept them.
    pub app_name: Option<String>,
    pub app_handle: Option<String>,
    /// Working, or what went wrong: its access was revoked in Linear, or
    /// fctl can't renew it.
    pub status: LinearStatus,
    /// Who connected it, and as whom they signed in to Linear.
    pub connected_by: Option<UserRef>,
    pub linear_user: Option<String>,
    pub created_at: Millis,
    /// The projects of the workspace that take tickets from it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<LinearProjectUse>>", optional))]
    pub projects: Vec<LinearProjectUse>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LinearStatus {
    Active,
    /// Its access was revoked, or ran out: an admin connects it again.
    Revoked,
}

/// A project that takes tickets from a team of a connected Linear workspace.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct LinearProjectUse {
    pub slug: String,
    pub name: String,
    pub team_key: Option<String>,
}

/// `GET /api/v1/workspaces/{id}/linear`: the Linear workspaces it is
/// connected to.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WorkspaceLinear {
    /// The instance has a Linear app (`LINEAR_CLIENT_ID`, and its secret).
    pub configured: bool,
    /// It takes Linear's webhook (`LINEAR_WEBHOOK_SECRET`): without it,
    /// nothing delegated in Linear reaches fctl.
    pub webhook: bool,
    pub connections: Vec<LinearConnection>,
}

/// A team of a connected Linear workspace, for a project to choose, with its
/// workflow and its projects.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct LinearTeam {
    pub id: String,
    pub key: String,
    pub name: String,
    pub color: Option<String>,
    /// Its workflow, in order.
    pub states: Vec<TicketState>,
    /// The state its tickets move to when their sessions start: its first
    /// started one.
    pub start_state: Option<String>,
    /// The state they move to when their pull requests are up: its first
    /// started one whose name says review, or else its first started one.
    pub review_state: Option<String>,
    /// The state they move to when their pull requests merged: its first
    /// completed one.
    pub done_state: Option<String>,
    /// Its projects that aren't done or canceled.
    pub projects: Vec<LinearProject>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LinearProject {
    pub id: String,
    pub name: String,
}

/// `GET /api/v1/workspaces/{id}/linear/{organization}/teams`: the teams
/// fctl's app can reach there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LinearTeamList {
    pub teams: Vec<LinearTeam>,
}

/// One of your Linear accounts, which fctl knows you by in that Linear
/// workspace: issues you delegate to fctl there, and your replies to its
/// agents, run as you.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct LinearAccount {
    pub organization: String,
    /// The Linear workspace's name and address (`linear.app/<url_key>`).
    pub organization_name: String,
    pub url_key: Option<String>,
    /// Linear's id for you there.
    pub linear_user: String,
    pub name: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub linked_at: Millis,
}

/// `GET /api/v1/me/linear`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MyLinear {
    /// The instance has a Linear app.
    pub configured: bool,
    pub accounts: Vec<LinearAccount>,
}

/// A link that signs in to Linear, for the browser to open: to connect a
/// Linear workspace, or your Linear account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LinearLink {
    pub url: String,
    pub expires_at: Millis,
}

/// `POST /api/v1/workspaces/{id}/linear/connect` and
/// `POST /api/v1/me/linear/connect`: where the browser comes back to.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ConnectLinearRequest {
    /// The project whose Tickets tab it started from, by slug: the browser
    /// comes back there.
    pub project: Option<String>,
}

/// `GET /api/v1/projects/{id}/tickets`: the project's tracker as it works
/// now, for its Tickets tab.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ProjectTickets {
    pub tracker: Tracker,
    /// What of it agents reach, in words.
    pub scope: String,
    /// How agents reach it.
    pub via: TicketsVia,
    /// Why it doesn't work as chosen, when it doesn't: no Linear workspace
    /// connected, a repository agents no longer write to.
    pub problem: Option<String>,
    /// The project's workspace.
    pub workspace: WorkspaceId,
    /// For Linear: its connection, as it is now.
    pub linear: Option<LinearConnection>,
    /// Whether the caller's Linear account there is linked: issues they
    /// delegate start sessions only once it is.
    pub linked: Option<bool>,
    /// The repositories its agents write to, for GitHub Issues of one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<RepoRef>>", optional))]
    pub writable: Vec<RepoRef>,
    /// The sessions tickets started lately, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<TicketRun>>", optional))]
    pub recent: Vec<TicketRun>,
}

/// A session that started from a ticket, as a project's Tickets tab lists
/// it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TicketRun {
    pub ticket: SessionTicket,
    pub session: crate::session::SessionRef,
    pub status: crate::session::SessionStatus,
    /// Whose session it is: who delegated it.
    pub by: Option<UserRef>,
    pub at: Millis,
}
