//! Webhooks (docs/architecture.md#webhooks): what fctl posts to the
//! addresses a workspace's admins and a project's maintainers choose, as
//! things happen in its projects.
//!
//! - **Events** ([`Kind`]): what happened to a session, an agent's pull
//!   request, a review or a schedule, by name, in groups, with what each
//!   means ([`catalog`]).
//! - **Who gets one** ([`wants`]): a webhook's events, its projects and how
//!   its sessions started.
//! - **Where** ([`check_url`]): public `https://` addresses only.
//! - **Signing** ([`sign`], [`verify`]) as Standard Webhooks do, so that
//!   their libraries check fctl's posts.
//! - **Retries and limits** ([`retry_at`], [`turns_off`]).
//! - **What posts carry** ([`session`], [`envelope`], the events' builders
//!   and [`sample`]), and what a session's pull requests need to be ready
//!   for a person's review ([`ready_for_review`]).

use crate::autopilot::waits_for_background;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fctl_proto::api::ReviewState;
use fctl_proto::autopilot::AutopilotRole;
use fctl_proto::event::{FollowUp, FollowUpEvent, PassedOnReview};
use fctl_proto::hooks::{
    HookChecks, HookEnvelope, HookPerson, HookPlace, HookPullRequest, HookQuestion, HookReview,
    HookRun, HookSchedule, HookScheduleData, HookSession, HookSessionData, HookTicket,
    WebhookAttempt, WebhookDeliveryState, WebhookEventGroup, WebhookEventInfo, WebhookEvents,
    WebhookOrigin, WebhookReach,
};
use fctl_proto::session::{PullRequestRef, Session, SessionStatus, Tracker};
use fctl_proto::{ApiError, ScheduleId, SessionId, WebhookEventId};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub type Result<T> = std::result::Result<T, ApiError>;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// A workspace has at most this many webhooks, its projects' included.
pub const MAX_WEBHOOKS: u32 = 20;
/// A webhook's name, in characters.
pub const NAME_MAX: usize = 80;
/// An address, in bytes.
pub const URL_MAX: usize = 2048;

/// How long fctl waits for a receiver's answer, its body included.
pub const TIMEOUT_MS: i64 = 10_000;
/// How long after each failed attempt fctl tries again: 7 retries over
/// about 21 hours, so 8 attempts in all.
pub const RETRIES_MS: [i64; 7] = [
    10_000,
    MINUTE,
    5 * MINUTE,
    30 * MINUTE,
    2 * HOUR,
    6 * HOUR,
    12 * HOUR,
];
/// The longest a receiver's `Retry-After` puts a retry off.
pub const RETRY_AFTER_MAX_MS: i64 = 12 * HOUR;
/// Deliveries posted to one webhook in a minute, at most: the rest wait.
pub const PER_MINUTE: u32 = 120;
/// Deliveries that wait for one webhook, at most: events past that are
/// dropped, and counted.
pub const WAITING_MAX: u32 = 1_000;
/// A webhook all of whose attempts failed for this long turns itself off.
pub const OFF_AFTER_MS: i64 = 3 * DAY;

/// How long the delivery log keeps a delivery, and how many of each
/// webhook's at most (the newest).
pub const KEPT_MS: i64 = 30 * DAY;
pub const KEPT: u32 = 1_000;
/// What the log keeps of an answer's body, and of each attempt.
pub const RESPONSE_KEPT: usize = 4096;
pub const HISTORY_KEPT: usize = 20;

/// People resend a delivery at most this many times.
pub const RESENDS: u32 = 5;
/// Tests and resends a webhook takes in an hour, at most.
pub const MANUAL_PER_HOUR: u32 = 30;
/// Deliveries one "resend the failed ones" queues again, at most.
pub const RESEND_FAILED_MAX: u32 = 1_000;
/// How long a rotated secret's predecessor signs deliveries too.
pub const SECRET_OVERLAP_MS: i64 = DAY;
/// How long a session's pull requests stay ready before fctl says so: a
/// review's findings, posted as its check ends, reach the agent within
/// seconds and set it to work again.
pub const READY_SETTLE_MS: i64 = 2 * MINUTE;
/// How long a session waits for a message before fctl says it finished: as
/// its background work ends, it waits for one for a moment, until the turn
/// that takes the work's result starts.
pub const FINISHED_SETTLE_MS: i64 = 5_000;
/// How long the start of an agent's reply that `session.finished` carries
/// is, at most, in characters.
pub const MESSAGE_MAX: usize = 2000;
/// A delivery's gist, in characters.
pub const GIST_MAX: usize = 140;

// ───────────────────────────── events ─────────────────────────────

/// A group of events, as people choose them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Sessions,
    PullRequests,
    Reviews,
    Schedules,
}

impl Group {
    pub const ALL: [Group; 4] = [
        Self::Sessions,
        Self::PullRequests,
        Self::Reviews,
        Self::Schedules,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::PullRequests => "pull_requests",
            Self::Reviews => "reviews",
            Self::Schedules => "schedules",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Sessions => "Sessions",
            Self::PullRequests => "Agents' pull requests",
            Self::Reviews => "Reviews",
            Self::Schedules => "Schedules",
        }
    }
}

/// Something that happened, which webhooks post. Its name (`as_str`) is
/// the payload's `type` and the `fctl-event` header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    SessionStarted,
    SessionStatusChanged,
    SessionFinished,
    SessionReadyForReview,
    SessionQuestion,
    SessionFollowUp,
    SessionPaused,
    SessionStopped,
    SessionFailed,
    PullRequestOpened,
    PullRequestMerged,
    PullRequestClosed,
    ReviewQueued,
    ReviewStarted,
    ReviewFinished,
    ReviewFailed,
    ReviewCancelled,
    ScheduleRunSkipped,
    SchedulePaused,
}

impl Kind {
    /// Every event, in the order people choose them.
    pub const ALL: [Kind; 19] = [
        Self::SessionStarted,
        Self::SessionFinished,
        Self::SessionReadyForReview,
        Self::SessionQuestion,
        Self::SessionFollowUp,
        Self::SessionPaused,
        Self::SessionStopped,
        Self::SessionFailed,
        Self::SessionStatusChanged,
        Self::PullRequestOpened,
        Self::PullRequestMerged,
        Self::PullRequestClosed,
        Self::ReviewQueued,
        Self::ReviewStarted,
        Self::ReviewFinished,
        Self::ReviewFailed,
        Self::ReviewCancelled,
        Self::ScheduleRunSkipped,
        Self::SchedulePaused,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionStarted => "session.started",
            Self::SessionStatusChanged => "session.status_changed",
            Self::SessionFinished => "session.finished",
            Self::SessionReadyForReview => "session.ready_for_review",
            Self::SessionQuestion => "session.question",
            Self::SessionFollowUp => "session.follow_up",
            Self::SessionPaused => "session.paused",
            Self::SessionStopped => "session.stopped",
            Self::SessionFailed => "session.failed",
            Self::PullRequestOpened => "pull_request.opened",
            Self::PullRequestMerged => "pull_request.merged",
            Self::PullRequestClosed => "pull_request.closed",
            Self::ReviewQueued => "review.queued",
            Self::ReviewStarted => "review.started",
            Self::ReviewFinished => "review.finished",
            Self::ReviewFailed => "review.failed",
            Self::ReviewCancelled => "review.cancelled",
            Self::ScheduleRunSkipped => "schedule.run_skipped",
            Self::SchedulePaused => "schedule.paused",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    pub fn group(self) -> Group {
        use Kind::*;
        match self {
            SessionStarted
            | SessionStatusChanged
            | SessionFinished
            | SessionReadyForReview
            | SessionQuestion
            | SessionFollowUp
            | SessionPaused
            | SessionStopped
            | SessionFailed => Group::Sessions,
            PullRequestOpened | PullRequestMerged | PullRequestClosed => Group::PullRequests,
            ReviewQueued | ReviewStarted | ReviewFinished | ReviewFailed | ReviewCancelled => {
                Group::Reviews
            }
            ScheduleRunSkipped | SchedulePaused => Group::Schedules,
        }
    }

    /// It's about a session, which a webhook's `started_by` narrows down.
    pub fn of_sessions(self) -> bool {
        matches!(self.group(), Group::Sessions | Group::PullRequests)
    }

    /// What happened, as a sentence's clause: "an agent finishes a turn".
    pub fn phrase(self) -> &'static str {
        use Kind::*;
        match self {
            SessionStarted => "a session starts",
            SessionStatusChanged => "a session's status changes",
            SessionFinished => "an agent finishes a turn",
            SessionReadyForReview => "pull requests are ready for review",
            SessionQuestion => "an agent asks something",
            SessionFollowUp => "a follow-up brings an agent back",
            SessionPaused => "a session pauses",
            SessionStopped => "a session stops",
            SessionFailed => "a session fails",
            PullRequestOpened => "an agent opens a pull request",
            PullRequestMerged => "an agent's pull request merges",
            PullRequestClosed => "an agent's pull request closes",
            ReviewQueued => "a review is queued",
            ReviewStarted => "a review starts",
            ReviewFinished => "a review is posted",
            ReviewFailed => "a review fails",
            ReviewCancelled => "a review is cancelled",
            ScheduleRunSkipped => "a schedule's run starts nothing",
            SchedulePaused => "fctl pauses a schedule",
        }
    }

    /// What happened, as a list shows it: "An agent finishes a turn".
    pub fn label(self) -> String {
        capitalized(self.phrase())
    }

    /// When it comes, and what it carries.
    pub fn description(self) -> &'static str {
        use Kind::*;
        match self {
            SessionStarted => {
                "Someone, a schedule, an autopilot or a ticket started a session: it waits for a runner."
            }
            SessionStatusChanged => {
                "Every change of status, with the one before: queued, starting, working, waiting, paused, stopping, stopped or failed. The busiest event."
            }
            SessionFinished => {
                "Its turn ended with no background work running: it waits for a message. With the start of the agent's reply."
            }
            SessionReadyForReview => {
                "The agent finished, and its open pull requests passed every check, with no review agent at work on them, for two minutes: they wait for a person."
            }
            SessionQuestion => {
                "The agent asked something with fctl ask and waits for an answer, with the answers people can click."
            }
            SessionFollowUp => {
                "fctl brought the agent back to its pull request: a comment that addresses it, failed checks, conflicts, a merge or a close, or a review's findings."
            }
            SessionPaused => {
                "Its Claude plan reached a usage limit: it carries on once the limit resets."
            }
            SessionStopped => {
                "Its sandbox stopped: it waited for a message long enough, its work ran out of time, or someone stopped it. A message resumes it."
            }
            SessionFailed => "It couldn't start or carry on, with why.",
            PullRequestOpened => {
                "A session's agent opened a pull request, or found the one of its branch."
            }
            PullRequestMerged => "A pull request a session's agent opened merged.",
            PullRequestClosed => "A pull request a session's agent opened closed without merging.",
            ReviewQueued => {
                "A new commit, or someone asking, started a review agent's review of a pull request: it waits for a runner."
            }
            ReviewStarted => "A runner took the review, and its agent reads the pull request.",
            ReviewFinished => {
                "The review is on the pull request: how many new findings it reported, earlier ones it found fixed and those still open. It passed when there was nothing to point out."
            }
            ReviewFailed => "The review couldn't run, or couldn't post, with why.",
            ReviewCancelled => {
                "A newer commit took the review's place, or someone stopped it, before it posted."
            }
            ScheduleRunSkipped => {
                "A run at its time or on an event started no session, with why: its last run still at work, its owner's plan at its limit, its cap of sessions an hour or a day reached, or a session that couldn't start."
            }
            SchedulePaused => {
                "Its owner can no longer run it, or its project was archived: it starts nothing until someone resumes it."
            }
        }
    }
}

