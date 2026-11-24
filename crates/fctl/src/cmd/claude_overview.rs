//! What `fctl workspaces members --claude` and `fctl admin claude` print:
//! whose Claude connection works, on which plan, whose token runs out soon
//! and whose sessions pause on usage limits (#627). Never a token, its
//! account or its label: the control plane sends none of them.

use crate::style;
use fctl_domain::claude_oversight::{FREQUENT_PAUSES, SOON_DAYS};
use fctl_domain::cron::{DAY_MS, civil_from_days, days_from_civil};
use fctl_proto::Millis;
use fctl_proto::api::{
    ClaudeAdminEmail, ClaudeConnectionKind, ClaudeOversight, ClaudeOversightSettings,
    ClaudeOverview, ClaudeState, MemberClaude,
};

/// How much admins see of people's Claude connections, as the CLI takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Oversight {
    /// Each connection's kind and who set it, its plan, when Claude last
    /// answered it, when its token runs out and, where allowed, how often
    /// its sessions paused on usage limits.
    Details,
    /// Whether each person's sessions can run, when their token runs out
    /// and whether they wait for a usage limit to reset.
    Readiness,
    /// Nothing.
    Off,
}

impl From<Oversight> for ClaudeOversight {
    fn from(o: Oversight) -> Self {
        match o {
            Oversight::Details => Self::Details,
            Oversight::Readiness => Self::Readiness,
            Oversight::Off => Self::Off,
        }
    }
}

/// How a workspace's admins hear by email of members' tokens that run out
/// or stop working, as the CLI takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum AdminEmail {
    /// An email for each, as the member hears: 30 and 7 days before their
    /// token runs out, and as soon as one stops working.
    Each,
    /// One email a week, while any runs out within 30 days or stopped
    /// working (the default).
    Weekly,
    /// Not by email: the People tab and `fctl workspaces members --claude`
    /// show them.
    Off,
}

impl From<AdminEmail> for ClaudeAdminEmail {
    fn from(e: AdminEmail) -> Self {
        match e {
            AdminEmail::Each => Self::Each,
            AdminEmail::Weekly => Self::Weekly,
            AdminEmail::Off => Self::Off,
        }
    }
}

/// `30d`, `2w`, `12h`, or a number of days, in milliseconds: how far
/// ahead `--expiring` looks.
pub fn within(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let wrong = || format!("{s:?} isn't a duration: use 30d, 2w or 12h");
    let n: i64 = n.parse().map_err(|_| wrong())?;
    let per = match unit {
        "" | "d" => DAY_MS,
        "w" => 7 * DAY_MS,
        "h" => 3_600_000,
        _ => return Err(wrong()),
    };
    n.checked_mul(per)
        .filter(|ms| *ms > 0 && *ms <= 400 * DAY_MS)
        .ok_or_else(|| format!("{s:?} is out of range: a token lasts a year at most"))
}

/// A day `claude setup-token` made a token: `YYYY-MM-DD` (midnight UTC),
/// `today` or `yesterday`.
pub fn made_at(s: &str) -> Result<Millis, String> {
    let today = style::now_ms().div_euclid(DAY_MS) * DAY_MS;
    made_at_from(s, today)
}

fn made_at_from(s: &str, today: Millis) -> Result<Millis, String> {
    match s.trim() {
        "today" => return Ok(today),
        "yesterday" => return Ok(today - DAY_MS),
        _ => {}
    }
    let wrong = || {
        format!(
            "{s:?} isn't a day: give the one `claude setup-token` made the token, as YYYY-MM-DD"
        )
    };
    let mut parts = s.trim().split('-');
    let (Some(y), Some(m), Some(d), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(wrong());
    };
    let (y, m, d): (i64, u32, u32) = (
        y.parse().map_err(|_| wrong())?,
        m.parse().map_err(|_| wrong())?,
        d.parse().map_err(|_| wrong())?,
    );
    if y.to_string().len() != 4 || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err(wrong());
    }
    let days = days_from_civil(y, m, d);
    // 2026-02-30 isn't a day either.
    if civil_from_days(days) != (y, m, d) {
        return Err(wrong());
    }
    let at = days * DAY_MS;
    if at > today {
        return Err(
            "That day is still to come: give the day `claude setup-token` made the token".into(),
        );
    }
    Ok(at)
}

