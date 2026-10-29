//! Ticket trackers (`ProjectSettings::tickets`): where a project's agents
//! read and write tickets, none, GitHub Issues or Linear, and what that
//! comes to for its sessions. Without a tracker, agents use none, and their
//! GitHub tokens carry no issues permission. With GitHub Issues, of the
//! repositories they write to, `gh issue` works as it always did; of one
//! repository, their tokens reach no issues unless it is the only one they
//! write to, and `fctl ticket` reaches that one's through the control
//! plane. With Linear, their tokens reach no issues either, and `fctl
//! ticket` reaches the team's tickets through the control plane, which
//! holds the Linear workspace's credentials (`crate::linear`).
//!
//! This is what that decides without I/O: which issues a session's GitHub
//! tokens may touch, how its agent reaches the tracker, what it is told,
//! how it names tickets, and how its replies go back to a ticket.

use crate::validate::Result;
use fctl_proto::ApiError;
use fctl_proto::session::PullRequestRef;
use fctl_proto::session::{
    GithubAccess, LinearTickets, RepoAccess, RepoGrant, RepoRef, SessionTicket, Tickets, Tracker,
};
use fctl_proto::tickets::{StateKind, TicketState, TicketsVia};

/// The longest a label that asks for an exploration may be.
pub const MAX_LABEL: usize = 80;

/// The most tickets `fctl ticket list` shows at once.
pub const MAX_LIST: u32 = 50;

/// How many tickets `fctl ticket list` shows unless asked for more.
pub const DEFAULT_LIST: u32 = 20;

/// The longest title a new ticket may have.
pub const MAX_TITLE: usize = 255;

/// The longest description or comment an agent may write: Linear takes
/// far more, but a ticket is for people to read.
pub const MAX_BODY: usize = 20_000;

/// The most comments a session's agent posts on tickets in an hour, the
/// most tickets it files and changes: an agent that loops would otherwise
/// fill a team's tracker for good.
pub const MAX_COMMENTS_PER_HOUR: u32 = 30;
pub const MAX_CREATED_PER_HOUR: u32 = 20;
pub const MAX_UPDATED_PER_HOUR: u32 = 60;
/// The most a session's agent reads from its tracker in an hour: one Linear
/// installation serves every fctl workspace connected to it, and an agent
/// that loops would use up its share of Linear's API for all of them.
pub const MAX_READS_PER_HOUR: u32 = 300;
/// The most writes of a session's agent that fail in an hour, which every
/// write needs room in: a write that fails doesn't count as one, but it
/// asked the tracker all the same, so an agent looping on one that fails
/// must stop too. What `ticket_writes` calls them.
pub const MAX_FAILED_PER_HOUR: u32 = 60;
pub const FAILED: &str = "failed";

/// What an agent's call to its tracker counts as, for the limits above.
pub fn limit_of(request: &fctl_proto::tickets::TicketRequest) -> (&'static str, u32) {
    use fctl_proto::tickets::TicketRequest as R;
    match request {
        R::Comment { .. } => ("comment", MAX_COMMENTS_PER_HOUR),
        R::Create { .. } => ("create", MAX_CREATED_PER_HOUR),
        R::Update { .. } => ("update", MAX_UPDATED_PER_HOUR),
        _ => ("read", MAX_READS_PER_HOUR),
    }
}

/// Most labels one call files a ticket with, adds or removes.
pub const MAX_LABELS_PER_CALL: usize = 20;

/// Checks the labels a call names before anything reaches the tracker:
/// GitHub takes each label a call removes in a request of its own, so that
/// one call, which counts once, can't send the tracker's API any number of
/// them, nor names no tracker keeps.
pub fn check_labels(request: &fctl_proto::tickets::TicketRequest) -> Result<()> {
    use fctl_proto::tickets::TicketRequest as R;
    let lists: Vec<(&str, &[String])> = match request {
        R::Create { labels, .. } => vec![("labels", labels)],
        R::Update {
            add_labels,
            remove_labels,
            ..
        } => vec![("add_labels", add_labels), ("remove_labels", remove_labels)],
        _ => vec![],
    };
    for (field, labels) in lists {
        if labels.len() > MAX_LABELS_PER_CALL {
            return Err(ApiError::validation(
                field,
                format!("Name at most {MAX_LABELS_PER_CALL} labels at a time"),
            ));
        }
        if labels
            .iter()
            .any(|l| l.chars().count() > MAX_LABEL || l.chars().any(char::is_control))
        {
            return Err(ApiError::validation(
                field,
                format!("A label has at most {MAX_LABEL} characters, and no control characters"),
            ));
        }
    }
    Ok(())
}

/// What a session's agent hears when it reached a limit above.
pub fn limited(kind: &str, most: u32) -> String {
    match kind {
        FAILED => format!(
            "This session's agent's writes to its tracker failed {most} times in the last hour, which is as many as fctl lets an agent: say in your reply what's left to do in the tracker, and what went wrong."
        ),
        "read" => format!(
            "This session's agent has read its tracker {most} times in the last hour, which is as often as fctl lets an agent: work with what you have read, and say in your reply what's left to look up."
        ),
        _ => format!(
            "This session's agent has done that {most} times in the last hour, which is as often as fctl lets an agent: say in your reply what's left to do in the tracker."
        ),
    }
}

// ───────────────────────────── GitHub's issues ─────────────────────────────

/// Which GitHub issues a project's sessions reach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GithubIssues {
    /// None: the project uses another tracker, or none, or the repository
    /// it chose is no longer one its agents write to.
    None,
    /// Those of the repositories its agents write to (and they read those
    /// of the others they read), as before projects chose a tracker.
    Writable,
    /// Those of this repository, one its agents write to, and no others.
    One(RepoRef),
}

/// Whether a project's agents write to `repo`: its primary repository, or
/// one it lists as read and write.
pub fn writes_to(primary: Option<&RepoRef>, grants: &[RepoGrant], repo: &RepoRef) -> bool {
    crate::repos::access(primary, grants, repo) == Some(RepoAccess::Write)
}

/// The repositories a project's agents write to, its primary one first.
pub fn writable(primary: Option<&RepoRef>, grants: &[RepoGrant]) -> Vec<RepoRef> {
    let mut out: Vec<RepoRef> = primary.cloned().into_iter().collect();
    for g in grants.iter().filter(|g| g.access == RepoAccess::Write) {
        if let Some(r) = g.repo()
            && !out.iter().any(|o| o.same(&r))
        {
            out.push(r);
        }
    }
    out
}

/// Which GitHub issues a project's sessions reach, from its tracker and
/// its repositories.
pub fn github_issues(t: &Tickets, primary: Option<&RepoRef>, grants: &[RepoGrant]) -> GithubIssues {
    if t.tracker != Tracker::Github {
        return GithubIssues::None;
    }
    match &t.repo {
        None => GithubIssues::Writable,
        Some(r) if writes_to(primary, grants, r) => GithubIssues::One(r.clone()),
        Some(_) => GithubIssues::None,
    }
}

/// Which GitHub issues one of a project's sessions reaches: those its
/// project's tracker gives it, or, when the maintainer who started it let
/// it reach GitHub's issues (`CreateSessionRequest::github_issues`), those
/// of the repositories its agent writes to, whatever the tracker says, as
/// projects that chose none reached: to move them into the project's
/// tracker, say.
pub fn session_issues(
    t: &Tickets,
    primary: Option<&RepoRef>,
    grants: &[RepoGrant],
    asked: bool,
) -> GithubIssues {
    if asked {
        GithubIssues::Writable
    } else {
        github_issues(t, primary, grants)
    }
}

/// Whether a session's two GitHub tokens may touch issues: the one for the
/// repositories its agent writes to (at the level its GitHub access gives
/// it), and the read-only one for the others.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IssueTokens {
    pub write: bool,
    pub read: bool,
}

/// What a session's GitHub tokens may do with issues, given which issues
/// the project reaches and the names of the repositories its write token
/// is for. A token carries one set of permissions for all its
/// repositories, so the issues of one repository go on the write token
/// only when it is the only one there; otherwise agents reach them through
/// the control plane (`fctl ticket`). An agent that only reads gets one
/// token for every repository, which reads issues only where all of them
/// may.
pub fn issue_tokens(
    issues: &GithubIssues,
    write_names: &[String],
    access: GithubAccess,
) -> IssueTokens {
    match issues {
        GithubIssues::None => IssueTokens::default(),
        GithubIssues::Writable => IssueTokens {
            write: true,
            read: true,
        },
        GithubIssues::One(r) if access == GithubAccess::Read => {
            let _ = r;
            IssueTokens::default()
        }
        GithubIssues::One(r) => IssueTokens {
            write: matches!(write_names, [only] if only.eq_ignore_ascii_case(&r.name)),
            read: false,
        },
    }
}