/// `s` with a capital first letter, as a list starts an item; fctl's name
/// stays as it is.
fn capitalized(s: &str) -> String {
    if s.starts_with("fctl ") {
        return s.to_owned();
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Every event webhooks get, in their groups (`GET /api/v1/webhook-events`,
/// which adds where the instance lets them post, [`Reach::shown`]).
pub fn catalog() -> WebhookEvents {
    WebhookEvents {
        groups: Group::ALL
            .into_iter()
            .map(|g| WebhookEventGroup {
                id: g.id().into(),
                label: g.label().into(),
            })
            .collect(),
        events: Kind::ALL
            .into_iter()
            .map(|k| WebhookEventInfo {
                kind: k.as_str().into(),
                group: k.group().id().into(),
                label: k.label(),
                description: k.description().into(),
                sessions: k.of_sessions(),
            })
            .collect(),
        reach: WebhookReach::default(),
    }
}

// ───────────────────────────── who gets one ─────────────────────────────

/// A webhook, as far as choosing what it gets goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hook {
    /// A project's own webhook: only its events.
    pub project_id: Option<String>,
    /// Empty: every event.
    pub events: Vec<String>,
    /// A workspace's webhook: only these projects'; empty: every one.
    pub projects: Vec<String>,
    /// Sessions' events: only of sessions that started so; empty: all.
    pub started_by: Vec<WebhookOrigin>,
    pub enabled: bool,
}

/// Whether webhook `h` wants an event of `kind` in project `project_id`
/// (none: the workspace's own), of a session that started as `origin`.
pub fn wants(
    h: &Hook,
    kind: Kind,
    project_id: Option<&str>,
    origin: Option<WebhookOrigin>,
) -> bool {
    if !h.enabled {
        return false;
    }
    if !h.events.is_empty() && !h.events.iter().any(|e| e == kind.as_str()) {
        return false;
    }
    if let Some(own) = &h.project_id
        && project_id != Some(own.as_str())
    {
        return false;
    }
    if !h.projects.is_empty() && !project_id.is_some_and(|p| h.projects.iter().any(|x| x == p)) {
        return false;
    }
    if kind.of_sessions()
        && !h.started_by.is_empty()
        && !origin.is_some_and(|o| h.started_by.contains(&o))
    {
        return false;
    }
    true
}

/// How a session started, for `started_by`; none for a review agent's
/// session, which sends no sessions' events: its review's events say how
/// it goes.
pub fn origin(s: &Session) -> Option<WebhookOrigin> {
    if s.review.is_some() {
        return None;
    }
    Some(if s.autopilot.is_some() {
        WebhookOrigin::Autopilot
    } else if s.schedule.is_some() {
        WebhookOrigin::Schedule
    } else if s.ticket.is_some() {
        WebhookOrigin::Ticket
    } else {
        WebhookOrigin::Person
    })
}

// ───────────────────────────── settings ─────────────────────────────

/// A webhook's name: trimmed, at most [`NAME_MAX`] characters; empty is
/// none, so that it goes by its address's host.
pub fn check_name(name: Option<&str>) -> Result<Option<String>> {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    if name.chars().any(char::is_control) {
        return Err(ApiError::validation("name", "Remove control characters"));
    }
    if name.chars().count() > NAME_MAX {
        return Err(ApiError::validation(
            "name",
            format!("Use at most {NAME_MAX} characters"),
        ));
    }
    Ok(Some(name.to_owned()))
}

/// The events a webhook asks for, checked: each one fctl has, once, in the
/// catalog's order. Empty is every event.
pub fn check_events(events: &[String]) -> Result<Vec<String>> {
    for e in events {
        if Kind::parse(e.trim()).is_none() {
            return Err(ApiError::validation(
                "events",
                format!(
                    "fctl has no event called {}: GET /api/v1/webhook-events lists them",
                    e.trim()
                ),
            ));
        }
    }
    Ok(Kind::ALL
        .into_iter()
        .filter(|k| events.iter().any(|e| e.trim() == k.as_str()))
        .map(|k| k.as_str().to_owned())
        .collect())
}

/// How its sessions started, checked: each once, in order.
pub fn check_started_by(origins: &[WebhookOrigin]) -> Result<Vec<WebhookOrigin>> {
    if origins.contains(&WebhookOrigin::Other) {
        return Err(ApiError::validation(
            "started_by",
            "Sessions start from a person, a schedule, an autopilot or a ticket",
        ));
    }
    Ok(WebhookOrigin::ALL
        .into_iter()
        .filter(|o| origins.contains(o))
        .collect())
}

// ───────────────────────────── where webhooks post ─────────────────────────────

/// Where an instance lets webhooks post besides public `https://`
/// addresses, for receivers on its own network: plain `http://`
/// (`FCTL_WEBHOOK_HTTP`), and the private networks and local names its
/// settings list (`FCTL_WEBHOOK_PRIVATE_HOSTS`). Self-hosted instances set
/// them; app.fctl.sh sets neither, so its webhooks post only to public
/// `https://` addresses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reach {
    /// `http://` as well as `https://`.
    pub http: bool,
    /// The private addresses and local names it takes.
    pub private: Vec<Private>,
}

/// A private network or local name webhooks may post to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Private {
    /// An IP range, `10.0.0.0/8`, or one address, `192.168.1.20`.
    Range(IpAddr, u8),
    /// A host name, `hooks.lan`.
    Name(String),
    /// The names under a domain, `*.corp.internal`.
    Under(String),
    /// Any address: a development instance's, to try receivers on the same
    /// machine.
    Any,
}

impl Reach {
    /// A development instance's: `http://`, and any address.
    pub fn anything() -> Self {
        Self {
            http: true,
            private: vec![Private::Any],
        }
    }

    /// From the settings: `http` is `FCTL_WEBHOOK_HTTP` (`1` or `true`), and
    /// `private` `FCTL_WEBHOOK_PRIVATE_HOSTS`, IP ranges and host names
    /// separated by commas or spaces ([`Private::parse`]). Answers the
    /// entries it can't read too, which it leaves out.
    pub fn parse(http: Option<&str>, private: Option<&str>) -> (Self, Vec<String>) {
        let mut unread = Vec::new();
        let private = private
            .unwrap_or_default()
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|e| !e.is_empty())
            .filter_map(|e| {
                let p = Private::parse(e);
                if p.is_none() {
                    unread.push(e.to_owned());
                }
                p
            })
            .collect();
        let http = http.is_some_and(|v| matches!(v.trim(), "1" | "true"));
        (Self { http, private }, unread)
    }

    /// Whether it takes `ip`, an address that isn't public.
    pub fn takes_ip(&self, ip: IpAddr) -> bool {
        self.private.iter().any(|p| p.takes_ip(ip))
    }

    /// Whether it takes `name`, a name only a local network resolves.
    pub fn takes_name(&self, name: &str) -> bool {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        self.private.iter().any(|p| p.takes_name(&name))
    }

    /// The private addresses it takes, as its settings list them.
    pub fn listed(&self) -> Vec<String> {
        self.private.iter().map(ToString::to_string).collect()
    }

    /// As the API shows it to those who add webhooks (`GET
    /// /api/v1/webhook-events`).
    pub fn shown(&self) -> WebhookReach {
        WebhookReach {
            http: self.http,
            private: self.listed(),
        }
    }
}

impl Private {
    /// One entry of `FCTL_WEBHOOK_PRIVATE_HOSTS`: an IP range
    /// (`10.0.0.0/8`, `fd00::/8`), one address (`192.168.1.20`), a host name
    /// (`hooks.lan`, `jenkins`) or the names under a domain
    /// (`*.corp.internal`). None for anything else.
    pub fn parse(entry: &str) -> Option<Self> {
        let entry = entry.trim();
        let bare = |s: &str| s.trim_start_matches('[').trim_end_matches(']').to_owned();
        if let Some((ip, bits)) = entry.split_once('/') {
            let ip: IpAddr = bare(ip).parse().ok()?;
            let bits: u8 = bits.parse().ok()?;
            return (bits <= max_bits(ip)).then_some(Self::Range(ip, bits));
        }
        if let Ok(ip) = bare(entry).parse::<IpAddr>() {
            return Some(Self::Range(ip, max_bits(ip)));
        }
        let name = entry.trim_end_matches('.').to_ascii_lowercase();
        let (under, name) = match name.strip_prefix("*.") {
            Some(domain) => (true, domain.to_owned()),
            None => (false, name),
        };
        let label = |l: &str| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        };
        if name.len() > 253 || !name.split('.').all(label) {
            return None;
        }
        Some(if under {
            Self::Under(name)
        } else {
            Self::Name(name)
        })
    }

    fn takes_ip(&self, ip: IpAddr) -> bool {
        match self {
            Self::Any => true,
            Self::Range(net, bits) => in_range(ip, *net, *bits),
            Self::Name(_) | Self::Under(_) => false,
        }
    }

    fn takes_name(&self, name: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Name(n) => name == n,
            Self::Under(domain) => name
                .strip_suffix(domain.as_str())
                .is_some_and(|rest| rest.len() > 1 && rest.ends_with('.')),
            Self::Range(..) => false,
        }
    }
}

impl std::fmt::Display for Private {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Range(ip, bits) if *bits == max_bits(*ip) => write!(f, "{ip}"),
            Self::Range(ip, bits) => write!(f, "{ip}/{bits}"),
            Self::Name(name) => f.write_str(name),
            Self::Under(domain) => write!(f, "*.{domain}"),
            Self::Any => f.write_str("*"),
        }
    }
}

fn max_bits(ip: IpAddr) -> u8 {
    if ip.is_ipv4() { 32 } else { 128 }
}

/// Whether `ip` is in the range `net`/`bits`; an IPv4 address mapped into
/// IPv6 is in IPv4's ranges.
fn in_range(ip: IpAddr, net: IpAddr, bits: u8) -> bool {
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    };
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(n)) => {
            let mask = u32::MAX.checked_shl(32 - u32::from(bits)).unwrap_or(0);
            u32::from(a) & mask == u32::from(n) & mask
        }
        (IpAddr::V6(a), IpAddr::V6(n)) => {
            let mask = u128::MAX.checked_shl(128 - u32::from(bits)).unwrap_or(0);
            u128::from(a) & mask == u128::from(n) & mask
        }
        _ => false,
    }
}

/// An address fctl posts a webhook's events to, checked and normalized:
/// `https://`, without a user name or password, to a host on the public
/// internet: not an IP address of a private network, loopback, link-local
/// or otherwise not routed, nor `localhost` or a name that only a local
/// network resolves; besides those, what the instance's `reach` takes
/// (`http://`, the private networks and local names its settings list).
/// Checked as a webhook is added or changed, and again before each post,
/// as an instance's settings may take less since.
pub fn check_url(input: &str, reach: &Reach) -> Result<String> {
    let field = |m: &str| ApiError::validation("url", m);
    let input = input.trim();
    if input.is_empty() {
        return Err(field("Give the address fctl posts to: https://…"));
    }
    if input.len() > URL_MAX {
        return Err(field(&format!("Use at most {URL_MAX} characters")));
    }
    let url = url::Url::parse(input)
        .map_err(|_| field("That isn't an address: it starts with https:// and a host"))?;
    match url.scheme() {
        "https" => {}
        "http" if reach.http => {}
        "http" => {
            return Err(field(
                "Use https://: fctl posts only over HTTPS, so that nobody on the way reads or changes what it sends",
            ));
        }
        _ if reach.http => return Err(field("Use an https:// or http:// address")),
        _ => return Err(field("Use an https:// address")),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(field(
            "Leave the user name and password out of the address: check fctl's signature instead, or put a token in its query",
        ));
    }
    let local = || {
        let listed = reach.listed();
        field(&if listed.is_empty() {
            "Use a public address: fctl doesn't post to private networks, loopback or local names"
                .to_owned()
        } else {
            format!(
                "Use a public address, or one on the network this instance lets webhooks reach ({}): fctl doesn't post to other private networks, loopback or local names",
                listed.join(", ")
            )
        })
    };
    match url.host() {
        None => return Err(field("Give the address a host")),
        Some(url::Host::Ipv4(ip)) => {
            if !public_v4(ip) && !reach.takes_ip(IpAddr::V4(ip)) {
                return Err(local());
            }
        }
        Some(url::Host::Ipv6(ip)) => {
            if !public_v6(ip) && !reach.takes_ip(IpAddr::V6(ip)) {
                return Err(local());
            }
        }
        Some(url::Host::Domain(name)) => {
            if local_name(name) && !reach.takes_name(name) {
                return Err(local());
            }
        }
    }
    let mut url = url;
    url.set_fragment(None);
    Ok(url.into())
}

/// A webhook's address as those who don't manage it see it: where it goes,
/// without its path or query, which can hold a credential (Slack's do).
pub fn shortened(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => match (u.host_str(), u.port()) {
            (Some(host), Some(port)) => format!("{}://{host}:{port}/…", u.scheme()),
            (Some(host), None) => format!("{}://{host}/…", u.scheme()),
            _ => "…".to_owned(),
        },
        Err(_) => "…".to_owned(),
    }
}

/// The host of a webhook's address, which it goes by without a name.
pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| url.to_owned())
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        // Shared address space (carrier-grade NAT), 100.64.0.0/10.
        || (a == 100 && (64..128).contains(&b))
        // IETF protocol assignments, 192.0.0.0/24.
        || (a == 192 && b == 0 && c == 0)
        // Benchmarking, 198.18.0.0/15.
        || (a == 198 && (18..20).contains(&b))
        // Reserved, 240.0.0.0/4.
        || a >= 240)
}

fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // Unique local, fc00::/7.
        || (first & 0xfe00) == 0xfc00
        // Link-local, fe80::/10.
        || (first & 0xffc0) == 0xfe80
        // Site-local (deprecated), fec0::/10.
        || (first & 0xffc0) == 0xfec0
        // Documentation, 2001:db8::/32.
        || (first == 0x2001 && ip.segments()[1] == 0x0db8)
        // NAT64, 64:ff9b::/96, and IPv4-compatible ::a.b.c.d: an IPv4
        // address inside.
        || (first == 0x64 && ip.segments()[1] == 0xff9b)
        || ip.segments()[..6].iter().all(|s| *s == 0))
}

/// Names only a local network resolves: `localhost`, a name without a dot,
/// and the suffixes local networks use.
fn local_name(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if !name.contains('.') {
        return true;
    }
    const LOCAL: [&str; 7] = [
        "localhost",
        "local",
        "internal",
        "localdomain",
        "lan",
        "home.arpa",
        "intranet",
    ];
    LOCAL
        .iter()
        .any(|l| name == *l || name.ends_with(&format!(".{l}")))
        || name.parse::<IpAddr>().is_ok()
}

// ───────────────────────────── signing ─────────────────────────────

/// How a webhook's secret starts, as Standard Webhooks' libraries take it.
pub const SECRET_PREFIX: &str = "whsec_";
/// How far a post's timestamp may be from the receiver's clock before
/// [`verify`] refuses it, in seconds: older posts may be replays.
pub const TOLERANCE_S: i64 = 300;

/// A webhook's secret, from 32 random bytes.
pub fn secret(bytes: &[u8; 32]) -> String {
    format!("{SECRET_PREFIX}{}", STANDARD.encode(bytes))
}

/// The key a secret signs with: its bytes.
pub fn key(secret: &str) -> Option<Vec<u8>> {
    STANDARD
        .decode(secret.trim().strip_prefix(SECRET_PREFIX)?)
        .ok()
        .filter(|k| !k.is_empty())
}

