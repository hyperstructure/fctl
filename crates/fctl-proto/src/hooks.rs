//! Webhooks: the addresses a workspace's or a project's events are posted
//! to, what each wants, what went out to them (the REST API's
//! `/webhooks…`), and what the posts themselves carry (`Hook…`).
//! docs/architecture.md#webhooks has the rules.

use crate::session::{SessionStatus, UserRef};
use crate::{
    Millis, ProjectId, ScheduleId, ScheduleRunId, SessionId, WebhookDeliveryId, WebhookEventId,
    WebhookId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

// ───────────────────────────── the API ─────────────────────────────

/// How the sessions a webhook hears of started: what `started_by` narrows
/// its sessions' events (and their pull requests') down to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum WebhookOrigin {
    /// A person started it, or a tool with their access token.
    Person,
    /// One of the project's schedules: at its time, on an event, or when
    /// someone ran it.
    Schedule,
    /// An autopilot: its own session, or one it started.
    Autopilot,
    /// A ticket delegated to fctl in the project's tracker (Linear's).
    Ticket,
    /// One a newer control plane knows, which this version doesn't.
    #[serde(other)]
    Other,
}

impl WebhookOrigin {
    pub const ALL: [WebhookOrigin; 4] =
        [Self::Person, Self::Schedule, Self::Autopilot, Self::Ticket];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Schedule => "schedule",
            Self::Autopilot => "autopilot",
            Self::Ticket => "ticket",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.as_str() == s)
    }
}

/// A webhook: an address fctl posts events to as they happen, signed with
/// its secret. A workspace's (for all its projects, or those in
/// `projects`), or a project's own.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Webhook {
    pub id: WebhookId,
    pub workspace_id: WorkspaceId,
    /// The project it belongs to, for a project's own webhook; absent for
    /// one of the workspace's.
    pub project: Option<HookPlace>,
    /// What people call it; absent: its address's host.
    pub name: Option<String>,
    /// Where fctl posts its events: `https://`, a public host. For those
    /// who don't manage it (`can_manage`), its scheme and host alone
    /// (`https://hooks.slack.com/…`): its path or query can hold a
    /// credential.
    pub url: String,
    /// The events it wants, by their type (`session.finished`); empty:
    /// every event, also those a later version adds.
    #[serde(default)]
    pub events: Vec<String>,
    /// A workspace's webhook: only these projects' events; empty: every
    /// project's, also those created later. Always empty for a project's.
    #[serde(default)]
    pub projects: Vec<ProjectId>,
    /// Sessions' events (and their pull requests'): only those of sessions
    /// that started this way; empty: however they started.
    #[serde(default)]
    pub started_by: Vec<WebhookOrigin>,
    /// Off, it gets nothing: deliveries waiting for it go nowhere.
    pub enabled: bool,
    /// Why fctl turned it off: every delivery failed for three days.
    /// Absent while it's on, and when a person turned it off.
    pub disabled_reason: Option<String>,
    pub disabled_at: Option<Millis>,
    /// Until when its previous secret signs deliveries too, after a
    /// rotation: a receiver that still checks with it keeps working.
    pub previous_secret_until: Option<Millis>,
    /// Who added it (absent once their account is gone).
    pub created_by: Option<UserRef>,
    pub created_at: Millis,
    pub updated_at: Millis,
    /// How its deliveries went lately. Absent where the workspace's
    /// webhooks couldn't be asked.
    pub health: Option<WebhookHealth>,
    /// Whether the caller may change it, see its secret and its
    /// deliveries: a project's page lists the workspace's webhooks that
    /// send its events, which only the workspace's admins manage.
    #[serde(default)]
    pub can_manage: bool,
}

