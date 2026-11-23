//! Whether each person's Claude connection works, and who hears of it (#627,
//! docs/architecture.md#claude-oversight): what a Claude message says of the
//! connection a session runs on (an answer, a refusal), when a long-lived
//! token runs out and when to warn of it, which connection a workspace's
//! admins see for each member and how much of it, and what fctl emails
//! people and their admins.

use crate::cron::{DAY_MS, civil_from_days};
use fctl_proto::Millis;
use fctl_proto::api::{
    ClaudeConnectionKind, ClaudeOversight, ClaudeState, LimitPauses, MemberClaude,
    MemberClaudeConnection,
};
use fctl_proto::session::UserRef;
use serde_json::Value;

/// A long-lived token works until a year after `claude setup-token` made
/// it (docs/security.md).
pub const TOKEN_LIFE_MS: i64 = 365 * DAY_MS;

/// The days before a token runs out that its person hears of it first, and
/// that "runs out soon" covers.
pub const SOON_DAYS: i64 = 30;

/// The days before a token runs out that its person hears of it again, and
/// that the Agent Floor shows it.
pub const LAST_DAYS: i64 = 7;

/// How long fctl keeps each limit pause, and each refusal.
pub const KEPT_MS: i64 = 30 * DAY_MS;

/// So many limit pauses in a week say that a plan's limits are short for
/// the work.
pub const FREQUENT_PAUSES: u32 = 5;

/// Claude Code's own name for the error of a request Claude refused the
/// credential of (`SDKAssistantMessage.error`).
const AUTHENTICATION_FAILED: &str = "authentication_failed";

/// Whether a Claude message says Claude refused the credential the session
/// runs on: an assistant message Claude Code made of an authentication
/// error, as a revoked or expired token gets.
pub fn refused(v: &Value) -> bool {
    v.get("type").and_then(Value::as_str) == Some("assistant")
        && v.get("error").and_then(Value::as_str) == Some(AUTHENTICATION_FAILED)
}

/// Whether a Claude message is an answer from Claude, which means it took
/// the credential: an assistant message, of the conversation or a
/// subagent, that isn't one Claude Code made itself of an error.
pub fn answered(v: &Value) -> bool {
    v.get("type").and_then(Value::as_str) == Some("assistant")
        && v.get("error").is_none_or(Value::is_null)
        && v.get("isApiErrorMessage").and_then(Value::as_bool) != Some(true)
        && v.pointer("/message/model").and_then(Value::as_str) != Some("<synthetic>")
}

/// When a long-lived token runs out: a year after `made_at`, when its
/// person said when `claude setup-token` made it, else a year after it was
/// connected (`connected_at`), which is an estimate (`true`): it may run
/// out sooner.
pub fn runs_out(made_at: Option<Millis>, connected_at: Millis) -> (Millis, bool) {
    match made_at {
        Some(made) => (made.saturating_add(TOKEN_LIFE_MS), false),
        None => (connected_at.saturating_add(TOKEN_LIFE_MS), true),
    }
}

/// What is wrong with `made_at` as when `claude setup-token` made a token
/// someone connects at `now`, or `None`.
pub fn made_at_problem(made_at: Millis, now: Millis) -> Option<String> {
    if made_at > now.saturating_add(DAY_MS) {
        return Some(
            "That day is still to come: give the day `claude setup-token` made the token".into(),
        );
    }
    let ends = made_at.saturating_add(TOKEN_LIFE_MS);
    (ends <= now).then(|| {
        format!(
            "A token made then ran out on {}: make a new one with `claude setup-token`",
            day(ends, now)
        )
    })
}

/// Which warning is due, at `now`, for a token that runs out at
/// `runs_out_at` and whose person fctl last warned at `warned_at`: the
/// days it warns at (30, then 7), once each, and only the nearer when both
/// are due. None once it ran out: Claude's refusals tell then.
pub fn warning_due(runs_out_at: Millis, warned_at: Option<Millis>, now: Millis) -> Option<i64> {
    let left = runs_out_at.saturating_sub(now);
    if left <= 0 {
        return None;
    }
    let stage = if left <= LAST_DAYS * DAY_MS {
        LAST_DAYS
    } else if left <= SOON_DAYS * DAY_MS {
        SOON_DAYS
    } else {
        return None;
    };
    let from = runs_out_at.saturating_sub(stage * DAY_MS);
    match warned_at {
        Some(w) if w >= from => None,
        _ => Some(stage),
    }
}