/// The signature of a post with key `key`: `v1,` and the HMAC-SHA256 of
/// `{id}.{timestamp}.{body}` in base64, Standard Webhooks' `v1` scheme.
pub fn sign(key: &[u8], id: &str, timestamp: i64, body: &[u8]) -> String {
    let tag = mac(key, id, &timestamp.to_string(), body)
        .map(|m| m.finalize().into_bytes().to_vec())
        .unwrap_or_default();
    format!("v1,{}", STANDARD.encode(tag))
}

/// The MAC of what a post signs, ready to finish or check.
fn mac(key: &[u8], id: &str, timestamp: &str, body: &[u8]) -> Option<Hmac<Sha256>> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).ok()?;
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    Some(mac)
}

/// The `webhook-signature` header: a signature with each key, the newest
/// first, separated by spaces (a rotated secret's predecessor signs too
/// for a day).
pub fn signatures(keys: &[Vec<u8>], id: &str, timestamp: i64, body: &[u8]) -> String {
    keys.iter()
        .map(|k| sign(k, id, timestamp, body))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Why [`verify`] refused a post.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unverified {
    /// The secret isn't one fctl made (`whsec_…`).
    Secret,
    /// `webhook-timestamp` isn't a number of seconds.
    Timestamp,
    /// The timestamp is more than [`TOLERANCE_S`] from now.
    TooOld,
    /// No signature in `webhook-signature` matches.
    Signature,
}

/// Checks a post as a receiver does: `secret` is the webhook's, `id`,
/// `timestamp` and `signature` its `webhook-*` headers, and `now_s` the
/// receiver's clock in seconds. `fctl webhooks verify` and the docs'
/// examples do the same.
pub fn verify(
    secret: &str,
    id: &str,
    timestamp: &str,
    body: &[u8],
    signature: &str,
    now_s: i64,
) -> std::result::Result<(), Unverified> {
    let key = key(secret).ok_or(Unverified::Secret)?;
    let timestamp = timestamp.trim();
    let ts: i64 = timestamp.parse().map_err(|_| Unverified::Timestamp)?;
    if now_s.abs_diff(ts) > TOLERANCE_S.unsigned_abs() {
        return Err(Unverified::TooOld);
    }
    // Each signature is checked in constant time (`verify_slice`).
    let signed = signature
        .split_whitespace()
        .filter_map(|s| s.strip_prefix("v1,"))
        .filter_map(|b| STANDARD.decode(b).ok())
        .any(|given| {
            mac(&key, id, timestamp, body).is_some_and(|m| m.verify_slice(&given).is_ok())
        });
    if signed {
        Ok(())
    } else {
        Err(Unverified::Signature)
    }
}

/// What a post's headers say besides its body, in the order fctl sends
/// them.
pub struct Headers<'a> {
    pub event_id: &'a str,
    pub kind: &'a str,
    /// None for an address's challenge, which is no delivery.
    pub delivery_id: Option<&'a str>,
    pub webhook_id: &'a str,
    pub workspace_id: &'a str,
    pub project_id: Option<&'a str>,
    /// Seconds since the epoch.
    pub timestamp: i64,
    pub signature: &'a str,
    /// This attempt's number, from 1.
    pub attempt: u32,
    /// The control plane's version, for `User-Agent`.
    pub version: &'a str,
    pub test: bool,
}

impl Headers<'_> {
    pub fn to_vec(&self) -> Vec<(String, String)> {
        let mut h = vec![
            ("content-type".into(), "application/json".into()),
            (
                "user-agent".into(),
                format!(
                    "fctl-webhooks/{} (+https://fctl.sh/docs/automation#webhooks)",
                    self.version
                ),
            ),
            ("webhook-id".into(), self.event_id.into()),
            ("webhook-timestamp".into(), self.timestamp.to_string()),
            ("webhook-signature".into(), self.signature.into()),
            ("fctl-event".into(), self.kind.into()),
        ];
        if let Some(d) = self.delivery_id {
            h.push(("fctl-delivery".into(), d.into()));
        }
        h.push(("fctl-webhook".into(), self.webhook_id.into()));
        h.push(("fctl-workspace-id".into(), self.workspace_id.into()));
        if let Some(p) = self.project_id {
            h.push(("fctl-project-id".into(), p.into()));
        }
        h.push(("fctl-attempt".into(), self.attempt.to_string()));
        if self.test {
            h.push(("fctl-test".into(), "1".into()));
        }
        h
    }
}

/// Headers the delivery log shows with their value hidden.
pub fn hidden_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("webhook-signature")
}

// ───────────────────────────── an address's challenge ─────────────────────────────

/// The type of the request that checks an address as a webhook is added
/// there or pointed there, as Slack's Events API names its own: the address
/// answers it with its `challenge`, which shows that it takes fctl's
/// requests before any event goes there.
pub const CHALLENGE: &str = "url_verification";

/// The challenges a workspace's webhooks may post in an hour, to addresses
/// people add them at or point them to: fctl doesn't knock on an address
/// over and over for whoever asks.
pub const CHALLENGES_PER_HOUR: u32 = 20;

/// A challenge, from 32 random bytes: base64url, 43 characters.
pub fn challenge(bytes: &[u8; 32]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// What fctl posts to check an address, at `at`: the challenge, and no
/// more of the workspace than its headers carry.
pub fn challenge_body(event_id: &str, challenge: &str, at: i64) -> String {
    json!({
        "id": event_id,
        "type": CHALLENGE,
        "timestamp": crate::account::iso8601(at),
        "challenge": challenge,
    })
    .to_string()
}

/// Whether an answer's `body` gives `challenge` back, as Slack takes one:
/// the challenge as text, JSON's `{"challenge": …}`, or a form's
/// `challenge=…`.
pub fn gives_back(challenge: &str, body: &str) -> bool {
    let body = body.trim();
    body == challenge
        || serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| {
                v.get("challenge")
                    .and_then(Value::as_str)
                    .map(|c| c == challenge)
            })
            .unwrap_or(false)
        || url::form_urlencoded::parse(body.as_bytes())
            .any(|(k, v)| k == "challenge" && v == challenge)
}

/// Why an address's answer to fctl's challenge, its `status` (none when no
/// answer came, for `error`) and `body`, doesn't let a webhook post there,
/// in the API's words; None when it gave the challenge back.
pub fn challenge_refused(
    challenge: &str,
    status: Option<u16>,
    error: Option<&str>,
    body: Option<&str>,
) -> Option<String> {
    const ANSWER: &str = "It answers fctl's url_verification request with the challenge, before it checks the signature: a new webhook's secret reaches it only once the webhook is added";
    let Some(status) = status else {
        return Some(format!(
            "fctl posted a challenge to this address and got no answer: {}. {ANSWER}",
            error.unwrap_or("no answer").trim_end_matches('.')
        ));
    };
    if (300..400).contains(&status) {
        return Some(format!(
            "This address answered fctl's challenge with {}",
            status_error(status).trim_end_matches('.')
        ));
    }
    if !delivered(status) {
        return Some(format!(
            "This address answered fctl's challenge with HTTP {status}. {ANSWER}"
        ));
    }
    if body.is_some_and(|b| gives_back(challenge, b)) {
        return None;
    }
    let said: String = body.unwrap_or_default().trim().chars().take(60).collect();
    Some(if said.is_empty() {
        format!("This address answered fctl's challenge with nothing. {ANSWER}")
    } else {
        format!(
            "This address answered fctl's challenge with \"{said}\" rather than the challenge. {ANSWER}"
        )
    })
}

// ───────────────────────────── retries and limits ─────────────────────────────

/// Whether an answer's status counts as delivered.
pub fn delivered(status: u16) -> bool {
    (200..300).contains(&status)
}

/// When fctl tries a delivery again after its `attempts`th attempt failed
/// at `now`: the next of [`RETRIES_MS`], or later where the receiver
/// asked (`retry_after_ms`, from `Retry-After`), up to
/// [`RETRY_AFTER_MAX_MS`]. None once it had all its attempts.
pub fn retry_at(attempts: u32, now: i64, retry_after_ms: Option<i64>) -> Option<i64> {
    let wait = *RETRIES_MS.get(usize::try_from(attempts).ok()?.checked_sub(1)?)?;
    let wait = retry_after_ms.map_or(wait, |asked| {
        asked.clamp(wait, RETRY_AFTER_MAX_MS.max(wait))
    });
    Some(now.saturating_add(wait))
}

/// Who makes an attempt at a delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    /// fctl, when it's due: its first attempt and the retries.
    Fctl,
    /// A person resending it.
    Resend,
    /// A person's test, a delivery of its own.
    Test,
}

/// Whether an attempt by `by` posts a delivery that stands as `state`, due
/// at `next_at`, at `now`: a person's always; fctl's only while it waits
/// and is due, as a person's resend may have delivered it, or turning its
/// webhook off failed it, since the alarm picked it.
pub fn posts(by: By, state: WebhookDeliveryState, next_at: Option<i64>, now: i64) -> bool {
    by != By::Fctl
        || (state == WebhookDeliveryState::Pending && next_at.is_some_and(|at| at <= now))
}

/// fctl's own attempts in a delivery's history: those no person made. Its
/// retries go by them, so that people's resends take none.
pub fn own_attempts(history: &[WebhookAttempt]) -> u32 {
    u32::try_from(history.iter().filter(|a| a.by.is_none()).count()).unwrap_or(u32::MAX)
}

/// What a delivery becomes after an attempt.
#[derive(Debug, PartialEq, Eq)]
pub struct Settled {
    pub state: WebhookDeliveryState,
    pub next_at: Option<i64>,
    /// When it was done; `None` keeps the time it has (none while it waits).
    pub done_at: Option<i64>,
    /// Another settled it while the attempt waited for its answer: it keeps
    /// why, as it keeps its state.
    pub kept: bool,
}

/// What a delivery becomes after an attempt by `by` that went through
/// (`ok`) or not at `now`, standing as `was` with `next_at` as the attempt
/// ends (another may have changed it meanwhile), after `own` attempts of
/// fctl's ([`own_attempts`], this one included when it's fctl's). fctl's
/// own attempt at one that waits goes again on the retries' times
/// ([`retry_at`]); one that a resend delivered, or turning its webhook off
/// failed, meanwhile stays as it was left; a person's resend of one that
/// waits leaves its retries as they were, and of one that was done, or a
/// test, leaves it failed.
pub fn settle(
    by: By,
    was: WebhookDeliveryState,
    next_at: Option<i64>,
    ok: bool,
    own: u32,
    now: i64,
    retry_after_ms: Option<i64>,
) -> Settled {
    use WebhookDeliveryState::{Delivered, Failed, Pending};
    let done = |state| Settled {
        state,
        next_at: None,
        done_at: Some(now),
        kept: false,
    };
    if ok {
        return done(Delivered);
    }
    match (by, was) {
        (By::Fctl, Pending) => match retry_at(own, now, retry_after_ms) {
            Some(at) => Settled {
                state: Pending,
                next_at: Some(at),
                done_at: None,
                kept: false,
            },
            None => done(Failed),
        },
        (By::Fctl, settled) => Settled {
            state: settled,
            next_at,
            done_at: None,
            kept: true,
        },
        (By::Resend, Pending) => Settled {
            state: Pending,
            next_at,
            done_at: None,
            kept: false,
        },
        (By::Resend | By::Test, _) => done(Failed),
    }
}

/// A `Retry-After` header's wait, in milliseconds: its seconds. (Its other
/// form, a date, is left to the retries' own times.)
pub fn retry_after(value: &str) -> Option<i64> {
    value
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|s| *s >= 0)
        .map(|s| s.saturating_mul(1000))
}

/// What went wrong with an answer of `status`, in the log's words.
pub fn status_error(status: u16) -> String {
    if (300..400).contains(&status) {
        format!(
            "HTTP {status}: a redirect, which fctl doesn't follow. Give the address it redirects to."
        )
    } else {
        format!("HTTP {status}")
    }
}

/// What the log says of an attempt that got no answer within
/// [`TIMEOUT_MS`].
pub fn timeout_error() -> String {
    format!(
        "No answer within {} seconds: answer first, then do the slow work.",
        TIMEOUT_MS / 1000
    )
}

/// Whether a webhook that has failed every attempt since `failing_since`
/// turns itself off at `now`, as a delivery failed for good (`gave_up`:
/// its last attempt, after about 21 hours of them): a first attempt that
/// fails after days without one has its retries to go yet.
pub fn turns_off(failing_since: Option<i64>, now: i64, gave_up: bool) -> bool {
    gave_up && failing_since.is_some_and(|since| now - since >= OFF_AFTER_MS)
}

/// Why fctl turned a webhook off, the last attempt having failed with
/// `last` ("HTTP 502").
pub fn off_reason(last: &str) -> String {
    let last = last.trim_end_matches('.');
    format!(
        "Every delivery failed for 3 days, the last with {last}. Turn it on again once its address answers."
    )
}

/// Which webhook gets its next attempt first when several are due: the
/// one whose oldest due delivery waited longest (`due`, its time), so that
/// a busy one doesn't hold the others back. Ties go by id.
pub fn order_due(due: &mut [(String, i64)]) {
    due.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
}

/// How many more of a webhook's deliveries may go now, with `sent` of them
/// posted within the last minute.
pub fn room(sent: u32) -> u32 {
    PER_MINUTE.saturating_sub(sent)
}

// ───────────────────────────── sessions ─────────────────────────────

/// Whether a session with `status` and its `reason` waits for a message,
/// with no background work running: its agent finished.
pub fn waits_for_message(status: SessionStatus, reason: Option<&str>) -> bool {
    status == SessionStatus::Idle && !waits_for_background(reason)
}

/// How a session stood, as far as its events go: its status, and whether
/// it waited for a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stood {
    pub status: SessionStatus,
    pub waiting: bool,
}