/// What to say when the control plane won't list people's Claude
/// connections: who may see them and how it changes, for a `workspace`'s
/// list or the `instance`'s.
pub fn refused(e: fctl_client::Error, scope: &str) -> anyhow::Error {
    use fctl_proto::ErrorCode;
    let Some(api) = e.api() else {
        return e.into();
    };
    match (api.code, scope) {
        (ErrorCode::NotFound, _) => anyhow::anyhow!(
            "This instance doesn't list people's Claude connections yet: its control plane needs a newer fctl."
        ),
        (ErrorCode::Forbidden, "instance") if !api.message.contains("--claude-instance-view") => {
            anyhow::anyhow!(
                "{} Instance admins see everyone's Claude connection once one of them turns it on: `fctl admin set --claude-instance-view on`.",
                api.message.trim_end()
            )
        }
        _ => e.into(),
    }
}

/// Which people to list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// Those whose token runs out within this many milliseconds.
    pub expiring: Option<i64>,
    /// Those whose sessions can't run, or whose token runs out within
    /// `expiring` (30 days without it).
    pub problems: bool,
}

impl Filter {
    fn runs_out_within(m: &MemberClaude, ms: i64, now: Millis) -> bool {
        m.state != ClaudeState::Ended && m.runs_out_at.is_some_and(|at| at - now <= ms)
    }

    pub fn keeps(&self, m: &MemberClaude, now: Millis) -> bool {
        if self.problems {
            let soon = self.expiring.unwrap_or(SOON_DAYS * DAY_MS);
            return matches!(m.state, ClaudeState::None | ClaudeState::Ended)
                || Self::runs_out_within(m, soon, now);
        }
        match self.expiring {
            Some(ms) => Self::runs_out_within(m, ms, now),
            None => true,
        }
    }

    /// The overview with only the people it keeps.
    pub fn apply(&self, mut o: ClaudeOverview, now: Millis) -> ClaudeOverview {
        o.members.retain(|m| self.keeps(m, now));
        o
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// "Dec 14", in UTC, with the year when it isn't `now`'s: "Jan 3, 2027".
pub fn short_day(at: Millis, now: Millis) -> String {
    let (year, month, day) = civil_from_days(at.div_euclid(DAY_MS));
    let (this_year, _, _) = civil_from_days(now.div_euclid(DAY_MS));
    let name = MONTHS
        .get(usize::try_from(month).unwrap_or(1).saturating_sub(1))
        .unwrap_or(&"");
    if year == this_year {
        format!("{name} {day}")
    } else {
        format!("{name} {day}, {year}")
    }
}

/// When a token runs out, for people: "around Dec 14" for an estimate,
/// else "on Dec 14".
pub fn runs_out_words(at: Millis, estimated: bool, now: Millis) -> String {
    format!(
        "{} {}",
        if estimated { "around" } else { "on" },
        short_day(at, now)
    )
}

/// "in 21 days", "in 1 day", "today", or "passed".
pub fn left_words(at: Millis, now: Millis) -> String {
    if at <= now {
        return "passed".into();
    }
    match fctl_domain::claude_oversight::days_left(at, now) {
        1 if at - now < DAY_MS && same_day(at, now) => "today".into(),
        1 => "in 1 day".into(),
        n => format!("in {n} days"),
    }
}

fn same_day(a: Millis, b: Millis) -> bool {
    a.div_euclid(DAY_MS) == b.div_euclid(DAY_MS)
}

/// A plan's name as people write it: "Max", "Pro", "Team", "Ultra".
pub fn plan_name(plan: &str) -> String {
    let mut c = plan.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// What kind of connection it is, in a table cell.
fn kind(m: &MemberClaude) -> Option<&'static str> {
    let c = m.connection.as_ref()?;
    Some(match c.kind {
        ClaudeConnectionKind::Token if c.set_by_admin_token => "token, set by admin token",
        ClaudeConnectionKind::Token => "token",
        ClaudeConnectionKind::Login => "login",
        ClaudeConnectionKind::OnRunners => "token on runners",
    })
}

/// "in 45m", "in 2h 10m", "in 3d 4h", as of `now`.
fn until_from(at: Millis, now: Millis) -> String {
    let m = ((at - now) / 60_000).max(1);
    match m {
        0..=59 => format!("in {m}m"),
        60..=1439 => format!("in {}h {}m", m / 60, m % 60),
        _ => format!("in {}d {}h", m / 1440, m % 1440 / 60),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Plain,
    Dim,
    Amber,
    Red,
}

#[derive(Clone, Debug)]
struct Cell {
    text: String,
    tone: Tone,
}

fn cell(text: impl Into<String>, tone: Tone) -> Cell {
    Cell {
        text: text.into(),
        tone,
    }
}

fn paint(c: &Cell) -> String {
    match c.tone {
        Tone::Plain => c.text.clone(),
        Tone::Dim => style::dim(&c.text),
        Tone::Amber => style::amber(&c.text),
        Tone::Red => style::red(&c.text),
    }
}

/// Where the people are, for the words around the table.
#[derive(Clone, Copy, Debug)]
pub struct Place<'a> {
    /// The workspace's name; None for the whole instance.
    pub workspace: Option<&'a str>,
    /// Its slug, for the commands that change what its admins see.
    pub slug: Option<&'a str>,
}

impl Place<'_> {
    /// " here", in a workspace.
    fn here(&self) -> &'static str {
        if self.workspace.is_some() {
            " here"
        } else {
            ""
        }
    }
}