/// How a session's agent reaches its project's tracker.
pub fn via(
    t: &Tickets,
    primary: Option<&RepoRef>,
    grants: &[RepoGrant],
    access: GithubAccess,
) -> TicketsVia {
    match t.tracker {
        Tracker::None => TicketsVia::Nothing,
        Tracker::Linear if t.linear.is_some() => TicketsVia::Fctl,
        Tracker::Linear => TicketsVia::Nothing,
        Tracker::Github => {
            let issues = github_issues(t, primary, grants);
            let names: Vec<String> = writable(primary, grants)
                .into_iter()
                .map(|r| r.name)
                .collect();
            match &issues {
                GithubIssues::None => TicketsVia::Nothing,
                GithubIssues::Writable => TicketsVia::Gh,
                GithubIssues::One(_) if issue_tokens(&issues, &names, access).write => {
                    TicketsVia::Gh
                }
                GithubIssues::One(_) => TicketsVia::Fctl,
            }
        }
    }
}

/// How one of a project's sessions reaches its tracker: as [`via`] says,
/// but with `gh` for GitHub Issues when the session reaches GitHub's
/// issues beyond its tracker ([`session_issues`]), since its tokens then
/// reach those of every repository it writes to, the tracker's among them.
pub fn session_via(
    t: &Tickets,
    primary: Option<&RepoRef>,
    grants: &[RepoGrant],
    access: GithubAccess,
    asked: bool,
) -> TicketsVia {
    match via(t, primary, grants, access) {
        TicketsVia::Fctl if asked && t.tracker == Tracker::Github => TicketsVia::Gh,
        v => v,
    }
}

/// Whether an agent that may do `access` on GitHub writes tickets: one that
/// only reads there reads them alone, in any tracker.
pub fn writes(access: GithubAccess) -> bool {
    access >= GithubAccess::Comment
}

// ───────────────────────────── what a project chose ─────────────────────────────

/// What a project's tracker reaches, in words: "GitHub issues of acme/web",
/// "Linear team ENG (Acme)".
pub fn scope(
    t: &Tickets,
    primary: Option<&RepoRef>,
    grants: &[RepoGrant],
    linear_name: Option<&str>,
) -> String {
    match t.tracker {
        Tracker::None => "No ticket tracker".to_owned(),
        Tracker::Github => match github_issues(t, primary, grants) {
            GithubIssues::None if t.repo.is_some() => format!(
                "GitHub issues of {}, which agents no longer write to",
                t.repo.as_ref().map(RepoRef::full_name).unwrap_or_default()
            ),
            GithubIssues::None => "GitHub issues, of no repository".to_owned(),
            GithubIssues::Writable => {
                let all = writable(primary, grants);
                match all.as_slice() {
                    [only] => format!("GitHub issues of {}", only.full_name()),
                    _ => "GitHub issues of the repositories agents write to".to_owned(),
                }
            }
            GithubIssues::One(r) => format!("GitHub issues of {}", r.full_name()),
        },
        Tracker::Linear => match &t.linear {
            Some(l) => linear_scope(l, linear_name),
            None => "Linear, without a team".to_owned(),
        },
    }
}

/// A Linear team (and project) in words: "Linear team ENG, project Website
/// (Acme)".
pub fn linear_scope(l: &LinearTickets, workspace: Option<&str>) -> String {
    let team = l
        .team_key
        .as_deref()
        .or(l.team_name.as_deref())
        .unwrap_or("of the project");
    let mut s = format!("Linear team {team}");
    if let Some(p) = l.project_name.as_deref().filter(|p| !p.is_empty()) {
        s.push_str(&format!(", project {p}"));
    }
    if let Some(w) = workspace.filter(|w| !w.is_empty()) {
        s.push_str(&format!(" ({w})"));
    }
    s
}

/// Checks and cleans a project's tracker before it is stored: a repository
/// for GitHub Issues of one must be one its agents write to, and a label
/// that asks for an exploration must be one a tracker takes. A Linear team
/// is checked against its connection by the caller.
pub fn check(t: &Tickets, primary: Option<&RepoRef>, grants: &[RepoGrant]) -> Result<Tickets> {
    let mut t = t.clone();
    if let Some(r) = &t.repo
        && t.tracker == Tracker::Github
        && !writes_to(primary, grants, r)
    {
        return Err(ApiError::validation(
            "settings.tickets.repo",
            format!(
                "Agents don't write to {}: choose the project's repository, or one it lists as read and write",
                r.full_name()
            ),
        ));
    }
    if t.tracker == Tracker::Linear && t.linear.is_none() {
        return Err(ApiError::validation(
            "settings.tickets.linear",
            "Choose the Linear team whose tickets this project works on",
        ));
    }
    if let Some(l) = t.linear.as_mut() {
        if let Some(label) = l.explore_label.as_deref() {
            let label = label.trim();
            if label.chars().count() > MAX_LABEL {
                return Err(ApiError::validation(
                    "settings.tickets.linear.explore_label",
                    format!("Use at most {MAX_LABEL} characters"),
                ));
            }
            if label.chars().any(char::is_control) {
                return Err(ApiError::validation(
                    "settings.tickets.linear.explore_label",
                    "Remove control characters",
                ));
            }
            l.explore_label = Some(label.to_owned());
        }
        for (field, v) in [
            ("settings.tickets.linear.organization", &l.organization),
            ("settings.tickets.linear.team", &l.team),
        ] {
            if !is_linear_id(v) {
                return Err(ApiError::validation(field, "Choose a team from the list"));
            }
        }
        if let Some(p) = &l.project
            && !is_linear_id(p)
        {
            return Err(ApiError::validation(
                "settings.tickets.linear.project",
                "Choose a project from the list",
            ));
        }
    }
    Ok(t)
}

/// Whether a project's tracker, as it is saved (`new`, after [`check`]),
/// needs checking with Linear: it takes its tickets from Linear now and
/// didn't before, or from another team. A Linear choice kept while the
/// tracker was something else was never checked, and turning the tracker
/// to Linear alone mustn't make it count unchecked.
pub fn linear_changed(new: &Tickets, old: &Tickets) -> bool {
    new.tracker == Tracker::Linear
        && new.linear.is_some()
        && (old.tracker != Tracker::Linear || new.linear != old.linear)
}

/// Whether `s` looks like one of Linear's ids: a UUID.
pub fn is_linear_id(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

// ───────────────────────────── naming tickets ─────────────────────────────

/// A GitHub issue as people name it: `#12`, `12`, `acme/web#12`, or its
/// URL. Returns the repository when it names one, and the number.
pub fn github_key(input: &str) -> Option<(Option<RepoRef>, u64)> {
    let s = input.trim();
    for prefix in ["https://github.com/", "http://github.com/", "github.com/"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let mut parts = rest.trim_end_matches('/').split('/');
            let (owner, name, kind, number) =
                (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            if kind != "issues" {
                return None;
            }
            let repo = RepoRef::parse(&format!("{owner}/{name}"))?;
            return Some((Some(repo), number.parse().ok().filter(|n| *n > 0)?));
        }
    }
    let (repo, number) = match s.rsplit_once('#') {
        Some(("", n)) => (None, n),
        Some((r, n)) => (Some(RepoRef::parse(r)?), n),
        None => (None, s),
    };
    let number: u64 = number.parse().ok().filter(|n| *n > 0)?;
    Some((repo, number))
}

/// A GitHub issue's key: `acme/web#12`.
pub fn github_issue_key(repo: &RepoRef, number: u64) -> String {
    format!("{}#{number}", repo.full_name())
}

/// A Linear issue as people name it: `ENG-123` (in any case) or its URL
/// (`https://linear.app/acme/issue/ENG-123/…`). Returns its identifier in
/// capitals.
pub fn linear_key(input: &str) -> Option<String> {
    let s = input.trim();
    let id = match s.find("linear.app/") {
        Some(i) => {
            let mut parts = s[i + "linear.app/".len()..].split('/');
            let (_org, kind, id) = (parts.next()?, parts.next()?, parts.next()?);
            if kind != "issue" {
                return None;
            }
            id
        }
        None => s,
    };
    let (team, number) = id.split_once('-')?;
    let ok = !team.is_empty()
        && team.len() <= 10
        && team.chars().all(|c| c.is_ascii_alphanumeric())
        && team.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && !number.is_empty()
        && number.len() <= 9
        && number.bytes().all(|b| b.is_ascii_digit());
    ok.then(|| format!("{}-{number}", team.to_ascii_uppercase()))
}

/// The branch a session that starts from a ticket works on: its number,
/// the ticket's key and its title, `fctl/12-eng-123-fix-the-login-redirect`.
/// The key in the name lets the tracker's GitHub integration (Linear's)
/// link the branch's pull request to the ticket, and move the ticket on as
/// the pull request opens and merges.
pub fn branch(number: u32, key: &str, title: &str) -> String {
    let key = crate::naming::slugify(key, 30);
    let title = crate::naming::slugify(title, 40);
    match (key.is_empty(), title.is_empty()) {
        (true, _) => crate::naming::session_branch(number, title.as_str()),
        (false, true) => format!("fctl/{number}-{key}"),
        (false, false) => format!("fctl/{number}-{key}-{title}"),
    }
}

// ───────────────────────────── what agents are told ─────────────────────────────

/// What a session's agent is told about its project's tracker.
pub struct Told<'a> {
    pub tickets: &'a Tickets,
    pub via: TicketsVia,
    pub primary: Option<&'a RepoRef>,
    pub grants: &'a [RepoGrant],
    /// What its agent may do on GitHub.
    pub access: GithubAccess,
    /// Its tokens reach the GitHub issues of the repositories it writes
    /// to, whatever the tracker says ([`session_issues`]).
    pub github_issues: bool,
    /// The Linear workspace's name, for Linear.
    pub linear_workspace: Option<&'a str>,
    /// The ticket the session started from, and how.
    pub started_from: Option<StartedFrom<'a>>,
}