impl Stood {
    pub fn of(s: &Session) -> Self {
        Self {
            status: s.status,
            waiting: waits_for_message(s.status, s.status_reason.as_deref()),
        }
    }

    /// As meta keeps it: `idle:1`.
    pub fn key(self) -> String {
        format!("{}:{}", self.status.as_str(), u8::from(self.waiting))
    }

    pub fn parse(key: &str) -> Option<Self> {
        let (status, waiting) = key.split_once(':')?;
        Some(Self {
            status: SessionStatus::parse(status)?,
            waiting: waiting == "1",
        })
    }
}

/// The events a session's move from `before` to `now` sends.
pub fn moved(before: Stood, now: Stood) -> Vec<Kind> {
    let mut kinds = Vec::new();
    if before.status != now.status {
        kinds.push(Kind::SessionStatusChanged);
        match now.status {
            SessionStatus::Paused => kinds.push(Kind::SessionPaused),
            SessionStatus::Stopped => kinds.push(Kind::SessionStopped),
            SessionStatus::Failed => kinds.push(Kind::SessionFailed),
            _ => {}
        }
    }
    if now.waiting && !(before.waiting && before.status == now.status) {
        kinds.push(Kind::SessionFinished);
    }
    kinds
}

/// What a session's pull requests did between `before` and `now`, by url:
/// new ones that aren't merged or closed opened; others merged or closed.
pub fn pull_moves<'a>(
    before: &[PullRequestRef],
    now: &'a [PullRequestRef],
) -> Vec<(Kind, &'a PullRequestRef)> {
    let mut moves = Vec::new();
    for pr in now {
        let state = pr.state.as_deref();
        match before.iter().find(|b| b.url == pr.url) {
            None if !matches!(state, Some("merged" | "closed")) => {
                moves.push((Kind::PullRequestOpened, pr));
            }
            None => {}
            Some(old) if old.state.as_deref() != state => match state {
                Some("merged") => moves.push((Kind::PullRequestMerged, pr)),
                Some("closed") if old.state.as_deref() != Some("merged") => {
                    moves.push((Kind::PullRequestClosed, pr));
                }
                _ => {}
            },
            Some(_) => {}
        }
    }
    moves
}

/// The open pull requests of `s` that wait for a person's review, and a
/// key that says which commits they are at, to say so once: its agent
/// waits for a message with no background work and nothing asked; it has
/// an open pull request (not a draft, queued, merged or closed), and each
/// of its open ones has checks GitHub told fctl about, none failed or
/// still to finish, and no review agent's review waiting for a runner or
/// under way. None for a review's session or an archived one.
pub fn ready_for_review(s: &Session) -> Option<(Vec<&PullRequestRef>, String)> {
    if s.review.is_some()
        || s.archived
        || s.question.is_some()
        || !waits_for_message(s.status, s.status_reason.as_deref())
    {
        return None;
    }
    let open: Vec<&PullRequestRef> = s
        .pull_requests
        .iter()
        .filter(|pr| pr.state.as_deref() == Some("open"))
        .collect();
    if open.is_empty() {
        return None;
    }
    let mut key = Vec::new();
    for pr in &open {
        let checks = pr.checks.as_ref()?;
        if checks.failed > 0 || checks.pending > 0 {
            return None;
        }
        if checks
            .review
            .as_ref()
            .is_some_and(|r| r.state == ReviewState::Running)
        {
            return None;
        }
        key.push(format!(
            "{}@{}",
            pr.url,
            checks.sha.as_deref().unwrap_or_default()
        ));
    }
    key.sort();
    Some((open, key.join(" ")))
}

// ───────────────────────────── what posts carry ─────────────────────────────

/// How people refer to a session: `web#12`.
pub fn reference(s: &Session) -> String {
    format!("{}#{}", s.project_slug, s.number)
}

/// A pull request as posts give it.
pub fn pull_request(pr: &PullRequestRef) -> HookPullRequest {
    HookPullRequest {
        repo: pr
            .repo
            .as_ref()
            .map(|r| r.full_name())
            .or_else(|| repo_of_url(&pr.url))
            .unwrap_or_default(),
        number: pr.number,
        url: pr.url.clone(),
        title: pr.title.clone(),
        state: pr.state.clone(),
        checks: pr.checks.as_ref().map(|c| HookChecks {
            passed: c.passed,
            failed: c.failed,
            pending: c.pending,
            sha: c.sha.clone(),
            review: c.review.as_ref().map(|r| r.state.as_str().to_owned()),
        }),
    }
}

/// `owner/name` of a pull request's address on GitHub.
fn repo_of_url(url: &str) -> Option<String> {
    let path = url.split("://").nth(1)?.split_once('/')?.1;
    let mut parts = path.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    Some(format!("{owner}/{name}"))
}

/// `base`'s address of `path`: the instance's public URL and a path of
/// the app.
fn link(base: &str, path: &str) -> String {
    format!("{}{path}", base.trim_end_matches('/'))
}

/// A schedule's page in the app.
pub fn schedule_url(base: &str, project_slug: &str, id: &str) -> String {
    link(
        base,
        &format!("/p/{project_slug}?tab=schedules&schedule={id}"),
    )
}

/// A session as posts give it; `base` is the instance's public URL.
pub fn session(s: &Session, base: &str) -> HookSession {
    HookSession {
        id: s.id.clone(),
        number: s.number,
        reference: reference(s),
        url: link(base, &format!("/s/{}", s.id)),
        title: s.title.clone(),
        description: s.description.clone(),
        status: s.status,
        status_reason: s.status_reason.clone(),
        started_by: origin(s).unwrap_or(WebhookOrigin::Person),
        created_by: HookPerson {
            id: s.created_by.id.0.clone(),
            name: s.created_by.name.clone(),
        },
        schedule: s.schedule.as_ref().map(|sc| HookSchedule {
            id: sc.id.clone(),
            name: sc.name.clone(),
            url: schedule_url(base, &s.project_slug, sc.id.as_str()),
        }),
        autopilot: s.autopilot.as_ref().map(|a| {
            match a.role {
                AutopilotRole::Pilot => "pilot",
                AutopilotRole::Crew => "crew",
                AutopilotRole::Other => "other",
            }
            .to_owned()
        }),
        ticket: s.ticket.as_ref().map(|t| HookTicket {
            tracker: match t.tracker {
                Tracker::Linear => "linear",
                Tracker::Github => "github",
                Tracker::None => "none",
            }
            .to_owned(),
            key: t.key.clone(),
            url: t.url.clone(),
        }),
        branch: s.branch.clone(),
        base_branch: s.base_branch.clone(),
        pull_requests: s.pull_requests.iter().map(pull_request).collect(),
        question: s.question.as_ref().map(|q| HookQuestion {
            text: q.text.clone(),
            options: q.options.clone(),
            asked_at: q.asked_at,
        }),
        model: s.model.clone(),
        effort: s.effort.clone(),
        turns: s.stats.turns,
        created_at: s.created_at,
        updated_at: s.updated_at,
        started_at: s.started_at,
        ended_at: s.ended_at,
        archived: s.archived,
    }
}

/// Something that happened, ready for the webhooks that want it: its
/// kind, where, of whom, its line in the log and its data.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: Kind,
    pub project_id: Option<String>,
    pub session_id: Option<SessionId>,
    pub origin: Option<WebhookOrigin>,
    pub gist: String,
    pub data: Value,
    pub at: i64,
}

/// `text` cut to `max` characters, with an ellipsis where it was cut.
pub fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out = out.trim_end().to_owned();
    out.push('…');
    out
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What a session's status says in a log's line: "is working".
fn status_words(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Queued => "is queued",
        SessionStatus::Provisioning | SessionStatus::Preparing => "is starting",
        SessionStatus::Running => "is working",
        SessionStatus::Idle => "waits",
        SessionStatus::Paused => "paused",
        SessionStatus::Stopping => "is stopping",
        SessionStatus::Stopped => "stopped",
        SessionStatus::Failed => "failed",
    }
}

/// What else a session's event carries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionExtra {
    pub previous_status: Option<SessionStatus>,
    /// The start of the agent's last reply, for `session.finished`.
    pub message: Option<String>,
    /// What brought the agent back, for `session.follow_up`, and a few
    /// words of what it was for the log: "checks failed on web#45".
    pub follow_up: Option<(Value, String)>,
}

