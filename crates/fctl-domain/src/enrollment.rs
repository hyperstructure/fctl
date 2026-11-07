//! Enrollment tokens (docs/architecture.md#enrollment-tokens): machines
//! enroll themselves as runners of a workspace with one, without anyone
//! signing in on them, as autoscaling creates them: the workspace's own,
//! for everyone's sessions, or, with a token a member made for their own
//! sessions, that member's. What a token may be made with, whether it
//! enrolls another runner now, what it says it does, and when the runners
//! it enrolled are removed.

use crate::validate::Result;
use fctl_proto::api::{CreateEnrollmentTokenRequest, PushMessage, RunnerFacts};
use fctl_proto::{ApiError, Millis};

pub const MINUTE_MS: i64 = 60_000;
pub const HOUR_MS: i64 = 60 * MINUTE_MS;
/// How long a runner waits for approval (by an admin of its workspace, or
/// its owner) before it is removed.
pub const PENDING_MS: i64 = 7 * 24 * HOUR_MS;
/// The longest an enrollment token lasts, when it expires at all.
pub const MAX_EXPIRY_HOURS: u32 = 365 * 24;
/// The most runners one token may keep enrolled at once.
pub const MAX_RUNNERS: u32 = 1000;
/// The longest a token may let its runners stay offline before they are
/// removed, in minutes.
pub const MAX_REMOVE_OFFLINE_MINUTES: u32 = 30 * 24 * 60;
/// Longer than this between two of FleetObject's ticks (every 30 seconds
/// while it watches a runner that may go), it wasn't watching: it was down,
/// or had nothing to watch. Time a runner was offline counts only from the
/// tick after such a gap ([`watching_since`]). Just over a tick, so that
/// any outage of the control plane longer than that starts every window
/// over, and a host, whose reconnect backoff grows to half a minute (with
/// jitter, 34 seconds), is back within a window of a minute: a tick that is
/// late only delays a removal.
pub const MAX_TICK_GAP_MS: i64 = 40_000;
/// Revoked and expired tokens are deleted this long after they ended, once
/// no runner they enrolled is left.
pub const KEEP_ENDED_MS: i64 = 90 * 24 * HOUR_MS;

/// A new enrollment token's settings, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewToken {
    pub name: String,
    /// For its maker's own sessions (`fctl_domain::workspace::enrollment_for_own`
    /// decides it, by their role).
    pub personal: bool,
    pub expires_at: Option<i64>,
    pub max_runners: Option<u32>,
    pub approval: bool,
    pub remove_offline_after_minutes: Option<u32>,
}

/// Checks what a new enrollment token is to be made with, at `now`, for its
/// maker's own sessions or not (`personal`).
pub fn check_new(req: &CreateEnrollmentTokenRequest, personal: bool, now: i64) -> Result<NewToken> {
    let name = crate::validate::name("name", &req.name)?;
    let expires_at = match req.expires_in_hours {
        None => None,
        Some(h) if (1..=MAX_EXPIRY_HOURS).contains(&h) => Some(now + i64::from(h) * HOUR_MS),
        Some(_) => {
            return Err(ApiError::validation(
                "expires_in_hours",
                "Choose an expiry between an hour and a year, or none",
            ));
        }
    };
    let max_runners = match req.max_runners {
        None => None,
        Some(n) if (1..=MAX_RUNNERS).contains(&n) => Some(n),
        Some(_) => {
            return Err(ApiError::validation(
                "max_runners",
                format!("Allow between 1 and {MAX_RUNNERS} runners at once, or leave it empty"),
            ));
        }
    };
    let remove_offline_after_minutes = match req.remove_offline_after_minutes {
        None => None,
        Some(m) if (1..=MAX_REMOVE_OFFLINE_MINUTES).contains(&m) => Some(m),
        Some(_) => {
            return Err(ApiError::validation(
                "remove_offline_after_minutes",
                "Remove offline runners after a minute to 30 days, or keep them",
            ));
        }
    };
    Ok(NewToken {
        name,
        personal,
        expires_at,
        max_runners,
        approval: req.approval.unwrap_or(false),
        remove_offline_after_minutes,
    })
}