/// The ticket a session started from: who delegated it, or mentioned fctl
/// on it, whether they asked for an exploration, and what fctl did with it.
pub struct StartedFrom<'a> {
    pub ticket: &'a SessionTicket,
    /// The person, as fctl knows them.
    pub by: &'a str,
    /// They mentioned fctl on it rather than delegate it.
    pub mentioned: bool,
    pub explore: bool,
    /// The state fctl moved it to as the session started.
    pub moved_to: Option<&'a str>,
}

/// The system prompt's lines about the project's tracker: which one, how
/// to reach it, and for a session that started from a ticket, how its
/// replies go back there and what moves the ticket on.
pub fn instructions(t: &Told<'_>) -> String {
    let mut s = String::new();
    let writes = writes(t.access);
    match t.tickets.tracker {
        Tracker::None => s.push_str(&no_tracker(t)),
        Tracker::Github => {
            let issues = github_issues(t.tickets, t.primary, t.grants);
            match (&issues, t.via) {
                (GithubIssues::None, _) | (_, TicketsVia::Nothing) => s.push_str(&no_tracker(t)),
                (GithubIssues::Writable, _) => {
                    let what = writable_words(t);
                    let how = if writes {
                        "read, open and comment on them with `gh issue`"
                    } else {
                        "read them with `gh issue`; you can't write to them in this session"
                    };
                    s.push_str(&format!(
                        "- This project tracks its work in GitHub issues of {what}: {how}. When a pull request does what an issue asks, write `Fixes #12` in its description, which closes the issue as the pull request merges.\n"
                    ));
                }
                (GithubIssues::One(r), via) => {
                    let name = r.full_name();
                    let how = match (via, writes) {
                        (TicketsVia::Gh, true) => {
                            "read, open and comment on them with `gh issue`".to_owned()
                        }
                        (TicketsVia::Gh, false) => "read them with `gh issue`".to_owned(),
                        (_, true) => format!(
                            "gh can't reach issues in this session, so read, open and comment on {name}'s with `fctl ticket` (below)"
                        ),
                        (_, false) => format!(
                            "gh can't reach issues in this session, so read {name}'s with `fctl ticket` (below); you can't write to them"
                        ),
                    };
                    let others = if t.github_issues {
                        format!(
                            "The person who started this session let it reach the issues of {} too: use other repositories' only as the task asks (to move them to {name}, say).",
                            writable_words(t)
                        )
                    } else {
                        "Leave the issues of other repositories alone, and don't refer to them in commits and pull requests.".to_owned()
                    };
                    s.push_str(&format!(
                        "- This project tracks its work in GitHub issues of {name}, and only there: {how}. {others} When a pull request does what an issue of {name} asks, write `Fixes {name}#12` in its description, which closes the issue as the pull request merges.\n"
                    ));
                }
            }
        }
        Tracker::Linear => match &t.tickets.linear {
            None => s.push_str(&no_tracker(t)),
            Some(l) if t.via == TicketsVia::Nothing => {
                let github = if t.github_issues {
                    beyond(t, "Linear")
                } else {
                    format!("Leave GitHub's issues alone too. {ASK_FOR_ISSUES}")
                };
                s.push_str(&format!(
                    "- This project tracks its work in Linear, in {}, but fctl can't reach it now: its workspace no longer works in that Linear workspace. Leave its tickets alone, and say in your reply what you would do in the tracker. {github}\n",
                    linear_scope(l, t.linear_workspace)
                ));
            }
            Some(l) => {
                let scope = linear_scope(l, t.linear_workspace);
                let how = if writes {
                    "read and write its tickets with `fctl ticket` (below), which acts as fctl's agent in Linear"
                } else {
                    "read its tickets with `fctl ticket` (below); you can't write to them in this session"
                };
                let github = if t.github_issues {
                    format!(
                        "Refer to tickets by their keys (`ENG-123`). {}",
                        beyond(t, "Linear")
                    )
                } else {
                    format!(
                        "Leave GitHub's issues alone (its tokens here can't reach them), and refer to tickets by their keys (`ENG-123`). {ASK_FOR_ISSUES}"
                    )
                };
                let moving = if writes && t.started_from.is_none() {
                    own_moves(l)
                } else {
                    String::new()
                };
                s.push_str(&format!(
                    "- This project tracks its work in Linear, in {scope}: {how}. {github}{moving}\n"
                ));
            }
        },
    }
    if let Some(from) = &t.started_from {
        s.push_str(&started_from(t, from));
    }
    if t.via == TicketsVia::Fctl {
        s.push_str(&fctl_ticket(t.tickets.tracker, writes));
    }
    s
}

/// The repositories whose issues a session reaches through `gh`, in words.
fn writable_words(t: &Told<'_>) -> String {
    match writable(t.primary, t.grants).as_slice() {
        [only] => only.full_name(),
        _ => "the repositories you can write to".to_owned(),
    }
}

/// What a session that reaches GitHub's issues beyond its tracker
/// ([`session_issues`]) is told about them, for a project that tracks its
/// work in `tracker`.
fn beyond(t: &Told<'_>, tracker: &str) -> String {
    let how = if writes(t.access) {
        "read, open, comment on, label and close them with `gh issue`"
    } else {
        "read them with `gh issue`"
    };
    format!(
        "The person who started this session let it reach the GitHub issues of {} as well: {how}, only as the task asks (to move them into {tracker}, say).",
        writable_words(t)
    )
}

/// What a session in a project without a tracker is told.
fn no_tracker(t: &Told<'_>) -> String {
    if t.github_issues {
        format!(
            "- This project uses no ticket tracker. {} Leave other trackers' tickets alone.\n",
            beyond(t, "a tracker")
        )
    } else {
        format!("{NO_TRACKER} {ASK_FOR_ISSUES}\n")
    }
}

const NO_TRACKER: &str = "- This project uses no ticket tracker. Don't read, open, comment on, label or close issues or tickets, on GitHub or anywhere else, even when other instructions say to, and leave issue references out of commits and pull requests (no `Fixes #12`): this session's GitHub tokens can't reach issues. When the task names a ticket, say that this project keeps its agents away from ticket trackers.";

/// What a session whose tokens reach no GitHub issues is told, so that a
/// task that needs them ends with a way to them.
const ASK_FOR_ISSUES: &str = "When a task needs GitHub's issues (to move them into a tracker, say), say that a maintainer of the project can start a session that reaches them: with `fctl run --github-issues`, or \"GitHub issues too\" in the start form's GitHub menu.";

/// What a session that started from a ticket is told about it.
fn started_from(t: &Told<'_>, from: &StartedFrom<'_>) -> String {
    let key = &from.ticket.key;
    let mut s = String::new();
    match from.ticket.tracker {
        Tracker::Linear => {
            let how = if from.mentioned {
                format!("where {} mentioned fctl in Linear", from.by)
            } else {
                format!("which {} delegated to fctl in Linear", from.by)
            };
            s.push_str(&format!(
                "- This session started from {key}, {how}. Your last message of each turn goes to the ticket's thread in Linear as your reply: write it for the people there, in a few short paragraphs, saying what you did, what's left and the pull request's link, without the details the session already shows. When you need someone to decide something, ask it at the end of that message, ending with a question mark; their answers in the thread come to you here, from people fctl knows.\n"
            ));
            if from.explore {
                s.push_str(&format!(
                    "- They asked for an exploration ({key} has the label that asks for one): find out what it needs (read the code, and run it if that helps), then reply with what you found and a plan. Change no files and open no pull request until someone replies to go ahead.\n"
                ));
            }
            if t.access.pushes() {
                s.push_str(&format!(
                    "- Your branch is named after {key}, which links its pull request to the ticket. Write `Fixes {key}` in the pull request's description when it does what the ticket asks, or `Part of {key}` when it does part of it.{}\n",
                    moves(t, from)
                ));
            }
        }
        Tracker::Github | Tracker::None => s.push_str(&format!(
            "- This session started from {key}, which {} asked fctl to work on.\n",
            from.by
        )),
    }
    s
}