/// A session's event of `kind` (`session.*`), at `at`.
pub fn session_event(kind: Kind, s: &Session, base: &str, at: i64, extra: SessionExtra) -> Event {
    let r = reference(s);
    let title = one_line(&s.title);
    let ready: Vec<HookPullRequest> = match kind {
        Kind::SessionReadyForReview => ready_for_review(s)
            .map(|(prs, _)| prs.into_iter().map(pull_request).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let gist = match kind {
        Kind::SessionStarted => format!("{r} started: {title}"),
        Kind::SessionStatusChanged => format!("{r} {}: {title}", status_words(s.status)),
        Kind::SessionFinished => format!("{r} finished a turn: {title}"),
        Kind::SessionReadyForReview => {
            let prs: Vec<String> = ready
                .iter()
                .map(|p| format!("{}#{}", short_repo(&p.repo), p.number))
                .collect();
            format!("{r} is ready for review: {}", prs.join(", "))
        }
        Kind::SessionQuestion => match &s.question {
            Some(q) => format!("{r} asks: {}", one_line(&q.text)),
            None => format!("{r} asks something"),
        },
        Kind::SessionFollowUp => match &extra.follow_up {
            Some((_, what)) => format!("{r} was brought back: {what}"),
            None => format!("{r} was brought back"),
        },
        Kind::SessionPaused => format!("{r} paused: {title}"),
        Kind::SessionStopped => format!("{r} stopped: {title}"),
        Kind::SessionFailed => match s.status_reason.as_deref() {
            Some(why) => format!("{r} failed: {}", one_line(why)),
            None => format!("{r} failed: {title}"),
        },
        _ => format!("{r}: {title}"),
    };
    let data = HookSessionData {
        session: session(s, base),
        previous_status: extra
            .previous_status
            .filter(|_| kind == Kind::SessionStatusChanged),
        message: extra
            .message
            .filter(|_| kind == Kind::SessionFinished)
            .map(|m| clip(&m, MESSAGE_MAX))
            .filter(|m| !m.is_empty()),
        ready,
        follow_up: extra
            .follow_up
            .filter(|_| kind == Kind::SessionFollowUp)
            .map(|(v, _)| v),
    };
    Event {
        kind,
        project_id: Some(s.project_id.0.clone()),
        session_id: Some(s.id.clone()),
        origin: origin(s),
        gist: clip(&gist, GIST_MAX),
        data: serde_json::to_value(data).unwrap_or(Value::Null),
        at,
    }
}

/// `web` of `acme/web`, as session references name projects.
fn short_repo(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}

/// An event of a session's pull request (`pull_request.*`).
pub fn pull_request_event(
    kind: Kind,
    s: &Session,
    pr: &PullRequestRef,
    base: &str,
    at: i64,
) -> Event {
    let p = pull_request(pr);
    let name = format!("{}#{}", p.repo, p.number);
    let title = p.title.as_deref().map(one_line).unwrap_or_default();
    let what = match kind {
        Kind::PullRequestOpened => format!("{} opened {name}", reference(s)),
        Kind::PullRequestMerged => format!("{name} merged"),
        _ => format!("{name} closed"),
    };
    let gist = if title.is_empty() {
        what
    } else {
        format!("{what}: {title}")
    };
    Event {
        kind,
        project_id: Some(s.project_id.0.clone()),
        session_id: Some(s.id.clone()),
        origin: origin(s),
        gist: clip(&gist, GIST_MAX),
        data: json!({ "session": session(s, base), "pull_request": p }),
        at,
    }
}

/// What brought an agent back to its pull request (GitHub's follow-up), as
/// `session.follow_up` carries it, and what it was in a few words for the
/// log: "checks failed on web#45".
pub fn follow_up(f: &FollowUp) -> (Value, String) {
    let pr = format!("{}#{}", f.repo.name, f.number);
    let comments: Vec<&str> = f
        .events
        .iter()
        .filter_map(|e| match e {
            FollowUpEvent::Comment(c) => Some(c.login.as_str()),
            _ => None,
        })
        .collect();
    let what = if let [login] = comments.as_slice() {
        format!("a comment by @{login} on {pr}")
    } else if !comments.is_empty() {
        format!("{} comments on {pr}", comments.len())
    } else {
        match f.events.first() {
            Some(FollowUpEvent::ChecksFailed { .. }) => format!("checks failed on {pr}"),
            Some(FollowUpEvent::Merged { .. }) => format!("{pr} merged"),
            Some(FollowUpEvent::Closed { .. }) => format!("{pr} closed"),
            Some(FollowUpEvent::Conflicts { base, .. }) => match base {
                Some(b) => format!("{pr} conflicts with {b}"),
                None => format!("{pr} conflicts with its base branch"),
            },
            _ => format!("news of {pr}"),
        }
    };
    let data = json!({
        "kind": "github",
        "pull_request": {
            "repo": f.repo.full_name(),
            "number": f.number,
            "url": f.url,
            "title": f.title,
            "state": f.state,
            "base": f.base,
        },
        "events": f.events,
    });
    (data, what)
}

/// A review's findings that fctl passed on to the agent that opened the
/// pull request, as `session.follow_up` carries them, and its words for
/// the log: "a review's 2 findings on web#45".
pub fn findings(r: &PassedOnReview) -> (Value, String) {
    let pr = format!("{}#{}", r.repo.name, r.number);
    let what = match r.findings.len() {
        1 => format!("a review's finding on {pr}"),
        n => format!("a review's {n} findings on {pr}"),
    };
    let data = json!({
        "kind": "review",
        "pull_request": {
            "repo": r.repo.full_name(),
            "number": r.number,
            "url": r.url,
            "title": r.title,
        },
        "review": {
            "commit": r.sha,
            "summary_url": r.summary_url,
            "session_id": r.session,
            "findings": r.findings,
        },
    });
    (data, what)
}

/// A review's event (`review.*`), of `review` in project `project_id`.
pub fn review_event(kind: Kind, review: HookReview, project_id: &str, at: i64) -> Event {
    let name = format!(
        "{}#{}",
        review.pull_request.repo, review.pull_request.number
    );
    let sha = crate::reviews::short(&review.commit);
    let gist = match kind {
        Kind::ReviewQueued => format!("Review of {name} at {sha} queued"),
        Kind::ReviewStarted => format!("Review of {name} at {sha} started"),
        Kind::ReviewFinished if review.state == ReviewState::Passed.as_str() => {
            format!("Review of {name} at {sha} passed")
        }
        Kind::ReviewFinished => {
            let found = match review.findings {
                0 => "no new issues".to_owned(),
                1 => "1 new issue".to_owned(),
                n => format!("{n} new issues"),
            };
            let open = match review.still_open {
                0 => String::new(),
                n => format!(", {n} still open"),
            };
            format!("Review of {name} at {sha} found {found}{open}")
        }
        Kind::ReviewFailed => format!("Review of {name} at {sha} failed"),
        _ => format!("Review of {name} at {sha} was cancelled"),
    };
    Event {
        kind,
        project_id: Some(project_id.to_owned()),
        session_id: Some(review.session_id.clone()),
        origin: None,
        gist: clip(&gist, GIST_MAX),
        data: json!({ "review": review }),
        at,
    }
}

/// A schedule's event (`schedule.*`): a run that started nothing, or
/// fctl pausing it, saying why.
pub fn schedule_event(
    kind: Kind,
    schedule: HookSchedule,
    run: Option<HookRun>,
    reason: Option<String>,
    project_id: &str,
    at: i64,
) -> Event {
    let name = schedule.name.clone().unwrap_or_else(|| "A schedule".into());
    let why = run
        .as_ref()
        .and_then(|r| r.reason.clone())
        .or_else(|| reason.clone())
        .map(|w| one_line(&w))
        .unwrap_or_default();
    let gist = match kind {
        Kind::SchedulePaused => format!("fctl paused {name}: {why}"),
        _ => format!("{name} started nothing: {why}"),
    };
    let data = HookScheduleData {
        schedule,
        run,
        reason: reason.filter(|_| kind == Kind::SchedulePaused),
    };
    Event {
        kind,
        project_id: Some(project_id.to_owned()),
        session_id: None,
        origin: None,
        gist: clip(gist.trim_end_matches([' ', ':']), GIST_MAX),
        data: serde_json::to_value(data).unwrap_or(Value::Null),
        at,
    }
}

/// The body fctl posts for event `id`.
pub fn envelope(
    id: &WebhookEventId,
    kind: &str,
    at: i64,
    workspace: HookPlace,
    project: Option<HookPlace>,
    data: Value,
    test: bool,
) -> HookEnvelope {
    HookEnvelope {
        id: id.clone(),
        kind: kind.to_owned(),
        timestamp: crate::account::iso8601(at),
        workspace,
        project,
        data,
        test,
    }
}

// ───────────────────────────── describing one ─────────────────────────────

/// What a webhook posts, and where, in a sentence, as the app's form and
/// `fctl webhooks` say it (web/src/lib/webhooks.ts `describeWebhook` says
/// the same): "Posts to hooks.slack.com when an agent asks something or
/// pull requests are ready for review, in Web and Docs, for sessions
/// started by people." `projects` are the names of the projects a
/// workspace's webhook is narrowed to.
pub fn describe(
    url: &str,
    events: &[String],
    projects: &[String],
    started_by: &[WebhookOrigin],
) -> String {
    let host = host_of(url);
    let kinds: Vec<Kind> = Kind::ALL
        .into_iter()
        .filter(|k| events.iter().any(|e| e == k.as_str()))
        .collect();
    let mut out = match kinds.len() {
        0 => format!("Posts every event to {host}"),
        1..=3 => {
            let phrases: Vec<&str> = kinds.iter().map(|k| k.phrase()).collect();
            format!("Posts to {host} when {}", either(&phrases))
        }
        n => format!("Posts {n} kinds of events to {host}"),
    };
    match projects.len() {
        0 => {}
        1..=3 => out.push_str(&format!(", in {}", both(projects))),
        n => out.push_str(&format!(", in {n} projects")),
    }
    let sessions = kinds.is_empty() || kinds.iter().any(|k| k.of_sessions());
    if sessions && !started_by.is_empty() {
        let who: Vec<&str> = started_by
            .iter()
            .filter_map(|o| match o {
                WebhookOrigin::Person => Some("people"),
                WebhookOrigin::Schedule => Some("schedules"),
                WebhookOrigin::Autopilot => Some("an autopilot"),
                WebhookOrigin::Ticket => Some("tickets"),
                WebhookOrigin::Other => None,
            })
            .collect();
        if !who.is_empty() {
            out.push_str(&format!(", for sessions started by {}", either(&who)));
        }
    }
    out.push('.');
    out
}

/// "a, b or c".
fn either<S: AsRef<str>>(items: &[S]) -> String {
    joined(items, "or")
}

/// "a, b and c".
fn both<S: AsRef<str>>(items: &[S]) -> String {
    joined(items, "and")
}

fn joined<S: AsRef<str>>(items: &[S], word: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.as_ref().to_owned(),
        [rest @ .., last] => format!(
            "{} {word} {}",
            rest.iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", "),
            last.as_ref()
        ),
    }
}

// ───────────────────────────── samples ─────────────────────────────

/// A made-up event of `kind` in `project` (whose primary repository is
/// `repo`), for Send a test: the same shape a real one has, in the world
/// the app's previews draw (an upload worker whose retries never stopped
/// when the server was down, fixed in `3f2a1c9`).
pub fn sample(kind: Kind, project: &HookPlace, repo: &str, base: &str, now: i64) -> Event {
    let s = sample_session(kind, project, repo, now);
    match kind.group() {
        Group::Sessions => {
            let extra = SessionExtra {
                previous_status: Some(SessionStatus::Idle),
                message: Some(
                    "Uploads that fail now go again after 2, 4, 8 and 16 seconds, then give up \
                     and say why. A test covers a server that stays down. The pull request is \
                     up: give it a look when you can."
                        .into(),
                ),
                follow_up: Some((
                    json!({
                        "repo": { "owner": owner_of(repo), "name": short_repo(repo) },
                        "number": 45,
                        "url": format!("https://github.com/{repo}/pull/45"),
                        "title": "Retry failed uploads in the sync worker",
                        "state": "open",
                        "base": "main",
                        "events": [{
                            "kind": "checks_failed",
                            "sha": "3f2a1c9e5b7d4a6c8e0f1a2b3c4d5e6f7a8b9c0d",
                            "names": ["test (ubuntu-latest)"],
                            "passed": 6,
                            "failed": 1,
                            "pending": 0
                        }]
                    }),
                    format!("checks failed on {}#45", short_repo(repo)),
                )),
            };
            session_event(kind, &s, base, now, extra)
        }
        Group::PullRequests => {
            let pr = s
                .pull_requests
                .first()
                .cloned()
                .unwrap_or_else(|| sample_pr(repo, None));
            pull_request_event(kind, &s, &pr, base, now)
        }
        Group::Reviews => {
            let (state, findings, still_open, review_url, reason, finished) = match kind {
                Kind::ReviewQueued | Kind::ReviewStarted => ("running", 0, 0, None, None, None),
                Kind::ReviewFinished => (
                    "commented",
                    1,
                    1,
                    Some(format!(
                        "https://github.com/{repo}/pull/45#issuecomment-2481957311"
                    )),
                    None,
                    Some(now),
                ),
                Kind::ReviewFailed => (
                    "failed",
                    0,
                    0,
                    None,
                    Some("The review session failed: no runner could start its sandbox.".into()),
                    Some(now),
                ),
                _ => (
                    "superseded",
                    0,
                    0,
                    None,
                    Some("A newer commit came, 8d0e4b2: fctl reviews that one instead.".to_owned()),
                    Some(now),
                ),
            };
            let review_session = SessionId("ses_01KSAMPLEREVIEW00000000000".into());
            let review = HookReview {
                session_url: link(base, &format!("/s/{review_session}")),
                session_id: review_session,
                pull_request: pull_request(&sample_pr(repo, Some("open"))),
                commit: "3f2a1c9e5b7d4a6c8e0f1a2b3c4d5e6f7a8b9c0d".into(),
                state: state.into(),
                findings,
                fixed: u32::from(kind == Kind::ReviewFinished),
                still_open,
                review_url,
                reason,
                operator: Some(HookPerson {
                    id: "usr_01KSAMPLEPERSON00000000000".into(),
                    name: "Ada Lovelace".into(),
                }),
                created_at: now - 4 * MINUTE,
                finished_at: finished,
            };
            review_event(kind, review, &project.id, now)
        }
        Group::Schedules => {
            let id = ScheduleId("sch_01KSAMPLESCHEDULE000000000".into());
            let schedule = HookSchedule {
                url: schedule_url(base, &project.slug, id.as_str()),
                id,
                name: Some("Update dependencies".into()),
            };
            match kind {
                Kind::SchedulePaused => schedule_event(
                    kind,
                    schedule,
                    None,
                    Some(format!(
                        "Ada Lovelace can no longer start sessions in {}.",
                        project.name
                    )),
                    &project.id,
                    now,
                ),
                _ => schedule_event(
                    kind,
                    schedule,
                    Some(HookRun {
                        id: fctl_proto::ScheduleRunId("srn_01KSAMPLERUN0000000000000".into()),
                        trigger: "time".into(),
                        outcome: "skipped".into(),
                        reason: Some(format!(
                            "Its last run, session #{}, was still working on it.",
                            s.number
                        )),
                        at: now,
                        event_url: None,
                    }),
                    None,
                    &project.id,
                    now,
                ),
            }
        }
    }
}

fn owner_of(repo: &str) -> &str {
    repo.split('/').next().unwrap_or(repo)
}

/// The sample's pull request, `repo#45`, its checks all passed and its
/// review posted.
fn sample_pr(repo: &str, state: Option<&str>) -> PullRequestRef {
    serde_json::from_value(json!({
        "number": 45,
        "url": format!("https://github.com/{repo}/pull/45"),
        "title": "Retry failed uploads in the sync worker",
        "state": state.unwrap_or("open"),
        "repo": { "owner": owner_of(repo), "name": short_repo(repo) },
        "checks": {
            "passed": 7,
            "failed": 0,
            "pending": 0,
            "sha": "3f2a1c9e5b7d4a6c8e0f1a2b3c4d5e6f7a8b9c0d",
            "at": 0,
            "review": { "state": "passed" }
        }
    }))
    .unwrap_or_else(|_| PullRequestRef {
        number: 45,
        url: format!("https://github.com/{repo}/pull/45"),
        ..Default::default()
    })
}