/// An enrollment token as it is now, for enrolling another runner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub revoked_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub max_runners: Option<u32>,
    /// The runners it enrolled that are still there, waiting ones too.
    pub runners: u32,
}

/// Why an enrollment token enrolls no runner now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Revoked {
        at: i64,
    },
    Expired {
        at: i64,
    },
    /// As many runners as it may keep are enrolled with it.
    Full {
        max: u32,
    },
}

/// Whether `token` enrolls another runner at `now`.
pub fn check(token: &State, now: i64) -> std::result::Result<(), Refusal> {
    if let Some(at) = token.revoked_at {
        return Err(Refusal::Revoked { at });
    }
    if let Some(at) = token.expires_at.filter(|at| *at <= now) {
        return Err(Refusal::Expired { at });
    }
    if let Some(max) = token.max_runners.filter(|max| token.runners >= *max) {
        return Err(Refusal::Full { max });
    }
    Ok(())
}

impl Refusal {
    /// What the host that tries to enroll is told, naming the token's
    /// workspace. `personal`: the token is for its maker's own sessions,
    /// and they make another themselves.
    pub fn message(&self, workspace: &str, personal: bool) -> String {
        let another = if personal {
            format!("make another on {workspace}'s Runners tab")
        } else {
            "ask one of its admins for another".to_owned()
        };
        match self {
            Self::Revoked { at } => format!(
                "This enrollment token for {workspace} was revoked on {}: {another}.",
                day(*at)
            ),
            Self::Expired { at } => format!(
                "This enrollment token for {workspace} expired on {}: {another}.",
                day(*at)
            ),
            Self::Full { max } => format!(
                "This enrollment token for {workspace} has as many runners as it may keep at once ({max}). Remove one from {workspace}'s Runners tab, or {}.",
                if personal {
                    "make another token there"
                } else {
                    "ask one of its admins for another token"
                }
            ),
        }
    }

    pub fn error(&self, workspace: &str, personal: bool) -> ApiError {
        ApiError::forbidden(self.message(workspace, personal))
    }
}

/// Why a token for its maker's own sessions enrolls nothing, whatever its
/// state: its maker is no longer an active member of its workspace, so the
/// runners it would enroll would run nothing.
pub fn maker_gone(workspace: &str) -> ApiError {
    ApiError::forbidden(format!(
        "This enrollment token was made by someone who is no longer a member of {workspace}, for their own sessions: it enrolls no runner."
    ))
}

/// `2026-11-05`, in UTC: messages from the control plane don't know the
/// reader's time zone.
fn day(ms: i64) -> String {
    let iso = crate::account::iso8601(ms);
    iso.get(..10).unwrap_or(&iso).to_owned()
}

/// Until when a runner that enrolls at `now` with a token that needs
/// approval waits for it, before it is removed.
pub fn pending_until(now: i64) -> i64 {
    now + PENDING_MS
}

/// Whether a runner whose token removes its runners once they have been
/// offline `minutes` is due for removal at `now`, as the hourly job sees
/// it: it was last seen (or, never connected, enrolled) longer ago than
/// that. A runner that is online now is never due, whatever D1 last heard
/// of it. FleetObject removes the ones it saw connected sooner
/// ([`removal_due`]); this is for those it never saw.
pub fn offline_too_long(
    minutes: u32,
    last_seen_at: Option<i64>,
    created_at: i64,
    online: bool,
    now: i64,
) -> bool {
    !online
        && now - last_seen_at.unwrap_or(created_at).max(created_at) > i64::from(minutes) * MINUTE_MS
}

/// Since when FleetObject has been watching its runners without a gap, as
/// of a tick at `now`: `since`, unless its last tick (`last_tick`) was
/// longer ago than [`MAX_TICK_GAP_MS`] or there was none, when it starts
/// watching now. Time a runner spent offline while the control plane was
/// down, or before it watched, doesn't count towards its removal: a deploy
/// or an outage longer than a token's window would otherwise remove every
/// runner before its host could connect again.
pub fn watching_since(last_tick: Option<i64>, since: Option<i64>, now: i64) -> i64 {
    match (last_tick, since) {
        (Some(t), Some(s)) if now - t <= MAX_TICK_GAP_MS => s,
        _ => now,
    }
}