/// What a session that started from a Linear ticket is told of how the
/// ticket moves on: what fctl does as its project lets it
/// (`LinearTickets::move_started` and the rest), and what is left to the
/// agent.
fn moves(t: &Told<'_>, from: &StartedFrom<'_>) -> String {
    let key = &from.ticket.key;
    let on = |f: fn(&LinearTickets) -> bool| t.tickets.linear.as_ref().is_some_and(f);
    let mut steps = Vec::new();
    if from.mentioned && on(LinearTickets::moves_started) {
        steps.push("to In Progress once you change something");
    }
    if on(LinearTickets::moves_review) {
        steps.push("to In Review when a turn ends with your pull request up");
    }
    if on(LinearTickets::moves_done) {
        steps
            .push("to Done once the pull requests that complete it merge (`Fixes`, not `Part of`)");
    }
    let steps = match steps.as_slice() {
        [] => None,
        [one] => Some((*one).to_owned()),
        [rest @ .., last] => Some(format!("{}, and {last}", rest.join(", "))),
    };
    let mut s = match (from.moved_to, steps) {
        (Some(state), Some(steps)) => format!(
            " fctl moved the ticket to {state} as the session started, and moves it on in Linear as you go: {steps}, or to the states its team has for them. Leave its state alone unless you are asked."
        ),
        (None, Some(steps)) => format!(
            " fctl moves the ticket on in Linear as you go: {steps}, or to the states its team has for them. Leave its state alone unless you are asked."
        ),
        (moved, None) => format!(
            "{} Where the Linear workspace has Linear's GitHub integration, it moves the ticket on as the pull request opens and merges: leave its state alone unless you are asked.",
            moved
                .map(|state| format!(" fctl moved the ticket to {state} as the session started."))
                .unwrap_or_default()
        ),
    };
    if on(LinearTickets::moves_done) && writes(t.access) {
        s.push_str(&format!(
            " When the ticket needs no pull request and you have done all it asks, move it to Done yourself: `fctl ticket update {key} --state Done` (`fctl ticket states` lists its team's states)."
        ));
    }
    s
}

/// What a session that didn't start from a ticket is told of moving the
/// tickets it works on, as the project lets fctl move tickets: fctl doesn't
/// know which those are, so the agent moves them.
fn own_moves(l: &LinearTickets) -> String {
    let mut steps = Vec::new();
    if l.moves_started() {
        steps.push("to In Progress as you start, unless it has started");
    }
    if l.moves_review() {
        steps.push("to In Review once your pull requests are up");
    }
    if l.moves_done() {
        steps.push("to Done once you have done all it asks, when it needs no pull request");
    }
    let steps = match steps.as_slice() {
        [] => return String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    };
    format!(
        " When your task is to do a ticket's work, move the ticket on as you go with `fctl ticket update`: {steps}. Write `Fixes ENG-123` in a pull request that does what the ticket asks, or `Part of ENG-123` when it does part of it."
    )
}

/// How to use `fctl ticket`, for agents that reach their tracker with it.
fn fctl_ticket(tracker: Tracker, writes: bool) -> String {
    let key = match tracker {
        Tracker::Github => "#12",
        _ => "ENG-123",
    };
    let mut s = format!(
        "\n`fctl ticket` reaches the project's tickets through fctl, which talks to this session directly:\n\n- `fctl ticket` says what it reaches and which tickets this session works on; `fctl ticket list` lists open tickets (`--closed`, `--all`, `--search WORDS`, `--label NAME`), and `fctl ticket view {key}` shows one with its comments.\n"
    );
    if writes {
        s.push_str(&format!(
            "- `fctl ticket comment {key} \"…\"` comments (`--reply-to ID` answers a comment); `fctl ticket create --title \"…\" --description \"…\"` files a ticket (`--parent {key}` makes it a sub-ticket); `fctl ticket update {key} --state \"In Review\" --add-label bug` changes one; `fctl ticket states` and `fctl ticket labels` list what they can be.\n- Comment when people there need to know something: a question only they can answer, a decision, what changed. Don't narrate your progress: the session shows it, and fctl turns away an agent that comments too often.\n"
        ));
    }
    s.push_str(
        "- If `fctl ticket` fails, say in your reply what you meant to do in the tracker.\n",
    );
    s
}

// ───────────────────────────── replies ─────────────────────────────

/// The longest reply fctl posts to a ticket's thread for the agent: a
/// longer one ends with a link to the rest, in the session.
pub const MAX_REPLY: usize = 3_000;

/// What a reply asks of people.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyKind {
    /// Here is what the agent did: the work is done, or waits.
    Response,
    /// The agent asks something, and waits for an answer.
    Question,
}

/// The agent's last message of a turn as fctl posts it to the ticket's
/// thread: at most [`MAX_REPLY`] characters, cut at a paragraph or a line
/// where it can, with a link to the session for the rest; and whether it
/// asks something (its last line ends with a question mark). `None` for an
/// empty message.
pub fn reply(text: &str, session_url: &str) -> Option<(ReplyKind, String)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let asks = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .is_some_and(|l| {
            l.trim_end_matches(['*', '_', ')', '"', '\''])
                .ends_with('?')
        });
    let kind = if asks {
        ReplyKind::Question
    } else {
        ReplyKind::Response
    };
    if text.chars().count() <= MAX_REPLY {
        return Some((kind, text.to_owned()));
    }
    let cut: String = text.chars().take(MAX_REPLY).collect();
    let at = cut
        .rfind("\n\n")
        .filter(|i| *i > MAX_REPLY / 2)
        .or_else(|| cut.rfind('\n').filter(|i| *i > MAX_REPLY / 2))
        .unwrap_or(cut.len());
    let mut body = cut[..at].trim_end().to_owned();
    body.push_str(&format!(
        "\n\n… [The rest is in the session]({session_url})."
    ));
    Some((ReplyKind::Response, body))
}

// ───────────────────────────── moving tickets on ─────────────────────────────

/// How far a session's work on its ticket has come: what fctl moves the
/// ticket on for, in the order tickets go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Progress {
    /// It works on the ticket: the ticket was delegated to it, or its
    /// agent changed something.
    Started,
    /// A pull request of its is up for review: open, and not a draft.
    Review,
    /// Its pull requests merged: one did, and none is open any more.
    Merged,
}

/// How far a session has come, as its pull requests and whether its agent
/// changed anything show: `None` while it has neither. A pull request
/// whose state nobody looked up yet is still open, but not known to be up
/// for review: it may be a draft.
pub fn progress(prs: &[PullRequestRef], changed: bool) -> Option<Progress> {
    let state = |p: &PullRequestRef| p.state.clone().unwrap_or_default();
    let open: Vec<String> = prs
        .iter()
        .map(state)
        .filter(|s| !matches!(s.as_str(), "merged" | "closed"))
        .collect();
    if open.is_empty() && prs.iter().any(|p| state(p) == "merged") {
        Some(Progress::Merged)
    } else if open.iter().any(|s| matches!(s.as_str(), "open" | "queued")) {
        Some(Progress::Review)
    } else {
        (changed || !open.is_empty()).then_some(Progress::Started)
    }
}

/// Whether to see if a session's ticket is done, now that what fctl knows
/// of its pull requests went from `before` to `now` (as [`progress`] has
/// them): once none is open any more, one merged among them, or once one
/// more merged (`merged_more`) while none is.
pub fn done_due(before: Option<Progress>, now: Option<Progress>, merged_more: bool) -> bool {
    now == Some(Progress::Merged) && (before != Some(Progress::Merged) || merged_more)
}

/// How long fctl waits before it sees again whether a session's ticket is
/// done, after GitHub or Linear couldn't say: after each try in turn, and
/// then no more.
pub const DONE_AGAIN_IN_MS: [i64; 4] = [60_000, 10 * 60_000, 60 * 60_000, 6 * 60 * 60_000];

/// How far an agent's report moves its session's ticket on, from `before`
/// to `now` (as [`progress`] has them): to started once it changed
/// something, at once, and to review once a pull request is up, but only
/// after its turn (`turn_over`), as a turn's end moves it too. Merges
/// move it on their own, as GitHub says or a turn ends.
pub fn reported(
    before: Option<Progress>,
    now: Option<Progress>,
    turn_over: bool,
) -> Option<Progress> {
    match now {
        _ if now <= before => None,
        Some(Progress::Review) if turn_over => Some(Progress::Review),
        Some(Progress::Started | Progress::Review) if before.is_none() => Some(Progress::Started),
        _ => None,
    }
}

/// The state among a team's workflow `states` (in order: by kind, then
/// position) that a ticket in `current` moves to for `progress`: its
/// first started state (In Progress) once work starts, as Linear asks of
/// agents; its review state (In Review: the first started state whose
/// name says review, or else its first started one) once a pull request is
/// up; and its first completed state (Done) once they merged. `None`
/// leaves it where it is: done or canceled, there or further along
/// already, or in a workflow without such a state.
pub fn next_state<'a>(
    progress: Progress,
    current: Option<&TicketState>,
    states: &'a [TicketState],
) -> Option<&'a TicketState> {
    if current.is_some_and(|c| !c.kind.is_open()) {
        return None;
    }
    let first = |kind: StateKind| states.iter().find(|s| s.kind == kind);
    let target = match progress {
        Progress::Started => first(StateKind::Started),
        Progress::Review => states
            .iter()
            .find(|s| s.kind == StateKind::Started && says_review(&s.name))
            .or_else(|| first(StateKind::Started)),
        Progress::Merged => first(StateKind::Completed),
    }?;
    let Some(current) = current else {
        return Some(target);
    };
    let place = |s: &TicketState| {
        states.iter().position(|x| match (&x.id, &s.id) {
            (Some(a), Some(b)) => a == b,
            _ => x.name.eq_ignore_ascii_case(&s.name),
        })
    };
    match (place(current), place(target)) {
        (Some(c), Some(t)) => (c < t).then_some(target),
        // A state the workflow doesn't list: by its kind alone.
        _ => (current.kind < target.kind).then_some(target),
    }
}