/// Whether a token that runs out at `runs_out_at` runs out within
/// [`SOON_DAYS`] of `now` (or ran out).
pub fn soon(runs_out_at: Millis, now: Millis) -> bool {
    runs_out_at.saturating_sub(now) <= SOON_DAYS * DAY_MS
}

/// Whole days from `now` until `at`, rounded up: "in 3 days".
pub fn days_left(at: Millis, now: Millis) -> i64 {
    let ms = at.saturating_sub(now).max(0);
    (ms + DAY_MS - 1) / DAY_MS
}

/// What a workspace's admins see of its members' connections: what it
/// chose, no more than the instance lets them, or the most the instance
/// lets them while it chose nothing.
pub fn effective(most: ClaudeOversight, chosen: Option<ClaudeOversight>) -> ClaudeOversight {
    chosen.map_or(most, |c| c.min(most))
}

/// One of a person's Claude connections, as the control plane keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub id: String,
    pub kind: ClaudeConnectionKind,
    /// Claude takes it, as far as fctl knows (`status = 'active'`).
    pub active: bool,
    /// An admin token set it.
    pub provisioned: bool,
    pub created_at: Millis,
    pub answered_at: Option<Millis>,
    pub used_at: Option<Millis>,
    /// When its long-lived token runs out, and whether that is an
    /// estimate: tokens fctl keeps only.
    pub runs_out: Option<(Millis, bool)>,
    pub ended_at: Option<Millis>,
    /// `pro`, `max`, `ultra`, …, where known.
    pub plan: Option<String>,
}

/// A time someone's sessions paused on a usage limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pause {
    pub at: Millis,
    /// Claude's name for the limit (`five_hour`, `seven_day`, …).
    pub limit: Option<String>,
    /// When it resets: the sessions wait until then.
    pub resets_at: Option<Millis>,
}

/// The connection someone's sessions run on, as `api::sessions::
/// credential_for` picks it among those that work: the one they chose for
/// the workspace (`chosen`), else their default, else their newest; and
/// whether they chose it there. `conns` newest first.
pub fn in_use<'a>(
    conns: &'a [Connection],
    chosen: Option<&str>,
    default: Option<&str>,
) -> Option<(&'a Connection, bool)> {
    let working =
        |id: Option<&str>| id.and_then(|id| conns.iter().find(|c| c.active && c.id == id));
    if let Some(c) = working(chosen) {
        return Some((c, true));
    }
    working(default)
        .or_else(|| conns.iter().find(|c| c.active))
        .map(|c| (c, false))
}

/// When none of someone's connections works, the one that ended which would
/// run their sessions: the one they chose, else their default, else the
/// one that ended last.
pub fn ended<'a>(
    conns: &'a [Connection],
    chosen: Option<&str>,
    default: Option<&str>,
) -> Option<&'a Connection> {
    let named = |id: Option<&str>| id.and_then(|id| conns.iter().find(|c| c.id == id));
    named(chosen)
        .or_else(|| named(default))
        .or_else(|| {
            conns
                .iter()
                .max_by_key(|c| c.ended_at.unwrap_or(c.created_at))
        })
        .filter(|c| !c.active)
}