/// The sample's session, as it would stand for `kind`.
fn sample_session(kind: Kind, project: &HookPlace, repo: &str, now: i64) -> Session {
    let (status, reason) = match kind {
        Kind::SessionStarted => ("queued", None),
        Kind::SessionStatusChanged | Kind::SessionFollowUp => ("running", None),
        Kind::SessionPaused => (
            "paused",
            Some("Ada Lovelace's Claude plan reached its 5-hour limit: it resets at 14:00."),
        ),
        Kind::SessionStopped => ("stopped", Some("Stopped after being idle")),
        Kind::SessionFailed => (
            "failed",
            Some("The runner couldn't start its sandbox: docker isn't running."),
        ),
        _ => ("idle", Some("Waiting for a message")),
    };
    let pr_state = match kind {
        Kind::PullRequestMerged => "merged",
        Kind::PullRequestClosed => "closed",
        _ => "open",
    };
    let pull_requests = match kind {
        Kind::SessionStarted => json!([]),
        _ => json!([serde_json::to_value(sample_pr(repo, Some(pr_state))).unwrap_or(Value::Null)]),
    };
    let question = (kind == Kind::SessionQuestion).then(|| {
        json!({
            "text": "Should uploads that fail five times go to the dead-letter queue, or be dropped with a log line?",
            "options": ["Dead-letter queue", "Drop them"],
            "asked_at": now
        })
    });
    let value = json!({
        "id": "ses_01KSAMPLESESSION0000000000",
        "project_id": project.id,
        "project_slug": project.slug,
        "number": 12,
        "title": "Retry failed uploads in the sync worker",
        "description": "Uploads that fail are tried again with backoff, and the worker gives up on a server that stays down.",
        "status": status,
        "status_reason": reason,
        "created_by": { "id": "usr_01KSAMPLEPERSON00000000000", "name": "Ada Lovelace" },
        "created_at": now - 40 * MINUTE,
        "updated_at": now,
        "started_at": now - 39 * MINUTE,
        "ended_at": matches!(kind, Kind::SessionStopped | Kind::SessionFailed).then_some(now),
        "branch": "fctl/12-retry-failed-uploads",
        "base_branch": "main",
        "pull_requests": pull_requests,
        "question": question,
        "model": "opus",
        "effort": "high",
        "stats": { "turns": 3 },
        "archived": false
    });
    serde_json::from_value(value).unwrap_or_else(|_| Session {
        id: SessionId("ses_01KSAMPLESESSION0000000000".into()),
        project_id: fctl_proto::ProjectId(project.id.clone()),
        project_slug: project.slug.clone(),
        workspace: None,
        number: 12,
        title: "Retry failed uploads in the sync worker".into(),
        description: None,
        status: SessionStatus::Idle,
        status_reason: None,
        resume_at: None,
        created_by: fctl_proto::session::UserRef {
            id: fctl_proto::UserId("usr_01KSAMPLEPERSON00000000000".into()),
            name: "Ada Lovelace".into(),
            avatar_url: None,
        },
        created_at: now,
        updated_at: now,
        started_at: None,
        ended_at: None,
        last_activity_at: None,
        messaged_at: None,
        requested_at: None,
        branch: None,
        base_branch: None,
        pull_request: None,
        pull_requests: Vec::new(),
        runner: None,
        model: None,
        effort: None,
        github_access: None,
        github_issues: None,
        remote_control: None,
        stats: Default::default(),
        role: None,
        archived: false,
        following: None,
        review: None,
        schedule: None,
        ticket: None,
        question: None,
        autopilot: None,
        claude: None,
        claude_connection: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::api::ReviewState;
    use fctl_proto::session::{PullRequestChecks, ReviewCheck};

    const BASE: &str = "https://app.fctl.test";

    fn place() -> HookPlace {
        HookPlace {
            id: "prj_web".into(),
            slug: "web".into(),
            name: "Web".into(),
        }
    }

    fn sess(kind: Kind) -> Session {
        sample_session(kind, &place(), "acme/web", 1_800_000_000_000)
    }

    fn strings(kinds: &[Kind]) -> Vec<String> {
        kinds.iter().map(|k| k.as_str().to_owned()).collect()
    }

    #[test]
    fn every_event_has_a_name_a_group_and_words() {
        let mut names: Vec<&str> = Kind::ALL.iter().map(|k| k.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Kind::ALL.len());
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
            assert!(
                k.as_str()
                    .starts_with(&format!("{}.", k.group().id().trim_end_matches('s')))
                    || k.group() == Group::PullRequests && k.as_str().starts_with("pull_request.")
            );
            assert!(
                k.label().chars().next().is_some_and(char::is_uppercase)
                    || k.label().starts_with("fctl ")
            );
            assert!(k.description().ends_with('.'));
        }
        assert_eq!(Kind::parse("session.nope"), None);
        let c = catalog();
        assert_eq!(c.events.len(), Kind::ALL.len());
        assert_eq!(c.groups.len(), 4);
        assert!(
            c.events
                .iter()
                .all(|e| c.groups.iter().any(|g| g.id == e.group))
        );
        assert_eq!(c.events[0].kind, "session.started");
        assert_eq!(c.events[0].label, "A session starts");
        assert_eq!(Kind::SchedulePaused.label(), "fctl pauses a schedule");
    }

    #[test]
    fn a_webhook_gets_the_events_projects_and_sessions_it_chose() {
        let all = Hook {
            enabled: true,
            ..Default::default()
        };
        assert!(wants(&all, Kind::ReviewQueued, Some("prj_a"), None));
        assert!(wants(
            &all,
            Kind::SessionStarted,
            Some("prj_a"),
            Some(WebhookOrigin::Schedule)
        ));
        let off = Hook {
            enabled: false,
            ..all.clone()
        };
        assert!(!wants(
            &off,
            Kind::SessionStarted,
            Some("prj_a"),
            Some(WebhookOrigin::Person)
        ));

        let some = Hook {
            events: strings(&[Kind::SessionQuestion, Kind::ReviewFinished]),
            ..all.clone()
        };
        assert!(wants(
            &some,
            Kind::SessionQuestion,
            Some("prj_a"),
            Some(WebhookOrigin::Person)
        ));
        assert!(!wants(
            &some,
            Kind::SessionStarted,
            Some("prj_a"),
            Some(WebhookOrigin::Person)
        ));

        let own = Hook {
            project_id: Some("prj_a".into()),
            ..all.clone()
        };
        assert!(wants(
            &own,
            Kind::SessionFailed,
            Some("prj_a"),
            Some(WebhookOrigin::Person)
        ));
        assert!(!wants(
            &own,
            Kind::SessionFailed,
            Some("prj_b"),
            Some(WebhookOrigin::Person)
        ));
        assert!(!wants(
            &own,
            Kind::SessionFailed,
            None,
            Some(WebhookOrigin::Person)
        ));

        let projects = Hook {
            projects: vec!["prj_a".into(), "prj_c".into()],
            ..all.clone()
        };
        assert!(wants(&projects, Kind::ReviewFailed, Some("prj_c"), None));
        assert!(!wants(&projects, Kind::ReviewFailed, Some("prj_b"), None));

        // Whom the sessions are of narrows sessions' and their pull
        // requests' events, and nothing else.
        let scheduled = Hook {
            started_by: vec![WebhookOrigin::Schedule],
            ..all
        };
        assert!(wants(
            &scheduled,
            Kind::SessionStopped,
            Some("prj_a"),
            Some(WebhookOrigin::Schedule)
        ));
        assert!(!wants(
            &scheduled,
            Kind::SessionStopped,
            Some("prj_a"),
            Some(WebhookOrigin::Person)
        ));
        assert!(!wants(
            &scheduled,
            Kind::PullRequestMerged,
            Some("prj_a"),
            Some(WebhookOrigin::Ticket)
        ));
        assert!(wants(&scheduled, Kind::ReviewFinished, Some("prj_a"), None));
        assert!(wants(&scheduled, Kind::SchedulePaused, Some("prj_a"), None));
    }

    #[test]
    fn sessions_start_from_people_schedules_autopilots_and_tickets() {
        let mut s = sess(Kind::SessionStarted);
        assert_eq!(origin(&s), Some(WebhookOrigin::Person));
        s.ticket = serde_json::from_value(json!({
            "tracker": "linear", "id": "x", "key": "ENG-1", "url": "https://linear.app/x"
        }))
        .ok();
        assert_eq!(origin(&s), Some(WebhookOrigin::Ticket));
        s.schedule = serde_json::from_value(json!({ "id": "sch_1", "name": "Nightly" })).ok();
        assert_eq!(origin(&s), Some(WebhookOrigin::Schedule));
        s.autopilot = serde_json::from_value(json!({
            "role": "crew", "project_id": "prj_web", "project_slug": "web"
        }))
        .ok();
        assert_eq!(origin(&s), Some(WebhookOrigin::Autopilot));
        s.review = serde_json::from_value(json!({
            "repo": { "owner": "acme", "name": "web" }, "number": 45,
            "url": "https://github.com/acme/web/pull/45", "head_sha": "abc"
        }))
        .ok();
        assert_eq!(origin(&s), None);
    }

    #[test]
    fn names_events_and_whom_are_checked() {
        assert_eq!(
            check_name(Some("  Deploy bot ")).ok().flatten().as_deref(),
            Some("Deploy bot")
        );
        assert_eq!(check_name(Some("   ")).ok(), Some(None));
        assert_eq!(check_name(None).ok(), Some(None));
        assert!(check_name(Some(&"x".repeat(NAME_MAX + 1))).is_err());
        assert!(check_name(Some("a\u{7}b")).is_err());

        let events = check_events(&[
            "review.finished".into(),
            " session.started ".into(),
            "review.finished".into(),
        ]);
        assert_eq!(
            events.ok(),
            Some(vec![
                "session.started".to_owned(),
                "review.finished".to_owned()
            ])
        );
        let unknown = check_events(&["session.exploded".into()]);
        assert!(
            unknown
                .err()
                .is_some_and(|e| e.message.contains("session.exploded"))
        );
        assert_eq!(check_events(&[]).ok(), Some(vec![]));

        assert_eq!(
            check_started_by(&[
                WebhookOrigin::Ticket,
                WebhookOrigin::Person,
                WebhookOrigin::Ticket
            ])
            .ok(),
            Some(vec![WebhookOrigin::Person, WebhookOrigin::Ticket])
        );
        assert!(check_started_by(&[WebhookOrigin::Other]).is_err());
    }

    #[test]
    fn webhooks_post_to_public_https_addresses_only() {
        let public = Reach::default();
        let ok = |u: &str| check_url(u, &public).ok();
        let refused = |u: &str| check_url(u, &public).err().map(|e| e.message);
        assert_eq!(
            ok("https://hooks.slack.com/services/T0/B0/x"),
            Some("https://hooks.slack.com/services/T0/B0/x".into())
        );
        assert_eq!(
            ok(" https://example.com/fctl?token=abc#frag "),
            Some("https://example.com/fctl?token=abc".into())
        );
        assert_eq!(
            ok("https://93.184.216.34/hook"),
            Some("https://93.184.216.34/hook".into())
        );
        assert!(ok("https://[2606:4700:4700::1111]/hook").is_some());
        assert!(ok("https://example.com:8443/hook").is_some());
        assert!(refused("http://example.com/hook").is_some_and(|m| m.contains("https://")));
        assert!(refused("ftp://example.com/").is_some());
        assert!(refused("example.com/hook").is_some());
        assert!(refused("").is_some());
        assert!(refused("https://user:pass@example.com/").is_some_and(|m| m.contains("password")));
        for local in [
            "https://localhost/",
            "https://LOCALHOST./",
            "https://api.localhost/",
            "https://printer.local/",
            "https://metadata.google.internal/",
            "https://jenkins/",
            "https://router.lan/",
            "https://127.0.0.1/",
            "https://2130706433/",
            "https://0x7f.0.0.1/",
            "https://10.0.0.8/",
            "https://172.16.4.2/",
            "https://192.168.1.10/",
            "https://169.254.169.254/latest/meta-data/",
            "https://100.64.0.1/",
            "https://0.0.0.0/",
            "https://224.0.0.1/",
            "https://255.255.255.255/",
            "https://[::1]/",
            "https://[::]/",
            "https://[fc00::1]/",
            "https://[fd12:3456::1]/",
            "https://[fe80::1]/",
            "https://[::ffff:127.0.0.1]/",
            "https://[::ffff:10.0.0.1]/",
            "https://[64:ff9b::a00:1]/",
        ] {
            assert!(
                refused(local).is_some_and(|m| m.contains("public")),
                "{local}"
            );
        }
        // A development instance tries receivers on the same machine.
        let dev = Reach::anything();
        assert_eq!(
            check_url("http://127.0.0.1:9099/hook", &dev).ok(),
            Some("http://127.0.0.1:9099/hook".into())
        );
        assert!(check_url("http://localhost:9099/", &dev).is_ok());
        assert!(check_url("https://user:pw@localhost/", &dev).is_err());
        assert!(
            check_url(
                &format!("https://example.com/{}", "a".repeat(URL_MAX)),
                &public
            )
            .is_err()
        );
    }

    #[test]
    fn a_self_hosted_instance_lets_webhooks_reach_the_network_its_settings_list() {
        let (reach, unread) = Reach::parse(
            Some("1"),
            Some(
                "10.20.0.0/16, 192.168.1.20 [fd00::]/8,hooks.lan  *.corp.internal JENKINS. 10.0.0.0/33 not_a_host *.-x",
            ),
        );
        assert_eq!(unread, ["10.0.0.0/33", "not_a_host", "*.-x"]);
        assert!(reach.http);
        assert_eq!(
            reach.listed(),
            [
                "10.20.0.0/16",
                "192.168.1.20",
                "fd00::/8",
                "hooks.lan",
                "*.corp.internal",
                "jenkins"
            ]
        );
        let ok = |u: &str| check_url(u, &reach).is_ok();
        for allowed in [
            "http://10.20.3.4:8080/hook",
            "https://192.168.1.20/hook",
            "http://[fd00::1]/hook",
            "http://[::ffff:10.20.0.9]/hook",
            "http://hooks.lan/fctl",
            "https://ci.corp.internal/hook",
            "http://a.b.corp.internal./hook",
            "http://jenkins:8080/fctl",
            "http://example.com/hook",
        ] {
            assert!(ok(allowed), "{allowed}");
        }
        for refused in [
            "http://10.21.0.1/hook",
            "https://192.168.1.21/hook",
            "http://127.0.0.1/hook",
            "http://169.254.169.254/latest/meta-data/",
            "http://corp.internal/hook",
            "http://evilcorp.internal/hook",
            "http://printer.local/hook",
            "http://[fe80::1]/hook",
        ] {
            let said = check_url(refused, &reach).err().map(|e| e.message);
            assert!(
                said.as_deref()
                    .is_some_and(|m| m.contains("10.20.0.0/16, 192.168.1.20")),
                "{refused}: {said:?}"
            );
        }
        // Without http, private addresses still go over https only.
        let (https_only, _) = Reach::parse(None, Some("10.0.0.0/8"));
        assert!(check_url("https://10.1.2.3/hook", &https_only).is_ok());
        assert!(
            check_url("http://10.1.2.3/hook", &https_only)
                .err()
                .is_some_and(|e| e.message.contains("https://"))
        );
        // Settings that list nothing take nothing more, whatever FCTL_WEBHOOK_HTTP says
        // besides 1 or true.
        assert_eq!(
            Reach::parse(Some("yes"), Some(" , ")),
            (Reach::default(), vec![])
        );
        // A /0 range takes every address of its family; one address, itself.
        let (all_v4, _) = Reach::parse(None, Some("0.0.0.0/0"));
        assert!(
            all_v4.takes_ip(
                "127.0.0.1"
                    .parse()
                    .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
            )
        );
        assert!(!all_v4.takes_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert_eq!(
            Private::parse("192.168.1.20/32")
                .map(|p| p.to_string())
                .as_deref(),
            Some("192.168.1.20")
        );
    }

    #[test]
    fn those_who_dont_manage_a_webhook_see_where_it_goes_not_its_credential() {
        assert_eq!(
            shortened("https://hooks.slack.com/services/T0/B0/secret?x=1"),
            "https://hooks.slack.com/…"
        );
        assert_eq!(
            shortened("https://example.com:8443/hook"),
            "https://example.com:8443/…"
        );
        assert_eq!(shortened("nonsense"), "…");
        assert_eq!(
            host_of(&shortened("https://hooks.slack.com/services/x")),
            "hooks.slack.com"
        );
    }

    #[test]
    fn a_timestamp_at_the_ends_of_i64_is_too_old_not_an_overflow() {
        let secret = secret(&[7u8; 32]);
        let k = key(&secret).unwrap_or_default();
        let header = sign(&k, "evt_1", 1, b"{}");
        assert_eq!(
            verify(
                &secret,
                "evt_1",
                &i64::MIN.to_string(),
                b"{}",
                &header,
                1_614_265_330
            ),
            Err(Unverified::TooOld)
        );
        assert_eq!(
            verify(&secret, "evt_1", &i64::MAX.to_string(), b"{}", &header, -1),
            Err(Unverified::TooOld)
        );
    }

    #[test]
    fn a_webhook_goes_by_its_host() {
        assert_eq!(
            host_of("https://hooks.slack.com/services/x"),
            "hooks.slack.com"
        );
        assert_eq!(host_of("not a url"), "not a url");
    }

    #[test]
    fn posts_are_signed_as_standard_webhooks_sign_them() {
        // Standard Webhooks' own example.
        let secret = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
        let id = "msg_p5jXN8AQM9LWM0D4loKWxJek";
        let body = br#"{"test": 2432232314}"#;
        let k = key(secret).unwrap_or_default();
        assert_eq!(
            sign(&k, id, 1_614_265_330, body),
            "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE="
        );
        let header = sign(&k, id, 1_614_265_330, body);
        assert_eq!(
            verify(secret, id, "1614265330", body, &header, 1_614_265_330),
            Ok(())
        );
        assert_eq!(
            verify(
                secret,
                id,
                "1614265330",
                br#"{"test": 1}"#,
                &header,
                1_614_265_330
            ),
            Err(Unverified::Signature)
        );
        assert_eq!(
            verify(
                secret,
                id,
                "1614265330",
                body,
                &header,
                1_614_265_330 + TOLERANCE_S + 1
            ),
            Err(Unverified::TooOld)
        );
        assert_eq!(
            verify(secret, id, "soon", body, &header, 0),
            Err(Unverified::Timestamp)
        );
        assert_eq!(
            verify("nope", id, "1", body, &header, 1),
            Err(Unverified::Secret)
        );
    }

    #[test]
    fn a_rotated_secret_and_its_predecessor_both_sign() {
        let new = secret(&[7u8; 32]);
        let old = secret(&[9u8; 32]);
        assert!(new.starts_with("whsec_"));
        assert_eq!(key(&new).map(|k| k.len()), Some(32));
        let keys = vec![key(&new).unwrap_or_default(), key(&old).unwrap_or_default()];
        let header = signatures(&keys, "evt_1", 100, b"{}");
        assert_eq!(header.split(' ').count(), 2);
        assert!(header.split(' ').all(|s| s.starts_with("v1,")));
        assert_eq!(verify(&new, "evt_1", "100", b"{}", &header, 100), Ok(()));
        assert_eq!(verify(&old, "evt_1", "100", b"{}", &header, 100), Ok(()));
        assert_eq!(
            verify(&secret(&[1u8; 32]), "evt_1", "100", b"{}", &header, 100),
            Err(Unverified::Signature)
        );
    }

    #[test]
    fn headers_name_the_event_the_delivery_and_where_it_happened() {
        let h = Headers {
            event_id: "evt_1",
            kind: "session.finished",
            delivery_id: Some("whd_1"),
            webhook_id: "whk_1",
            workspace_id: "wks_1",
            project_id: Some("prj_1"),
            timestamp: 1_700_000_000,
            signature: "v1,abc",
            attempt: 2,
            version: "0.36.0",
            test: true,
        }
        .to_vec();
        let get = |n: &str| h.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
        assert_eq!(get("webhook-id"), Some("evt_1"));
        assert_eq!(get("webhook-timestamp"), Some("1700000000"));
        assert_eq!(get("webhook-signature"), Some("v1,abc"));
        assert_eq!(get("fctl-event"), Some("session.finished"));
        assert_eq!(get("fctl-workspace-id"), Some("wks_1"));
        assert_eq!(get("fctl-project-id"), Some("prj_1"));
        assert_eq!(get("fctl-attempt"), Some("2"));
        assert_eq!(get("fctl-test"), Some("1"));
        assert!(get("user-agent").is_some_and(|ua| ua.starts_with("fctl-webhooks/0.36.0")));
        assert!(hidden_header("Webhook-Signature"));
        assert!(!hidden_header("webhook-id"));
    }

    #[test]
    fn failed_deliveries_go_again_seven_times_over_a_day() {
        let now = 1_000_000;
        assert_eq!(retry_at(1, now, None), Some(now + 10_000));
        assert_eq!(retry_at(2, now, None), Some(now + MINUTE));
        assert_eq!(retry_at(7, now, None), Some(now + 12 * HOUR));
        assert_eq!(retry_at(8, now, None), None);
        assert_eq!(retry_at(0, now, None), None);
        let total: i64 = RETRIES_MS.iter().sum();
        assert!(total > 20 * HOUR && total < 22 * HOUR);
        // A receiver that asks for later gets it, up to 12 hours; sooner
        // than fctl's own wait changes nothing.
        assert_eq!(retry_at(1, now, Some(120_000)), Some(now + 120_000));
        assert_eq!(retry_at(1, now, Some(2_000)), Some(now + 10_000));
        assert_eq!(retry_at(1, now, Some(48 * HOUR)), Some(now + 12 * HOUR));
        assert_eq!(retry_after(" 120 "), Some(120_000));
        assert_eq!(retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
        assert_eq!(retry_after("-5"), None);
        assert!(delivered(200) && delivered(204) && !delivered(301) && !delivered(500));
        assert!(status_error(302).contains("redirect"));
        assert_eq!(status_error(502), "HTTP 502");
        assert!(timeout_error().contains("10 seconds"));
    }

    #[test]
    fn an_attempt_settles_a_delivery_by_what_stands_as_it_ends() {
        use WebhookDeliveryState::{Delivered, Failed, Pending};
        let now = 1_000_000;
        // fctl posts only one that waits and is due; a person, any.
        assert!(posts(By::Fctl, Pending, Some(now), now));
        assert!(!posts(By::Fctl, Pending, Some(now + 1), now));
        assert!(!posts(By::Fctl, Delivered, None, now));
        assert!(!posts(By::Fctl, Failed, None, now));
        assert!(posts(By::Resend, Delivered, None, now));
        assert!(posts(By::Test, Pending, None, now));

        // Five resends of one that waits take none of fctl's retries: its
        // second own attempt goes again in a minute, not as its seventh.
        let by = |person: bool| WebhookAttempt {
            at: 0,
            ms: 1,
            status: Some(500),
            error: None,
            by: person.then(|| fctl_proto::session::UserRef {
                id: fctl_proto::UserId("usr_ada".into()),
                name: "Ada".into(),
                avatar_url: None,
            }),
            response: None,
        };
        let history = [
            by(false),
            by(true),
            by(true),
            by(true),
            by(true),
            by(true),
            by(false),
        ];
        assert_eq!(own_attempts(&history), 2);
        let again = settle(By::Fctl, Pending, Some(now), false, 2, now, None);
        assert_eq!((again.state, again.next_at), (Pending, Some(now + MINUTE)));
        assert_eq!(again.done_at, None);
        assert_eq!(
            settle(By::Fctl, Pending, Some(now), false, 8, now, None),
            Settled {
                state: Failed,
                next_at: None,
                done_at: Some(now),
                kept: false,
            }
        );

        // One a resend delivered, or turning its webhook off failed, while
        // fctl's attempt waited stays as it was left, and keeps why.
        for was in [Delivered, Failed] {
            assert_eq!(
                settle(By::Fctl, was, None, false, 3, now, None),
                Settled {
                    state: was,
                    next_at: None,
                    done_at: None,
                    kept: true,
                }
            );
        }
        // An attempt that went through delivers it, whoever made it.
        assert_eq!(
            settle(By::Fctl, Failed, None, true, 3, now, None).state,
            Delivered
        );

        // A person's resend of one that waits leaves its retries as they
        // were; of one that was done, it fails; so does a test.
        let resent = settle(By::Resend, Pending, Some(now + HOUR), false, 2, now, None);
        assert_eq!(
            (resent.state, resent.next_at, resent.done_at),
            (Pending, Some(now + HOUR), None)
        );
        assert_eq!(
            settle(By::Resend, Delivered, None, false, 1, now, None).state,
            Failed
        );
        assert_eq!(
            settle(By::Test, Pending, None, false, 0, now, None).done_at,
            Some(now)
        );
    }

    #[test]
    fn an_address_answers_fctl_s_challenge_before_a_webhook_posts_there() {
        let c = challenge(&[7; 32]);
        assert_eq!(c.len(), 43);
        let body: Value = serde_json::from_str(&challenge_body("evt_1", &c, 0)).unwrap_or_default();
        assert_eq!(body["type"], "url_verification");
        assert_eq!(body["challenge"], c.as_str());
        assert_eq!(body["timestamp"], "1970-01-01T00:00:00.000Z");
        // Given back as Slack takes it: text, JSON or a form.
        for answer in [
            c.clone(),
            format!("  {c}\n"),
            format!(r#"{{"challenge": "{c}"}}"#),
            format!("challenge={c}"),
        ] {
            assert_eq!(
                challenge_refused(&c, Some(200), None, Some(&answer)),
                None,
                "{answer}"
            );
        }
        let refused = |status: Option<u16>, error: Option<&str>, body: Option<&str>| {
            challenge_refused(&c, status, error, body).unwrap_or_default()
        };
        assert!(
            refused(Some(200), None, Some("ok")).contains("with \"ok\" rather than the challenge")
        );
        assert!(refused(Some(204), None, None).contains("with nothing"));
        assert!(
            refused(Some(200), None, Some(r#"{"challenge": "another"}"#)).contains("rather than")
        );
        assert!(
            refused(Some(401), None, Some(&c))
                .starts_with("This address answered fctl's challenge with HTTP 401. It answers")
        );
        assert!(refused(Some(302), None, None).contains("a redirect, which fctl doesn't follow"));
        assert!(
            refused(None, Some("No answer within 10 seconds."), None)
                .starts_with("fctl posted a challenge to this address and got no answer: No answer within 10 seconds. It answers")
        );
    }

    #[test]
    fn a_webhook_that_failed_for_three_days_turns_itself_off() {
        assert!(!turns_off(None, 10 * DAY, true));
        assert!(!turns_off(Some(DAY), 3 * DAY, true));
        assert!(turns_off(Some(DAY), 4 * DAY, true));
        // A delivery's first attempt failing after days without one has its
        // retries to go: only one that fails for good turns it off.
        assert!(!turns_off(Some(DAY), 4 * DAY, false));
        assert_eq!(
            off_reason("HTTP 502"),
            "Every delivery failed for 3 days, the last with HTTP 502. Turn it on again once its address answers."
        );
        assert_eq!(room(0), PER_MINUTE);
        assert_eq!(room(PER_MINUTE + 3), 0);
        let mut due = vec![
            ("whk_b".to_owned(), 5),
            ("whk_a".to_owned(), 5),
            ("whk_c".to_owned(), 1),
        ];
        order_due(&mut due);
        assert_eq!(
            due.iter().map(|d| d.0.as_str()).collect::<Vec<_>>(),
            ["whk_c", "whk_a", "whk_b"]
        );
    }

    fn stood(status: SessionStatus, waiting: bool) -> Stood {
        Stood { status, waiting }
    }

    #[test]
    fn a_session_s_moves_send_its_events() {
        use SessionStatus::*;
        assert_eq!(
            moved(stood(Running, false), stood(Idle, true)),
            vec![Kind::SessionStatusChanged, Kind::SessionFinished]
        );
        // Background work ended: it finished now.
        assert_eq!(
            moved(stood(Idle, false), stood(Idle, true)),
            vec![Kind::SessionFinished]
        );
        // It waits for its background work: not finished yet.
        assert_eq!(
            moved(stood(Running, false), stood(Idle, false)),
            vec![Kind::SessionStatusChanged]
        );
        assert_eq!(moved(stood(Idle, true), stood(Idle, true)), vec![]);
        assert_eq!(
            moved(stood(Idle, true), stood(Stopped, false)),
            vec![Kind::SessionStatusChanged, Kind::SessionStopped]
        );
        assert_eq!(
            moved(stood(Running, false), stood(Failed, false)),
            vec![Kind::SessionStatusChanged, Kind::SessionFailed]
        );
        assert_eq!(
            moved(stood(Running, false), stood(Paused, false)),
            vec![Kind::SessionStatusChanged, Kind::SessionPaused]
        );
        assert_eq!(
            moved(stood(Queued, false), stood(Provisioning, false)),
            vec![Kind::SessionStatusChanged]
        );
        assert!(waits_for_message(Idle, Some("Waiting for a message")));
        assert!(waits_for_message(
            Idle,
            Some(crate::autopilot::CUT_OFF_TWICE)
        ));
        assert!(!waits_for_message(
            Idle,
            Some("Waiting for 2 background tasks")
        ));
        assert!(!waits_for_message(Running, None));
        let s = stood(Idle, true);
        assert_eq!(Stood::parse(&s.key()), Some(s));
        assert_eq!(Stood::parse("nope"), None);
    }

    fn pr(url: &str, state: Option<&str>) -> PullRequestRef {
        PullRequestRef {
            number: 1,
            url: url.into(),
            state: state.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn an_agent_s_pull_requests_open_merge_and_close() {
        let a = "https://github.com/acme/web/pull/1";
        let b = "https://github.com/acme/web/pull/2";
        let found = [pr(a, Some("open")), pr(b, Some("merged"))];
        let moves = pull_moves(&[], &found);
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].0, Kind::PullRequestOpened);
        let merged = [pr(a, Some("merged"))];
        assert_eq!(
            pull_moves(&[pr(a, Some("open"))], &merged)[0].0,
            Kind::PullRequestMerged
        );
        let closed = [pr(a, Some("closed"))];
        assert_eq!(
            pull_moves(&[pr(a, Some("draft"))], &closed)[0].0,
            Kind::PullRequestClosed
        );
        assert!(pull_moves(&[pr(a, Some("merged"))], &closed).is_empty());
        assert!(pull_moves(&[pr(a, Some("open"))], &[pr(a, Some("queued"))]).is_empty());
        assert_eq!(
            pull_moves(&[], &[pr(a, None)])[0].0,
            Kind::PullRequestOpened
        );
    }

    fn ready_session() -> Session {
        sess(Kind::SessionReadyForReview)
    }

    fn checks(
        failed: u32,
        pending: u32,
        sha: &str,
        review: Option<ReviewState>,
    ) -> PullRequestChecks {
        PullRequestChecks {
            passed: 3,
            failed,
            pending,
            sha: Some(sha.into()),
            at: 1,
            review: review.and_then(|state| {
                serde_json::from_value::<ReviewCheck>(json!({ "state": state.as_str() })).ok()
            }),
        }
    }

    #[test]
    fn pull_requests_are_ready_for_review_once_their_agent_finished_and_checks_passed() {
        let s = ready_session();
        let (prs, key) = ready_for_review(&s).unwrap_or_default();
        assert_eq!(prs.len(), 1);
        assert!(key.ends_with("@3f2a1c9e5b7d4a6c8e0f1a2b3c4d5e6f7a8b9c0d"));

        let with = |f: &dyn Fn(&mut Session)| {
            let mut s = ready_session();
            f(&mut s);
            ready_for_review(&s).map(|(_, k)| k)
        };
        assert!(with(&|s| s.pull_requests[0].checks = Some(checks(0, 1, "a", None))).is_none());
        assert!(with(&|s| s.pull_requests[0].checks = Some(checks(1, 0, "a", None))).is_none());
        assert!(with(&|s| s.pull_requests[0].checks = None).is_none());
        assert!(
            with(&|s| s.pull_requests[0].checks =
                Some(checks(0, 0, "a", Some(ReviewState::Running))))
            .is_none()
        );
        // A review that found issues is a person's to read: ready.
        assert!(
            with(&|s| s.pull_requests[0].checks =
                Some(checks(0, 0, "a", Some(ReviewState::Commented))))
            .is_some()
        );
        assert!(with(&|s| s.pull_requests[0].state = Some("draft".into())).is_none());
        assert!(with(&|s| s.pull_requests[0].state = Some("merged".into())).is_none());
        assert!(
            with(&|s| s.status_reason = Some("Waiting for a background task".into())).is_none()
        );
        assert!(with(&|s| s.status = SessionStatus::Running).is_none());
        assert!(with(&|s| s.archived = true).is_none());
        assert!(
            with(&|s| {
                s.question = serde_json::from_value(json!({ "text": "?", "asked_at": 1 })).ok()
            })
            .is_none()
        );
        // A draft beside the open one neither counts nor holds it back.
        let mut s = ready_session();
        s.pull_requests
            .push(pr("https://github.com/acme/web/pull/9", Some("draft")));
        assert_eq!(ready_for_review(&s).map(|(p, _)| p.len()), Some(1));
        // A new commit is a new key: said again once it's ready again.
        let a = with(&|s| s.pull_requests[0].checks = Some(checks(0, 0, "aaa", None)));
        let b = with(&|s| s.pull_requests[0].checks = Some(checks(0, 0, "bbb", None)));
        assert!(a.is_some() && b.is_some() && a != b);
    }

    #[test]
    fn a_session_posts_as_the_api_shows_it_with_its_links() {
        let mut s = sess(Kind::SessionFinished);
        s.schedule = serde_json::from_value(json!({ "id": "sch_1", "name": "Nightly" })).ok();
        let h = session(&s, "https://app.fctl.test/");
        assert_eq!(h.reference, "web#12");
        assert_eq!(
            h.url,
            "https://app.fctl.test/s/ses_01KSAMPLESESSION0000000000"
        );
        assert_eq!(h.started_by, WebhookOrigin::Schedule);
        assert_eq!(
            h.schedule.map(|sc| sc.url),
            Some("https://app.fctl.test/p/web?tab=schedules&schedule=sch_1".into())
        );
        assert_eq!(h.pull_requests[0].repo, "acme/web");
        assert_eq!(
            h.pull_requests[0]
                .checks
                .as_ref()
                .and_then(|c| c.review.clone()),
            Some("passed".into())
        );
        let json = serde_json::to_value(session(&s, BASE)).unwrap_or_default();
        assert_eq!(json["ref"], "web#12");
        assert_eq!(json["status"], "idle");
    }

    #[test]
    fn events_say_what_happened_in_a_line() {
        let s = sess(Kind::SessionFinished);
        let long = "word ".repeat(1000);
        let e = session_event(
            Kind::SessionFinished,
            &s,
            BASE,
            5,
            SessionExtra {
                message: Some(long),
                previous_status: Some(SessionStatus::Running),
                ..Default::default()
            },
        );
        assert_eq!(
            e.gist,
            "web#12 finished a turn: Retry failed uploads in the sync worker"
        );
        assert_eq!(e.project_id.as_deref(), Some("prj_web"));
        assert_eq!(e.origin, Some(WebhookOrigin::Person));
        let message = e.data["message"].as_str().unwrap_or_default();
        assert_eq!(message.chars().count(), MESSAGE_MAX);
        assert!(message.ends_with('…'));
        // Only a status change carries the status before.
        assert!(e.data.get("previous_status").is_none());
        assert!(e.data.get("follow_up").is_none());

        let q = session_event(
            Kind::SessionQuestion,
            &sess(Kind::SessionQuestion),
            BASE,
            5,
            SessionExtra::default(),
        );
        assert!(q.gist.starts_with("web#12 asks: Should uploads that fail"));
        assert!(q.gist.chars().count() <= GIST_MAX);

        let r = session_event(
            Kind::SessionReadyForReview,
            &ready_session(),
            BASE,
            5,
            SessionExtra::default(),
        );
        assert_eq!(r.gist, "web#12 is ready for review: web#45");
        assert_eq!(r.data["ready"][0]["number"], 45);

        let p = pull_request_event(Kind::PullRequestMerged, &s, &s.pull_requests[0], BASE, 5);
        assert_eq!(
            p.gist,
            "acme/web#45 merged: Retry failed uploads in the sync worker"
        );
        assert_eq!(
            p.data["pull_request"]["url"],
            "https://github.com/acme/web/pull/45"
        );
    }

    #[test]
    fn every_event_has_a_sample_of_its_own_shape() {
        for kind in Kind::ALL {
            let e = sample(kind, &place(), "acme/web", BASE, 1_800_000_000_000);
            assert_eq!(e.kind, kind);
            assert_eq!(e.project_id.as_deref(), Some("prj_web"), "{kind:?}");
            assert!(
                !e.gist.is_empty() && !e.gist.ends_with(':'),
                "{kind:?}: {}",
                e.gist
            );
            assert!(e.data.is_object(), "{kind:?}");
            match kind.group() {
                Group::Sessions => assert!(e.data["session"]["ref"] == "web#12", "{kind:?}"),
                Group::PullRequests => assert!(e.data["pull_request"]["number"] == 45),
                Group::Reviews => assert!(e.data["review"]["pull_request"]["number"] == 45),
                Group::Schedules => assert!(e.data["schedule"]["name"] == "Update dependencies"),
            }
        }
        let ready = sample(Kind::SessionReadyForReview, &place(), "acme/web", BASE, 1);
        assert_eq!(ready.data["ready"].as_array().map(Vec::len), Some(1));
        let follow = sample(Kind::SessionFollowUp, &place(), "acme/web", BASE, 1);
        assert_eq!(
            follow.data["follow_up"]["events"][0]["kind"],
            "checks_failed"
        );
        assert_eq!(
            follow.gist,
            "web#12 was brought back: checks failed on web#45"
        );
        let review = sample(Kind::ReviewFinished, &place(), "acme/web", BASE, 1);
        assert_eq!(
            review.gist,
            "Review of acme/web#45 at 3f2a1c9 found 1 new issue, 1 still open"
        );
        let skipped = sample(Kind::ScheduleRunSkipped, &place(), "acme/web", BASE, 1);
        assert_eq!(
            skipped.gist,
            "Update dependencies started nothing: Its last run, session #12, was still working on it."
        );
    }

    #[test]
    fn an_envelope_carries_where_it_happened() {
        let e = envelope(
            &WebhookEventId("evt_1".into()),
            "review.started",
            0,
            HookPlace {
                id: "wks_1".into(),
                slug: "acme".into(),
                name: "Acme".into(),
            },
            Some(place()),
            json!({ "x": 1 }),
            false,
        );
        let v = serde_json::to_value(&e).unwrap_or_default();
        assert_eq!(v["timestamp"], "1970-01-01T00:00:00.000Z");
        assert_eq!(v["workspace"]["id"], "wks_1");
        assert_eq!(v["project"]["slug"], "web");
        assert_eq!(v["type"], "review.started");
    }

    #[test]
    fn a_webhook_is_described_in_a_sentence() {
        let url = "https://hooks.slack.com/services/x";
        assert_eq!(
            describe(url, &[], &[], &[]),
            "Posts every event to hooks.slack.com."
        );
        assert_eq!(
            describe(
                url,
                &strings(&[Kind::SessionQuestion, Kind::SessionReadyForReview]),
                &["Web".into(), "Docs".into()],
                &[WebhookOrigin::Person],
            ),
            "Posts to hooks.slack.com when pull requests are ready for review or an agent asks something, in Web and Docs, for sessions started by people."
        );
        assert_eq!(
            describe(
                url,
                &strings(&[Kind::ReviewFinished]),
                &[],
                &[WebhookOrigin::Schedule]
            ),
            "Posts to hooks.slack.com when a review is posted."
        );
        assert_eq!(
            describe(
                url,
                &strings(&[
                    Kind::SessionStopped,
                    Kind::SessionFailed,
                    Kind::SessionStarted,
                    Kind::ReviewFailed
                ]),
                &["A".into(), "B".into(), "C".into(), "D".into()],
                &[WebhookOrigin::Schedule, WebhookOrigin::Autopilot],
            ),
            "Posts 4 kinds of events to hooks.slack.com, in 4 projects, for sessions started by schedules or an autopilot."
        );
        assert_eq!(
            describe(
                url,
                &[],
                &["Web".into()],
                &[WebhookOrigin::Ticket, WebhookOrigin::Person]
            ),
            "Posts every event to hooks.slack.com, in Web, for sessions started by tickets or people."
        );
    }

    #[test]
    fn follow_ups_say_what_brought_the_agent_back() {
        let f: FollowUp = serde_json::from_value(json!({
            "repo": { "owner": "acme", "name": "web" },
            "number": 45,
            "url": "https://github.com/acme/web/pull/45",
            "events": [{ "kind": "checks_failed", "sha": "abc", "names": ["test"], "passed": 1, "failed": 1, "pending": 0 }]
        }))
        .unwrap_or_else(|_| FollowUp {
            repo: fctl_proto::session::RepoRef { owner: "x".into(), name: "y".into() },
            number: 0,
            url: String::new(),
            title: None,
            state: None,
            base: None,
            events: Vec::new(),
        });
        let (data, what) = follow_up(&f);
        assert_eq!(what, "checks failed on web#45");
        assert_eq!(data["kind"], "github");
        assert_eq!(data["pull_request"]["repo"], "acme/web");
        assert_eq!(data["events"][0]["kind"], "checks_failed");
        let mut c = f.clone();
        c.events = serde_json::from_value(json!([
            { "kind": "comment", "id": 1, "login": "ada", "url": "u", "body": "b" }
        ]))
        .unwrap_or_default();
        assert_eq!(follow_up(&c).1, "a comment by @ada on web#45");
        c.events =
            serde_json::from_value(json!([{ "kind": "merged", "by": "ada" }])).unwrap_or_default();
        assert_eq!(follow_up(&c).1, "web#45 merged");
        c.events =
            serde_json::from_value(json!([{ "kind": "conflicts", "sha": "a", "base": "main" }]))
                .unwrap_or_default();
        assert_eq!(follow_up(&c).1, "web#45 conflicts with main");

        let r: Option<PassedOnReview> = serde_json::from_value(json!({
            "repo": { "owner": "acme", "name": "web" },
            "number": 45,
            "url": "https://github.com/acme/web/pull/45",
            "sha": "3f2a1c9",
            "session": "ses_r",
            "findings": [
                { "severity": "high", "title": "Retries never stop", "path": "src/sync/upload.rs", "line": 12 },
                { "severity": "low", "title": "Typo", "path": "README.md", "line": 3 }
            ]
        }))
        .ok();
        let (data, what) = r.as_ref().map(findings).unwrap_or_default();
        assert_eq!(what, "a review's 2 findings on web#45");
        assert_eq!(data["review"]["findings"][0]["title"], "Retries never stop");
        assert_eq!(data["kind"], "review");
    }

    #[test]
    fn long_text_is_cut_where_it_ends_with_an_ellipsis() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("a b c d e f", 6), "a b c…");
        assert_eq!(clip("  padded  ", 10), "padded");
    }
}