/// Whether a runner that disconnected at `last_seen` is due for removal at
/// `now`, its token removing runners after `after_ms` offline, counted from
/// when FleetObject started watching (`watching`) if that is later.
pub fn removal_due(after_ms: i64, last_seen: Option<i64>, watching: i64, now: i64) -> bool {
    now - last_seen.unwrap_or(watching).max(watching) >= after_ms
}

/// `a minute`, `5 minutes`, `an hour`, `6 hours`, `a day`, `3 days`, `a
/// week`, `30 days`.
pub fn minutes(m: u32) -> String {
    match m {
        1 => "a minute".into(),
        60 => "an hour".into(),
        1440 => "a day".into(),
        10080 => "a week".into(),
        m if m % 1440 == 0 => format!("{} days", m / 1440),
        m if m % 60 == 0 => format!("{} hours", m / 60),
        m => format!("{m} minutes"),
    }
}

/// Whose sessions the runners a token enrolls run, as the person who reads
/// what it does sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whose<'a> {
    /// The workspace's own runners: everyone's sessions there, once an
    /// admin approves them where the token asks for that.
    Everyone,
    /// The reader made the token for their own sessions.
    Yours,
    /// Someone else made it for theirs: their name.
    Theirs(&'a str),
}

impl Whose<'_> {
    /// `everyone's sessions`, `your sessions`, `Ada's sessions`.
    pub fn sessions(self) -> String {
        match self {
            Self::Everyone => "everyone's sessions".into(),
            Self::Yours => "your sessions".into(),
            Self::Theirs(name) => format!("{name}'s sessions"),
        }
    }

    /// `an admin's approval`, `your approval`, `Ada's approval`.
    pub fn approval(self) -> String {
        match self {
            Self::Everyone => "an admin's approval".into(),
            Self::Yours => "your approval".into(),
            Self::Theirs(name) => format!("{name}'s approval"),
        }
    }
}

/// What a token does with the runners it enrolls, in a sentence: `Enrolls
/// up to 20 runners at a time, which wait for an admin's approval and are
/// removed once they have been offline for 5 minutes.` `remove_offline` is
/// in minutes.
pub fn describe(
    whose: Whose,
    approval: bool,
    max_runners: Option<u32>,
    remove_offline: Option<u32>,
) -> String {
    let how_many = match max_runners {
        None => "Enrolls any number of runners".to_owned(),
        Some(1) => "Enrolls one runner at a time".to_owned(),
        Some(n) => format!("Enrolls up to {n} runners at a time"),
    };
    let one = max_runners == Some(1);
    let sessions = whose.sessions();
    let approver = whose.approval();
    let then = match (approval, remove_offline) {
        (false, None) => format!(
            ", which {} {sessions} at once",
            if one { "runs" } else { "run" }
        ),
        (true, None) => format!(
            ", which {} for {approver} before {} anything",
            if one { "waits" } else { "wait" },
            if one { "it runs" } else { "they run" }
        ),
        (false, Some(h)) => format!(
            ", which {} {sessions} at once and {} removed once {} been offline for {}",
            if one { "runs" } else { "run" },
            if one { "is" } else { "are" },
            if one { "it has" } else { "they have" },
            minutes(h)
        ),
        (true, Some(h)) => format!(
            ", which {} for {approver} and {} removed once {} been offline for {}",
            if one { "waits" } else { "wait" },
            if one { "is" } else { "are" },
            if one { "it has" } else { "they have" },
            minutes(h)
        ),
    };
    format!("{how_many}{then}.")
}