/// A person as their admins see them: the connection their sessions run
/// on (or the one that ended), its limit pauses (`pauses`, the last 30
/// days', any order), as much as `oversight` and `limit_pauses` show, at
/// `now`.
#[allow(clippy::too_many_arguments)]
pub fn member(
    user: UserRef,
    email: String,
    conns: &[Connection],
    chosen: Option<&str>,
    default: Option<&str>,
    pauses: &[Pause],
    oversight: ClaudeOversight,
    limit_pauses: bool,
    now: Millis,
) -> MemberClaude {
    let (conn, chosen_here) = match in_use(conns, chosen, default) {
        Some((c, here)) => (Some(c), here),
        None => (ended(conns, chosen, default), false),
    };
    let state = match conn {
        None => ClaudeState::None,
        Some(c) if !c.active => ClaudeState::Ended,
        Some(c) if c.answered_at.is_none() && c.used_at.is_none() => ClaudeState::Unused,
        Some(_) => ClaudeState::Works,
    };
    let waiting = pauses
        .iter()
        .filter(|p| p.resets_at.is_some_and(|r| r > now))
        .max_by_key(|p| p.resets_at);
    let details = oversight >= ClaudeOversight::Details;
    let mut out = MemberClaude {
        user,
        email,
        state,
        runs_out_at: conn.and_then(|c| c.runs_out).map(|r| r.0),
        runs_out_estimated: conn.and_then(|c| c.runs_out).is_some_and(|r| r.1),
        ended_at: conn.filter(|c| !c.active).and_then(|c| c.ended_at),
        paused_until: conn.and(waiting.and_then(|p| p.resets_at)),
        paused_limit: conn.and(waiting.and_then(|p| p.limit.clone())),
        connection: conn.filter(|_| details).map(|c| MemberClaudeConnection {
            kind: c.kind,
            set_by_admin_token: c.provisioned,
            chosen_here,
            plan: c.plan.clone(),
            connected_at: c.created_at,
            answered_at: c.answered_at,
            used_at: c.used_at,
        }),
        pauses: (details && limit_pauses && conn.is_some()).then(|| count_pauses(pauses, now)),
    };
    if oversight == ClaudeOversight::Off {
        out = MemberClaude {
            state: ClaudeState::None,
            runs_out_at: None,
            runs_out_estimated: false,
            ended_at: None,
            paused_until: None,
            paused_limit: None,
            connection: None,
            pauses: None,
            ..out
        };
    }
    out
}

/// How often sessions paused on limits, in the last 7 and 30 days.
pub fn count_pauses(pauses: &[Pause], now: Millis) -> LimitPauses {
    let within = |days: i64| {
        let n = pauses
            .iter()
            .filter(|p| p.at > now.saturating_sub(days * DAY_MS) && p.at <= now)
            .count();
        u32::try_from(n).unwrap_or(u32::MAX)
    };
    let last = pauses
        .iter()
        .filter(|p| p.at > now.saturating_sub(KEPT_MS))
        .max_by_key(|p| p.at);
    LimitPauses {
        week: within(7),
        month: within(30),
        last_at: last.map(|p| p.at),
        last_limit: last.and_then(|p| p.limit.clone()),
    }
}

/// A day, in UTC, for people: "December 14", with the year when it isn't
/// `now`'s: "January 3, 2027".
pub fn day(at: Millis, now: Millis) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let (year, month, d) = civil_from_days(at.div_euclid(DAY_MS));
    let (this_year, _, _) = civil_from_days(now.div_euclid(DAY_MS));
    let name = MONTHS
        .get(usize::try_from(month).unwrap_or(1).saturating_sub(1))
        .unwrap_or(&"");
    if year == this_year {
        format!("{name} {d}")
    } else {
        format!("{name} {d}, {year}")
    }
}

/// "around December 14" for an estimate, else "on December 14".
fn when(at: Millis, estimated: bool, now: Millis) -> String {
    format!(
        "{} {}",
        if estimated { "around" } else { "on" },
        day(at, now)
    )
}

/// "in 1 day", "in 21 days".
fn in_days(at: Millis, now: Millis) -> String {
    match days_left(at, now) {
        1 => "in 1 day".into(),
        n => format!("in {n} days"),
    }
}

/// The email that tells someone their long-lived token runs out, at the
/// warning days ([`warning_due`]): subject and text. `label` is what they
/// call it; `settings` the link to Settings → Claude; `provisioned` says
/// an admin token set it, so their admins give them the next.
pub fn expiry_notice(
    instance: &str,
    label: Option<&str>,
    runs_out_at: Millis,
    estimated: bool,
    provisioned: bool,
    settings: &str,
    now: Millis,
) -> (String, String) {
    let at = when(runs_out_at, estimated, now);
    let which = label.map_or_else(String::new, |l| format!(" ({l})"));
    let why = if estimated {
        " That is a year after it was connected: fctl can't read the date from the token, so it may be sooner."
    } else {
        ""
    };
    let next = if provisioned {
        format!(
            "Your admins set this token for you: they set the next one too. You can also connect one of your own in Settings → Claude before then:\n{settings}"
        )
    } else {
        format!(
            "Make a new one before then, or your sessions stop: run `claude setup-token` on any machine with Claude Code, and paste what it prints in Settings → Claude:\n{settings}"
        )
    };
    (
        format!("Your Claude token for {instance} runs out {at}"),
        format!(
            "The long-lived Claude token your sessions on {instance} run on{which} runs out {at}, {}.{why}\n\n{next}",
            in_days(runs_out_at, now)
        ),
    )
}