/// How a webhook's deliveries went: the last one, and what happened in the
/// last day.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WebhookHealth {
    /// Its newest attempt of fctl's own, or a person's test or resend that
    /// went through.
    pub last: Option<WebhookAttempt>,
    /// Deliveries in the last 24 hours that it took.
    pub delivered: u32,
    /// Deliveries in the last 24 hours that failed every attempt.
    pub failed: u32,
    /// Deliveries waiting now: for their first attempt or a retry.
    pub pending: u32,
    /// Since when every attempt of fctl's own failed; absent once one, or a
    /// person's test or resend, went through. A person's that failed starts
    /// nothing, and turning the webhook on again or pointing it elsewhere
    /// starts it afresh.
    pub failing_since: Option<Millis>,
    /// Events dropped because 1,000 deliveries waited for it already,
    /// since the last one it took.
    pub dropped: u32,
}

/// `GET /api/v1/workspaces/{id}/webhooks` (the workspace's and its
/// projects', for its admins) and `GET /api/v1/projects/{id}/webhooks`
/// (the project's own, and the workspace's that send its events).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WebhookList {
    pub webhooks: Vec<Webhook>,
    /// How many webhooks the workspace has, all of them, and may have.
    pub count: u32,
    pub limit: u32,
}

/// `POST /api/v1/workspaces/{id}/webhooks` (its admins) or `POST
/// /api/v1/projects/{id}/webhooks` (its owners and maintainers).
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateWebhookRequest {
    pub url: String,
    pub name: Option<String>,
    /// Empty: every event.
    #[serde(default)]
    pub events: Vec<String>,
    /// A workspace's webhook only. Empty: every project.
    #[serde(default)]
    pub projects: Vec<ProjectId>,
    /// Empty: sessions however they started.
    #[serde(default)]
    pub started_by: Vec<WebhookOrigin>,
    /// Off from the start: it gets nothing until it's turned on. On when
    /// absent.
    pub enabled: Option<bool>,
}

/// `PATCH /api/v1/webhooks/{id}`: absent fields stay as they are.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateWebhookRequest {
    pub url: Option<String>,
    /// Empty: none, so that it goes by its host.
    pub name: Option<String>,
    pub events: Option<Vec<String>>,
    pub projects: Option<Vec<ProjectId>>,
    pub started_by: Option<Vec<WebhookOrigin>>,
    pub enabled: Option<bool>,
}

/// A webhook as it was added, with its secret: shown once here, and later
/// to those who manage it (`GET /api/v1/webhooks/{id}/secret`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreatedWebhook {
    pub webhook: Webhook,
    pub secret: String,
}

/// A webhook's signing secret (`GET /api/v1/webhooks/{id}/secret`), or the
/// new one after a rotation (`POST`): `whsec_` and 32 random bytes in
/// base64, as Standard Webhooks' libraries take it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WebhookSecret {
    pub secret: String,
    /// Until when the one before it signs deliveries too.
    pub previous_until: Option<Millis>,
}

/// `POST /api/v1/webhooks/{id}/test`: posts a sample of an event, marked
/// `test`, now, and answers how it went.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TestWebhookRequest {
    /// The event's type, such as `session.ready_for_review`.
    pub event: String,
}

/// One kind of event webhooks get (`GET /api/v1/webhook-events`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WebhookEventInfo {
    /// What payloads and `fctl-event` call it: `session.finished`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Its group's id: `sessions`, `pull_requests`, `reviews`, `schedules`.
    pub group: String,
    /// What happened, in a few words: "An agent finishes a turn".
    pub label: String,
    /// When it comes, and what it carries, in a sentence or two.
    pub description: String,
    /// It's about a session, which `started_by` narrows down.
    pub sessions: bool,
}

/// A group of events, in the order people choose them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WebhookEventGroup {
    pub id: String,
    pub label: String,
}

/// `GET /api/v1/webhook-events`: every event webhooks get, in their groups.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WebhookEvents {
    pub groups: Vec<WebhookEventGroup>,
    pub events: Vec<WebhookEventInfo>,
}