/// A duration in minutes as people type it: `5m`, `12h`, `30d`, `2w`,
/// `1y`, with its unit. `never`, `none`, `off` and `keep` are `None`.
pub fn parse_minutes(input: &str) -> std::result::Result<Option<u32>, String> {
    let s = input.trim().to_ascii_lowercase();
    if matches!(s.as_str(), "never" | "none" | "off" | "keep") {
        return Ok(None);
    }
    let bad = || format!("{input:?} isn't a duration: use 5m, 1h, 7d, 1y or never");
    let digits = s.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let unit = s[digits.len()..].trim();
    let n: u64 = digits.trim().parse().map_err(|_| bad())?;
    let per: u64 = match unit {
        "m" | "min" | "mins" | "minute" | "minutes" => 1,
        "h" | "hr" | "hrs" | "hour" | "hours" => 60,
        "d" | "day" | "days" => 1440,
        "w" | "week" | "weeks" => 10080,
        "y" | "year" | "years" => 525_600,
        "" => {
            return Err(format!(
                "{input:?} needs a unit: 5m, 1h, 7d or 1y, for instance"
            ));
        }
        _ => return Err(bad()),
    };
    // Too long to count, as too long to keep below.
    let minutes = n.checked_mul(per).ok_or_else(bad)?;
    if minutes == 0 {
        return Err(format!("{input:?} is no time at all"));
    }
    u32::try_from(minutes).map(Some).map_err(|_| bad())
}

/// A duration as [`parse_minutes`] reads it, in whole hours, rounded up.
pub fn parse_hours(input: &str) -> std::result::Result<Option<u32>, String> {
    Ok(parse_minutes(input)?.map(|m| m.div_ceil(60)))
}

/// What a host said about its machine as it enrolled, trimmed to what the
/// runners list shows: no field longer than a hostname may be.
pub fn facts(f: RunnerFacts) -> RunnerFacts {
    fn text(s: Option<String>) -> Option<String> {
        s.map(|s| {
            crate::validate::plain(&s)
                .trim()
                .chars()
                .take(253)
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
    }
    RunnerFacts {
        version: text(f.version),
        hostname: text(f.hostname),
        os: text(f.os),
        arch: text(f.arch),
        driver: text(f.driver),
        cpus: f.cpus.filter(|n| *n > 0),
        memory_mb: f.memory_mb.filter(|n| *n > 0),
        capacity: f.capacity.filter(|n| *n > 0),
    }
}

/// A runner that waits for approval, as those who approve it hear of it:
/// its workspace's admins, or its owner (`yours`).
#[derive(Clone, Debug, Default)]
pub struct Waiting<'a> {
    /// It is the reader's, enrolled with a token they made for their own
    /// sessions: they approve it.
    pub yours: bool,
    pub runner_id: &'a str,
    pub runner: &'a str,
    pub hostname: Option<&'a str>,
    /// The enrollment token's name.
    pub token: &'a str,
    pub workspace_id: &'a str,
    pub workspace_slug: &'a str,
    pub workspace: &'a str,
    /// The runners that wait in the workspace now for the reader to
    /// approve them, this one included.
    pub waiting: u32,
    /// Where it enrolled from.
    pub address: Option<&'a str>,
    pub location: Option<&'a str>,
    /// `linux/x86_64, docker, fctl 0.28.0`, as far as it said.
    pub system: Option<&'a str>,
}

impl Waiting<'_> {
    /// `ip-10-0-3-17`, or `build-1 (ip-10-0-3-17)` when its name isn't
    /// its hostname.
    fn who(&self) -> String {
        match self.hostname.filter(|h| *h != self.runner) {
            Some(h) => format!("{} ({h})", self.runner),
            None => self.runner.to_owned(),
        }
    }

    /// The workspace's Runners tab, a path in the app.
    fn tab(&self) -> String {
        format!("/w/{}?tab=runners", self.workspace_slug)
    }
}