fn people(n: usize) -> String {
    style::count(n as u64, "person", "people")
}

fn peoples(n: usize) -> String {
    if n == 1 {
        "1 person's".into()
    } else {
        format!("{n} people's")
    }
}

/// The table, and what it means below it, line by line; `filter` is what
/// narrowed `o`'s list, for what an empty one says.
pub fn lines(o: &ClaudeOverview, place: Place<'_>, filter: Filter, now: Millis) -> Vec<String> {
    let mut out = Vec::new();
    if o.oversight == ClaudeOversight::Off {
        out.push(match (place.workspace, place.slug) {
            (Some(name), Some(slug)) => format!(
                "{name}'s admins see nothing of its members' Claude connections. An admin turns it on with `fctl workspaces set -w {slug} --claude-oversight details` (or readiness), where the instance lets them."
            ),
            _ => "Instance admins see nothing of people's Claude connections here.".into(),
        });
        return out;
    }
    let here = place.here();
    if o.members.is_empty() {
        let within = filter.expiring.unwrap_or(SOON_DAYS * DAY_MS) / DAY_MS;
        out.push(if filter.problems {
            format!(
                "Nobody's Claude connection{here} needs anything: every one works, and none runs out within {}.",
                style::count(within as u64, "day", "days")
            )
        } else if filter.expiring.is_some() {
            format!(
                "Nobody's token runs out within {}.",
                style::count(within as u64, "day", "days")
            )
        } else {
            "Nobody to show.".into()
        });
        return out;
    }
    let details = o.oversight >= ClaudeOversight::Details;
    let paused = o.members.iter().any(|m| m.paused_until.is_some());
    let mut header = vec!["NAME"];
    if place.workspace.is_none() {
        header.push("EMAIL");
    }
    header.push("CLAUDE");
    if details {
        header.extend(["PLAN", "ANSWERED"]);
    }
    header.push("RUNS OUT");
    if details && o.limit_pauses {
        header.push("LIMIT PAUSES (7D)");
    }
    if paused {
        header.push("WAITS FOR");
    }
    let mut rows: Vec<Vec<Cell>> = vec![header.iter().map(|h| cell(*h, Tone::Dim)).collect()];
    for m in &o.members {
        let mut row = vec![cell(style::plain(&m.user.name), Tone::Plain)];
        if place.workspace.is_none() {
            row.push(cell(style::plain(&m.email), Tone::Plain));
        }
        let ended_on = m
            .ended_at
            .map(|at| format!("ended {}", short_day(at, now)))
            .unwrap_or_else(|| "ended".into());
        row.push(match (m.state, details) {
            (ClaudeState::None, _) => cell("none", Tone::Amber),
            (ClaudeState::Ended, true) => cell(
                kind(m).map_or(ended_on.clone(), |k| format!("{k}, {ended_on}")),
                Tone::Red,
            ),
            (ClaudeState::Ended, false) => cell(ended_on, Tone::Red),
            (_, true) => cell(kind(m).unwrap_or("connected"), Tone::Plain),
            (ClaudeState::Unused, false) => cell("not used yet", Tone::Plain),
            (ClaudeState::Works, false) => cell("works", Tone::Plain),
        });
        if details {
            let c = m.connection.as_ref();
            row.push(match c.and_then(|c| c.plan.as_deref()) {
                Some(p) => cell(plan_name(&style::plain(p)), Tone::Plain),
                None => cell("-", Tone::Dim),
            });
            row.push(match (c.and_then(|c| c.answered_at), m.state) {
                (Some(at), _) => cell(style::ago_from(at, now), Tone::Plain),
                (None, ClaudeState::Unused) => cell("not yet", Tone::Dim),
                (None, _) => cell("-", Tone::Dim),
            });
        }
        row.push(match m.runs_out_at {
            Some(at) if m.state != ClaudeState::Ended => {
                let text = format!(
                    "{}{} ({})",
                    if m.runs_out_estimated { "~" } else { "" },
                    short_day(at, now),
                    match left_words(at, now).as_str() {
                        "passed" => "passed".to_owned(),
                        "today" => "today".to_owned(),
                        _ => format!("{}d", fctl_domain::claude_oversight::days_left(at, now)),
                    }
                );
                let tone = if at <= now {
                    Tone::Red
                } else if fctl_domain::claude_oversight::soon(at, now) {
                    Tone::Amber
                } else {
                    Tone::Plain
                };
                cell(text, tone)
            }
            _ => cell("-", Tone::Dim),
        });
        if details && o.limit_pauses {
            row.push(match &m.pauses {
                Some(p) if p.week >= FREQUENT_PAUSES => cell(p.week.to_string(), Tone::Amber),
                Some(p) => cell(p.week.to_string(), Tone::Plain),
                None => cell("-", Tone::Dim),
            });
        }
        if paused {
            row.push(match m.paused_until {
                Some(at) => cell(
                    format!(
                        "{}, resets {}",
                        fctl_domain::usage::label(&style::plain(
                            m.paused_limit.as_deref().unwrap_or("usage")
                        )),
                        until_from(at, now)
                    ),
                    Tone::Amber,
                ),
                None => cell("", Tone::Plain),
            });
        }
        rows.push(row);
    }
    let columns = rows.first().map_or(0, Vec::len);
    let widths: Vec<usize> = (0..columns)
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .map(|c| c.text.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for row in &rows {
        let mut line = String::new();
        for (i, c) in row.iter().enumerate() {
            line.push_str(&paint(c));
            if i + 1 < row.len() {
                let pad = widths.get(i).copied().unwrap_or(0) - c.text.chars().count();
                line.push_str(&" ".repeat(pad + 2));
            }
        }
        out.push(line.trim_end().to_owned());
    }
    out.extend(footer(o, place, now));
    out
}

/// What the table means: who can't run sessions, whose token runs out,
/// whose plan pauses them often, and the way out for each.
fn footer(o: &ClaudeOverview, place: Place<'_>, now: Millis) -> Vec<String> {
    let here = place.here();
    let mut out = Vec::new();
    let none = o
        .members
        .iter()
        .filter(|m| m.state == ClaudeState::None)
        .count();
    let ended = o
        .members
        .iter()
        .filter(|m| m.state == ClaudeState::Ended)
        .count();
    let soon = o
        .members
        .iter()
        .filter(|m| {
            m.state != ClaudeState::Ended
                && m.runs_out_at
                    .is_some_and(|at| fctl_domain::claude_oversight::soon(at, now))
        })
        .count();
    let frequent = o
        .members
        .iter()
        .filter(|m| m.pauses.as_ref().is_some_and(|p| p.week >= FREQUENT_PAUSES))
        .count();
    let way_out = if o.admin_tokens {
        "They connect a new one in Settings → Claude, or you set one with an admin token, from a token `claude setup-token` printed: `fctl admin users set EMAIL --claude-token -`."
    } else {
        "They connect a new one in Settings → Claude: fctl tells them too."
    };
    if none > 0 {
        let has = if none == 1 { "has" } else { "have" };
        out.push(style::amber(&match place.workspace {
            Some(name) => format!(
                "{} {has} no Claude connection for {name}: their sessions here can't start.",
                people(none)
            ),
            None => format!(
                "{} {has} no Claude connection: their sessions can't start.",
                people(none)
            ),
        }));
    }
    if ended > 0 {
        out.push(style::red(&format!(
            "{} Claude {} stopped working: their sessions{here} can't run until they have another.",
            peoples(ended),
            if ended == 1 {
                "connection"
            } else {
                "connections"
            }
        )));
    }
    if soon > 0 {
        out.push(style::amber(&format!(
            "{} {} out within {SOON_DAYS} days: their sessions{here} stop then, unless they connect a new one before.",
            peoples(soon),
            if soon == 1 { "token runs" } else { "tokens run" }
        )));
    }
    if ended > 0 || soon > 0 {
        out.push(style::dim(way_out));
    }
    if frequent > 0 {
        out.push(format!(
            "{} sessions paused on usage limits {FREQUENT_PAUSES} times or more this week: their plan's limits are short for this work, and a Max or Ultra seat has more.",
            peoples(frequent)
        ));
    }
    if out.is_empty() {
        out.push(style::dim(&format!(
            "Everyone{here} has a Claude connection that works."
        )));
    }
    if o.oversight == ClaudeOversight::Readiness
        && let (Some(name), Some(slug)) = (place.workspace, place.slug)
    {
        out.push(style::dim(&format!(
            "{name}'s admins see whether each member's sessions can run, not their connection's kind or plan (`fctl workspaces set -w {slug} --claude-oversight details`)."
        )));
    }
    out
}

/// What a workspace's admins see of its members' connections, and how
/// they hear of them, in a line: for `fctl workspaces show`.
pub fn settings_line(s: &ClaudeOversightSettings) -> String {
    let see = match s.oversight {
        ClaudeOversight::Off => "admins see nothing of members' Claude connections".to_owned(),
        ClaudeOversight::Readiness => {
            "admins see whether each member's sessions can run, and when their token runs out"
                .to_owned()
        }
        ClaudeOversight::Details => format!(
            "admins see each member's Claude connection: its kind, plan, last answer and when it runs out{}",
            if s.limit_pauses {
                ", and how often their sessions paused on limits"
            } else {
                ""
            }
        ),
    };
    let hear = match (s.oversight, s.admin_email) {
        (ClaudeOversight::Off, _) => "",
        (_, ClaudeAdminEmail::Off) => "; not by email",
        (_, ClaudeAdminEmail::Each) => "; they hear of each that runs out or stops working",
        (_, ClaudeAdminEmail::Weekly) => "; they hear once a week of those that run out or stopped",
    };
    let never = if s.oversight == ClaudeOversight::Off {
        ""
    } else {
        ", never the token"
    };
    let less = if s.oversight < s.most {
        style::dim(&format!(" (the instance allows {})", s.most.as_str()))
    } else {
        String::new()
    };
    format!("{see}{never}{hear}{less}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::api::{LimitPauses, MemberClaudeConnection};
    use fctl_proto::session::UserRef;

    /// Monday November 23, 2026, 12:00 UTC.
    const NOW: Millis = 1_795_435_200_000;
    const HOUR: i64 = 3_600_000;

    fn member(name: &str, state: ClaudeState) -> MemberClaude {
        MemberClaude {
            user: UserRef {
                id: fctl_proto::UserId(format!("usr_{name}")),
                name: name.into(),
                avatar_url: None,
            },
            email: format!("{}@acme.dev", name.to_lowercase().replace(' ', ".")),
            state,
            runs_out_at: None,
            runs_out_estimated: false,
            ended_at: None,
            paused_until: None,
            paused_limit: None,
            connection: None,
            pauses: None,
        }
    }

    fn connection(kind: ClaudeConnectionKind, plan: Option<&str>) -> MemberClaudeConnection {
        MemberClaudeConnection {
            kind,
            set_by_admin_token: false,
            chosen_here: false,
            plan: plan.map(Into::into),
            connected_at: NOW - 100 * DAY_MS,
            answered_at: None,
            used_at: None,
        }
    }

    fn acme() -> ClaudeOverview {
        let mut ada = member("Ada Lovelace", ClaudeState::Works);
        ada.connection = Some(MemberClaudeConnection {
            answered_at: Some(NOW - 5 * 60_000),
            ..connection(ClaudeConnectionKind::Login, Some("max"))
        });
        ada.pauses = Some(LimitPauses::default());
        let mut grace = member("Grace Hopper", ClaudeState::Works);
        grace.connection = Some(MemberClaudeConnection {
            set_by_admin_token: true,
            answered_at: Some(NOW - 2 * HOUR),
            ..connection(ClaudeConnectionKind::Token, Some("team"))
        });
        grace.runs_out_at = Some(NOW + 21 * DAY_MS);
        grace.runs_out_estimated = true;
        grace.pauses = Some(LimitPauses {
            week: 6,
            month: 9,
            last_at: Some(NOW - DAY_MS),
            last_limit: Some("seven_day".into()),
        });
        let linus = member("Linus Lee", ClaudeState::None);
        ClaudeOverview {
            oversight: ClaudeOversight::Details,
            limit_pauses: true,
            admin_tokens: false,
            members: vec![ada, grace, linus],
        }
    }

    const ACME: Place<'static> = Place {
        workspace: Some("Acme"),
        slug: Some("acme"),
    };

    #[test]
    fn lists_each_members_connection_as_the_issue_draws_it() {
        let lines = lines(&acme(), ACME, Filter::default(), NOW);
        assert_eq!(
            lines,
            vec![
                "NAME          CLAUDE                     PLAN  ANSWERED  RUNS OUT       LIMIT PAUSES (7D)",
                "Ada Lovelace  login                      Max   5m ago    -              0",
                "Grace Hopper  token, set by admin token  Team  2h ago    ~Dec 14 (21d)  6",
                "Linus Lee     none                       -     -         -              -",
                "1 person has no Claude connection for Acme: their sessions here can't start.",
                "1 person's token runs out within 30 days: their sessions here stop then, unless they connect a new one before.",
                "They connect a new one in Settings → Claude: fctl tells them too.",
                "1 person's sessions paused on usage limits 5 times or more this week: their plan's limits are short for this work, and a Max or Ultra seat has more.",
            ]
        );
    }

    #[test]
    fn readiness_shows_only_whether_sessions_can_run() {
        let mut o = acme();
        o.oversight = ClaudeOversight::Readiness;
        for m in &mut o.members {
            m.connection = None;
            m.pauses = None;
        }
        let mut ended = member("Hedy Lamarr", ClaudeState::Ended);
        ended.ended_at = Some(NOW - 3 * DAY_MS);
        o.members.push(ended);
        let mut waiting = member("Joan Clarke", ClaudeState::Unused);
        waiting.paused_until = Some(NOW + 2 * HOUR + 10 * 60_000);
        waiting.paused_limit = Some("five_hour".into());
        o.members.push(waiting);
        let lines = lines(&o, ACME, Filter::default(), NOW);
        assert_eq!(
            lines[..6],
            [
                "NAME          CLAUDE        RUNS OUT       WAITS FOR",
                "Ada Lovelace  works         -",
                "Grace Hopper  works         ~Dec 14 (21d)",
                "Linus Lee     none          -",
                "Hedy Lamarr   ended Nov 20  -",
                "Joan Clarke   not used yet  -              5-hour limit, resets in 2h 10m",
            ]
        );
        assert!(lines.contains(&"1 person's Claude connection stopped working: their sessions here can't run until they have another.".to_owned()));
        assert_eq!(
            lines.last().map(String::as_str),
            Some(
                "Acme's admins see whether each member's sessions can run, not their connection's kind or plan (`fctl workspaces set -w acme --claude-oversight details`)."
            )
        );
    }

    #[test]
    fn says_where_to_turn_it_on_when_admins_see_nothing() {
        let mut o = acme();
        o.oversight = ClaudeOversight::Off;
        o.members.clear();
        assert_eq!(
            lines(&o, ACME, Filter::default(), NOW),
            vec![
                "Acme's admins see nothing of its members' Claude connections. An admin turns it on with `fctl workspaces set -w acme --claude-oversight details` (or readiness), where the instance lets them."
            ]
        );
    }

    #[test]
    fn narrows_to_problems_and_tokens_that_run_out() {
        let all = acme();
        let problems = Filter {
            expiring: None,
            problems: true,
        };
        let names = |o: &ClaudeOverview| {
            o.members
                .iter()
                .map(|m| m.user.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(&problems.apply(all.clone(), NOW)),
            ["Grace Hopper", "Linus Lee"]
        );
        let week = Filter {
            expiring: Some(7 * DAY_MS),
            problems: false,
        };
        let nobody = week.apply(all.clone(), NOW);
        assert!(nobody.members.is_empty());
        assert_eq!(
            lines(&nobody, ACME, week, NOW),
            vec!["Nobody's token runs out within 7 days."]
        );
        let month = Filter {
            expiring: Some(30 * DAY_MS),
            problems: false,
        };
        assert_eq!(names(&month.apply(all.clone(), NOW)), ["Grace Hopper"]);
        // With both, problems look as far ahead as --expiring.
        let both = Filter {
            expiring: Some(7 * DAY_MS),
            problems: true,
        };
        assert_eq!(names(&both.apply(all, NOW)), ["Linus Lee"]);
    }

    #[test]
    fn the_instance_view_names_each_persons_address() {
        let mut o = acme();
        o.admin_tokens = true;
        let place = Place {
            workspace: None,
            slug: None,
        };
        let lines = lines(&o, place, Filter::default(), NOW);
        assert!(
            lines[0].starts_with("NAME          EMAIL                  CLAUDE"),
            "{}",
            lines[0]
        );
        assert!(lines[1].contains("ada.lovelace@acme.dev"));
        assert!(lines.contains(
            &"1 person has no Claude connection: their sessions can't start.".to_owned()
        ));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("`fctl admin users set EMAIL --claude-token -`"))
        );
    }

    #[test]
    fn durations_and_days_read_as_people_type_them() {
        assert_eq!(within("30d"), Ok(30 * DAY_MS));
        assert_eq!(within("2w"), Ok(14 * DAY_MS));
        assert_eq!(within("12h"), Ok(12 * 3_600_000));
        assert_eq!(within("45"), Ok(45 * DAY_MS));
        assert!(within("soon").is_err());
        assert!(within("0d").is_err());
        let today = NOW.div_euclid(DAY_MS) * DAY_MS;
        assert_eq!(made_at_from("today", today), Ok(today));
        assert_eq!(made_at_from("yesterday", today), Ok(today - DAY_MS));
        assert_eq!(made_at_from("2026-11-23", today), Ok(today));
        assert_eq!(
            made_at_from("2025-12-14", today),
            Ok(days_from_civil(2025, 12, 14) * DAY_MS)
        );
        assert!(made_at_from("2026-11-24", today).is_err_and(|e| e.contains("still to come")));
        assert!(made_at_from("2026-02-30", today).is_err_and(|e| e.contains("YYYY-MM-DD")));
        assert!(made_at_from("14/12/2025", today).is_err());
        assert!(made_at_from("26-12-14", today).is_err());
    }

    #[test]
    fn days_read_short_with_the_year_when_it_isnt_this_one() {
        assert_eq!(short_day(NOW + 21 * DAY_MS, NOW), "Dec 14");
        assert_eq!(short_day(NOW + 41 * DAY_MS, NOW), "Jan 3, 2027");
        assert_eq!(
            runs_out_words(NOW + 21 * DAY_MS, true, NOW),
            "around Dec 14"
        );
        assert_eq!(runs_out_words(NOW + 21 * DAY_MS, false, NOW), "on Dec 14");
        assert_eq!(left_words(NOW + 21 * DAY_MS, NOW), "in 21 days");
        assert_eq!(left_words(NOW + DAY_MS, NOW), "in 1 day");
        assert_eq!(left_words(NOW + HOUR, NOW), "today");
        assert_eq!(left_words(NOW - HOUR, NOW), "passed");
        assert_eq!(plan_name("enterprise"), "Enterprise");
    }

    #[test]
    fn a_workspace_says_what_its_admins_see_and_how_they_hear() {
        let s = ClaudeOversightSettings {
            oversight: ClaudeOversight::Details,
            most: ClaudeOversight::Details,
            limit_pauses: true,
            limit_pauses_allowed: true,
            admin_email: ClaudeAdminEmail::Weekly,
        };
        assert_eq!(
            settings_line(&s),
            "admins see each member's Claude connection: its kind, plan, last answer and when it runs out, and how often their sessions paused on limits, never the token; they hear once a week of those that run out or stopped"
        );
        let ready = ClaudeOversightSettings {
            oversight: ClaudeOversight::Readiness,
            admin_email: ClaudeAdminEmail::Off,
            ..s.clone()
        };
        assert_eq!(
            settings_line(&ready),
            "admins see whether each member's sessions can run, and when their token runs out, never the token; not by email (the instance allows details)"
        );
        let off = ClaudeOversightSettings {
            oversight: ClaudeOversight::Off,
            most: ClaudeOversight::Off,
            ..s
        };
        assert_eq!(
            settings_line(&off),
            "admins see nothing of members' Claude connections"
        );
    }
}
