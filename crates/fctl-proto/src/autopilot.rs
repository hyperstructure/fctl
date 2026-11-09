//! Autopilots (experimental): an agent that runs a project, or a whole
//! workspace, on its own (docs/architecture.md#autopilots).
//!
//! People put a project on autopilot with a direction. Its agent, the
//! autopilot, works in sessions of its own in that project (its home): it
//! keeps a plan, starts sessions to do the work, and sets up what the work
//! needs, as far as its powers go. fctl wakes it at least at its check-in
//! time, and sooner when something needs it: a session it started finished
//! or asks something, someone steered it or answered it. It asks people
//! for what only they can give (a decision, a secret), and they steer it at
//! any time with notes.
//!
//! The autopilot acts through the API as the person who put it on (its
//! owner), on their Claude subscription, from its own session's sandbox:
//! the sandbox's `fctl` passes its requests to fctl-agent, which hands them
//! to the control plane through its host ([`crate::api::AgentCall::Api`]).
//! No token enters the sandbox, and the control plane lets it do only what
//! its powers allow (`fctl_domain::autopilot::allows`).

use crate::Millis;
use crate::ids::{ProjectId, SessionId};
use crate::session::{SessionRef, SessionStatus, UserRef};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// Where an autopilot works.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AutopilotScope {
    /// Its project alone.
    #[default]
    Project,
    /// Every project of its project's workspace, and new ones it creates
    /// there. One per workspace.
    Workspace,
}

impl AutopilotScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Workspace => "workspace",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "project" => Some(Self::Project),
            "workspace" => Some(Self::Workspace),
            _ => None,
        }
    }
}

/// What an autopilot may do besides starting and steering sessions, which
/// every autopilot does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AutopilotPower {
    /// Change its projects' settings: their agents' instructions, model,
    /// setup, image, follow-ups, review agents and the rest.
    Settings,
    /// Set and remove its projects' secrets (it never reads their values).
    Secrets,
    /// Add and remove its projects' repositories, among those its
    /// workspace's GitHub accounts reach.
    Repos,
    /// Set up, change, pause, run and delete its projects' schedules.
    Schedules,
    /// Create projects in its workspace: a workspace's autopilot only.
    Projects,
    /// A power a newer control plane gives, which this version doesn't
    /// know.
    #[serde(other)]
    Other,
}

impl AutopilotPower {
    /// The powers there are, in the order people see them.
    pub const ALL: [Self; 5] = [
        Self::Settings,
        Self::Secrets,
        Self::Repos,
        Self::Schedules,
        Self::Projects,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Secrets => "secrets",
            Self::Repos => "repos",
            Self::Schedules => "schedules",
            Self::Projects => "projects",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }
}

/// Whether an autopilot works.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AutopilotState {
    /// It works: fctl wakes it at its check-ins and when something needs it.
    On,
    /// fctl wakes it no more until someone resumes it: a person paused it,
    /// it paused itself (its direction is done), or fctl did, saying why.
    Paused,
    /// Taken off: it keeps its direction, notes, plan, log and asks, for
    /// putting the project on autopilot again.
    Off,
}

impl AutopilotState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Paused => "paused",
            Self::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "on" => Some(Self::On),
            "paused" => Some(Self::Paused),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// A project's autopilot, as its people see it (`GET