/// The notification that tells a workspace's admins, or a runner's owner,
/// that a runner waits for their approval. Its tag is the workspace's, so
/// that a burst of them shows as one, which counts them.
pub fn waiting_push(w: &Waiting, now: Millis) -> PushMessage {
    let (title, body) = match (w.waiting > 1, w.yours) {
        (true, _) => (
            format!("{} runners wait for your approval", w.waiting),
            format!(
                "{} and {} more wait in {}. Approve or reject them on its Runners tab.",
                w.who(),
                w.waiting - 1,
                w.workspace
            ),
        ),
        (false, false) => (
            "A runner waits for your approval".to_owned(),
            format!(
                "{} enrolled in {} with “{}”. Approve or reject it on its Runners tab.",
                w.who(),
                w.workspace,
                w.token
            ),
        ),
        (false, true) => (
            "Your runner waits for your approval".to_owned(),
            format!(
                "{} enrolled in {} with your token “{}”, for your sessions. Approve or reject it on its Runners tab.",
                w.who(),
                w.workspace,
                w.token
            ),
        ),
    };
    PushMessage {
        title,
        body,
        tag: format!("runners-waiting-{}", w.workspace_id),
        url: w.tab(),
        replaces: Vec::new(),
        at: now,
    }
}

/// The email that tells a workspace's admins, or a runner's owner, that a
/// runner waits for their approval: its subject and text.
pub fn waiting_email(w: &Waiting, public_url: &str) -> (String, String) {
    let subject = format!("{} waits for your approval in {}", w.runner, w.workspace);
    let first = if w.yours {
        format!(
            "A machine enrolled in {} as your runner {}, with your enrollment token “{}”, and waits for you to approve it. Until you do, it runs none of your sessions and gets none of their code, secrets or tokens.",
            w.workspace, w.runner, w.token
        )
    } else {
        format!(
            "A machine enrolled in {} as the runner {}, with the enrollment token “{}”, and waits for an admin to approve it. Until one does, it runs none of {}'s sessions and gets none of its code, secrets or tokens.",
            w.workspace, w.runner, w.token, w.workspace
        )
    };
    let mut lines = vec![first, String::new()];
    if let Some(h) = w.hostname {
        lines.push(format!("Hostname: {h}"));
    }
    if let Some(sys) = w.system {
        lines.push(format!("System: {sys}"));
    }
    match (w.address, w.location) {
        (Some(a), Some(l)) => lines.push(format!("Enrolled from: {a} ({l})")),
        (Some(a), None) => lines.push(format!("Enrolled from: {a}")),
        _ => {}
    }
    lines.extend([
        String::new(),
        format!("Approve or reject it on {}'s Runners tab:", w.workspace),
        format!("{}{}", public_url.trim_end_matches('/'), w.tab()),
        String::new(),
        "Or with the CLI:".to_owned(),
        format!("  fctl runners approve {}", w.runner_id),
        format!("  fctl runners reject {}", w.runner_id),
        String::new(),
        "If nobody approves it within 7 days, it is removed.".to_owned(),
    ]);
    (subject, lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn request(name: &str) -> CreateEnrollmentTokenRequest {
        CreateEnrollmentTokenRequest {
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn new_tokens_take_a_name_and_settings_within_bounds() {
        let t = check_new(
            &CreateEnrollmentTokenRequest {
                name: "  Build   fleet ".into(),
                expires_in_hours: Some(24 * 90),
                max_runners: Some(20),
                approval: Some(true),
                remove_offline_after_minutes: Some(5),
                ..Default::default()
            },
            true,
            NOW,
        )
        .ok();
        assert_eq!(
            t,
            Some(NewToken {
                name: "Build fleet".into(),
                personal: true,
                expires_at: Some(NOW + 90 * 24 * HOUR_MS),
                max_runners: Some(20),
                approval: true,
                remove_offline_after_minutes: Some(5),
            })
        );
        // Without settings: never expires, no limit, no approval, kept.
        let t = check_new(&request("CI"), false, NOW).ok();
        assert_eq!(
            t.map(|t| (
                t.expires_at,
                t.max_runners,
                t.approval,
                t.remove_offline_after_minutes
            )),
            Some((None, None, false, None))
        );
        let field =
            |r: CreateEnrollmentTokenRequest| check_new(&r, false, NOW).err().and_then(|e| e.field);
        assert_eq!(field(request(" ")).as_deref(), Some("name"));
        for (expires, max, offline, which) in [
            (Some(0), None, None, "expires_in_hours"),
            (Some(MAX_EXPIRY_HOURS + 1), None, None, "expires_in_hours"),
            (None, Some(0), None, "max_runners"),
            (None, Some(MAX_RUNNERS + 1), None, "max_runners"),
            (None, None, Some(0), "remove_offline_after_minutes"),
            (
                None,
                None,
                Some(MAX_REMOVE_OFFLINE_MINUTES + 1),
                "remove_offline_after_minutes",
            ),
        ] {
            let r = CreateEnrollmentTokenRequest {
                expires_in_hours: expires,
                max_runners: max,
                remove_offline_after_minutes: offline,
                ..request("CI")
            };
            assert_eq!(field(r).as_deref(), Some(which));
        }
    }

    #[test]
    fn a_token_enrolls_until_it_is_revoked_expires_or_is_full() {
        let open = State {
            expires_at: Some(NOW + 1),
            max_runners: Some(2),
            runners: 1,
            ..Default::default()
        };
        assert_eq!(check(&open, NOW), Ok(()));
        assert_eq!(check(&State::default(), NOW), Ok(()));
        assert_eq!(
            check(&State { runners: 2, ..open }, NOW),
            Err(Refusal::Full { max: 2 })
        );
        assert_eq!(check(&open, NOW + 1), Err(Refusal::Expired { at: NOW + 1 }));
        // Revoked says so, whatever else is true of it.
        assert_eq!(
            check(
                &State {
                    revoked_at: Some(NOW - 5),
                    runners: 9,
                    ..open
                },
                NOW + 9
            ),
            Err(Refusal::Revoked { at: NOW - 5 })
        );
    }

    #[test]
    fn refusals_say_why_and_what_to_do() {
        // 2026-11-03 12:00 UTC.
        let at = 1_793_707_200_000;
        assert_eq!(day(at), "2026-11-03");
        assert_eq!(
            Refusal::Revoked { at }.message("Acme", false),
            "This enrollment token for Acme was revoked on 2026-11-03: ask one of its admins for another."
        );
        assert_eq!(
            Refusal::Expired { at }.message("Acme", false),
            "This enrollment token for Acme expired on 2026-11-03: ask one of its admins for another."
        );
        assert_eq!(
            Refusal::Full { max: 3 }.message("Acme", false),
            "This enrollment token for Acme has as many runners as it may keep at once (3). Remove one from Acme's Runners tab, or ask one of its admins for another token."
        );
        assert_eq!(
            Refusal::Full { max: 3 }.error("Acme", false).code,
            fctl_proto::ErrorCode::Forbidden
        );
        // A token for its maker's own sessions is theirs to replace.
        assert_eq!(
            Refusal::Expired { at }.message("Acme", true),
            "This enrollment token for Acme expired on 2026-11-03: make another on Acme's Runners tab."
        );
        assert_eq!(
            Refusal::Full { max: 3 }.message("Acme", true),
            "This enrollment token for Acme has as many runners as it may keep at once (3). Remove one from Acme's Runners tab, or make another token there."
        );
        assert_eq!(
            maker_gone("Acme").message,
            "This enrollment token was made by someone who is no longer a member of Acme, for their own sessions: it enrolls no runner."
        );
    }

    #[test]
    fn runners_wait_a_week_for_approval() {
        assert_eq!(pending_until(NOW), NOW + 7 * 24 * HOUR_MS);
    }

    #[test]
    fn runners_are_removed_once_offline_for_longer_than_their_token_says() {
        let made = NOW - 10 * HOUR_MS;
        let seen = Some(NOW - 90 * MINUTE_MS);
        // Seen 90 minutes ago, removed after an hour offline.
        assert!(offline_too_long(60, seen, made, false, NOW));
        assert!(!offline_too_long(120, seen, made, false, NOW));
        // Online now, whatever the last heartbeat D1 kept says.
        assert!(!offline_too_long(60, seen, made, true, NOW));
        // Never connected: counted from when it enrolled.
        assert!(offline_too_long(9 * 60, None, made, false, NOW));
        assert!(!offline_too_long(11 * 60, None, made, false, NOW));
    }

    #[test]
    fn only_time_the_control_plane_watched_counts_towards_removal() {
        let tick = NOW - 30_000;
        // Ticking along: it has watched since it started.
        assert_eq!(
            watching_since(Some(tick), Some(NOW - HOUR_MS), NOW),
            NOW - HOUR_MS
        );
        // After a gap (it was down, or had nothing to watch), or before its
        // first tick, it watches from now: a minute's outage gives every
        // host its window again.
        assert_eq!(
            watching_since(Some(NOW - 10 * MINUTE_MS), Some(NOW - HOUR_MS), NOW),
            NOW
        );
        assert_eq!(
            watching_since(Some(NOW - MINUTE_MS), Some(NOW - HOUR_MS), NOW),
            NOW
        );
        assert_eq!(watching_since(None, None, NOW), NOW);
        let five = 5 * MINUTE_MS;
        // Gone for six minutes while watched: due after five.
        assert!(removal_due(
            five,
            Some(NOW - 6 * MINUTE_MS),
            NOW - HOUR_MS,
            NOW
        ));
        assert!(!removal_due(
            five,
            Some(NOW - 4 * MINUTE_MS),
            NOW - HOUR_MS,
            NOW
        ));
        // Gone for an hour, but the control plane came back two minutes
        // ago: the host gets its five minutes to connect again.
        assert!(!removal_due(
            five,
            Some(NOW - HOUR_MS),
            NOW - 2 * MINUTE_MS,
            NOW
        ));
        assert!(removal_due(
            five,
            Some(NOW - HOUR_MS),
            NOW - 5 * MINUTE_MS,
            NOW
        ));
    }

    #[test]
    fn durations_read_as_people_type_them() {
        assert_eq!(parse_minutes("1m"), Ok(Some(1)));
        assert_eq!(parse_minutes("15 minutes"), Ok(Some(15)));
        assert_eq!(parse_minutes("1h"), Ok(Some(60)));
        assert_eq!(parse_minutes("7d"), Ok(Some(10080)));
        assert_eq!(parse_minutes("2w"), Ok(Some(20160)));
        assert_eq!(parse_minutes("Never"), Ok(None));
        assert_eq!(parse_minutes("keep"), Ok(None));
        // A number alone could mean anything.
        assert!(parse_minutes("30").is_err_and(|e| e.contains("needs a unit")));
        assert!(parse_minutes("0m").is_err());
        assert!(parse_minutes("soon").is_err());
        assert!(parse_minutes("3 fortnights").is_err());
        assert!(parse_minutes("99999999y").is_err());
        // Past what a u64 holds, rather than wrapping round to a short one.
        assert!(parse_minutes("35096545041305y").is_err());
        assert!(parse_hours("18446744073709551615h").is_err());
        // Expiries are whole hours.
        assert_eq!(parse_hours("90m"), Ok(Some(2)));
        assert_eq!(parse_hours("90d"), Ok(Some(2160)));
        assert_eq!(parse_hours("1y"), Ok(Some(8760)));
        assert_eq!(parse_hours("never"), Ok(None));
    }

    #[test]
    fn tokens_say_what_they_do_with_the_runners_they_enroll() {
        use Whose::*;
        assert_eq!(
            describe(Everyone, false, None, None),
            "Enrolls any number of runners, which run everyone's sessions at once."
        );
        assert_eq!(
            describe(Everyone, true, Some(20), Some(5)),
            "Enrolls up to 20 runners at a time, which wait for an admin's approval and are removed once they have been offline for 5 minutes."
        );
        assert_eq!(
            describe(Everyone, false, Some(1), Some(1440)),
            "Enrolls one runner at a time, which runs everyone's sessions at once and is removed once it has been offline for a day."
        );
        assert_eq!(
            describe(Everyone, true, Some(1), None),
            "Enrolls one runner at a time, which waits for an admin's approval before it runs anything."
        );
        // A member's, for their own sessions, as they and others read it.
        assert_eq!(
            describe(Yours, false, Some(4), Some(5)),
            "Enrolls up to 4 runners at a time, which run your sessions at once and are removed once they have been offline for 5 minutes."
        );
        assert_eq!(
            describe(Theirs("Ada"), true, None, None),
            "Enrolls any number of runners, which wait for Ada's approval before they run anything."
        );
        assert_eq!(minutes(1), "a minute");
        assert_eq!(minutes(15), "15 minutes");
        assert_eq!(minutes(60), "an hour");
        assert_eq!(minutes(360), "6 hours");
        assert_eq!(minutes(4320), "3 days");
        assert_eq!(minutes(10080), "a week");
    }

    #[test]
    fn admins_hear_which_runner_waits_and_how_many_do() {
        let w = Waiting {
            yours: false,
            runner_id: "rnr_01",
            runner: "build-1",
            hostname: Some("ip-10-0-3-17"),
            token: "Build fleet",
            workspace_id: "wsp_1",
            workspace_slug: "acme",
            workspace: "Acme",
            waiting: 1,
            address: Some("203.0.113.7"),
            location: Some("Stockholm, SE"),
            system: Some("linux/x86_64, docker, fctl 0.28.0"),
        };
        let one = waiting_push(&w, NOW);
        assert_eq!(one.title, "A runner waits for your approval");
        assert_eq!(
            one.body,
            "build-1 (ip-10-0-3-17) enrolled in Acme with “Build fleet”. Approve or reject it on its Runners tab."
        );
        assert_eq!(one.url, "/w/acme?tab=runners");
        // The next one replaces it, counting them.
        let three = waiting_push(
            &Waiting {
                waiting: 3,
                hostname: Some("build-1"),
                ..w.clone()
            },
            NOW,
        );
        assert_eq!(three.tag, one.tag);
        assert_eq!(three.title, "3 runners wait for your approval");
        assert_eq!(
            three.body,
            "build-1 and 2 more wait in Acme. Approve or reject them on its Runners tab."
        );
        let (subject, text) = waiting_email(&w, "https://app.fctl.sh/");
        assert_eq!(subject, "build-1 waits for your approval in Acme");
        assert!(
            text.contains("Enrolled from: 203.0.113.7 (Stockholm, SE)"),
            "{text}"
        );
        assert!(
            text.contains("https://app.fctl.sh/w/acme?tab=runners"),
            "{text}"
        );
        assert!(text.contains("fctl runners approve rnr_01"), "{text}");
        assert!(text.contains("waits for an admin to approve it"), "{text}");
    }

    #[test]
    fn owners_hear_that_their_own_runner_waits() {
        let w = Waiting {
            yours: true,
            runner_id: "rnr_01",
            runner: "pod-7",
            token: "My cluster",
            workspace_id: "wsp_1",
            workspace_slug: "acme",
            workspace: "Acme",
            waiting: 1,
            ..Default::default()
        };
        let push = waiting_push(&w, NOW);
        assert_eq!(push.title, "Your runner waits for your approval");
        assert_eq!(
            push.body,
            "pod-7 enrolled in Acme with your token “My cluster”, for your sessions. Approve or reject it on its Runners tab."
        );
        let (_, text) = waiting_email(&w, "https://app.fctl.sh");
        assert!(
            text.starts_with("A machine enrolled in Acme as your runner pod-7, with your enrollment token “My cluster”, and waits for you to approve it."),
            "{text}"
        );
    }

    #[test]
    fn what_hosts_say_of_their_machine_is_trimmed() {
        let f = facts(RunnerFacts {
            hostname: Some(format!("  {}  ", "h".repeat(300))),
            os: Some("   ".into()),
            arch: Some("aarch64\u{7}".into()),
            cpus: Some(0),
            memory_mb: Some(16384),
            ..Default::default()
        });
        assert_eq!(f.hostname.map(|h| h.len()), Some(253));
        assert_eq!(f.os, None);
        assert_eq!(f.arch.as_deref(), Some("aarch64"));
        assert_eq!((f.cpus, f.memory_mb), (None, Some(16384)));
    }
}