/// How a delivery stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum WebhookDeliveryState {
    /// It waits for its first attempt, or a retry (`next_at`).
    Pending,
    /// An attempt got a 2xx answer.
    Delivered,
    /// Every attempt failed, or its webhook was turned off or deleted
    /// before one went through.
    Failed,
    #[serde(other)]
    Other,
}

impl WebhookDeliveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "delivered" => Self::Delivered,
            "failed" => Self::Failed,
            _ => Self::Other,
        }
    }
}

/// A header of a request fctl sent, or of an answer it got.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WebhookHeader {
    pub name: String,
    pub value: String,
}

/// The start of what a receiver answered.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WebhookResponse {
    #[serde(default)]
    pub headers: Vec<WebhookHeader>,
    /// Its first 4 KB, as text.
    pub body: Option<String>,
    /// There was more than `body` has.
    #[serde(default)]
    pub truncated: bool,
}

/// One time fctl posted a delivery.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WebhookAttempt {
    pub at: Millis,
    /// How long until the answer, or until fctl gave up on it.
    pub ms: u32,
    /// The answer's HTTP status; absent when none came.
    pub status: Option<u16>,
    /// What went wrong, when it did: no answer within 10 seconds, the
    /// address couldn't be reached, a redirect, a status other than 2xx.
    pub error: Option<String>,
    /// The person who resent it or sent the test; absent for fctl's own
    /// attempts.
    pub by: Option<UserRef>,
    /// What the receiver answered: the newest attempt's alone.
    pub response: Option<WebhookResponse>,
}

/// The request fctl posts, as it posted it last (its timestamp and
/// signature change with each attempt).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WebhookRequest {
    pub url: String,
    pub headers: Vec<WebhookHeader>,
    /// The payload, JSON.
    pub body: String,
}

/// One event on its way to one webhook, or gone.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WebhookDelivery {
    pub id: WebhookDeliveryId,
    pub webhook_id: WebhookId,
    /// The event's id: `webhook-id` in each attempt.
    pub event_id: WebhookEventId,
    /// The event's type, `session.finished`.
    #[serde(rename = "type")]
    pub kind: String,
    /// What it's about, in a line: "web#12 finished a turn: Retry failed
    /// uploads".
    pub gist: String,
    /// The project it's of.
    pub project_id: Option<ProjectId>,
    /// The session it's of, where it's of one.
    pub session_id: Option<SessionId>,
    pub created_at: Millis,
    pub state: WebhookDeliveryState,
    /// How many times fctl posted it.
    pub attempts: u32,
    /// When fctl posts it again, while it's pending.
    pub next_at: Option<Millis>,
    /// Its newest attempt.
    pub last: Option<WebhookAttempt>,
    /// A sample someone sent with Send a test.
    #[serde(default)]
    pub test: bool,
    /// How many times people resent it.
    #[serde(default)]
    pub resends: u32,
    /// The request, as fctl posted it last, or would: one delivery's
    /// (`GET /api/v1/webhooks/{id}/deliveries/{delivery}`), not a list's.
    pub request: Option<WebhookRequest>,
    /// Every attempt, the newest first: one delivery's, not a list's.
    #[serde(default)]
    pub history: Vec<WebhookAttempt>,
}

/// `GET /api/v1/webhooks/{id}/deliveries`: newest first, a page at a time.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WebhookDeliveryList {
    pub deliveries: Vec<WebhookDelivery>,
    /// The next page's cursor (`?before=`), while there are more.
    pub next: Option<String>,
}

/// `POST /api/v1/webhooks/{id}/deliveries/resend-failed`: the deliveries
/// that failed since `since` (the last day when absent) wait for an
/// attempt again, in their turn.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ResendFailedRequest {
    pub since: Option<Millis>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResentFailed {
    /// How many wait for an attempt again.
    pub queued: u32,
}

// ───────────────────────────── what posts carry ─────────────────────────────