/// The email that tells someone Claude stopped taking their connection,
/// once fctl marks it ended: subject and text. `login` for a Claude login
/// Claude wouldn't renew, else a long-lived token Claude refused.
pub fn ended_notice(
    instance: &str,
    label: Option<&str>,
    login: bool,
    provisioned: bool,
    settings: &str,
) -> (String, String) {
    let which = label.map_or_else(String::new, |l| format!(" ({l})"));
    let (what, again) = if login {
        (
            format!("Claude ended the login your sessions on {instance} ran on{which}"),
            "Sign in again in Settings → Claude, or connect a long-lived token, which lasts a year:",
        )
    } else if provisioned {
        (
            format!(
                "Claude refused the long-lived token your admins set for your sessions on {instance}{which}: it was revoked, or it ran out"
            ),
            "Your admins set the next one: ask them. You can also connect one of your own in Settings → Claude:",
        )
    } else {
        (
            format!(
                "Claude refused the long-lived token your sessions on {instance} ran on{which}: it was revoked, or it ran out"
            ),
            "Make a new one with `claude setup-token` on any machine with Claude Code, and paste what it prints in Settings → Claude:",
        )
    };
    (
        format!("Claude stopped taking your Claude connection on {instance}"),
        format!(
            "{what}. Sessions can't run on it any more: until you connect another, yours run on another connection of yours, if you have one that works, and otherwise can't start.\n\n{again}\n{settings}"
        ),
    )
}

/// A member whose connection fctl tells a workspace's admins of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Told {
    pub name: String,
    /// The address they are a member as.
    pub email: String,
    /// When their token runs out, and whether that is an estimate; None
    /// for a connection that stopped working.
    pub runs_out: Option<(Millis, bool)>,
    /// When it stopped working.
    pub ended_at: Option<Millis>,
    /// An admin token set it.
    pub provisioned: bool,
}

/// How admins give `t` a new token: with an admin token, where the instance
/// takes them (`admin_tokens`) and one set it, else in their own Settings.
fn way_out(t: &Told, admin_tokens: bool) -> String {
    if admin_tokens && t.provisioned {
        format!(
            "Set them a new one with an admin token, from a token `claude setup-token` printed: fctl admin users set {} --claude-token -",
            t.email
        )
    } else {
        "They connect a new one in Settings → Claude: fctl told them too.".into()
    }
}

/// The email that tells a workspace's admins, who hear of each, of one
/// member's connection: subject and text. `people` links to its People
/// tab and `settings` to its settings.
pub fn admin_notice(
    workspace: &str,
    t: &Told,
    admin_tokens: bool,
    people: &str,
    settings: &str,
    now: Millis,
) -> (String, String) {
    let (subject, what) = match t.runs_out {
        Some((at, estimated)) => (
            format!(
                "{}'s Claude token runs out {}",
                t.name,
                when(at, estimated, now)
            ),
            format!(
                "{}'s sessions in {workspace} run on a long-lived Claude token that runs out {}, {}: then they stop.",
                t.name,
                when(at, estimated, now),
                in_days(at, now)
            ),
        ),
        None => (
            format!("Claude stopped taking {}'s connection", t.name),
            format!(
                "Claude refused {}'s Claude connection{}: their sessions in {workspace} can't run until they have another.",
                t.name,
                t.ended_at
                    .map_or_else(String::new, |at| format!(" on {}", day(at, now)))
            ),
        ),
    };
    (
        subject,
        format!(
            "{what} {}\n\nEveryone's, on {workspace}'s People tab:\n{people}\n\n{workspace}'s admins hear of each one; change that in its settings:\n{settings}",
            way_out(t, admin_tokens)
        ),
    )
}