/// Whether a state's name says review ("In Review", "Code review",
/// "Reviewing"), and not a word that holds it ("Preview").
fn says_review(name: &str) -> bool {
    name.split(|c: char| !c.is_alphanumeric())
        .any(|w| w.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("review")))
}

/// Linear's GitHub integration's words before an issue's key in a pull
/// request (`Fixes ENG-123`): those that complete the issue as it merges,
/// and those that only link it (`Part of ENG-123`, `Related to ENG-123`)
/// or keep it apart (`Skip ENG-123`).
const CLOSING: &[&str] = &[
    "close",
    "closes",
    "closed",
    "closing",
    "fix",
    "fixes",
    "fixed",
    "fixing",
    "resolve",
    "resolves",
    "resolved",
    "resolving",
    "complete",
    "completes",
    "completed",
    "completing",
    "implement",
    "implements",
    "implemented",
    "implementing",
];
const NOT_CLOSING: &[&str] = &[
    "ref",
    "refs",
    "references",
    "part of",
    "contributes to",
    "toward",
    "towards",
    "relates to",
    "related to",
    "skip",
    "ignore",
];

/// Whether a pull request completes ticket `key` (`ENG-123`) as it merges,
/// as Linear's GitHub integration reads it: by the words before the key
/// in its title and description (`Fixes ENG-123` does; `Part of ENG-123`
/// doesn't), and without any, by the key in its title or in its branch,
/// which fctl names after a session's ticket.
pub fn completes(key: &str, title: &str, body: &str, branch: &str) -> bool {
    let words: Vec<Option<bool>> = words_before(title, key)
        .into_iter()
        .chain(words_before(body, key))
        .collect();
    if words.contains(&Some(true)) {
        true
    } else if words.contains(&Some(false)) {
        false
    } else {
        names(title, key) || names(branch, key)
    }
}

/// Whether `text` names ticket `key`, in any case, as a word of its own.
fn names(text: &str, key: &str) -> bool {
    !keys_in(text, key).is_empty()
}

/// Where `text` names `key`, in any case, as a word of its own
/// (`ENG-12` isn't in `ENG-123` or `XENG-12`).
fn keys_in(text: &str, key: &str) -> Vec<usize> {
    let lower = text.to_ascii_lowercase();
    let key = key.trim().to_ascii_lowercase();
    if key.is_empty() {
        return Vec::new();
    }
    let bytes = lower.as_bytes();
    lower
        .match_indices(&key)
        .map(|(at, _)| at)
        .filter(|&at| {
            let before = at.checked_sub(1).and_then(|i| bytes.get(i));
            let after = bytes.get(at + key.len());
            before.is_none_or(|b| !b.is_ascii_alphanumeric())
                && after.is_none_or(|b| !b.is_ascii_alphanumeric())
        })
        .collect()
}

/// What the words before each place `text` names `key` say: `Some(true)`
/// for a closing word (`Fixes ENG-1`, `Fixes ENG-2, ENG-1`, `Fixes
/// https://linear.app/acme/issue/ENG-1/…`), `Some(false)` for another
/// magic word, `None` for none.
fn words_before(text: &str, key: &str) -> Vec<Option<bool>> {
    keys_in(text, key)
        .into_iter()
        .map(|at| {
            let line = text[..at].rsplit('\n').next().unwrap_or_default();
            magic_word(line)
        })
        .collect()
}

/// What the end of `before`, the line before a key, says of it: past the
/// keys and links of a list (`Fixes ENG-1, ENG-2 and `), a closing word
/// (`Some(true)`), another magic word (`Some(false)`), or none.
fn magic_word(before: &str) -> Option<bool> {
    let apart = |c: char| c.is_whitespace() || ",;:&*_`'\"()[]<>#".contains(c);
    let mut rest = before;
    loop {
        rest = rest.trim_end_matches(apart);
        let start = rest
            .char_indices()
            .rev()
            .find(|(_, c)| apart(*c))
            .map_or(0, |(i, c)| i + c.len_utf8());
        let word = &rest[start..];
        let listed = word.eq_ignore_ascii_case("and")
            || is_key(word)
            || word.starts_with("http")
            || word.contains("linear.app/");
        if word.is_empty() || !listed {
            break;
        }
        rest = &rest[..start];
    }
    let rest = rest.to_ascii_lowercase();
    let ends = |phrase: &str| {
        rest.strip_suffix(phrase)
            .is_some_and(|b| b.chars().last().is_none_or(|c| !c.is_alphanumeric()))
    };
    if CLOSING.iter().any(|w| ends(w)) {
        Some(true)
    } else if NOT_CLOSING.iter().any(|w| ends(w)) {
        Some(false)
    } else {
        None
    }
}