/// Every post's body (`webhook-id`'s event), in the shape Standard
/// Webhooks gives: its type, when it happened and its data, with the
/// workspace and the project it happened in, to check against the ones
/// the receiver expects.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HookEnvelope {
    pub id: WebhookEventId,
    #[serde(rename = "type")]
    pub kind: String,
    /// When it happened, RFC 3339 in UTC.
    pub timestamp: String,
    pub workspace: HookPlace,
    /// The project it happened in; absent for a workspace's test.
    pub project: Option<HookPlace>,
    /// What the event says: [`HookSessionData`], [`HookPullRequestData`],
    /// [`HookReviewData`] or [`HookScheduleData`], by its group.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub data: serde_json::Value,
    /// A sample someone sent with Send a test: made up, nothing happened.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub test: bool,
}

/// A workspace or a project, as posts name them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HookPlace {
    pub id: String,
    pub slug: String,
    pub name: String,
}

/// A person, as posts name them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HookPerson {
    pub id: String,
    pub name: String,
}

/// The schedule a session started from, or that a schedule's event is of.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HookSchedule {
    pub id: ScheduleId,
    /// Absent once it's deleted.
    pub name: Option<String>,
    /// Its page in fctl.
    pub url: String,
}

/// The ticket a session started from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HookTicket {
    /// `linear`, or `github`.
    pub tracker: String,
    /// `ENG-123`, `acme/web#12`.
    pub key: String,
    pub url: String,
}

/// What an agent asks (`fctl ask`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HookQuestion {
    pub text: String,
    /// Answers people can give with a click.
    #[serde(default)]
    pub options: Vec<String>,
    pub asked_at: Millis,
}

/// The checks of a pull request's newest commit, each once.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HookChecks {
    pub passed: u32,
    pub failed: u32,
    pub pending: u32,
    /// The commit they're of.
    pub sha: Option<String>,
    /// How the review agent's review of it stands, where one reviews it:
    /// `running`, `passed`, `commented`, `superseded`, `stopped` or
    /// `failed`.
    pub review: Option<String>,
}

/// A pull request, as posts give it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HookPullRequest {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub title: Option<String>,
    /// `open`, `draft`, `queued`, `merged` or `closed`; absent until
    /// someone looked it up.
    pub state: Option<String>,
    /// Absent until GitHub told fctl about them.
    pub checks: Option<HookChecks>,
}

/// A session, as posts give it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HookSession {
    pub id: SessionId,
    pub number: u32,
    /// How people refer to it: `web#12`.
    #[serde(rename = "ref")]
    pub reference: String,
    /// Its page in fctl.
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub status: SessionStatus,
    /// What its status means now: "Waiting for a message", "Waiting for 2
    /// background tasks", why it failed.
    pub status_reason: Option<String>,
    /// How it started.
    pub started_by: WebhookOrigin,
    /// Whoever started it, or whom a schedule, an autopilot or a ticket
    /// started it for.
    pub created_by: HookPerson,
    /// The schedule it started from.
    pub schedule: Option<HookSchedule>,
    /// The autopilot it belongs to: `pilot` for an autopilot's own
    /// session, `crew` for one it started.
    pub autopilot: Option<String>,
    pub ticket: Option<HookTicket>,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    /// Its agent's pull requests, in the order it opened them.
    #[serde(default)]
    pub pull_requests: Vec<HookPullRequest>,
    /// What its agent asks, until someone answers.
    pub question: Option<HookQuestion>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Its turns so far.
    pub turns: u32,
    pub created_at: Millis,
    pub updated_at: Millis,
    pub started_at: Option<Millis>,
    pub ended_at: Option<Millis>,
    pub archived: bool,
}