/// /api/v1/projects/{id}/autopilot`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Autopilot {
    /// Its home: the project whose sessions it works in.
    pub project_id: ProjectId,
    pub project_slug: String,
    pub scope: AutopilotScope,
    pub state: AutopilotState,
    /// Why it is paused or off, when fctl or the autopilot itself did that,
    /// or a person said why.
    pub paused_reason: Option<String>,
    /// Who paused or took it off; absent when fctl or the autopilot did.
    pub paused_by: Option<UserRef>,
    pub paused_at: Option<Millis>,
    /// Whose Claude subscription it runs on, and as whom it acts: who put
    /// the project on autopilot, or took it over since. Absent once their
    /// account is gone.
    pub owner: Option<UserRef>,
    /// Where people want it to take the project: Markdown, theirs to change.
    pub direction: String,
    pub direction_by: Option<UserRef>,
    pub direction_at: Millis,
    /// Its own plan: Markdown it keeps (`fctl autopilot plan`), usually a
    /// checklist. Absent until it writes one.
    pub plan: Option<String>,
    pub plan_at: Option<Millis>,
    /// What it may do besides starting and steering sessions.
    pub powers: Vec<AutopilotPower>,
    /// How long it goes without waking, at most, unless it asks for longer
    /// (`fctl autopilot wake --in`).
    pub check_in_minutes: u32,
    /// How many of its sessions work at once, at most.
    pub crew_max: u32,
    /// The model and effort level of its own sessions; absent: the
    /// project's.
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Its own session now: the one fctl wakes it in.
    pub session: Option<AutopilotSession>,
    /// When fctl wakes it next, unless something wakes it sooner; absent
    /// while it is paused or off.
    pub wake_at: Option<Millis>,
    /// Why it wakes then: its check-in, or the time it asked for.
    pub wake_reason: Option<AutopilotWakeReason>,
    /// When fctl last woke it, and how many times it has.
    pub woken_at: Option<Millis>,
    pub check_ins: u32,
    /// Why it wouldn't wake now, while it is on: its owner can't run it
    /// here any more, has no Claude connection, or their plan is at its
    /// limit.
    pub problem: Option<String>,
    /// What people told it, newest first: notes stay until someone removes
    /// them.
    pub notes: Vec<AutopilotNote>,
    /// What it asked people: those still open, then those answered,
    /// declined or withdrawn in the last week, newest first.
    pub asks: Vec<AutopilotAsk>,
    /// The sessions it started that still work or wait, then the latest
    /// others (at most 20 in all), newest first.
    pub crew: Vec<AutopilotCrew>,
    /// How many sessions it started in all.
    pub crew_total: u32,
    /// What the caller may do with it.
    pub can: AutopilotCan,
    pub created_at: Millis,
    pub updated_at: Millis,
}

/// What the caller may do with an autopilot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AutopilotCan {
    /// Steer it (notes, its direction), answer it, wake, pause and resume
    /// it, and change its settings: the project's maintainers and owners,
    /// and for a workspace's autopilot the workspace's admins.
    pub steer: bool,
    /// Give it the secrets it asks for: those who may set them.
    pub secrets: bool,
}

/// An autopilot's own session.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AutopilotSession {
    pub id: SessionId,
    pub number: u32,
    pub status: SessionStatus,
    pub started_at: Millis,
}

/// Why fctl wakes an autopilot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AutopilotWakeReason {
    /// It was put on.
    Started,
    /// People resumed it after a pause.
    Resumed,
    /// Its check-in time came.
    CheckIn,
    /// The time it asked to wake at came.
    Asked,
    /// Someone woke it.
    Woken,
    /// Someone steered it: a note, or a new direction.
    Steered,
    /// Someone answered what it asked, gave it a secret or declined.
    Answered,
    /// One of its sessions finished, asks something or failed.
    Crew,
    /// Its own last session failed, and fctl tries again.
    Retry,
    /// A reason a newer control plane gives.
    #[serde(other)]
    Other,
}

impl AutopilotWakeReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Resumed => "resumed",
            Self::CheckIn => "check_in",
            Self::Asked => "asked",
            Self::Woken => "woken",
            Self::Steered => "steered",
            Self::Answered => "answered",
            Self::Crew => "crew",
            Self::Retry => "retry",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::Started,
            Self::Resumed,
            Self::CheckIn,
            Self::Asked,
            Self::Woken,
            Self::Steered,
            Self::Answered,
            Self::Crew,
            Self::Retry,
        ]
        .into_iter()
        .find(|r| r.as_str() == s)
    }
}

/// A note someone gave an autopilot to steer it: a priority, a constraint,
/// context it should know. It stays with the autopilot, which every wake
/// reminds of it, until someone removes it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AutopilotNote {
    pub id: String,
    pub text: String,
    /// Absent once their account is gone.
    pub author: Option<UserRef>,
    pub created_at: Millis,
    /// When the autopilot first got it: absent until it next wakes, or
    /// reads its brief.
    pub seen_at: Option<Millis>,
}

