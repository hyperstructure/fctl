//! Enrollment tokens (docs/architecture.md#enrollment-tokens): machines
//! enroll themselves as a workspace's runners with one, without anyone
//! signing in on them, as autoscaling creates them. What a token may be
//! made with, whether it enrolls another runner now, what it says it does,
//! and when the runners it enrolled are removed.

use crate::validate::Result;
use fctl_proto::api::{CreateEnrollmentTokenRequest, PushMessage, RunnerFacts};
use fctl_proto::{ApiError, Millis};

pub const HOUR_MS: i64 = 3_600_000;
/// How long a runner waits for an admin of its workspace to approve it
/// before it is removed.
pub const PENDING_MS: i64 = 7 * 24 * HOUR_MS;
/// The longest an enrollment token lasts, when it expires at all.
pub const MAX_EXPIRY_HOURS: u32 = 365 * 24;
/// The most runners one token may keep enrolled at once.
pub const MAX_RUNNERS: u32 = 1000;
/// The longest a token may let its runners stay offline before they are
/// removed.
pub const MAX_REMOVE_OFFLINE_HOURS: u32 = 30 * 24;
/// Revoked and expired tokens are deleted this long after they ended, once
/// no runner they enrolled is left.
pub const KEEP_ENDED_MS: i64 = 90 * 24 * HOUR_MS;

/// A new enrollment token's settings, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewToken {
    pub name: String,
    pub expires_at: Option<i64>,
    pub max_runners: Option<u32>,
    pub approval: bool,
    pub remove_offline_after_hours: Option<u32>,
}