/// The data of a session's event (`session.*`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HookSessionData {
    pub session: HookSession,
    /// `session.status_changed`: its status before.
    pub previous_status: Option<SessionStatus>,
    /// `session.finished`: the start of the agent's last reply, at most
    /// 2,000 characters.
    pub message: Option<String>,
    /// `session.ready_for_review`: its open pull requests, which wait for
    /// a person.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<HookPullRequest>>", optional))]
    pub ready: Vec<HookPullRequest>,
    /// `session.follow_up`: what brought the agent back, as fctl told it.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub follow_up: Option<serde_json::Value>,
}

/// The data of a pull request's event (`pull_request.*`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HookPullRequestData {
    pub session: HookSession,
    pub pull_request: HookPullRequest,
}

/// A review agent's review, as posts give it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HookReview {
    /// The review's session, whose page shows how it went.
    pub session_id: SessionId,
    pub session_url: String,
    pub pull_request: HookPullRequest,
    /// The commit it reviews.
    pub commit: String,
    /// `running`, `passed` (nothing to point out), `commented` (it found
    /// issues, or earlier ones are still open), `superseded`, `stopped` or
    /// `failed`.
    pub state: String,
    /// New findings it posted.
    pub findings: u32,
    /// Earlier findings the commit fixed, which it marked so.
    pub fixed: u32,
    /// Earlier findings still open.
    pub still_open: u32,
    /// Its summary on GitHub, once posted.
    pub review_url: Option<String>,
    /// Why it ended without posting.
    pub reason: Option<String>,
    /// Whose Claude subscription it runs on.
    pub operator: Option<HookPerson>,
    pub created_at: Millis,
    pub finished_at: Option<Millis>,
}

/// The data of a review's event (`review.*`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HookReviewData {
    pub review: HookReview,
}

/// A schedule's run that started no session.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HookRun {
    pub id: ScheduleRunId,
    /// What started it: `time`, `person`, `token` or `event`.
    pub trigger: String,
    /// `skipped`, or `failed` when fctl couldn't start its session.
    pub outcome: String,
    pub reason: Option<String>,
    pub at: Millis,
    /// What happened on GitHub, for a run on an event: its link.
    pub event_url: Option<String>,
}

/// The data of a schedule's event (`schedule.*`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct HookScheduleData {
    pub schedule: HookSchedule,
    /// `schedule.run_skipped`: the run.
    pub run: Option<HookRun>,
    /// `schedule.paused`: why fctl paused it.
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_read_and_write_as_their_names() {
        for o in WebhookOrigin::ALL {
            assert_eq!(WebhookOrigin::parse(o.as_str()), Some(o));
            let json = serde_json::to_string(&o).unwrap_or_default();
            assert_eq!(json, format!("\"{}\"", o.as_str()));
        }
        let later: Option<WebhookOrigin> = serde_json::from_str("\"cron\"").ok();
        assert_eq!(later, Some(WebhookOrigin::Other));
    }

    #[test]
    fn delivery_states_read_what_a_later_version_adds_as_other() {
        assert_eq!(
            WebhookDeliveryState::parse("delivered"),
            WebhookDeliveryState::Delivered
        );
        assert_eq!(
            WebhookDeliveryState::parse("lost"),
            WebhookDeliveryState::Other
        );
        let later: Option<WebhookDeliveryState> = serde_json::from_str("\"lost\"").ok();
        assert_eq!(later, Some(WebhookDeliveryState::Other));
    }

    #[test]
    fn an_envelope_says_test_only_when_it_is_one() {
        let e = HookEnvelope {
            id: WebhookEventId("evt_1".into()),
            kind: "session.finished".into(),
            timestamp: "2026-11-17T12:00:00.000Z".into(),
            workspace: HookPlace {
                id: "wks_1".into(),
                slug: "acme".into(),
                name: "Acme".into(),
            },
            project: None,
            data: serde_json::json!({}),
            test: false,
        };
        let json = serde_json::to_value(&e).unwrap_or_default();
        assert_eq!(json["type"], "session.finished");
        assert!(json.get("test").is_none());
        assert!(json.get("project").is_none());
    }
}