/// Whether a word is a ticket's key (`ENG-123`).
fn is_key(word: &str) -> bool {
    let Some((team, number)) = word.rsplit_once('-') else {
        return false;
    };
    team.starts_with(|c: char| c.is_ascii_alphabetic())
        && team.chars().all(|c| c.is_ascii_alphanumeric())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// Whether a ticket's labels ask for an exploration: one of them is the
/// project's label for it, in any case.
pub fn explores(labels: &[String], label: Option<&str>) -> bool {
    label.is_some_and(|l| {
        labels
            .iter()
            .any(|x| x.trim().eq_ignore_ascii_case(l.trim()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_call_counts_within_its_limit() {
        use fctl_proto::tickets::{StateFilter, TicketRequest as R};
        let comment = R::Comment {
            key: "ENG-1".into(),
            body: "On it".into(),
            reply_to: None,
        };
        assert_eq!(limit_of(&comment), ("comment", MAX_COMMENTS_PER_HOUR));
        let list = R::List {
            state: StateFilter::Open,
            query: None,
            label: None,
            limit: None,
        };
        assert_eq!(limit_of(&list), ("read", MAX_READS_PER_HOUR));
        assert_eq!(limit_of(&R::About), ("read", MAX_READS_PER_HOUR));
        assert!(limited("read", 300).contains("read its tracker 300 times"));
        assert!(limited("comment", 30).contains("done that 30 times"));
        assert!(limited(FAILED, 60).contains("failed 60 times"));
    }

    fn r(full: &str) -> RepoRef {
        RepoRef::parse(full).unwrap_or_else(|| RepoRef {
            owner: "x".into(),
            name: "y".into(),
        })
    }

    fn g(name: &str, access: RepoAccess) -> RepoGrant {
        RepoGrant {
            owner: "acme".into(),
            name: Some(name.into()),
            access,
        }
    }

    fn github(repo: Option<&str>) -> Tickets {
        Tickets {
            tracker: Tracker::Github,
            repo: repo.map(r),
            linear: None,
        }
    }

    fn linear() -> Tickets {
        Tickets {
            tracker: Tracker::Linear,
            repo: None,
            linear: Some(LinearTickets {
                organization: "3f1d9d7c-1111-4c2e-9a5b-0d2f6c8e7a10".into(),
                team: "8b2a4c6e-2222-4d3f-8e1a-1c3e5a7b9d20".into(),
                team_key: Some("ENG".into()),
                team_name: Some("Engineering".into()),
                project: None,
                project_name: None,
                move_started: None,
                move_review: None,
                move_done: None,
                explore_label: None,
            }),
        }
    }

    #[test]
    fn a_call_names_a_few_labels_of_a_tracker_size() {
        use fctl_proto::tickets::TicketRequest as R;
        let update = |add: Vec<String>, remove: Vec<String>| R::Update {
            key: "ENG-1".into(),
            title: None,
            description: None,
            state: None,
            add_labels: add,
            remove_labels: remove,
            parent: None,
            priority: None,
        };
        let some = vec!["bug".to_owned(), "needs review".to_owned()];
        assert!(check_labels(&update(some.clone(), some.clone())).is_ok());
        let many: Vec<String> = (0..=MAX_LABELS_PER_CALL).map(|i| format!("l{i}")).collect();
        assert!(check_labels(&update(vec![], many.clone())).is_err());
        assert!(check_labels(&update(many, vec![])).is_err());
        let long = "x".repeat(MAX_LABEL + 1);
        assert!(check_labels(&update(vec![], vec![long])).is_err());
        assert!(check_labels(&update(vec!["a\nb".into()], vec![])).is_err());
        assert!(check_labels(&R::States).is_ok());
    }

    #[test]
    fn a_linear_team_is_checked_whenever_the_tracker_turns_to_linear() {
        let now = linear();
        // A team chosen, or another one: checked.
        assert!(linear_changed(&now, &github(None)));
        let mut other = linear();
        if let Some(l) = other.linear.as_mut() {
            l.team = "5d7e9a1b-3333-4e4f-9b2c-2d4f6b8c0e30".into();
        }
        assert!(linear_changed(&now, &other));
        // The same team kept while the tracker was GitHub's, never checked
        // then, when only the tracker turns to Linear: checked too.
        let kept = Tickets {
            tracker: Tracker::Github,
            ..linear()
        };
        assert!(linear_changed(&now, &kept));
        // Unchanged, or not Linear's: nothing to check.
        assert!(!linear_changed(&now, &linear()));
        assert!(!linear_changed(&kept, &now));
        assert!(!linear_changed(&github(None), &now));
    }

    #[test]
    fn projects_from_before_trackers_reach_the_issues_they_always_did() {
        let t: Tickets = serde_json::from_str("{}").unwrap_or_default();
        assert_eq!(t.tracker, Tracker::Github);
        let primary = r("acme/web");
        let grants = [g("lib", RepoAccess::Write), g("docs", RepoAccess::Read)];
        let issues = github_issues(&t, Some(&primary), &grants);
        assert_eq!(issues, GithubIssues::Writable);
        assert_eq!(
            issue_tokens(&issues, &["web".into(), "lib".into()], GithubAccess::Push),
            IssueTokens {
                write: true,
                read: true
            }
        );
        assert_eq!(
            via(&t, Some(&primary), &grants, GithubAccess::Push),
            TicketsVia::Gh
        );
    }

    #[test]
    fn no_tracker_and_linear_keep_tokens_away_from_issues() {
        let primary = r("acme/web");
        for t in [
            Tickets {
                tracker: Tracker::None,
                ..Default::default()
            },
            linear(),
        ] {
            let issues = github_issues(&t, Some(&primary), &[]);
            assert_eq!(issues, GithubIssues::None);
            for access in GithubAccess::ALL {
                assert_eq!(
                    issue_tokens(&issues, &["web".into()], access),
                    IssueTokens::default()
                );
            }
        }
        assert_eq!(
            via(&linear(), Some(&primary), &[], GithubAccess::Push),
            TicketsVia::Fctl
        );
    }

    #[test]
    fn one_repository_goes_on_the_token_only_when_it_is_the_only_one() {
        let primary = r("acme/web");
        let t = github(Some("acme/web"));
        let issues = github_issues(&t, Some(&primary), &[]);
        assert_eq!(issues, GithubIssues::One(r("acme/web")));
        assert_eq!(
            issue_tokens(&issues, &["web".into()], GithubAccess::Push),
            IssueTokens {
                write: true,
                read: false
            }
        );
        assert_eq!(
            via(&t, Some(&primary), &[], GithubAccess::Push),
            TicketsVia::Gh
        );
        // Another writable repository shares the token: fctl ticket instead.
        let grants = [g("lib", RepoAccess::Write)];
        let issues = github_issues(&t, Some(&primary), &grants);
        assert_eq!(
            issue_tokens(&issues, &["web".into(), "lib".into()], GithubAccess::Push),
            IssueTokens::default()
        );
        assert_eq!(
            via(&t, Some(&primary), &grants, GithubAccess::Push),
            TicketsVia::Fctl
        );
        // An agent that reads has one token for everything.
        assert_eq!(
            issue_tokens(
                &github_issues(&t, Some(&primary), &[]),
                &["web".into()],
                GithubAccess::Read
            ),
            IssueTokens::default()
        );
        assert_eq!(
            via(&t, Some(&primary), &[], GithubAccess::Read),
            TicketsVia::Fctl
        );
    }

    #[test]
    fn a_repository_agents_no_longer_write_to_reaches_nothing() {
        let primary = r("acme/web");
        let t = github(Some("acme/docs"));
        let grants = [g("docs", RepoAccess::Read)];
        assert_eq!(
            github_issues(&t, Some(&primary), &grants),
            GithubIssues::None
        );
        assert_eq!(
            via(&t, Some(&primary), &grants, GithubAccess::Push),
            TicketsVia::Nothing
        );
        assert!(check(&t, Some(&primary), &grants).is_err());
        let grants = [g("docs", RepoAccess::Write)];
        assert!(check(&t, Some(&primary), &grants).is_ok());
        assert!(scope(&github(Some("acme/docs")), Some(&primary), &[], None).contains("no longer"));
    }

    #[test]
    fn linear_needs_a_team_and_cleans_its_label() {
        let t = Tickets {
            tracker: Tracker::Linear,
            ..Default::default()
        };
        assert!(check(&t, None, &[]).is_err());
        let mut t = linear();
        if let Some(l) = t.linear.as_mut() {
            l.explore_label = Some("  Investigate ".into());
        }
        let checked = check(&t, None, &[]).unwrap_or_default();
        assert_eq!(
            checked
                .linear
                .as_ref()
                .and_then(LinearTickets::explore_label),
            Some("Investigate")
        );
        if let Some(l) = t.linear.as_mut() {
            l.team = "ENG".into();
        }
        assert!(check(&t, None, &[]).is_err());
    }

    #[test]
    fn explore_labels_default_and_turn_off() {
        let mut l = linear().linear.unwrap_or_else(|| unreachable!());
        assert_eq!(l.explore_label(), Some("explore"));
        assert!(explores(
            &["Bug".into(), "Explore".into()],
            l.explore_label()
        ));
        l.explore_label = Some(String::new());
        assert_eq!(l.explore_label(), None);
        assert!(!explores(&["explore".into()], l.explore_label()));
    }

    #[test]
    fn names_tickets_as_people_do() {
        assert_eq!(github_key("#12"), Some((None, 12)));
        assert_eq!(github_key("12"), Some((None, 12)));
        assert_eq!(github_key("acme/web#7"), Some((Some(r("acme/web")), 7)));
        assert_eq!(
            github_key("https://github.com/acme/web/issues/9"),
            Some((Some(r("acme/web")), 9))
        );
        assert_eq!(github_key("https://github.com/acme/web/pull/9"), None);
        assert_eq!(github_key("#0"), None);
        assert_eq!(github_key("ENG-1"), None);
        assert_eq!(linear_key("eng-123").as_deref(), Some("ENG-123"));
        assert_eq!(
            linear_key("https://linear.app/acme/issue/ENG-42/fix-login").as_deref(),
            Some("ENG-42")
        );
        assert_eq!(linear_key("ENG-"), None);
        assert_eq!(linear_key("12-34"), None);
        assert_eq!(linear_key("#12"), None);
    }

    #[test]
    fn branches_carry_the_ticket_key() {
        assert_eq!(
            branch(12, "ENG-123", "Fix the login redirect"),
            "fctl/12-eng-123-fix-the-login-redirect"
        );
        assert_eq!(branch(3, "ENG-9", ""), "fctl/3-eng-9");
    }

    #[test]
    fn replies_ask_when_they_end_with_a_question() {
        let url = "https://app.fctl.sh/s/ses_1";
        assert_eq!(reply("  ", url), None);
        let (kind, body) =
            reply("Fixed it in #4.", url).unwrap_or((ReplyKind::Question, String::new()));
        assert_eq!(kind, ReplyKind::Response);
        assert_eq!(body, "Fixed it in #4.");
        let (kind, _) = reply("I found two ways.\n\nWhich do you want, **A or B?**\n", url)
            .unwrap_or((ReplyKind::Response, String::new()));
        assert_eq!(kind, ReplyKind::Question);
        let long = format!("{}\n\n{}", "a".repeat(2_000), "b".repeat(2_000));
        let (kind, body) = reply(&long, url).unwrap_or((ReplyKind::Question, String::new()));
        assert_eq!(kind, ReplyKind::Response);
        assert!(body.starts_with(&"a".repeat(2_000)));
        assert!(body.ends_with("(https://app.fctl.sh/s/ses_1)."));
        assert!(!body.contains('b'));
    }

    fn workflow(states: &[(&str, StateKind)]) -> Vec<TicketState> {
        states
            .iter()
            .map(|(name, kind)| TicketState {
                name: (*name).to_owned(),
                kind: *kind,
                id: Some(format!(
                    "id-{}",
                    name.to_ascii_lowercase().replace(' ', "-")
                )),
            })
            .collect()
    }

    /// A new Linear team's workflow, with triage on.
    fn linears() -> Vec<TicketState> {
        workflow(&[
            ("Triage", StateKind::Triage),
            ("Backlog", StateKind::Backlog),
            ("Todo", StateKind::Unstarted),
            ("In Progress", StateKind::Started),
            ("In Review", StateKind::Started),
            ("Done", StateKind::Completed),
            ("Canceled", StateKind::Canceled),
            ("Duplicate", StateKind::Canceled),
        ])
    }

    fn moved(progress: Progress, from: Option<&str>, states: &[TicketState]) -> Option<String> {
        let current = from.and_then(|f| states.iter().find(|s| s.name == f));
        next_state(progress, current, states).map(|s| s.name.clone())
    }

    #[test]
    fn tickets_start_until_they_started() {
        let states = linears();
        for from in ["Triage", "Backlog", "Todo"] {
            assert_eq!(
                moved(Progress::Started, Some(from), &states).as_deref(),
                Some("In Progress"),
                "{from}"
            );
        }
        assert_eq!(
            moved(Progress::Started, None, &states).as_deref(),
            Some("In Progress")
        );
        for from in ["In Progress", "In Review", "Done", "Canceled", "Duplicate"] {
            assert_eq!(
                moved(Progress::Started, Some(from), &states),
                None,
                "{from}"
            );
        }
        assert_eq!(
            next_state(
                Progress::Started,
                None,
                &workflow(&[("Todo", StateKind::Unstarted)])
            ),
            None
        );
    }

    #[test]
    fn tickets_go_to_review_as_pull_requests_are_up_and_never_back() {
        let states = linears();
        for from in ["Backlog", "Todo", "In Progress"] {
            assert_eq!(
                moved(Progress::Review, Some(from), &states).as_deref(),
                Some("In Review"),
                "{from}"
            );
        }
        for from in ["In Review", "Done", "Canceled"] {
            assert_eq!(moved(Progress::Review, Some(from), &states), None, "{from}");
        }
        // Further along than review: a state after it stays.
        let qa = workflow(&[
            ("Todo", StateKind::Unstarted),
            ("In Progress", StateKind::Started),
            ("Code review", StateKind::Started),
            ("QA", StateKind::Started),
            ("Done", StateKind::Completed),
        ]);
        assert_eq!(
            moved(Progress::Review, Some("In Progress"), &qa).as_deref(),
            Some("Code review")
        );
        assert_eq!(moved(Progress::Review, Some("QA"), &qa), None);
        // Without a review state, a ticket that hasn't started starts.
        let plain = workflow(&[
            ("Todo", StateKind::Unstarted),
            ("Doing", StateKind::Started),
            ("Preview", StateKind::Started),
            ("Done", StateKind::Completed),
        ]);
        assert_eq!(
            moved(Progress::Review, Some("Todo"), &plain).as_deref(),
            Some("Doing")
        );
        assert_eq!(moved(Progress::Review, Some("Doing"), &plain), None);
        assert!(says_review("Reviewing") && says_review("Ready for review"));
        assert!(says_review("PR-review") && !says_review("Preview"));
        assert!(!says_review("Révision") && !says_review("Revi"));
    }

    #[test]
    fn tickets_are_done_once_their_pull_requests_merged() {
        let states = linears();
        for from in ["Todo", "In Progress", "In Review"] {
            assert_eq!(
                moved(Progress::Merged, Some(from), &states).as_deref(),
                Some("Done"),
                "{from}"
            );
        }
        for from in ["Done", "Canceled", "Duplicate"] {
            assert_eq!(moved(Progress::Merged, Some(from), &states), None, "{from}");
        }
        // A state the workflow doesn't list goes by its kind.
        let gone = TicketState {
            name: "Archived review".into(),
            kind: StateKind::Started,
            id: Some("id-gone".into()),
        };
        assert_eq!(
            next_state(Progress::Merged, Some(&gone), &states).map(|s| s.name.as_str()),
            Some("Done")
        );
        assert_eq!(next_state(Progress::Review, Some(&gone), &states), None);
    }

    fn pr(state: Option<&str>) -> PullRequestRef {
        PullRequestRef {
            number: 1,
            url: "https://github.com/acme/web/pull/1".into(),
            title: None,
            state: state.map(str::to_owned),
            repo: None,
            state_changed_at: None,
            updated_at: None,
            checks: None,
        }
    }

    #[test]
    fn progress_shows_in_changes_and_pull_requests() {
        assert_eq!(progress(&[], false), None);
        assert_eq!(progress(&[], true), Some(Progress::Started));
        assert_eq!(
            progress(&[pr(Some("draft"))], false),
            Some(Progress::Started)
        );
        assert_eq!(
            progress(&[pr(Some("closed"))], true),
            Some(Progress::Started)
        );
        assert_eq!(progress(&[pr(Some("closed"))], false), None);
        for up in ["open", "queued"] {
            assert_eq!(
                progress(&[pr(Some(up))], false),
                Some(Progress::Review),
                "{up}"
            );
        }
        // One nobody looked up yet may be a draft: it is open, not up.
        assert_eq!(progress(&[pr(None)], false), Some(Progress::Started));
        assert_eq!(
            progress(&[pr(Some("merged")), pr(None)], true),
            Some(Progress::Started)
        );
        assert_eq!(
            progress(&[pr(Some("draft")), pr(Some("open"))], true),
            Some(Progress::Review)
        );
        assert_eq!(
            progress(&[pr(Some("merged"))], false),
            Some(Progress::Merged)
        );
        assert_eq!(
            progress(&[pr(Some("merged")), pr(Some("closed"))], true),
            Some(Progress::Merged)
        );
        // Another still open: the ticket isn't done yet.
        assert_eq!(
            progress(&[pr(Some("merged")), pr(Some("open"))], true),
            Some(Progress::Review)
        );
        assert_eq!(
            progress(&[pr(Some("merged")), pr(Some("draft"))], true),
            Some(Progress::Started)
        );
    }

    #[test]
    fn what_agents_report_starts_tickets_at_once_and_has_them_reviewed_after_the_turn() {
        use Progress::*;
        // The first change starts it, during a turn or after.
        assert_eq!(reported(None, Some(Started), false), Some(Started));
        assert_eq!(reported(None, Some(Started), true), Some(Started));
        // A pull request up waits for the turn's end, and starts it meanwhile.
        assert_eq!(reported(None, Some(Review), false), Some(Started));
        assert_eq!(reported(Some(Started), Some(Review), false), None);
        assert_eq!(reported(Some(Started), Some(Review), true), Some(Review));
        assert_eq!(reported(None, Some(Review), true), Some(Review));
        // Nothing new, or less: nothing moves.
        for turn_over in [false, true] {
            assert_eq!(reported(Some(Started), Some(Started), turn_over), None);
            assert_eq!(reported(Some(Review), Some(Review), turn_over), None);
            assert_eq!(reported(Some(Review), Some(Started), turn_over), None);
            assert_eq!(reported(None, None, turn_over), None);
            // Merges move it on their own.
            assert_eq!(reported(Some(Review), Some(Merged), turn_over), None);
        }
    }

    #[test]
    fn tickets_are_seen_to_once_none_is_open_or_one_more_merged() {
        use Progress::*;
        // The last open one merged, or closed while another had merged.
        assert!(done_due(Some(Review), Some(Merged), true));
        assert!(done_due(Some(Review), Some(Merged), false));
        assert!(done_due(None, Some(Merged), true));
        // One more merged among those that did: it may be the one that
        // completes the ticket.
        assert!(done_due(Some(Merged), Some(Merged), true));
        assert!(!done_due(Some(Merged), Some(Merged), false));
        // One merged while another is open: not yet.
        assert!(!done_due(Some(Review), Some(Review), true));
        assert!(!done_due(Some(Merged), Some(Review), false));
        assert!(!done_due(None, None, false));
        assert!(DONE_AGAIN_IN_MS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn pull_requests_complete_tickets_as_linears_github_integration_reads_them() {
        let branch = "fctl/12-eng-123-fix-the-login-loop";
        // The branch links it.
        assert!(completes("ENG-123", "Fix the login loop", "", branch));
        assert!(!completes(
            "ENG-123",
            "Fix the login loop",
            "",
            "fctl/12-fix"
        ));
        assert!(!completes("ENG-12", "Fix", "", branch));
        // So do the words before the key.
        for body in [
            "Fixes ENG-123",
            "This change.\n\nCloses: eng-123.",
            "- **Resolves** ENG-123",
            "Fixes ENG-7, ENG-9 and ENG-123",
            "Implements https://linear.app/acme/issue/ENG-123/fix-the-login-loop",
            "Fixes [ENG-123](https://linear.app/acme/issue/ENG-123/x)",
        ] {
            assert!(
                completes("ENG-123", "Fix it", body, "fctl/12-fix"),
                "{body}"
            );
        }
        for body in [
            "Part of ENG-123",
            "Refs ENG-123",
            "Contributes to [ENG-123](https://linear.app/acme/issue/ENG-123/x)",
            "Related to ENG-123, ENG-9",
            "Skip ENG-123",
        ] {
            assert!(!completes("ENG-123", "Fix it", body, branch), "{body}");
        }
        // A closing word anywhere wins.
        assert!(completes(
            "ENG-123",
            "Fixes ENG-123",
            "Part of ENG-123",
            branch
        ));
        // Words that hold a magic word, or come before other words, don't count.
        assert!(!completes("ENG-123", "", "Prefixes ENG-123", "fctl/1-x"));
        assert!(!completes(
            "ENG-123",
            "",
            "This fixes the bug in ENG-123",
            "fctl/1-x"
        ));
        // Named alone, in the title it links, in the description it doesn't.
        assert!(completes(
            "ENG-123",
            "ENG-123: Fix the loop",
            "",
            "fctl/1-x"
        ));
        assert!(!completes("ENG-123", "Fix", "See ENG-123", "fctl/1-x"));
        // Odd text breaks nothing.
        assert!(!completes(
            "ENG-123",
            "",
            "Fixes\u{3000}\u{a0} ENG-1234",
            "fctl/1-x"
        ));
        assert!(completes(
            "ENG-123",
            "",
            "Fixes\u{3000}\u{a0}ENG-123",
            "fctl/1-x"
        ));
        assert!(!completes("", "Fixes", "", ""));
    }

    #[test]
    fn agents_hear_of_their_tracker() {
        let primary = r("acme/web");
        let none = Tickets {
            tracker: Tracker::None,
            ..Default::default()
        };
        let told = |t: &Tickets, access| {
            instructions(&Told {
                tickets: t,
                via: via(t, Some(&primary), &[], access),
                primary: Some(&primary),
                grants: &[],
                access,
                github_issues: false,
                linear_workspace: Some("Acme"),
                started_from: None,
            })
        };
        assert!(told(&none, GithubAccess::Push).contains("no ticket tracker"));
        let gh = told(&github(None), GithubAccess::Push);
        assert!(gh.contains("GitHub issues of acme/web: read, open and comment"));
        assert!(!gh.contains("fctl ticket"));
        assert!(told(&github(None), GithubAccess::Read).contains("can't write to them"));
        let lin = told(&linear(), GithubAccess::Push);
        assert!(lin.contains("Linear team ENG (Acme)"));
        assert!(lin.contains("`fctl ticket comment ENG-123"));
        // A workspace that disconnected Linear reaches none of its tickets.
        let gone = instructions(&Told {
            tickets: &linear(),
            via: TicketsVia::Nothing,
            primary: Some(&primary),
            grants: &[],
            access: GithubAccess::Push,
            github_issues: false,
            linear_workspace: Some("Acme"),
            started_from: None,
        });
        assert!(gone.contains("fctl can't reach it now"));
        assert!(!gone.contains("fctl ticket comment"));
        let ticket = SessionTicket {
            tracker: Tracker::Linear,
            id: "i".into(),
            key: "ENG-7".into(),
            title: Some("Fix it".into()),
            url: "https://linear.app/acme/issue/ENG-7".into(),
        };
        let started = instructions(&Told {
            tickets: &linear(),
            via: TicketsVia::Fctl,
            primary: Some(&primary),
            grants: &[],
            access: GithubAccess::Push,
            github_issues: false,
            linear_workspace: Some("Acme"),
            started_from: Some(StartedFrom {
                ticket: &ticket,
                by: "Ada Lovelace",
                mentioned: false,
                explore: true,
                moved_to: Some("In Progress"),
            }),
        });
        assert!(started.contains("ENG-7, which Ada Lovelace delegated"));
        assert!(started.contains("exploration"));
        assert!(started.contains("`Fixes ENG-7`"));
        assert!(started.contains(
            "fctl moved the ticket to In Progress as the session started, and moves it on in Linear as you go: to In Review when a turn ends with your pull request up, and to Done once the pull requests that complete it merge"
        ));
        assert!(started.contains("Leave its state alone unless you are asked."));
        assert!(
            started.contains("move it to Done yourself: `fctl ticket update ENG-7 --state Done`")
        );
        // It moves the tickets it starts from itself.
        assert!(!started.contains("When your task is to do a ticket's work"));
    }

    #[test]
    fn agents_hear_how_their_tickets_move_on_as_the_project_lets_fctl_move_them() {
        let primary = r("acme/web");
        let ticket = SessionTicket {
            tracker: Tracker::Linear,
            id: "i".into(),
            key: "ENG-7".into(),
            title: None,
            url: "https://linear.app/acme/issue/ENG-7".into(),
        };
        let with = |moves: [Option<bool>; 3]| {
            let mut t = linear();
            if let Some(l) = t.linear.as_mut() {
                [l.move_started, l.move_review, l.move_done] = moves;
            }
            t
        };
        let told = |t: &Tickets, from: Option<StartedFrom<'_>>, access| {
            instructions(&Told {
                tickets: t,
                via: TicketsVia::Fctl,
                primary: Some(&primary),
                grants: &[],
                access,
                github_issues: false,
                linear_workspace: Some("Acme"),
                started_from: from,
            })
        };
        let from = |mentioned, moved_to| {
            Some(StartedFrom {
                ticket: &ticket,
                by: "Ada Lovelace",
                mentioned,
                explore: false,
                moved_to,
            })
        };
        // A mention moves it once the agent changed something.
        let mentioned = told(&with([None; 3]), from(true, None), GithubAccess::Push);
        assert!(mentioned.contains("ENG-7, where Ada Lovelace mentioned fctl in Linear"));
        assert!(mentioned.contains(
            "fctl moves the ticket on in Linear as you go: to In Progress once you change something, to In Review when a turn ends with your pull request up, and to Done once"
        ));
        // Only what the project lets fctl do.
        let review = told(
            &with([Some(false), Some(true), Some(false)]),
            from(true, None),
            GithubAccess::Push,
        );
        assert!(
            review.contains(
                "as you go: to In Review when a turn ends with your pull request up, or to"
            )
        );
        assert!(!review.contains("Done"));
        let off = told(
            &with([Some(false); 3]),
            from(false, None),
            GithubAccess::Push,
        );
        assert!(off.contains("Linear's GitHub integration, it moves the ticket on"));
        assert!(!off.contains("fctl moves") && !off.contains("Done yourself"));
        // An agent that can't write tickets moves none.
        let reads = told(&with([None; 3]), from(false, None), GithubAccess::Read);
        assert!(!reads.contains("Done yourself") && !reads.contains("Your branch"));
        // A session of its own moves the tickets it works on.
        let own = told(&with([None; 3]), None, GithubAccess::Push);
        assert!(own.contains(
            "When your task is to do a ticket's work, move the ticket on as you go with `fctl ticket update`: to In Progress as you start, unless it has started, to In Review once your pull requests are up, and to Done once you have done all it asks, when it needs no pull request."
        ));
        let some = told(
            &with([Some(true), Some(false), Some(false)]),
            None,
            GithubAccess::Push,
        );
        assert!(some.contains("as you go with `fctl ticket update`: to In Progress as you start, unless it has started. Write `Fixes ENG-123`"));
        for t in [with([Some(false); 3]), linear()] {
            let quiet = told(&t, None, GithubAccess::Read);
            assert!(!quiet.contains("When your task is to do a ticket's work"));
        }
        assert!(
            !told(&with([Some(false); 3]), None, GithubAccess::Push).contains("When your task")
        );
    }

    #[test]
    fn sessions_that_ask_reach_the_issues_of_every_repository_they_write_to() {
        let primary = r("acme/web");
        let grants = [g("lib", RepoAccess::Write), g("docs", RepoAccess::Read)];
        let names = ["web".to_owned(), "lib".to_owned()];
        let none = Tickets {
            tracker: Tracker::None,
            ..Default::default()
        };
        for t in [none.clone(), linear(), github(Some("acme/web"))] {
            assert_ne!(
                session_issues(&t, Some(&primary), &grants, false),
                GithubIssues::Writable
            );
            let issues = session_issues(&t, Some(&primary), &grants, true);
            assert_eq!(issues, GithubIssues::Writable);
            for access in GithubAccess::ALL {
                assert_eq!(
                    issue_tokens(&issues, &names, access),
                    IssueTokens {
                        write: true,
                        read: true
                    }
                );
            }
        }
        // The tracker stays the project's: gh reaches GitHub's, fctl ticket
        // Linear's, and nothing a project without one.
        let reach =
            |t: &Tickets, asked| session_via(t, Some(&primary), &grants, GithubAccess::Push, asked);
        assert_eq!(reach(&github(Some("acme/web")), false), TicketsVia::Fctl);
        assert_eq!(reach(&github(Some("acme/web")), true), TicketsVia::Gh);
        assert_eq!(reach(&linear(), true), TicketsVia::Fctl);
        assert_eq!(reach(&none, true), TicketsVia::Nothing);
    }

    #[test]
    fn agents_hear_which_github_issues_they_reach() {
        let primary = r("acme/web");
        let told = |t: &Tickets, access, github_issues| {
            instructions(&Told {
                tickets: t,
                via: session_via(t, Some(&primary), &[], access, github_issues),
                primary: Some(&primary),
                grants: &[],
                access,
                github_issues,
                linear_workspace: Some("Acme"),
                started_from: None,
            })
        };
        let none = Tickets {
            tracker: Tracker::None,
            ..Default::default()
        };
        // Without it, they hear how to get it.
        for t in [&none, &linear()] {
            let s = told(t, GithubAccess::Push, false);
            assert!(s.contains("can't reach"), "{s}");
            assert!(s.contains("`fctl run --github-issues`"), "{s}");
        }
        let lin = told(&linear(), GithubAccess::Push, true);
        assert!(lin.contains("Linear team ENG (Acme)"));
        assert!(lin.contains("`fctl ticket comment ENG-123"));
        assert!(lin.contains("reach the GitHub issues of acme/web as well"));
        assert!(lin.contains("close them with `gh issue`"));
        assert!(lin.contains("to move them into Linear"));
        assert!(!lin.contains("can't reach"));
        let reads = told(&linear(), GithubAccess::Read, true);
        assert!(reads.contains("acme/web as well: read them with `gh issue`"));
        let nothing = told(&none, GithubAccess::Push, true);
        assert!(nothing.contains("no ticket tracker"));
        assert!(nothing.contains("GitHub issues of acme/web"));
        assert!(!nothing.contains("can't reach"));
        // The tracker's repository among several: gh reaches all of them.
        let grants = [g("lib", RepoAccess::Write)];
        let t = github(Some("acme/web"));
        let one = instructions(&Told {
            tickets: &t,
            via: session_via(&t, Some(&primary), &grants, GithubAccess::Push, true),
            primary: Some(&primary),
            grants: &grants,
            access: GithubAccess::Push,
            github_issues: true,
            linear_workspace: None,
            started_from: None,
        });
        assert!(one.contains("comment on them with `gh issue`"));
        assert!(one.contains("the repositories you can write to too"));
        assert!(!one.contains("fctl ticket"));
        assert!(!one.contains("Leave the issues of other repositories alone"));
    }
}