/// What an autopilot asks people for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AutopilotAskKind {
    /// A decision or an answer, with answers to click.
    Question,
    /// A secret for one of its projects, which people type in where it
    /// asks: it goes into the project's secrets, never into a transcript.
    Secret,
    /// A kind a newer control plane has.
    #[serde(other)]
    Other,
}

/// Where an autopilot's ask stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AutopilotAskStatus {
    /// It waits for someone.
    Open,
    /// Someone answered it, or gave the secret.
    Answered,
    /// Someone declined it, maybe saying why.
    Declined,
    /// The autopilot took it back.
    Withdrawn,
    #[serde(other)]
    Other,
}

impl AutopilotAskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Answered => "answered",
            Self::Declined => "declined",
            Self::Withdrawn => "withdrawn",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "open" => Self::Open,
            "answered" => Self::Answered,
            "declined" => Self::Declined,
            "withdrawn" => Self::Withdrawn,
            _ => Self::Other,
        }
    }
}

/// Something an autopilot asked people for, which it doesn't wait on: it
/// works on what it can meanwhile, and fctl wakes it with the answer.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AutopilotAsk {
    pub id: String,
    pub kind: AutopilotAskKind,
    /// The question, or what the secret is for: plain text, at most 500
    /// characters.
    pub text: String,
    /// What helps people answer: Markdown, with links and the screenshots
    /// it published (`fctl publish`), such as two designs to choose from.
    pub detail: Option<String>,
    /// Answers people can give with a click, for a question.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub options: Vec<String>,
    /// The secret it needs, for a secret.
    pub secret: Option<AskedSecret>,
    pub status: AutopilotAskStatus,
    /// The answer, for a question someone answered, or why someone
    /// declined. Never a secret's value.
    pub answer: Option<String>,
    /// Who answered, gave the secret or declined.
    pub answered_by: Option<UserRef>,
    pub answered_at: Option<Millis>,
    /// The autopilot's session that asked.
    pub session: Option<SessionRef>,
    pub created_at: Millis,
}

/// A secret an autopilot asks for: its name as an environment variable,
/// and the project whose secrets it goes into.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AskedSecret {
    pub name: String,
    pub project_id: ProjectId,
    pub project_slug: String,
    /// The project has a secret by that name already, which giving one
    /// replaces.
    #[serde(default)]
    pub exists: bool,
}

/// A session an autopilot started, to do part of the work.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AutopilotCrew {
    pub id: SessionId,
    pub project_id: ProjectId,
    pub project_slug: String,
    pub number: u32,
    pub title: String,
    pub status: SessionStatus,
    /// What its agent asks, until someone (the autopilot, say) answers.
    pub question: Option<String>,
    /// Its pull requests, as the session lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(
        feature = "ts",
        ts(as = "Option<Vec<crate::session::PullRequestRef>>", optional)
    )]
    pub pull_requests: Vec<crate::session::PullRequestRef>,
    pub created_at: Millis,
    pub last_activity_at: Option<Millis>,
}

/// What an entry in an autopilot's log is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AutopilotEntryKind {
    /// What the autopilot wrote down (`fctl autopilot log`): what it did,
    /// decided or plans, in its own words.
    Note,
    /// fctl woke it.
    Woke,
    /// It started a session.
    Session,
    /// It messaged, stopped or resumed one of its sessions.
    Steered,
    /// It changed a project's settings.
    Settings,
    /// It set or removed a secret, or someone gave it one.
    Secret,
    /// It added or removed a repository.
    Repository,
    /// It set up, changed, ran or deleted a schedule.
    Schedule,
    /// It created a project.
    Project,
    /// It changed its plan.
    Plan,
    /// It asked people something, or took it back.
    Asked,
    /// Someone answered or declined what it asked.
    Answered,
    /// Someone added a note, or changed its direction.
    Direction,
    /// It was put on, paused, resumed, changed or taken off.
    State,
    /// Its own session failed, or fctl couldn't wake it.
    Problem,
    #[serde(other)]
    Other,
}

