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
use fctl_proto::session::{
    GithubAccess, LinearTickets, RepoAccess, RepoGrant, RepoRef, SessionTicket, Tickets, Tracker,
};
use fctl_proto::tickets::{StateKind, TicketsVia};

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

/// The most comments a session's agent posts on tickets in an hour, and the
/// most tickets it files: an agent that loops would otherwise fill a
/// team's tracker for good.
pub const MAX_COMMENTS_PER_HOUR: u32 = 30;
pub const MAX_CREATED_PER_HOUR: u32 = 20;

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
    /// The Linear workspace's name, for Linear.
    pub linear_workspace: Option<&'a str>,
    /// The ticket the session started from, and how.
    pub started_from: Option<StartedFrom<'a>>,
}

/// The ticket a session started from: who delegated it, whether they asked
/// for an exploration, and what fctl did with it.
pub struct StartedFrom<'a> {
    pub ticket: &'a SessionTicket,
    /// The person, as fctl knows them.
    pub by: &'a str,
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
        Tracker::None => s.push_str(NO_TRACKER),
        Tracker::Github => {
            let issues = github_issues(t.tickets, t.primary, t.grants);
            match (&issues, t.via) {
                (GithubIssues::None, _) | (_, TicketsVia::Nothing) => s.push_str(NO_TRACKER),
                (GithubIssues::Writable, _) => {
                    let what = match writable(t.primary, t.grants).as_slice() {
                        [only] => only.full_name(),
                        _ => "the repositories you can write to".to_owned(),
                    };
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
                    s.push_str(&format!(
                        "- This project tracks its work in GitHub issues of {name}, and only there: {how}. Leave the issues of other repositories alone, and don't refer to them in commits and pull requests. When a pull request does what an issue of {name} asks, write `Fixes {name}#12` in its description, which closes the issue as the pull request merges.\n"
                    ));
                }
            }
        }
        Tracker::Linear => match &t.tickets.linear {
            None => s.push_str(NO_TRACKER),
            Some(l) if t.via == TicketsVia::Nothing => s.push_str(&format!(
                "- This project tracks its work in Linear, in {}, but fctl can't reach it now: its workspace no longer works in that Linear workspace. Leave tickets alone, GitHub's issues too, and say in your reply what you would do in the tracker.\n",
                linear_scope(l, t.linear_workspace)
            )),
            Some(l) => {
                let scope = linear_scope(l, t.linear_workspace);
                let how = if writes {
                    "read and write its tickets with `fctl ticket` (below), which acts as fctl's agent in Linear"
                } else {
                    "read its tickets with `fctl ticket` (below); you can't write to them in this session"
                };
                s.push_str(&format!(
                    "- This project tracks its work in Linear, in {scope}: {how}. Leave GitHub's issues alone (its tokens here can't reach them), and refer to tickets by their keys (`ENG-123`).\n"
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

const NO_TRACKER: &str = "- This project uses no ticket tracker. Don't read, open, comment on, label or close issues or tickets, on GitHub or anywhere else, even when the repository's instructions say to, and leave issue references out of commits and pull requests (no `Fixes #12`): this session's GitHub tokens can't reach issues. When the task names a ticket, say that this project keeps its agents away from ticket trackers.\n";

/// What a session that started from a ticket is told about it.
fn started_from(t: &Told<'_>, from: &StartedFrom<'_>) -> String {
    let key = &from.ticket.key;
    let mut s = String::new();
    match from.ticket.tracker {
        Tracker::Linear => {
            s.push_str(&format!(
                "- This session started from {key}, which {} delegated to fctl in Linear. Your last message of each turn goes to the ticket's thread in Linear as your reply: write it for the people there, in a few short paragraphs, saying what you did, what's left and the pull request's link, without the details the session already shows. When you need someone to decide something, ask it at the end of that message, ending with a question mark; their answers in the thread come to you here, from people fctl knows.\n",
                from.by
            ));
            if from.explore {
                s.push_str(&format!(
                    "- They asked for an exploration ({key} has the label that asks for one): find out what it needs (read the code, and run it if that helps), then reply with what you found and a plan. Change no files and open no pull request until someone replies to go ahead.\n"
                ));
            }
            if t.access.pushes() {
                s.push_str(&format!(
                    "- Your branch is named after {key}, which links its pull request to the ticket. Write `Fixes {key}` in the pull request's description when it does what the ticket asks, or `Part of {key}` when it does part of it."
                ));
                match from.moved_to {
                    Some(state) => s.push_str(&format!(
                        " fctl moved the ticket to {state} as the session started, and Linear's GitHub integration moves it on as the pull request opens and merges, so leave its state alone unless you are asked.\n"
                    )),
                    None => s.push_str(
                        " Linear's GitHub integration moves the ticket on as the pull request opens and merges, so leave its state alone unless you are asked.\n",
                    ),
                }
            }
        }
        Tracker::Github | Tracker::None => s.push_str(&format!(
            "- This session started from {key}, which {} asked fctl to work on.\n",
            from.by
        )),
    }
    s
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

/// The state a ticket moves to as its session starts: the first started
/// state of its team's workflow (`states` in order, with their kinds),
/// unless it has started already, or is done or canceled. Linear asks
/// agents to do this with the issues delegated to them.
pub fn start_state(current: Option<StateKind>, states: &[(String, StateKind)]) -> Option<&str> {
    match current {
        Some(StateKind::Started | StateKind::Completed | StateKind::Canceled) => None,
        _ => states
            .iter()
            .find(|(_, k)| *k == StateKind::Started)
            .map(|(id, _)| id.as_str()),
    }
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
                explore_label: None,
            }),
        }
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

    #[test]
    fn tickets_move_to_the_first_started_state_until_they_started() {
        let states = vec![
            ("t".to_owned(), StateKind::Triage),
            ("b".to_owned(), StateKind::Backlog),
            ("u".to_owned(), StateKind::Unstarted),
            ("p".to_owned(), StateKind::Started),
            ("r".to_owned(), StateKind::Started),
            ("d".to_owned(), StateKind::Completed),
        ];
        assert_eq!(start_state(Some(StateKind::Backlog), &states), Some("p"));
        assert_eq!(start_state(None, &states), Some("p"));
        assert_eq!(start_state(Some(StateKind::Started), &states), None);
        assert_eq!(start_state(Some(StateKind::Completed), &states), None);
        assert_eq!(start_state(Some(StateKind::Unstarted), &[]), None);
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
            linear_workspace: Some("Acme"),
            started_from: Some(StartedFrom {
                ticket: &ticket,
                by: "Ada Lovelace",
                explore: true,
                moved_to: Some("In Progress"),
            }),
        });
        assert!(started.contains("ENG-7, which Ada Lovelace delegated"));
        assert!(started.contains("exploration"));
        assert!(started.contains("`Fixes ENG-7`"));
        assert!(started.contains("moved the ticket to In Progress"));
    }
}