/// Checks what a new enrollment token is to be made with, at `now`.
pub fn check_new(req: &CreateEnrollmentTokenRequest, now: i64) -> Result<NewToken> {
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
    let remove_offline_after_hours = match req.remove_offline_after_hours {
        None => None,
        Some(h) if (1..=MAX_REMOVE_OFFLINE_HOURS).contains(&h) => Some(h),
        Some(_) => {
            return Err(ApiError::validation(
                "remove_offline_after_hours",
                "Remove offline runners after an hour to 30 days, or keep them",
            ));
        }
    };
    Ok(NewToken {
        name,
        expires_at,
        max_runners,
        approval: req.approval.unwrap_or(false),
        remove_offline_after_hours,
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
    /// workspace.
    pub fn message(&self, workspace: &str) -> String {
        match self {
            Self::Revoked { at } => format!(
                "This enrollment token for {workspace} was revoked on {}: ask one of its admins for another.",
                day(*at)
            ),
            Self::Expired { at } => format!(
                "This enrollment token for {workspace} expired on {}: ask one of its admins for another.",
                day(*at)
            ),
            Self::Full { max } => format!(
                "This enrollment token for {workspace} has as many runners as it may keep at once ({max}). Remove one from {workspace}'s Runners tab, or ask one of its admins for another token."
            ),
        }
    }

    pub fn error(&self, workspace: &str) -> ApiError {
        ApiError::forbidden(self.message(workspace))
    }
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
/// offline `hours` is due for removal at `now`: it was last seen (or, never
/// connected, enrolled) longer ago than that. A runner that is online now
/// is never due, whatever D1 last heard of it.
pub fn offline_too_long(
    hours: u32,
    last_seen_at: Option<i64>,
    created_at: i64,
    online: bool,
    now: i64,
) -> bool {
    !online && now - last_seen_at.unwrap_or(created_at).max(created_at) > i64::from(hours) * HOUR_MS
}

/// `an hour`, `6 hours`, `a day`, `3 days`, `a week`, `30 days`.
pub fn hours(h: u32) -> String {
    match h {
        1 => "an hour".into(),
        24 => "a day".into(),
        168 => "a week".into(),
        h if h % 24 == 0 => format!("{} days", h / 24),
        h => format!("{h} hours"),
    }
}

/// What a token does with the runners it enrolls, in a sentence: `Enrolls
/// up to 20 runners at a time, which wait for an admin's approval and are
/// removed once they have been offline for an hour.`
pub fn describe(approval: bool, max_runners: Option<u32>, remove_offline: Option<u32>) -> String {
    let how_many = match max_runners {
        None => "Enrolls any number of runners".to_owned(),
        Some(1) => "Enrolls one runner at a time".to_owned(),
        Some(n) => format!("Enrolls up to {n} runners at a time"),
    };
    let one = max_runners == Some(1);
    let then = match (approval, remove_offline) {
        (false, None) => format!(
            ", which {} everyone's sessions at once",
            if one { "runs" } else { "run" }
        ),
        (true, None) => format!(
            ", which {} for an admin's approval before {} anything",
            if one { "waits" } else { "wait" },
            if one { "it runs" } else { "they run" }
        ),
        (false, Some(h)) => format!(
            ", which {} everyone's sessions at once and {} removed once {} been offline for {}",
            if one { "runs" } else { "run" },
            if one { "is" } else { "are" },
            if one { "it has" } else { "they have" },
            hours(h)
        ),
        (true, Some(h)) => format!(
            ", which {} for an admin's approval and {} removed once {} been offline for {}",
            if one { "waits" } else { "wait" },
            if one { "is" } else { "are" },
            if one { "it has" } else { "they have" },
            hours(h)
        ),
    };
    format!("{how_many}{then}.")
}

/// A duration in hours as people type it: `90m` rounds up to whole hours,
/// `12h`, `30d`, `2w`, `1y`; a number alone is days. `never`, `none` and
/// `off` are `None`.
pub fn parse_hours(input: &str) -> std::result::Result<Option<u32>, String> {
    let s = input.trim().to_ascii_lowercase();
    if matches!(s.as_str(), "never" | "none" | "off" | "keep") {
        return Ok(None);
    }
    let bad = || format!("{input:?} isn't a duration: use 1h, 12h, 7d, 30d, 1y or never");
    let digits = s.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let unit = &s[digits.len()..];
    let n: u64 = digits.trim().parse().map_err(|_| bad())?;
    let hours = match unit {
        "m" | "min" | "mins" | "minute" | "minutes" => n.div_ceil(60),
        "h" | "hr" | "hrs" | "hour" | "hours" => n,
        "" | "d" | "day" | "days" => n * 24,
        "w" | "week" | "weeks" => n * 24 * 7,
        "y" | "year" | "years" => n * 24 * 365,
        _ => return Err(bad()),
    };
    if hours == 0 {
        return Err(format!("{input:?} is no time at all: use at least 1h"));
    }
    u32::try_from(hours).map(Some).map_err(|_| bad())
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

/// A runner that waits for approval, as its workspace's admins hear of it.
#[derive(Clone, Debug, Default)]
pub struct Waiting<'a> {
    pub runner_id: &'a str,
    pub runner: &'a str,
    pub hostname: Option<&'a str>,
    /// The enrollment token's name.
    pub token: &'a str,
    pub workspace_id: &'a str,
    pub workspace_slug: &'a str,
    pub workspace: &'a str,
    /// The runners that wait in the workspace now, this one included.
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

/// The notification that tells a workspace's admins that a runner waits for
/// their approval. Its tag is the workspace's, so that a burst of them
/// shows as one, which counts them.
pub fn waiting_push(w: &Waiting, now: Millis) -> PushMessage {
    let (title, body) = if w.waiting > 1 {
        (
            format!("{} runners wait for your approval", w.waiting),
            format!(
                "{} and {} more wait in {}. Approve or reject them on its Runners tab.",
                w.who(),
                w.waiting - 1,
                w.workspace
            ),
        )
    } else {
        (
            "A runner waits for your approval".to_owned(),
            format!(
                "{} enrolled in {} with “{}”. Approve or reject it on its Runners tab.",
                w.who(),
                w.workspace,
                w.token
            ),
        )
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

/// The email that tells a workspace's admins that a runner waits for their
/// approval: its subject and text.
pub fn waiting_email(w: &Waiting, public_url: &str) -> (String, String) {
    let subject = format!("{} waits for your approval in {}", w.runner, w.workspace);
    let mut lines = vec![
        format!(
            "A machine enrolled in {} as the runner {}, with the enrollment token “{}”, and waits for an admin to approve it. Until one does, it runs none of {}'s sessions and gets none of its code, secrets or tokens.",
            w.workspace, w.runner, w.token, w.workspace
        ),
        String::new(),
    ];
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
                remove_offline_after_hours: Some(1),
            },
            NOW,
        )
        .ok();
        assert_eq!(
            t,
            Some(NewToken {
                name: "Build fleet".into(),
                expires_at: Some(NOW + 90 * 24 * HOUR_MS),
                max_runners: Some(20),
                approval: true,
                remove_offline_after_hours: Some(1),
            })
        );
        // Without settings: never expires, no limit, no approval, kept.
        let t = check_new(&request("CI"), NOW).ok();
        assert_eq!(
            t.map(|t| (
                t.expires_at,
                t.max_runners,
                t.approval,
                t.remove_offline_after_hours
            )),
            Some((None, None, false, None))
        );
        let field =
            |r: CreateEnrollmentTokenRequest| check_new(&r, NOW).err().and_then(|e| e.field);
        assert_eq!(field(request(" ")).as_deref(), Some("name"));
        for (expires, max, offline, which) in [
            (Some(0), None, None, "expires_in_hours"),
            (Some(MAX_EXPIRY_HOURS + 1), None, None, "expires_in_hours"),
            (None, Some(0), None, "max_runners"),
            (None, Some(MAX_RUNNERS + 1), None, "max_runners"),
            (None, None, Some(0), "remove_offline_after_hours"),
            (
                None,
                None,
                Some(MAX_REMOVE_OFFLINE_HOURS + 1),
                "remove_offline_after_hours",
            ),
        ] {
            let r = CreateEnrollmentTokenRequest {
                expires_in_hours: expires,
                max_runners: max,
                remove_offline_after_hours: offline,
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
            Refusal::Revoked { at }.message("Acme"),
            "This enrollment token for Acme was revoked on 2026-11-03: ask one of its admins for another."
        );
        assert_eq!(
            Refusal::Expired { at }.message("Acme"),
            "This enrollment token for Acme expired on 2026-11-03: ask one of its admins for another."
        );
        assert_eq!(
            Refusal::Full { max: 3 }.message("Acme"),
            "This enrollment token for Acme has as many runners as it may keep at once (3). Remove one from Acme's Runners tab, or ask one of its admins for another token."
        );
        assert_eq!(
            Refusal::Full { max: 3 }.error("Acme").code,
            fctl_proto::ErrorCode::Forbidden
        );
    }

    #[test]
    fn runners_wait_a_week_for_approval() {
        assert_eq!(pending_until(NOW), NOW + 7 * 24 * HOUR_MS);
    }

    #[test]
    fn runners_are_removed_once_offline_for_longer_than_their_token_says() {
        let made = NOW - 10 * HOUR_MS;
        // Seen 90 minutes ago, removed after an hour offline.
        assert!(offline_too_long(
            1,
            Some(NOW - 90 * 60_000),
            made,
            false,
            NOW
        ));
        assert!(!offline_too_long(
            2,
            Some(NOW - 90 * 60_000),
            made,
            false,
            NOW
        ));
        // Online now, whatever the last heartbeat D1 kept says.
        assert!(!offline_too_long(
            1,
            Some(NOW - 90 * 60_000),
            made,
            true,
            NOW
        ));
        // Never connected: counted from when it enrolled.
        assert!(offline_too_long(9, None, made, false, NOW));
        assert!(!offline_too_long(11, None, made, false, NOW));
    }

    #[test]
    fn durations_read_as_people_type_them() {
        assert_eq!(parse_hours("1h"), Ok(Some(1)));
        assert_eq!(parse_hours("90m"), Ok(Some(2)));
        assert_eq!(parse_hours("12 hours"), Ok(Some(12)));
        assert_eq!(parse_hours("7d"), Ok(Some(168)));
        assert_eq!(parse_hours("30"), Ok(Some(720)));
        assert_eq!(parse_hours("2w"), Ok(Some(336)));
        assert_eq!(parse_hours("1y"), Ok(Some(8760)));
        assert_eq!(parse_hours("Never"), Ok(None));
        assert_eq!(parse_hours("off"), Ok(None));
        assert!(parse_hours("0h").is_err());
        assert!(parse_hours("soon").is_err());
        assert!(parse_hours("3 fortnights").is_err());
        assert!(parse_hours("99999999y").is_err());
    }

    #[test]
    fn tokens_say_what_they_do_with_the_runners_they_enroll() {
        assert_eq!(
            describe(false, None, None),
            "Enrolls any number of runners, which run everyone's sessions at once."
        );
        assert_eq!(
            describe(true, Some(20), Some(1)),
            "Enrolls up to 20 runners at a time, which wait for an admin's approval and are removed once they have been offline for an hour."
        );
        assert_eq!(
            describe(false, Some(1), Some(24)),
            "Enrolls one runner at a time, which runs everyone's sessions at once and is removed once it has been offline for a day."
        );
        assert_eq!(
            describe(true, Some(1), None),
            "Enrolls one runner at a time, which waits for an admin's approval before it runs anything."
        );
        assert_eq!(hours(168), "a week");
        assert_eq!(hours(72), "3 days");
        assert_eq!(hours(6), "6 hours");
    }

    #[test]
    fn admins_hear_which_runner_waits_and_how_many_do() {
        let w = Waiting {
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