impl AutopilotEntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Woke => "woke",
            Self::Session => "session",
            Self::Steered => "steered",
            Self::Settings => "settings",
            Self::Secret => "secret",
            Self::Repository => "repository",
            Self::Schedule => "schedule",
            Self::Project => "project",
            Self::Plan => "plan",
            Self::Asked => "asked",
            Self::Answered => "answered",
            Self::Direction => "direction",
            Self::State => "state",
            Self::Problem => "problem",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Self {
        [
            Self::Note,
            Self::Woke,
            Self::Session,
            Self::Steered,
            Self::Settings,
            Self::Secret,
            Self::Repository,
            Self::Schedule,
            Self::Project,
            Self::Plan,
            Self::Asked,
            Self::Answered,
            Self::Direction,
            Self::State,
            Self::Problem,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
        .unwrap_or(Self::Other)
    }
}

/// An entry in an autopilot's log: what it did, wrote down, or what
/// happened to it, newest first (`GET /api/v1/projects/{id}/autopilot/log`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AutopilotEntry {
    pub id: String,
    pub at: Millis,
    pub kind: AutopilotEntryKind,
    /// What happened, in a line, or what the autopilot wrote (Markdown).
    pub text: String,
    /// The person who did it; absent when the autopilot or fctl did.
    pub by: Option<UserRef>,
    /// The session it concerns: the one the autopilot started or steered,
    /// or its own that woke.
    pub session: Option<SessionRef>,
    /// The project it concerns, when another than the autopilot's own.
    pub project_slug: Option<String>,
    /// The autopilot's own session that did it.
    pub from: Option<SessionId>,
}

/// A page of an autopilot's log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AutopilotLog {
    pub entries: Vec<AutopilotEntry>,
    /// Older entries remain: ask with `?before=` the last one's id.
    pub has_more: bool,
}

/// What fctl woke an autopilot for, in the message that woke it, for the
/// apps to show in a line; the message's text tells the agent.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AutopilotWake {
    /// Why, most telling first.
    pub reasons: Vec<AutopilotWakeReason>,
    /// How many notes it hadn't seen.
    #[serde(default)]
    pub notes: u32,
    /// How many of its asks people answered or declined.
    #[serde(default)]
    pub answers: u32,
    /// The sessions it started that finished a turn, ask something or
    /// failed since it last woke.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<SessionRef>>", optional))]
    pub crew: Vec<SessionRef>,
    /// Its direction changed.
    #[serde(default)]
    pub direction: bool,
}

/// What an autopilot did, for its own session's transcript: the line its
/// log has, and the session it started or steered.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AutopilotNotice {
    pub kind: AutopilotEntryKind,
    /// The session it started or steered.
    pub session: Option<SessionRef>,
    /// That session's project, or the project it changed.
    pub project_slug: Option<String>,
}

/// What a session is to an autopilot (`Session.autopilot`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AutopilotRole {
    /// The autopilot's own session, where it plans and steers.
    Pilot,
    /// A session it started to do part of the work.
    Crew,
    #[serde(other)]
    Other,
}

impl AutopilotRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pilot => "pilot",
            Self::Crew => "crew",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "pilot" => Self::Pilot,
            "crew" => Self::Crew,
            _ => Self::Other,
        }
    }
}

/// The autopilot a session belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionAutopilot {
    pub role: AutopilotRole,
    /// The autopilot's home project, whose Autopilot tab shows it.
    pub project_id: ProjectId,
    pub project_slug: String,
}

/// A message an autopilot sent one of its sessions: its author is the
/// autopilot's owner, as whom it acts, and the apps say the autopilot wrote
/// it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AutopilotAuthor {
    /// The autopilot's home project.
    pub project_id: ProjectId,
    pub project_slug: String,
}

/// What a session's spec tells fctl-agent of the autopilot whose own
/// session it is: its sandbox's `fctl` reaches the API through fctl-agent
/// (`FCTL_AUTOPILOT`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AutopilotSpec {
    /// Its home project.
    pub project_id: ProjectId,
    pub project_slug: String,
    pub scope: AutopilotScope,
}

// ───────────────────────────── requests ─────────────────────────────

/// `PUT /api/v1/projects/{id}/autopilot`: puts the project on autopilot,
/// or changes its autopilot. Absent fields stay as they are, or take their
/// defaults for a new one; a direction is needed to put it on.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PutAutopilotRequest {
    pub direction: Option<String>,
    /// Only as it is put on: a workspace's autopilot is put on as one.
    pub scope: Option<AutopilotScope>,
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<AutopilotPower>>", optional))]
    pub powers: Option<Vec<AutopilotPower>>,
    pub check_in_minutes: Option<u32>,
    pub crew_max: Option<u32>,
    /// `"default"` for the project's.
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Run it on the caller's Claude subscription, as them, from now on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub take_over: bool,
}