/// The weekly email to a workspace's admins, who hear of them once a week,
/// of members whose token runs out within [`SOON_DAYS`] (`soon`) or whose
/// connection stopped working (`ended`): subject and text, or None when
/// there's nothing to say.
pub fn digest(
    workspace: &str,
    ended: &[Told],
    soon: &[Told],
    admin_tokens: bool,
    people: &str,
    settings: &str,
    now: Millis,
) -> Option<(String, String)> {
    if ended.is_empty() && soon.is_empty() {
        return None;
    }
    let count = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let mut parts = Vec::new();
    if !soon.is_empty() {
        parts.push(format!(
            "{} out within a month",
            count(soon.len(), "token runs", "tokens run")
        ));
    }
    if !ended.is_empty() {
        parts.push(format!(
            "{} working",
            count(ended.len(), "connection stopped", "connections stopped")
        ));
    }
    let mut text = String::new();
    if !ended.is_empty() {
        text.push_str("Stopped working: their sessions here can't run.\n");
        for t in ended {
            text.push_str(&format!(
                "- {}{}. {}\n",
                t.name,
                t.ended_at.map_or_else(String::new, |at| format!(
                    ": Claude refused it on {}",
                    day(at, now)
                )),
                way_out(t, admin_tokens)
            ));
        }
        text.push('\n');
    }
    if !soon.is_empty() {
        text.push_str("Running out: their sessions here stop then.\n");
        let mut soon: Vec<&Told> = soon.iter().collect();
        soon.sort_by_key(|t| t.runs_out.map(|r| r.0));
        for t in soon {
            let Some((at, estimated)) = t.runs_out else {
                continue;
            };
            text.push_str(&format!(
                "- {}: {}, {}. {}\n",
                t.name,
                when(at, estimated, now),
                in_days(at, now),
                way_out(t, admin_tokens)
            ));
        }
        text.push('\n');
    }
    Some((
        format!("Claude in {workspace}: {}", parts.join(", ")),
        format!(
            "{text}Everyone's, on {workspace}'s People tab:\n{people}\n\nYou get this email each week while a member's token runs out within a month or stopped working; change how {workspace}'s admins hear of them in its settings:\n{settings}"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::UserId;
    use serde_json::json;

    /// Monday November 23, 2026, 12:00 UTC.
    const NOW: Millis = 1_795_435_200_000;

    fn grace() -> UserRef {
        UserRef {
            id: UserId("usr_grace".into()),
            name: "Grace Hopper".into(),
            avatar_url: None,
        }
    }

    fn token(id: &str, created_at: Millis) -> Connection {
        Connection {
            id: id.into(),
            kind: ClaudeConnectionKind::Token,
            active: true,
            provisioned: false,
            created_at,
            answered_at: None,
            used_at: None,
            runs_out: Some(runs_out(None, created_at)),
            ended_at: None,
            plan: None,
        }
    }

    #[test]
    fn knows_a_refusal_from_an_answer() {
        let refusal = json!({"type": "assistant", "error": "authentication_failed",
            "message": {"model": "<synthetic>", "content": [{"type": "text", "text": "Invalid API key · Please run /login"}]}});
        assert!(refused(&refusal));
        assert!(!answered(&refusal));
        let answer =
            json!({"type": "assistant", "message": {"model": "claude-opus-4-6", "content": []}});
        assert!(answered(&answer));
        assert!(!refused(&answer));
        // Claude's other errors are neither: a rate limit means it took the token,
        // but says nothing of the plan's answer.
        let limited = json!({"type": "assistant", "error": "rate_limit", "message": {"model": "<synthetic>"}});
        assert!(!refused(&limited));
        assert!(!answered(&limited));
        assert!(!answered(
            &json!({"type": "assistant", "isApiErrorMessage": true})
        ));
        assert!(!answered(&json!({"type": "result", "subtype": "success"})));
        assert!(!refused(
            &json!({"type": "result", "error": "authentication_failed"})
        ));
    }

    #[test]
    fn a_token_runs_out_a_year_after_it_was_made_or_around_a_year_after_it_came() {
        assert_eq!(runs_out(Some(NOW), 0), (NOW + TOKEN_LIFE_MS, false));
        assert_eq!(runs_out(None, NOW), (NOW + TOKEN_LIFE_MS, true));
    }

    #[test]
    fn refuses_a_day_still_to_come_and_a_token_that_ran_out() {
        assert_eq!(made_at_problem(NOW - 30 * DAY_MS, NOW), None);
        assert_eq!(made_at_problem(NOW + DAY_MS / 2, NOW), None);
        assert!(
            made_at_problem(NOW + 2 * DAY_MS, NOW)
                .unwrap_or_default()
                .contains("still to come")
        );
        assert_eq!(
            made_at_problem(NOW - 400 * DAY_MS, NOW).as_deref(),
            Some(
                "A token made then ran out on October 19: make a new one with `claude setup-token`"
            )
        );
    }

    #[test]
    fn warns_at_30_days_then_7_once_each() {
        let at = NOW + 40 * DAY_MS;
        assert_eq!(warning_due(at, None, NOW), None);
        let at = NOW + 29 * DAY_MS;
        assert_eq!(warning_due(at, None, NOW), Some(30));
        assert_eq!(warning_due(at, Some(NOW), NOW + DAY_MS), None);
        // A week before, again.
        assert_eq!(warning_due(at, Some(NOW), at - 6 * DAY_MS), Some(7));
        assert_eq!(
            warning_due(at, Some(at - 6 * DAY_MS), at - 5 * DAY_MS),
            None
        );
        // Connected a few days before it runs out: the nearer one alone.
        assert_eq!(warning_due(NOW + 3 * DAY_MS, None, NOW), Some(7));
        // Once it ran out, Claude's refusals tell.
        assert_eq!(warning_due(NOW - 1, None, NOW), None);
        // A warning before the token's date moved doesn't count for the new one.
        assert_eq!(warning_due(at, Some(NOW - 90 * DAY_MS), NOW), Some(30));
    }

    #[test]
    fn workspaces_see_no_more_than_the_instance_lets_them() {
        use ClaudeOversight::*;
        assert_eq!(effective(Details, None), Details);
        assert_eq!(effective(Readiness, None), Readiness);
        assert_eq!(effective(Readiness, Some(Details)), Readiness);
        assert_eq!(effective(Details, Some(Off)), Off);
        assert_eq!(effective(Off, Some(Details)), Off);
    }

    #[test]
    fn picks_the_connection_as_sessions_do() {
        let mut ended_one = token("crd_old", NOW - 90 * DAY_MS);
        ended_one.active = false;
        ended_one.ended_at = Some(NOW - DAY_MS);
        let conns = vec![
            token("crd_new", NOW - DAY_MS),
            token("crd_default", NOW - 10 * DAY_MS),
            ended_one,
        ];
        assert_eq!(
            in_use(&conns, Some("crd_default"), None).map(|(c, here)| (c.id.as_str(), here)),
            Some(("crd_default", true))
        );
        // A choice that ended gives way to the default, then the newest.
        assert_eq!(
            in_use(&conns, Some("crd_old"), Some("crd_default")).map(|(c, h)| (c.id.as_str(), h)),
            Some(("crd_default", false))
        );
        assert_eq!(
            in_use(&conns, Some("crd_old"), None).map(|(c, _)| c.id.as_str()),
            Some("crd_new")
        );
        // With none that works, the one that ended.
        let only_ended = &conns[2..];
        assert!(in_use(only_ended, None, None).is_none());
        assert_eq!(
            ended(only_ended, None, None).map(|c| c.id.as_str()),
            Some("crd_old")
        );
        assert!(ended(&[], None, None).is_none());
    }

    #[test]
    fn shows_admins_as_much_as_they_may_see() {
        let mut c = token("crd_1", NOW - 340 * DAY_MS);
        c.provisioned = true;
        c.answered_at = Some(NOW - 2 * 3_600_000);
        c.plan = Some("team".into());
        let pauses = vec![
            Pause {
                at: NOW - DAY_MS,
                limit: Some("seven_day".into()),
                resets_at: Some(NOW + 2 * DAY_MS),
            },
            Pause {
                at: NOW - 3 * DAY_MS,
                limit: Some("five_hour".into()),
                resets_at: Some(NOW - 3 * DAY_MS + 3_600_000),
            },
            Pause {
                at: NOW - 20 * DAY_MS,
                limit: Some("five_hour".into()),
                resets_at: None,
            },
        ];
        let conns = vec![c];
        let m = |o, p| {
            member(
                grace(),
                "grace@acme.dev".into(),
                &conns,
                None,
                Some("crd_1"),
                &pauses,
                o,
                p,
                NOW,
            )
        };
        let all = m(ClaudeOversight::Details, true);
        assert_eq!(all.state, ClaudeState::Works);
        assert_eq!(all.runs_out_at, Some(NOW + 25 * DAY_MS));
        assert!(all.runs_out_estimated);
        assert_eq!(all.paused_until, Some(NOW + 2 * DAY_MS));
        assert_eq!(all.paused_limit.as_deref(), Some("seven_day"));
        let conn = all.connection.clone().unwrap_or_else(|| unreachable!());
        assert!(conn.set_by_admin_token);
        assert!(!conn.chosen_here);
        assert_eq!(conn.plan.as_deref(), Some("team"));
        assert_eq!(
            all.pauses,
            Some(LimitPauses {
                week: 2,
                month: 3,
                last_at: Some(NOW - DAY_MS),
                last_limit: Some("seven_day".into()),
            })
        );
        // Without the pauses.
        assert_eq!(m(ClaudeOversight::Details, false).pauses, None);
        // Whether their sessions can run, and when that ends, alone.
        let ready = m(ClaudeOversight::Readiness, true);
        assert_eq!(ready.state, ClaudeState::Works);
        assert_eq!(ready.runs_out_at, Some(NOW + 25 * DAY_MS));
        assert_eq!(ready.paused_until, Some(NOW + 2 * DAY_MS));
        assert_eq!(ready.connection, None);
        assert_eq!(ready.pauses, None);
        // Nothing.
        let off = m(ClaudeOversight::Off, true);
        assert_eq!(off.state, ClaudeState::None);
        assert_eq!(off.runs_out_at, None);
        assert_eq!(off.paused_until, None);
    }

    #[test]
    fn says_who_has_none_whose_never_ran_and_whose_ended() {
        let m = |conns: &[Connection]| {
            member(
                grace(),
                "grace@acme.dev".into(),
                conns,
                None,
                None,
                &[],
                ClaudeOversight::Details,
                true,
                NOW,
            )
        };
        let none = m(&[]);
        assert_eq!(none.state, ClaudeState::None);
        assert_eq!(none.connection, None);
        assert_eq!(none.pauses, None);
        assert_eq!(m(&[token("crd_1", NOW)]).state, ClaudeState::Unused);
        let mut gone = token("crd_1", NOW - 100 * DAY_MS);
        gone.active = false;
        gone.ended_at = Some(NOW - 3 * DAY_MS);
        let ended = m(&[gone]);
        assert_eq!(ended.state, ClaudeState::Ended);
        assert_eq!(ended.ended_at, Some(NOW - 3 * DAY_MS));
        let mut login = token("crd_2", NOW);
        login.kind = ClaudeConnectionKind::Login;
        login.runs_out = None;
        login.used_at = Some(NOW);
        let works = m(&[login]);
        assert_eq!(works.state, ClaudeState::Works);
        assert_eq!(works.runs_out_at, None);
    }

    #[test]
    fn names_days_in_utc_with_the_year_when_it_isnt_this_one() {
        assert_eq!(day(NOW + 21 * DAY_MS, NOW), "December 14");
        assert_eq!(day(NOW + 41 * DAY_MS, NOW), "January 3, 2027");
        assert_eq!(days_left(NOW + 21 * DAY_MS, NOW), 21);
        assert_eq!(days_left(NOW + 1, NOW), 1);
        assert_eq!(days_left(NOW - 1, NOW), 0);
    }

    #[test]
    fn tells_someone_their_token_runs_out_and_what_to_do() {
        let (subject, text) = expiry_notice(
            "Acme",
            Some("laptop"),
            NOW + 21 * DAY_MS,
            true,
            false,
            "https://fctl.acme.dev/settings?tab=claude",
            NOW,
        );
        assert_eq!(
            subject,
            "Your Claude token for Acme runs out around December 14"
        );
        assert_eq!(
            text,
            "The long-lived Claude token your sessions on Acme run on (laptop) runs out around December 14, in 21 days. That is a year after it was connected: fctl can't read the date from the token, so it may be sooner.\n\nMake a new one before then, or your sessions stop: run `claude setup-token` on any machine with Claude Code, and paste what it prints in Settings → Claude:\nhttps://fctl.acme.dev/settings?tab=claude"
        );
        let (subject, text) = expiry_notice("Acme", None, NOW + 6 * DAY_MS, false, true, "u", NOW);
        assert_eq!(
            subject,
            "Your Claude token for Acme runs out on November 29"
        );
        assert!(text.starts_with("The long-lived Claude token your sessions on Acme run on runs out on November 29, in 6 days.\n\nYour admins set this token for you"), "{text}");
    }

    #[test]
    fn tells_someone_claude_stopped_taking_it() {
        let (subject, text) = ended_notice("Acme", None, false, false, "u");
        assert_eq!(
            subject,
            "Claude stopped taking your Claude connection on Acme"
        );
        assert!(text.starts_with("Claude refused the long-lived token your sessions on Acme ran on: it was revoked, or it ran out. Sessions can't run on it any more"), "{text}");
        let (_, text) = ended_notice("Acme", Some("Max"), true, false, "u");
        assert!(
            text.starts_with("Claude ended the login your sessions on Acme ran on (Max)."),
            "{text}"
        );
        assert!(text.contains("Sign in again"));
    }

    fn told(name: &str, runs_out: Option<(Millis, bool)>, provisioned: bool) -> Told {
        Told {
            name: name.into(),
            email: format!("{}@acme.dev", name.to_lowercase().replace(' ', ".")),
            runs_out,
            ended_at: runs_out.is_none().then_some(NOW - 3 * DAY_MS),
            provisioned,
        }
    }

    #[test]
    fn tells_admins_of_each_with_the_way_out() {
        let grace = told("Grace Hopper", Some((NOW + 21 * DAY_MS, true)), true);
        let (subject, text) = admin_notice("Acme", &grace, true, "P", "S", NOW);
        assert_eq!(
            subject,
            "Grace Hopper's Claude token runs out around December 14"
        );
        assert!(text.starts_with("Grace Hopper's sessions in Acme run on a long-lived Claude token that runs out around December 14, in 21 days: then they stop. Set them a new one with an admin token, from a token `claude setup-token` printed: fctl admin users set grace.hopper@acme.dev --claude-token -\n\n"), "{text}");
        // Where the instance takes no admin tokens, they connect their own.
        let (_, text) = admin_notice("Acme", &grace, false, "P", "S", NOW);
        assert!(text.contains("They connect a new one in Settings → Claude: fctl told them too."));
        let linus = told("Linus Lee", None, false);
        let (subject, text) = admin_notice("Acme", &linus, true, "P", "S", NOW);
        assert_eq!(subject, "Claude stopped taking Linus Lee's connection");
        assert!(text.starts_with("Claude refused Linus Lee's Claude connection on November 20: their sessions in Acme can't run until they have another."), "{text}");
    }

    #[test]
    fn sums_up_the_week_for_admins_and_says_nothing_when_all_is_well() {
        assert_eq!(digest("Acme", &[], &[], true, "P", "S", NOW), None);
        let soon = vec![
            told("Grace Hopper", Some((NOW + 21 * DAY_MS, true)), false),
            told("Ada Lovelace", Some((NOW + 4 * DAY_MS, false)), false),
        ];
        let ended = vec![told("Linus Lee", None, false)];
        let (subject, text) =
            digest("Acme", &ended, &soon, false, "P", "S", NOW).unwrap_or_default();
        assert_eq!(
            subject,
            "Claude in Acme: 2 tokens run out within a month, 1 connection stopped working"
        );
        assert_eq!(
            text,
            "Stopped working: their sessions here can't run.\n- Linus Lee: Claude refused it on November 20. They connect a new one in Settings → Claude: fctl told them too.\n\nRunning out: their sessions here stop then.\n- Ada Lovelace: on November 27, in 4 days. They connect a new one in Settings → Claude: fctl told them too.\n- Grace Hopper: around December 14, in 21 days. They connect a new one in Settings → Claude: fctl told them too.\n\nEveryone's, on Acme's People tab:\nP\n\nYou get this email each week while a member's token runs out within a month or stopped working; change how Acme's admins hear of them in its settings:\nS"
        );
    }
}