/// `DELETE /api/v1/projects/{id}/autopilot`: takes the project off
/// autopilot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AutopilotOffRequest {
    /// Stop the sessions it started that still work, too.
    #[serde(default)]
    pub stop_sessions: bool,
}

/// `POST …/autopilot/pause`: by a person, or by the autopilot itself once
/// its direction is done.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PauseAutopilotRequest {
    pub reason: Option<String>,
}

/// `POST …/autopilot/wake`: a person wakes it now; the autopilot says
/// when it wakes next, unless something wakes it sooner: in `in_minutes`
/// (5 to 1,440), or at `at`.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WakeAutopilotRequest {
    pub in_minutes: Option<u32>,
    pub at: Option<Millis>,
}

/// `POST …/autopilot/notes`: steers the autopilot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AddAutopilotNoteRequest {
    pub text: String,
}

/// `POST …/autopilot/asks`: the autopilot asks people a question, or for a
/// secret.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateAutopilotAskRequest {
    pub text: String,
    /// Markdown under the question: links, screenshots of the options.
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub options: Vec<String>,
    /// The secret's name, for an ask for a secret.
    pub secret: Option<String>,
    /// The project the secret goes into (id or slug); absent: the
    /// autopilot's own.
    pub project: Option<String>,
}

/// `POST …/autopilot/asks/{ask}/answer`: answers a question. `secret`
/// gives a secret it asked for: its value goes into the project's secrets,
/// and the autopilot hears only that it is set.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AnswerAutopilotAskRequest {
    pub answer: Option<String>,
    pub secret: Option<String>,
}

/// `POST …/autopilot/asks/{ask}/decline`.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct DeclineAutopilotAskRequest {
    pub reason: Option<String>,
}

/// `POST …/autopilot/log`: the autopilot writes down what it did, decided
/// or plans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AutopilotLogRequest {
    pub text: String,
}

/// `PUT …/autopilot/plan`: the autopilot's plan as it is now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AutopilotPlanRequest {
    pub plan: String,
}

/// `GET …/autopilot/brief`: what the autopilot needs to know to go on, as
/// Markdown, for its own session (`fctl autopilot brief`). Reading it marks
/// its notes and answers seen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AutopilotBrief {
    pub brief: String,
}

/// The autopilots that ask the caller something, across their workspaces
/// (`GET /api/v1/autopilots/asks`), for *Needs you*.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AutopilotAsksList {
    pub asks: Vec<OpenAutopilotAsk>,
}

/// An ask that waits for the caller, with its autopilot's project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OpenAutopilotAsk {
    pub project_id: ProjectId,
    pub project_slug: String,
    pub project_name: String,
    pub ask: AutopilotAsk,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powers_parse_and_read_unknown_ones_as_other() {
        for p in AutopilotPower::ALL {
            assert_eq!(AutopilotPower::parse(p.as_str()), Some(p));
        }
        let read: Vec<AutopilotPower> =
            serde_json::from_str(r#"["settings","fly"]"#).unwrap_or_default();
        assert_eq!(read, vec![AutopilotPower::Settings, AutopilotPower::Other]);
    }

    #[test]
    fn entry_kinds_round_trip() {
        for k in [
            AutopilotEntryKind::Note,
            AutopilotEntryKind::Woke,
            AutopilotEntryKind::Schedule,
            AutopilotEntryKind::Problem,
        ] {
            assert_eq!(AutopilotEntryKind::parse(k.as_str()), k);
        }
        assert_eq!(AutopilotEntryKind::parse("new"), AutopilotEntryKind::Other);
    }

    #[test]
    fn a_wake_without_news_is_small() {
        let wake = AutopilotWake {
            reasons: vec![AutopilotWakeReason::CheckIn],
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&wake).ok().as_deref(),
            Some(r#"{"reasons":["check_in"],"notes":0,"answers":0,"direction":false}"#)
        );
    }
}
