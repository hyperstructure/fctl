//! Follow-ups: what brings an agent back to a pull request its session
//! opened, where its project turns them on (`ProjectSettings::follow_ups`).
//! GitHub's webhook says that a person wrote to the agent there, that a
//! check of the pull request's newest commit failed, or that it was merged
//! or closed, and the lookups after deliveries say when it conflicts with
//! its base branch; the control plane passes that on to the session in a
//! message from fctl, which resumes the session if it stopped, as a
//! person's message would.
//!
//! This is what that decides without I/O: whether a comment addresses the
//! agent (`@fctl`, `@agent`, or `fctl:` and `agent:` at its start), what
//! GitHub's deliveries say of comments and of who wrote them, the keys that
//! let each reach a session once, and what the agent is told.

use crate::reviews::{by_app, short};
use crate::webhooks::{Looked, Mergeable, Payload};
use fctl_proto::Millis;
use fctl_proto::event::{FollowUp, FollowUpComment, FollowUpEvent};
use fctl_proto::session::{PullRequestRef, RepoRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The names that address a project's agents on GitHub: as mentions
/// (`@fctl`, `@agent`), and at the start of a comment (`fctl:`, `agent,`).
pub const NAMES: [&str; 2] = ["fctl", "agent"];

/// How long a session gathers what comes about its pull requests before
/// its agent hears of it, in one message: a review comes as a delivery for
/// the review and one for each of its comments, and checks fail together.
pub const GATHER_MS: i64 = 10_000;

/// The most failed checks in a row that fctl passes on to a session while
/// nobody messages its agent: a fix that fails again would otherwise bring
/// it back for good, each time on someone's subscription.
pub const MAX_CHECKS_IN_A_ROW: u32 = 3;

/// The most conflicts in a row that fctl passes on to a session while
/// nobody messages its agent: in a busy repository, a pull request that
/// nobody merges would otherwise take in its base branch after every merge
/// into it, for good.
pub const MAX_CONFLICTS_IN_A_ROW: u32 = 3;

/// How long a session waits to ask GitHub again whether a pull request
/// conflicts with its base branch, after GitHub didn't know yet
/// ([`Mergeable::Unknown`]): before the second ask, the third, and so on.
/// GitHub works it out in seconds once asked, so the first ask after a
/// branch moved starts it, and one of these tells; after the last, the
/// session leaves it to the next delivery.
pub const ASK_AGAIN_MS: [i64; 5] = [15_000, 30_000, 60_000, 120_000, 240_000];

/// The longest a comment is in what fctl passes on: its link has the rest.
pub const MAX_BODY: usize = 4_000;

/// The most checks a message names; it says how many more failed.
const MAX_NAMES: usize = 6;

// ───────────────────────────── addressing the agent ─────────────────────────────

/// Whether a comment addresses the agent: it mentions `@fctl` or `@agent`
/// anywhere, or starts with `fctl:`, `agent:`, `fctl,` or `agent,` and a
/// space or a new line, in any case. What it quotes (`>`), shows as code
/// (in backticks or a fenced block) or hides in an HTML comment doesn't
/// count: people quote a request, and say how to make one.
pub fn addressed(body: &str) -> bool {
    let prose = prose(body);
    starts_with_name(&prose) || prose.lines().any(mentions)
}

/// What a comment says outright: its lines without HTML comments, quotes,
/// fenced code blocks and code spans, which leave empty lines.
fn prose(body: &str) -> String {
    let body = without_html_comments(body);
    let mut out = String::with_capacity(body.len());
    // The fence a code block opened: its character and how many.
    let mut fence: Option<(u8, usize)> = None;
    for line in body.lines() {
        let t = line.trim_start();
        let run = |c: u8| t.bytes().take_while(|&b| b == c).count();
        if let Some((c, n)) = fence {
            let closes = run(c);
            if closes >= n && t[closes..].trim().is_empty() {
                fence = None;
            }
        } else if let Some(c) = b"`~"
            .iter()
            .copied()
            .find(|&c| run(c) >= 3 && !(c == b'`' && t[run(c)..].contains('`')))
        {
            // (Backticks after a run of them make a code span, not a fence.)
            fence = Some((c, run(c)));
        } else if !t.starts_with('>') {
            out.push_str(&without_code_spans(line));
        }
        out.push('\n');
    }
    out
}

/// `body` without `<!-- … -->`; one that doesn't close hides the rest.
fn without_html_comments(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        match rest[i + 4..].find("-->") {
            Some(end) => {
                // Its lines stay, empty, so that the comment's start does.
                out.extend(rest[i..i + 4 + end].matches('\n'));
                rest = &rest[i + 4 + end + 3..];
            }
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// A line without its code spans: a run of backticks up to the next run of
/// as many. A run that nothing closes is just backticks.
fn without_code_spans(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find('`') {
        out.push_str(&rest[..i]);
        let ticks = rest[i..].bytes().take_while(|&b| b == b'`').count();
        let after = &rest[i + ticks..];
        match closing(after, ticks) {
            Some(end) => {
                out.push(' ');
                rest = &after[end + ticks..];
            }
            None => {
                out.push_str(&rest[i..i + ticks]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Where in `s` a run of exactly `n` backticks starts.
fn closing(s: &str, n: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            if run == n {
                return Some(i);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

/// Whether the first line that says something starts with a name and `:`
/// or `,`, then a space or nothing; emphasis around them (`**agent:**`)
/// counts as nothing.
fn starts_with_name(prose: &str) -> bool {
    let Some(first) = prose.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return false;
    };
    let first = first.trim_start_matches(['*', '_']);
    NAMES.iter().any(|name| {
        let Some(rest) = after_name(first, name) else {
            return false;
        };
        let Some(rest) = rest.strip_prefix([':', ',']) else {
            return false;
        };
        let rest = rest.trim_start_matches(['*', '_']);
        rest.is_empty() || rest.starts_with(char::is_whitespace)
    })
}

/// What follows `name` at the start of `s`, in any case.
fn after_name<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    s.get(..name.len())
        .filter(|head| head.eq_ignore_ascii_case(name))
        .and_then(|_| s.get(name.len()..))
}

/// Whether a line mentions a name as GitHub mentions people: `@` after
/// nothing that runs into it (an address, `a@agent.dev`, or a link), and
/// the name with nothing after it that a login could go on with
/// (`@agents`, `@fctl-bot`) or a team's name (`@acme/fctl`).
fn mentions(line: &str) -> bool {
    line.match_indices('@').any(|(i, _)| {
        let before = line[..i].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || "@-./\\`".contains(c)) {
            return false;
        }
        let after = &line[i + 1..];
        NAMES.iter().any(|name| {
            after_name(after, name).is_some_and(|rest| {
                rest.chars()
                    .next()
                    .is_none_or(|c| !(c.is_alphanumeric() || c == '-' || c == '/'))
            })
        })
    })
}

// ───────────────────────────── what GitHub says ─────────────────────────────

/// A pull request as fctl passes on what happened to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pull {
    pub repo: RepoRef,
    pub number: u64,
    /// On GitHub.
    pub url: String,
    pub title: Option<String>,
    /// `open`, `draft`, `queued`, `merged` or `closed`, as GitHub said.
    pub state: Option<String>,
    /// The branch it goes into.
    pub base: Option<String>,
}

impl Pull {
    /// `acme/web#12`.
    pub fn reference(&self) -> String {
        format!("{}#{}", self.repo.full_name(), self.number)
    }
}

/// Where a person wrote on a pull request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// A comment on its conversation (`issue_comment`).
    Conversation,
    /// A review's own text (`pull_request_review`).
    Review,
    /// A comment on lines of its diff, or a reply in such a comment's
    /// thread (`pull_request_review_comment`).
    Lines,
}

impl Place {
    /// Where a comment fctl passes on was made.
    pub fn of(c: &FollowUpComment) -> Self {
        if c.review.is_some() {
            Place::Review
        } else if c.path.is_some() {
            Place::Lines
        } else {
            Place::Conversation
        }
    }
}

/// A new comment on a pull request, as its delivery says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commented {
    pub pull: Pull,
    /// The pull request's author (`fctl[bot]`), and its branch, when the
    /// delivery says (one about its conversation doesn't).
    pub author: Option<String>,
    pub head_ref: Option<String>,
    pub comment: FollowUpComment,
    /// GitHub's node id of the comment or review, which takes reactions.
    pub node_id: Option<String>,
    /// What GitHub says of its author and the repository: `OWNER`,
    /// `MEMBER`, `COLLABORATOR`, `CONTRIBUTOR`, `NONE`…
    pub association: Option<String>,
    /// A person wrote it, not an app.
    pub person: bool,
    /// The installation that sent it, and GitHub's id of the account the
    /// repository belongs to.
    pub installation: Option<u64>,
    pub owner_id: Option<u64>,
}

impl Commented {
    pub fn place(&self) -> Place {
        Place::of(&self.comment)
    }
}

/// What a delivery of `event` (its `X-GitHub-Event`) says of a comment
/// someone just made on a pull request: on its conversation
/// (`issue_comment`, `created`), a review (`pull_request_review`,
/// `submitted`) or a comment on its lines (`pull_request_review_comment`,
/// `created`). `None` for anything else, such as an edited comment, or one
/// on an issue.
pub fn commented(event: &str, payload: &Value) -> Option<Commented> {
    let text = |v: &Value, path: &str| v.pointer(path).and_then(Value::as_str).map(str::to_owned);
    let action = text(payload, "/action")?;
    let repo = RepoRef::parse(&text(payload, "/repository/full_name")?)?;
    let installation = payload.pointer("/installation/id").and_then(Value::as_u64);
    let owner_id = payload
        .pointer("/repository/owner/id")
        .and_then(Value::as_u64);
    let (pr, wrote) = match (event, action.as_str()) {
        ("issue_comment", "created") => {
            let issue = payload.get("issue")?;
            // An issue's comments aren't a pull request's.
            issue.get("pull_request").filter(|p| p.is_object())?;
            (issue, payload.get("comment")?)
        }
        ("pull_request_review", "submitted") => {
            (payload.get("pull_request")?, payload.get("review")?)
        }
        ("pull_request_review_comment", "created") => {
            (payload.get("pull_request")?, payload.get("comment")?)
        }
        _ => return None,
    };
    let comment = match event {
        "pull_request_review" => FollowUpComment {
            review: Some(
                text(wrote, "/state")
                    .unwrap_or_else(|| "commented".into())
                    .to_ascii_lowercase(),
            ),
            at: text(wrote, "/submitted_at")
                .as_deref()
                .and_then(crate::pulls::github_time),
            ..written(wrote)?
        },
        "pull_request_review_comment" => on_lines(wrote)?,
        _ => written(wrote)?,
    };
    let merged = pr.get("merged").and_then(Value::as_bool) == Some(true)
        || [pr.get("merged_at"), pr.pointer("/pull_request/merged_at")]
            .into_iter()
            .flatten()
            .any(|m| !m.is_null());
    let state = crate::pulls::state_of(
        merged,
        text(pr, "/state").as_deref() == Some("closed"),
        false,
        pr.get("draft").and_then(Value::as_bool) == Some(true),
    );
    let url = text(pr, "/html_url")?;
    Some(Commented {
        pull: Pull {
            repo,
            number: pr.get("number")?.as_u64()?,
            url,
            title: text(pr, "/title"),
            state: Some(state.to_owned()),
            base: text(pr, "/base/ref"),
        },
        author: text(pr, "/user/login"),
        head_ref: text(pr, "/head/ref"),
        node_id: text(wrote, "/node_id"),
        association: text(wrote, "/author_association"),
        person: is_person(wrote.get("user").unwrap_or(&Value::Null)),
        comment,
        installation,
        owner_id,
    })
}

/// A comment or review as GitHub has it: who wrote what, where and when.
fn written(c: &Value) -> Option<FollowUpComment> {
    let text = |path: &str| c.pointer(path).and_then(Value::as_str).map(str::to_owned);
    Some(FollowUpComment {
        id: c.get("id")?.as_u64()?,
        login: text("/user/login")?,
        avatar_url: text("/user/avatar_url"),
        url: text("/html_url")?,
        body: truncate(text("/body").unwrap_or_default().trim(), MAX_BODY),
        review: None,
        path: None,
        line: None,
        start_line: None,
        deleted: false,
        review_id: None,
        in_reply_to: None,
        at: text("/created_at")
            .as_deref()
            .and_then(crate::pulls::github_time),
    })
}

/// A comment on lines of a pull request, as GitHub has it: its file, and
/// its lines in the version it is on (or were, once a commit moved them).
fn on_lines(c: &Value) -> Option<FollowUpComment> {
    let line = |key: &str| {
        c.get(key)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    Some(FollowUpComment {
        path: Some(c.get("path")?.as_str()?.to_owned()),
        line: line("line").or_else(|| line("original_line")),
        start_line: line("start_line").or_else(|| line("original_start_line")),
        deleted: c.get("side").and_then(Value::as_str) == Some("LEFT"),
        review_id: c.get("pull_request_review_id").and_then(Value::as_u64),
        in_reply_to: c.get("in_reply_to_id").and_then(Value::as_u64),
        ..written(c)?
    })
}

/// A review's comments on lines (`GET
/// /repos/{owner}/{name}/pulls/{number}/reviews/{id}/comments`), which fctl
/// passes on with the review.
pub fn review_comments(listed: &Value) -> Vec<FollowUpComment> {
    listed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(on_lines)
        .collect()
}

/// Whether a GitHub account is a person's: an app's bot (`fctl[bot]`, of
/// type `Bot`) is no person, and neither are fctl's own review agents.
pub fn is_person(user: &Value) -> bool {
    user.get("type").and_then(Value::as_str) == Some("User")
        && user
            .get("login")
            .and_then(Value::as_str)
            .is_some_and(|l| !l.ends_with("[bot]"))
}

/// Whether what GitHub says of a comment's author and the repository
/// (`author_association`) can come with write access: they own it, are a
/// member of its organization, or a collaborator. Nobody else can write to
/// it, so fctl doesn't ask GitHub about them.
pub fn may_write(association: Option<&str>) -> bool {
    matches!(association, Some("OWNER" | "MEMBER" | "COLLABORATOR"))
}

/// Whether GitHub's answer about a person's permission on a repository
/// (`GET /repos/{owner}/{name}/collaborators/{login}/permission`) lets them
/// write to it: admin, maintain or write. A member of its organization
/// may only read it.
pub fn can_write(answer: &Value) -> bool {
    matches!(
        answer.get("permission").and_then(Value::as_str),
        Some("admin" | "write")
    ) || answer
        .pointer("/user/permissions/push")
        .and_then(Value::as_bool)
        == Some(true)
}

/// Why fctl doesn't pass a comment on, as far as its delivery tells,
/// before anyone is asked: the pull request isn't one fctl's agents opened
/// (its author isn't the GitHub App, `app_login`), an app wrote it, its
/// author can't write to the repository, or it doesn't address the agent.
pub fn passed_over(c: &Commented, app_login: Option<&str>) -> Option<&'static str> {
    if !by_app(c.author.as_deref(), app_login) {
        Some("fctl's agents didn't open it")
    } else if !c.person {
        Some("an app wrote it, not a person")
    } else if !may_write(c.association.as_deref()) {
        Some("its author can't write to the repository")
    } else if !addressed(&c.comment.body) {
        Some("it doesn't address the agent")
    } else {
        None
    }
}

/// What a lookup of a pull request, after a delivery about its checks,
/// passes on to the session that opened it: that checks of its newest
/// commit failed. Nothing for a pull request that is merged or closed, or
/// whose failed checks are another commit's.
pub fn checks_failed(looked: &Looked) -> Option<FollowUpEvent> {
    let checks = looked.pr.checks.as_ref()?;
    let sha = checks.sha.clone().or_else(|| looked.head.clone())?;
    if looked.failing.is_empty()
        || !is_open(looked.pr.state.as_deref())
        || looked.head.as_deref().is_some_and(|h| h != sha)
    {
        return None;
    }
    Some(FollowUpEvent::ChecksFailed {
        sha,
        names: looked.failing.clone(),
        passed: checks.passed,
        failed: checks.failed,
        pending: checks.pending,
    })
}

/// What a `pull_request` delivery that closed its pull request says: it
/// was merged, by whom and into which branch, or closed without being
/// merged, by whom.
pub fn closed(payload: &Payload) -> Option<FollowUpEvent> {
    if payload.action != "closed" {
        return None;
    }
    match payload.seen.state.as_deref() {
        Some("merged") => Some(FollowUpEvent::Merged {
            by: payload.merged_by.clone().or_else(|| payload.sender.clone()),
            base: payload.base_ref.clone(),
        }),
        Some("closed") => Some(FollowUpEvent::Closed {
            by: payload.sender.clone(),
        }),
        _ => None,
    }
}

/// What a lookup of a pull request passes on to the session that opened it
/// when GitHub says it conflicts with its base branch: its head commit then,
/// and the branch. Nothing for one that is merged or closed, or that GitHub
/// says merges, or doesn't know about yet ([`unsure`]).
pub fn conflicts(looked: &Looked) -> Option<FollowUpEvent> {
    if looked.mergeable != Some(Mergeable::Conflicts) || !is_open(looked.pr.state.as_deref()) {
        return None;
    }
    Some(FollowUpEvent::Conflicts {
        sha: looked.head.clone()?,
        base: looked.base_ref.clone(),
    })
}

/// Whether GitHub didn't know yet whether a pull request that is neither
/// merged nor closed conflicts with its base branch, as it doesn't for a
/// while after either branch moved: the session that opened it asks again
/// ([`ask_again_in`]).
pub fn unsure(looked: &Looked) -> bool {
    looked.mergeable == Some(Mergeable::Unknown) && is_open(looked.pr.state.as_deref())
}

/// A pull request whose session asks GitHub again whether it conflicts
/// with its base branch, after GitHub didn't know ([`unsure`]): `asked`
/// times so far.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ask {
    pub repo: RepoRef,
    pub number: u64,
    /// On GitHub, which the session lists it by.
    pub url: String,
    pub asked: u32,
}

/// How long to wait before asking GitHub again whether a pull request
/// conflicts with its base branch, once it was asked `asked` times (the
/// lookup that found GitHub didn't know counts) without knowing; `None`
/// once it was asked as many times as [`ASK_AGAIN_MS`] allows.
pub fn ask_again_in(asked: u32) -> Option<i64> {
    let i = usize::try_from(asked.checked_sub(1)?).ok()?;
    ASK_AGAIN_MS.get(i).copied()
}

/// Whether a pull request in `state` can still change: neither merged nor
/// closed.
pub fn is_open(state: Option<&str>) -> bool {
    !matches!(state, Some("merged" | "closed"))
}

// ───────────────────────────── passing it on ─────────────────────────────

/// What happened to a pull request, for its session (`/follow-up`, or with
/// what GitHub says of its pull requests): a session gathers them for
/// [`GATHER_MS`] and passes them on together ([`message`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// What lets it reach the session once ([`key`]).
    pub key: String,
    pub pull: Pull,
    pub event: FollowUpEvent,
    /// When the lookup that found it asked GitHub, which tells whether a
    /// lookup the session heard of since is newer ([`conflicts_still`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<Millis>,
}

impl Item {
    pub fn new(pull: Pull, event: FollowUpEvent, at: Option<Millis>) -> Self {
        Item {
            key: key(&pull, &event, at),
            pull,
            event,
            seen: None,
        }
    }
}

/// Whether conflicts at commit `sha`, which a lookup at `seen` found, are
/// still news to a session that lists their pull request as `pr`: it is
/// neither merged nor closed, and no later lookup says its head is another
/// commit, as one after the agent's push does. Only a lookup of its checks
/// says which commit is its head (`pr.checks`): one that is older than
/// `seen`, or another commit's checks without a time to go by, can't make
/// the conflicts old news. A pull request the session doesn't list keeps
/// them.
pub fn conflicts_still(pr: Option<&PullRequestRef>, sha: &str, seen: Option<Millis>) -> bool {
    let Some(pr) = pr else {
        return true;
    };
    if !is_open(pr.state.as_deref()) {
        return false;
    }
    pr.checks.as_ref().is_none_or(|c| {
        c.sha.as_deref().is_none_or(|head| head == sha) || seen.is_some_and(|seen| c.at < seen)
    })
}

/// How [`key`] begins for failed checks.
pub const CHECKS_KEY: &str = "follow-up:checks:";

/// How [`key`] begins for conflicts.
pub const CONFLICTS_KEY: &str = "follow-up:conflicts:";

/// What lets `event` reach a session once: the comment or review by its
/// id, failed checks and conflicts by their commit (once the agent pushed
/// what resolves them, conflicts come again only with its next commit), a
/// merge by its pull request, and a close by when it happened (`at`),
/// since one can be opened again.
pub fn key(pull: &Pull, event: &FollowUpEvent, at: Option<Millis>) -> String {
    let pr = pull.reference().to_ascii_lowercase();
    match event {
        FollowUpEvent::Comment(c) => match Place::of(c) {
            Place::Conversation => format!("follow-up:comment:{}", c.id),
            Place::Review => format!("follow-up:review:{}", c.id),
            Place::Lines => format!("follow-up:line:{}", c.id),
        },
        FollowUpEvent::ChecksFailed { sha, .. } => format!("{CHECKS_KEY}{pr}@{sha}"),
        FollowUpEvent::Conflicts { sha, .. } => format!("{CONFLICTS_KEY}{pr}@{sha}"),
        FollowUpEvent::Merged { .. } => format!("follow-up:merged:{pr}"),
        FollowUpEvent::Closed { .. } => format!("follow-up:closed:{pr}@{}", at.unwrap_or(0)),
        FollowUpEvent::Other => format!("follow-up:other:{pr}"),
    }
}

/// What happened to each pull request among `items`, in the order they
/// came: one follow-up for each, with what its newest item says of it, and
/// the keys of its items.
pub fn group(items: &[Item]) -> Vec<(FollowUp, Vec<String>)> {
    let mut out: Vec<(FollowUp, Vec<String>)> = Vec::new();
    for item in items {
        let p = &item.pull;
        let (f, keys) = match out
            .iter()
            .position(|(f, _)| f.url.eq_ignore_ascii_case(&p.url))
        {
            Some(i) => &mut out[i],
            None => {
                out.push((
                    FollowUp {
                        repo: p.repo.clone(),
                        number: p.number,
                        url: p.url.clone(),
                        title: None,
                        state: None,
                        base: None,
                        events: Vec::new(),
                    },
                    Vec::new(),
                ));
                let last = out.len() - 1;
                &mut out[last]
            }
        };
        f.title = p.title.clone().or(f.title.take());
        f.state = p.state.clone().or(f.state.take());
        f.base = p.base.clone().or(f.base.take());
        f.events.push(item.event.clone());
        keys.push(item.key.clone());
    }
    out
}

/// Whether a follow-up has words from people in it, which count as a
/// person's message to the agent.
pub fn from_people(f: &FollowUp) -> bool {
    f.events
        .iter()
        .any(|e| matches!(e, FollowUpEvent::Comment(_)))
}

/// What comes back on its own, again and again, with nobody writing to the
/// agent: fctl passes each kind on at most so many times in a row while
/// nobody messages the agent, or writes to it on GitHub ([`in_a_row`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeats {
    /// Failed checks: a fix that fails again.
    Checks,
    /// Conflicts with the base branch: one after another merge into it.
    Conflicts,
}

impl Repeats {
    pub const ALL: [Repeats; 2] = [Repeats::Checks, Repeats::Conflicts];

    /// Whether `e` is of this kind.
    pub fn of(self, e: &FollowUpEvent) -> bool {
        match self {
            Repeats::Checks => matches!(e, FollowUpEvent::ChecksFailed { .. }),
            Repeats::Conflicts => matches!(e, FollowUpEvent::Conflicts { .. }),
        }
    }

    /// How the keys of its items begin ([`key`]).
    pub fn key(self) -> &'static str {
        match self {
            Repeats::Checks => CHECKS_KEY,
            Repeats::Conflicts => CONFLICTS_KEY,
        }
    }

    /// The most fctl passes on in a row.
    pub fn most(self) -> u32 {
        match self {
            Repeats::Checks => MAX_CHECKS_IN_A_ROW,
            Repeats::Conflicts => MAX_CONFLICTS_IN_A_ROW,
        }
    }
}

/// How many of `kind` in a row the agent has heard of once `f` reaches it,
/// `before` it, and whether fctl holds `f`'s news of that kind back: people
/// who wrote to it end a run, as a person's message does, and news past
/// [`Repeats::most`] stays out until someone does.
pub fn in_a_row(f: &FollowUp, kind: Repeats, before: u32) -> (u32, bool) {
    let before = if from_people(f) { 0 } else { before };
    let news = f.events.iter().any(|e| kind.of(e));
    match (news, before >= kind.most()) {
        (false, _) => (before, false),
        (true, true) => (before, true),
        (true, false) => (before + 1, false),
    }
}

/// The message fctl sends the agent of the session that opened the pull
/// request: what happened there, each part with how to act on it. People's
/// comments come first.
pub fn message(f: &FollowUp) -> String {
    let comments: Vec<&FollowUpComment> = f
        .events
        .iter()
        .filter_map(|e| match e {
            FollowUpEvent::Comment(c) => Some(c),
            _ => None,
        })
        .collect();
    let mut parts = Vec::new();
    if !comments.is_empty() {
        parts.push(comments_part(f, &comments));
    }
    for e in &f.events {
        match e {
            FollowUpEvent::ChecksFailed {
                sha,
                names,
                passed,
                pending,
                ..
            } => parts.push(checks_part(f, sha, names, *passed, *pending)),
            FollowUpEvent::Merged { by, base } => {
                parts.push(merged_part(f, by.as_deref(), base.as_deref()))
            }
            FollowUpEvent::Closed { by } => parts.push(closed_part(f, by.as_deref())),
            FollowUpEvent::Conflicts { sha, base } => {
                parts.push(conflicts_part(f, sha, base.as_deref()))
            }
            FollowUpEvent::Comment(_) | FollowUpEvent::Other => {}
        }
    }
    let mut s = parts.join("\n\n");
    s.push('\n');
    s
}

/// `your pull request acme/web#12, “Fix the login redirect”`.
fn your_pull_request(f: &FollowUp) -> String {
    let pr = format!("{}#{}", f.repo.full_name(), f.number);
    match title(f) {
        Some(t) => format!("your pull request {pr}, “{t}”"),
        None => format!("your pull request {pr}"),
    }
}

/// [`your_pull_request`] as the subject of what follows:
/// `your pull request acme/web#12, “Fix the login redirect”,`.
fn your_pull_request_then(f: &FollowUp) -> String {
    let your = your_pull_request(f);
    if title(f).is_some() {
        format!("{your},")
    } else {
        your
    }
}

fn title(f: &FollowUp) -> Option<&str> {
    f.title.as_deref().map(str::trim).filter(|t| !t.is_empty())
}

/// `src/login.rs line 42`, `src/login.rs lines 40–42 of the old version`.
fn lines_of(c: &FollowUpComment) -> String {
    let path = c.path.as_deref().unwrap_or("a file");
    let lines = match (c.start_line, c.line) {
        (Some(start), Some(line)) if start != line => format!(" lines {start}–{line}"),
        (_, Some(line)) => format!(" line {line}"),
        _ => String::new(),
    };
    let old = if c.deleted { " of the old version" } else { "" };
    format!("{path}{lines}{old}")
}

/// What a review did, after `reviewed …`: `, and approved it`.
fn verdict(c: &FollowUpComment) -> &'static str {
    match c.review.as_deref() {
        Some("approved") => ", and approved it",
        Some("changes_requested") => ", and requested changes",
        _ => "",
    }
}

/// A comment's text, quoted.
fn quoted(body: &str, indent: &str) -> String {
    if body.trim().is_empty() {
        return format!("{indent}(no text)\n");
    }
    body.trim()
        .lines()
        .map(|l| {
            if l.trim().is_empty() {
                format!("{indent}>\n")
            } else {
                format!("{indent}> {l}\n")
            }
        })
        .collect()
}

/// What people wrote: who, where and what, a review's comments on lines
/// under it, and how to answer each where it was made.
fn comments_part(f: &FollowUp, comments: &[&FollowUpComment]) -> String {
    let your = your_pull_request(f);
    // A review's comments on lines go under it.
    let reviews: Vec<u64> = comments
        .iter()
        .filter(|c| c.review.is_some())
        .map(|c| c.id)
        .collect();
    let under = |c: &FollowUpComment| c.review_id.is_some_and(|r| reviews.contains(&r));
    let top: Vec<&&FollowUpComment> = comments.iter().filter(|c| !under(c)).collect();
    let mut s = String::new();
    let what = |c: &FollowUpComment| match Place::of(c) {
        Place::Conversation => "commented".to_owned(),
        Place::Review => format!("reviewed it{}", verdict(c)),
        Place::Lines if c.in_reply_to.is_some() => format!("replied on {}", lines_of(c)),
        Place::Lines => format!("commented on {}", lines_of(c)),
    };
    if let [one] = top.as_slice() {
        let head = match Place::of(one) {
            Place::Conversation => format!("@{} commented on {your}", one.login),
            Place::Review => format!("@{} reviewed {your}{}", one.login, verdict(one)),
            Place::Lines if one.in_reply_to.is_some() => {
                format!("@{} replied on {} of {your}", one.login, lines_of(one))
            }
            Place::Lines => format!("@{} commented on {} of {your}", one.login, lines_of(one)),
        };
        s.push_str(&format!("On GitHub, {head} ({}):\n\n", one.url));
        s.push_str(&quoted(&one.body, ""));
        s.push_str(&thread_note(f, one, ""));
    } else {
        s.push_str(&format!("On GitHub, people wrote to you on {your}:\n"));
        for (n, c) in top.iter().enumerate() {
            s.push_str(&format!(
                "\n{}. @{} {} ({}):\n",
                n + 1,
                c.login,
                what(c),
                c.url
            ));
            s.push_str(&quoted(&c.body, "   "));
            s.push_str(&thread_note(f, c, "   "));
            s.push_str(&review_lines(comments, c, "   "));
        }
    }
    if let [one] = top.as_slice() {
        s.push_str(&review_lines(comments, one, ""));
    }
    let repo = f.repo.full_name();
    let what = if top.len() == 1 && comments.len() == 1 {
        "it asks"
    } else {
        "they ask"
    };
    let act = match f.state.as_deref() {
        Some("merged") => format!(
            "The pull request is merged: do what {what} in a new pull request from {}, not on its branch",
            updated(f.base.as_deref())
        ),
        Some("closed") => format!(
            "The pull request is closed: do what {what} if it still makes sense, without opening it again unless they ask you to"
        ),
        _ => format!("Do what {what} if it belongs in this pull request, and push"),
    };
    s.push_str(&format!(
        "\nOnly people who can write to {repo} reach you this way. {act}; answer questions; if a request seems wrong or out of place here, say why instead of doing it. "
    ));
    let (owner, name, number) = (&f.repo.owner, &f.repo.name, f.number);
    let mut answers = Vec::new();
    if comments.iter().any(|c| Place::of(c) != Place::Lines) {
        answers.push(format!(
            "on the pull request (`gh pr comment {number} --repo {owner}/{name} --body '…'`)"
        ));
    }
    if let Some(c) = comments.iter().find(|c| Place::of(c) == Place::Lines) {
        let id = c.in_reply_to.unwrap_or(c.id);
        answers.push(format!(
            "in a comment's thread (`gh api repos/{owner}/{name}/pulls/{number}/comments/{id}/replies -f body='…'`)"
        ));
    }
    s.push_str(&format!(
        "Then answer where they wrote: {}.",
        answers.join(", and ")
    ));
    s
}

/// For a comment on lines: the thread it answers, which the agent reads
/// first.
fn thread_note(f: &FollowUp, c: &FollowUpComment, indent: &str) -> String {
    match c.in_reply_to {
        Some(id) if Place::of(c) == Place::Lines => format!(
            "{indent}It answers comment {id} in its thread (`gh api repos/{}/{}/pulls/comments/{id}`).\n",
            f.repo.owner, f.repo.name
        ),
        _ => String::new(),
    }
}

/// A review's comments on lines, under it.
fn review_lines(all: &[&FollowUpComment], review: &FollowUpComment, indent: &str) -> String {
    if review.review.is_none() {
        return String::new();
    }
    let lines: Vec<&&FollowUpComment> = all
        .iter()
        .filter(|c| c.review_id == Some(review.id) && c.review.is_none())
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    let mut s = format!(
        "\n{indent}{}\n",
        if lines.len() == 1 {
            "Its comment on lines:"
        } else {
            "Its comments on lines:"
        }
    );
    let inner = format!("{indent}   ");
    for (n, c) in lines.iter().enumerate() {
        s.push_str(&format!(
            "\n{indent}{}. {} · comment {} ({}):\n",
            n + 1,
            lines_of(c),
            c.id,
            c.url
        ));
        s.push_str(&quoted(&c.body, &inner));
    }
    s
}

/// `Rust tests`, `Rust tests and Clippy`, `Lint, Rust tests and Clippy`,
/// and how many more failed past [`MAX_NAMES`].
fn names(names: &[String]) -> String {
    let shown: Vec<&str> = names.iter().take(MAX_NAMES).map(String::as_str).collect();
    let more = names.len().saturating_sub(MAX_NAMES);
    match (shown.as_slice(), more) {
        ([], _) => "A check".to_owned(),
        ([one], 0) => (*one).to_owned(),
        (all, 0) => {
            let (last, rest) = all.split_last().map_or(("", &[][..]), |(l, r)| (*l, r));
            format!("{} and {last}", rest.join(", "))
        }
        (all, more) => format!("{} and {more} more", all.join(", ")),
    }
}

/// That checks failed: which, what still runs, and how to look into them.
fn checks_part(f: &FollowUp, sha: &str, failed: &[String], passed: u32, pending: u32) -> String {
    let (owner, name, number) = (&f.repo.owner, &f.repo.name, f.number);
    let which = match failed.len() {
        0 | 1 => "a check".to_owned(),
        n => format!("{n} checks"),
    };
    let mut s = format!(
        "On GitHub, {which} of {} failed at commit {}: {}.",
        your_pull_request_then(f),
        short(sha),
        names(failed)
    );
    let running = match pending {
        1 => "1 is still running".to_owned(),
        n => format!("{n} are still running"),
    };
    match (pending, passed) {
        (0, 0) => {}
        (0, passed) => s.push_str(&format!(" The other {passed} passed.")),
        (_, 0) => s.push_str(&format!(" {running}.")),
        (_, passed) => s.push_str(&format!(" {running}, and {passed} passed.")),
    }
    s.push_str(&format!(
        "\n\nSee what failed with `gh pr checks {number} --repo {owner}/{name}`, and a workflow run's log with `gh run view RUN --repo {owner}/{name} --log-failed` (the run is in the check's link). Fix what your changes broke, in this pull request, and push."
    ));
    if pending > 0 {
        s.push_str(&format!(
            " Wait for the checks still running first (`gh pr checks {number} --repo {owner}/{name} --watch`), so that one push fixes them all."
        ));
    }
    s.push_str(
        " A failure that has nothing to do with your changes (a flaky test, an outage) is no reason to change them: say so on the pull request instead.",
    );
    s
}

/// That it merged: what may be left to do.
fn merged_part(f: &FollowUp, by: Option<&str>, base: Option<&str>) -> String {
    let base = base.or(f.base.as_deref());
    let into = base.map(|b| format!(" into {b}")).unwrap_or_default();
    let by = by.map(|b| format!(" by @{b}")).unwrap_or_default();
    format!(
        "On GitHub, {} was merged{into}{by}.\n\nDo what is left to do now that it merged, if anything is: what your instructions say to do after a merge, such as noting it in the issue it fixes. Don't push to its branch again: more changes go in a new pull request from {}. If nothing is left, say so in a sentence.",
        your_pull_request_then(f),
        updated(base)
    )
}

/// `the updated main`, or its base branch when nobody said which.
fn updated(base: Option<&str>) -> String {
    base.map_or_else(
        || "its base branch, updated".to_owned(),
        |b| format!("the updated {b}"),
    )
}

/// That it closed without merging: why, and that it stays closed.
fn closed_part(f: &FollowUp, by: Option<&str>) -> String {
    let (owner, name, number) = (&f.repo.owner, &f.repo.name, f.number);
    let by = by.map(|b| format!(", by @{b}")).unwrap_or_default();
    format!(
        "On GitHub, {} was closed without being merged{by}.\n\nIf someone said why, read it (`gh pr view {number} --repo {owner}/{name} --comments`), and do what they asked, if anything. Don't open it again or push to it unless someone asks you to. If nothing is left, say so in a sentence.",
        your_pull_request_then(f)
    )
}

/// That it conflicts with its base branch: how to resolve that.
fn conflicts_part(f: &FollowUp, sha: &str, base: Option<&str>) -> String {
    let (owner, name, number) = (&f.repo.owner, &f.repo.name, f.number);
    let base = base.or(f.base.as_deref());
    let (with, merge) = match base {
        Some(b) => (
            b.to_owned(),
            format!(
                "the updated {b} into its branch (`git fetch origin {b} && git merge origin/{b}`)"
            ),
        ),
        None => (
            "its base branch".to_owned(),
            format!(
                "its base branch, updated, into its branch (`gh pr view {number} --repo {owner}/{name} --json baseRefName` names it)"
            ),
        ),
    };
    format!(
        "On GitHub, {} has conflicts with {with} at commit {}: it can't merge until they're resolved, and GitHub runs none of its `pull_request` workflows until then.\n\nMerge {merge}, resolve each conflict so that both sides' changes still do what they were for, run the tests that cover them, and push. Don't rebase or force-push: that rewrites commits people may have reviewed. If a conflict takes a decision that the code and the pull request don't answer, ask on the pull request (`gh pr comment {number} --repo {owner}/{name} --body '…'`) instead of guessing.",
        your_pull_request_then(f),
        short(sha),
    )
}

/// What the transcript says instead of passing news of `kind` on, once the
/// agent heard of [`Repeats::most`] in a row with no message from a person
/// in between.
pub fn held_back(f: &FollowUp, kind: Repeats) -> Option<String> {
    let pr = format!("{}#{}", f.repo.full_name(), f.number);
    let most = kind.most();
    f.events.iter().find_map(|e| match e {
        FollowUpEvent::ChecksFailed { sha, names: failed, .. } if kind == Repeats::Checks => {
            Some(format!(
                "Checks failed on {pr} at commit {}: {}. fctl didn't pass it on: the agent heard of failed checks {most} times in a row, with no message from a person in between. Message the agent to have it look.",
                short(sha),
                names(failed)
            ))
        }
        FollowUpEvent::Conflicts { sha, base } if kind == Repeats::Conflicts => {
            let with = base
                .as_deref()
                .or(f.base.as_deref())
                .unwrap_or("its base branch");
            Some(format!(
                "{pr} has conflicts with {with} at commit {}. fctl didn't pass it on: the agent heard of conflicts {most} times in a row, with no message from a person in between. Message the agent to have it resolve them.",
                short(sha)
            ))
        }
        _ => None,
    })
}

/// `s` cut to `max` characters, with an ellipsis where it was cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webhooks::{self, Delivery, Hints};
    use fctl_proto::session::{PullRequestChecks, PullRequestRef};
    use serde_json::json;

    #[test]
    fn mentions_and_names_at_the_start_address_the_agent() {
        for yes in [
            "@fctl please keep the query string",
            "@Agent, rename the flag",
            "Thanks! @fctl, one more thing: the docs.",
            "Could you (@agent) look at this?",
            "**@fctl** the tests are flaky",
            "_@fctl_ have a look",
            "ask @fctl's agent",
            "@fctl.",
            "@fctl[bot] go",
            "fctl: rename it",
            "Agent, rename it",
            "AGENT: rename it",
            "agent:\nrename it",
            "agent:",
            "**agent:** rename it",
            "  fctl, rename it",
            "\n\nfctl: rename it",
            "> They said no.\n\nagent: do it anyway",
            "<!-- a template -->\nagent: rename it",
            "```\nsome code\n```\nthen @fctl",
        ] {
            assert!(addressed(yes), "{yes:?}");
        }
        for no in [
            "",
            "fctl",
            "fctl is great",
            "fctl:rename it",
            "the agent: did it",
            "agentic: no",
            "fctls: no",
            "Agents, rename it",
            "write to me@fctl.sh",
            "see https://example.com/@fctl",
            "@fctl-sh rename it",
            "@agents rename it",
            "@acme/fctl rename it",
            "@@fctl rename it",
            "> @fctl do it",
            "> fctl: do it\n\nI agree with the reviewer.",
            "Write `@fctl` in a comment to ask the agent",
            "Write `` @fctl `` in a comment",
            "```@fctl```",
            "```\n@fctl do it\n```",
            "~~~sh\nagent: do it\n~~~",
            "````\n```\n@fctl\n```\n````",
            "<!-- @fctl -->",
            "<!-- agent:\nhidden",
            "`fctl: ` at the start of a comment asks the agent",
        ] {
            assert!(!addressed(no), "{no:?}");
        }
    }

    fn user(login: &str, kind: &str) -> Value {
        json!({ "login": login, "type": kind, "avatar_url": format!("https://avatars.githubusercontent.com/u/1?v=4&l={login}") })
    }

    fn repository() -> Value {
        json!({ "full_name": "acme/web", "owner": { "login": "acme", "id": 314 } })
    }

    fn pull_request() -> Value {
        json!({
            "number": 12,
            "html_url": "https://github.com/acme/web/pull/12",
            "title": "Fix the login redirect",
            "state": "open",
            "draft": false,
            "merged_at": null,
            "user": user("fctl[bot]", "Bot"),
            "head": { "ref": "fctl/7-fix-login", "sha": "3f2a1c9d" },
            "base": { "ref": "main" },
        })
    }

    fn issue_comment(body: &str) -> Value {
        json!({
            "action": "created",
            "issue": {
                "number": 12,
                "html_url": "https://github.com/acme/web/pull/12",
                "title": "Fix the login redirect",
                "state": "open",
                "user": user("fctl[bot]", "Bot"),
                "pull_request": { "html_url": "https://github.com/acme/web/pull/12", "merged_at": null },
            },
            "comment": {
                "id": 123,
                "node_id": "IC_123",
                "html_url": "https://github.com/acme/web/pull/12#issuecomment-123",
                "body": body,
                "user": user("ada", "User"),
                "author_association": "MEMBER",
                "created_at": "2026-10-23T10:00:00Z",
            },
            "repository": repository(),
            "installation": { "id": 42 },
        })
    }

    fn some<T>(v: Option<T>) -> T {
        v.unwrap_or_else(|| panic!("nothing"))
    }

    #[test]
    fn comments_on_the_conversation_say_who_wrote_what() {
        let c = some(commented(
            "issue_comment",
            &issue_comment("@fctl keep the query string"),
        ));
        assert_eq!(c.pull.reference(), "acme/web#12");
        assert_eq!(c.pull.url, "https://github.com/acme/web/pull/12");
        assert_eq!(c.pull.state.as_deref(), Some("open"));
        assert_eq!(c.author.as_deref(), Some("fctl[bot]"));
        assert_eq!(c.head_ref, None);
        assert_eq!(c.place(), Place::Conversation);
        assert_eq!((c.comment.id, c.comment.login.as_str()), (123, "ada"));
        assert_eq!(c.comment.body, "@fctl keep the query string");
        assert_eq!(
            c.comment.at,
            crate::pulls::github_time("2026-10-23T10:00:00Z")
        );
        assert_eq!(c.node_id.as_deref(), Some("IC_123"));
        assert_eq!(c.association.as_deref(), Some("MEMBER"));
        assert!(c.person);
        assert_eq!((c.installation, c.owner_id), (Some(42), Some(314)));
        assert_eq!(passed_over(&c, Some("fctl[bot]")), None);

        // Its pull request merged meanwhile.
        let mut merged = issue_comment("@fctl one more thing");
        merged["issue"]["state"] = json!("closed");
        merged["issue"]["pull_request"]["merged_at"] = json!("2026-10-23T09:00:00Z");
        let c = some(commented("issue_comment", &merged));
        assert_eq!(c.pull.state.as_deref(), Some("merged"));

        // An edit, or a comment on an issue, is no news.
        let mut edited = issue_comment("@fctl hi");
        edited["action"] = json!("edited");
        assert_eq!(commented("issue_comment", &edited), None);
        let mut issue = issue_comment("@fctl hi");
        issue["issue"]
            .as_object_mut()
            .map(|o| o.remove("pull_request"));
        assert_eq!(commented("issue_comment", &issue), None);
        assert_eq!(commented("issues", &issue_comment("@fctl hi")), None);
    }

    #[test]
    fn only_people_who_can_write_reach_fctls_own_pull_requests() {
        let app = Some("fctl[bot]");
        let c = some(commented("issue_comment", &issue_comment("@fctl hi")));
        let with = |f: &dyn Fn(&mut Value)| {
            let mut p = issue_comment("@fctl hi");
            f(&mut p);
            passed_over(&some(commented("issue_comment", &p)), app)
        };
        assert_eq!(passed_over(&c, app), None);
        assert_eq!(
            with(&|p| p["issue"]["user"] = user("ada", "User")),
            Some("fctl's agents didn't open it")
        );
        assert_eq!(passed_over(&c, None), Some("fctl's agents didn't open it"));
        assert_eq!(
            with(&|p| p["comment"]["user"] = user("renovate[bot]", "Bot")),
            Some("an app wrote it, not a person")
        );
        // A bot of type User, as some apps' are, is still an app's.
        assert_eq!(
            with(&|p| p["comment"]["user"] = user("ci[bot]", "User")),
            Some("an app wrote it, not a person")
        );
        for outsider in ["CONTRIBUTOR", "FIRST_TIME_CONTRIBUTOR", "NONE", "MANNEQUIN"] {
            assert_eq!(
                with(&|p| p["comment"]["author_association"] = json!(outsider)),
                Some("its author can't write to the repository"),
                "{outsider}"
            );
        }
        assert_eq!(
            with(&|p| p["comment"]["body"] = json!("Looks good to me")),
            Some("it doesn't address the agent")
        );
        for insider in ["OWNER", "MEMBER", "COLLABORATOR"] {
            assert!(may_write(Some(insider)));
        }
        assert!(!may_write(None));
    }

    #[test]
    fn write_access_is_admin_maintain_or_write() {
        assert!(can_write(&json!({ "permission": "admin" })));
        assert!(can_write(
            &json!({ "permission": "write", "role_name": "maintain" })
        ));
        assert!(can_write(&json!({ "permission": "write" })));
        assert!(can_write(
            &json!({ "user": { "permissions": { "push": true } } })
        ));
        // Triage maps to read; an organization's member may only read.
        assert!(!can_write(
            &json!({ "permission": "read", "role_name": "triage",
            "user": { "permissions": { "push": false, "triage": true } } })
        ));
        assert!(!can_write(&json!({ "permission": "none" })));
        assert!(!can_write(&json!({ "message": "Not Found" })));
    }

    fn line_comment() -> Value {
        json!({
            "id": 456,
            "node_id": "PRRC_456",
            "pull_request_review_id": 789,
            "html_url": "https://github.com/acme/web/pull/12#discussion_r456",
            "body": "Use the shared helper here.",
            "path": "src/login.rs",
            "line": 42,
            "start_line": 40,
            "side": "RIGHT",
            "user": user("ada", "User"),
            "author_association": "COLLABORATOR",
            "created_at": "2026-10-23T10:00:00Z",
        })
    }

    #[test]
    fn reviews_and_comments_on_lines_say_where() {
        let mut comment = line_comment();
        comment["body"] = json!("@fctl use the shared helper");
        comment["in_reply_to_id"] = json!(450);
        let delivery = json!({
            "action": "created", "comment": comment, "pull_request": pull_request(),
            "repository": repository(), "installation": { "id": 42 },
        });
        let c = some(commented("pull_request_review_comment", &delivery));
        assert_eq!(c.place(), Place::Lines);
        assert_eq!(c.head_ref.as_deref(), Some("fctl/7-fix-login"));
        assert_eq!(c.pull.base.as_deref(), Some("main"));
        assert_eq!(c.comment.path.as_deref(), Some("src/login.rs"));
        assert_eq!((c.comment.start_line, c.comment.line), (Some(40), Some(42)));
        assert_eq!(
            (c.comment.review_id, c.comment.in_reply_to),
            (Some(789), Some(450))
        );
        assert!(!c.comment.deleted);

        // On the old version's lines, and outdated by a later commit.
        let mut old = line_comment();
        old["side"] = json!("LEFT");
        old["line"] = Value::Null;
        old["start_line"] = Value::Null;
        old["original_line"] = json!(9);
        let listed = review_comments(&json!([old, { "id": 1 }]));
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].line, listed[0].start_line), (Some(9), None));
        assert!(listed[0].deleted);

        let review = json!({
            "action": "submitted",
            "review": {
                "id": 789, "node_id": "PRR_789", "body": null, "state": "changes_requested",
                "html_url": "https://github.com/acme/web/pull/12#pullrequestreview-789",
                "user": user("ada", "User"), "author_association": "OWNER",
                "submitted_at": "2026-10-23T10:01:00Z",
            },
            "pull_request": pull_request(), "repository": repository(),
        });
        let c = some(commented("pull_request_review", &review));
        assert_eq!(c.place(), Place::Review);
        assert_eq!(c.comment.review.as_deref(), Some("changes_requested"));
        assert_eq!(c.comment.body, "");
        assert_eq!(
            c.comment.at,
            crate::pulls::github_time("2026-10-23T10:01:00Z")
        );
        assert_eq!(
            passed_over(&c, Some("fctl[bot]")),
            Some("it doesn't address the agent")
        );
        let mut dismissed = review.clone();
        dismissed["action"] = json!("dismissed");
        assert_eq!(commented("pull_request_review", &dismissed), None);
    }

    fn pull() -> Pull {
        Pull {
            repo: RepoRef {
                owner: "acme".into(),
                name: "web".into(),
            },
            number: 12,
            url: "https://github.com/acme/web/pull/12".into(),
            title: Some("Fix the login redirect".into()),
            state: Some("open".into()),
            base: Some("main".into()),
        }
    }

    fn comment(id: u64, login: &str, body: &str) -> FollowUpComment {
        FollowUpComment {
            id,
            login: login.into(),
            avatar_url: None,
            url: format!("https://github.com/acme/web/pull/12#issuecomment-{id}"),
            body: body.into(),
            review: None,
            path: None,
            line: None,
            start_line: None,
            deleted: false,
            review_id: None,
            in_reply_to: None,
            at: None,
        }
    }

    fn on_lines_of(id: u64, review: Option<u64>, body: &str) -> FollowUpComment {
        FollowUpComment {
            url: format!("https://github.com/acme/web/pull/12#discussion_r{id}"),
            path: Some("src/login.rs".into()),
            line: Some(42),
            review_id: review,
            ..comment(id, "ada", body)
        }
    }

    fn follow_up(events: Vec<FollowUpEvent>) -> FollowUp {
        let p = pull();
        FollowUp {
            repo: p.repo,
            number: p.number,
            url: p.url,
            title: p.title,
            state: p.state,
            base: p.base,
            events,
        }
    }

    #[test]
    fn keys_let_each_reach_a_session_once() {
        let p = pull();
        let c = |c: FollowUpComment| key(&p, &FollowUpEvent::Comment(c), None);
        assert_eq!(c(comment(123, "ada", "x")), "follow-up:comment:123");
        assert_eq!(c(on_lines_of(456, None, "x")), "follow-up:line:456");
        let mut review = comment(789, "ada", "x");
        review.review = Some("approved".into());
        assert_eq!(c(review), "follow-up:review:789");
        let checks = FollowUpEvent::ChecksFailed {
            sha: "abc".into(),
            names: vec![],
            passed: 0,
            failed: 1,
            pending: 0,
        };
        assert_eq!(key(&p, &checks, None), "follow-up:checks:acme/web#12@abc");
        let merged = FollowUpEvent::Merged {
            by: None,
            base: None,
        };
        assert_eq!(key(&p, &merged, Some(5)), "follow-up:merged:acme/web#12");
        let conflicts = FollowUpEvent::Conflicts {
            sha: "def".into(),
            base: Some("main".into()),
        };
        assert_eq!(
            key(&p, &conflicts, None),
            "follow-up:conflicts:acme/web#12@def"
        );
        assert!(Repeats::Conflicts.of(&conflicts) && !Repeats::Checks.of(&conflicts));
        assert!(key(&p, &conflicts, None).starts_with(Repeats::Conflicts.key()));
        assert!(key(&p, &checks, None).starts_with(Repeats::Checks.key()));
        // A pull request closed, opened again and closed again: twice.
        let closed = FollowUpEvent::Closed { by: None };
        assert_ne!(key(&p, &closed, Some(5)), key(&p, &closed, Some(6)));
    }

    #[test]
    fn items_group_by_pull_request_in_the_order_they_came() {
        let other = Pull {
            repo: RepoRef {
                owner: "acme".into(),
                name: "lib".into(),
            },
            number: 3,
            url: "https://github.com/acme/lib/pull/3".into(),
            title: None,
            state: None,
            base: None,
        };
        let mut newer = pull();
        newer.title = Some("Fix the login redirect for good".into());
        newer.state = Some("merged".into());
        let items = [
            Item::new(pull(), FollowUpEvent::Comment(comment(1, "ada", "a")), None),
            Item::new(other.clone(), FollowUpEvent::Closed { by: None }, Some(1)),
            Item::new(newer, FollowUpEvent::Comment(comment(2, "bob", "b")), None),
        ];
        let groups = group(&items);
        assert_eq!(groups.len(), 2);
        let (first, keys) = &groups[0];
        assert_eq!(first.events.len(), 2);
        assert_eq!(
            first.title.as_deref(),
            Some("Fix the login redirect for good")
        );
        assert_eq!(first.state.as_deref(), Some("merged"));
        assert_eq!(first.base.as_deref(), Some("main"));
        assert_eq!(keys, &["follow-up:comment:1", "follow-up:comment:2"]);
        let (second, keys) = &groups[1];
        assert_eq!((second.number, second.events.len()), (3, 1));
        assert_eq!(keys, &["follow-up:closed:acme/lib#3@1"]);
        assert!(from_people(first));
        assert!(!from_people(second));
    }

    #[test]
    fn failed_checks_reach_the_agent_three_times_in_a_row_at_most() {
        let checks = FollowUpEvent::ChecksFailed {
            sha: "abc".into(),
            names: vec!["Lint".into()],
            passed: 0,
            failed: 1,
            pending: 0,
        };
        let failed = follow_up(vec![checks.clone()]);
        let mut n = 0;
        for want in [1, 2, 3] {
            let (next, held) = in_a_row(&failed, Repeats::Checks, n);
            assert_eq!((next, held), (want, false));
            n = next;
        }
        assert_eq!(in_a_row(&failed, Repeats::Checks, n), (3, true));
        // Someone wrote to the agent: a new run starts, with them.
        let with_people = follow_up(vec![
            FollowUpEvent::Comment(comment(1, "ada", "@fctl look")),
            checks.clone(),
        ]);
        assert_eq!(in_a_row(&with_people, Repeats::Checks, n), (1, false));
        let comment_alone = follow_up(vec![FollowUpEvent::Comment(comment(1, "ada", "@fctl"))]);
        assert_eq!(in_a_row(&comment_alone, Repeats::Checks, n), (0, false));
        // A merge counts for nothing.
        let merged = follow_up(vec![FollowUpEvent::Merged {
            by: None,
            base: None,
        }]);
        assert_eq!(in_a_row(&merged, Repeats::Checks, 2), (2, false));
        // Conflicts count apart from failed checks, as many times.
        let conflicts = follow_up(vec![FollowUpEvent::Conflicts {
            sha: "def".into(),
            base: Some("main".into()),
        }]);
        assert_eq!(in_a_row(&conflicts, Repeats::Checks, 3), (3, false));
        let mut n = 0;
        for want in [1, 2, 3] {
            let (next, held) = in_a_row(&conflicts, Repeats::Conflicts, n);
            assert_eq!((next, held), (want, false));
            n = next;
        }
        assert_eq!(in_a_row(&conflicts, Repeats::Conflicts, n), (3, true));
        assert_eq!(in_a_row(&failed, Repeats::Conflicts, 2), (2, false));
        let both = follow_up(vec![
            FollowUpEvent::Comment(comment(1, "ada", "@fctl look")),
            checks,
            FollowUpEvent::Conflicts {
                sha: "def".into(),
                base: None,
            },
        ]);
        assert_eq!(in_a_row(&both, Repeats::Conflicts, 3), (1, false));
    }

    #[test]
    fn a_comment_reaches_the_agent_with_how_to_answer() {
        let m = message(&follow_up(vec![FollowUpEvent::Comment(comment(
            123,
            "ada",
            "agent: the redirect should keep the query string too.\n\nAnd the hash.",
        ))]));
        assert!(m.starts_with(
            "On GitHub, @ada commented on your pull request acme/web#12, “Fix the login redirect” (https://github.com/acme/web/pull/12#issuecomment-123):\n\n> agent: the redirect should keep the query string too.\n>\n> And the hash.\n"
        ), "{m}");
        assert!(m.contains("\nOnly people who can write to acme/web reach you this way. Do what it asks if it belongs in this pull request, and push;"), "{m}");
        assert!(m.contains("if a request seems wrong or out of place here, say why instead of doing it. Then answer where they wrote: "), "{m}");
        assert!(m.trim_end().ends_with(
            "Then answer where they wrote: on the pull request (`gh pr comment 12 --repo acme/web --body '…'`)."
        ), "{m}");
        assert!(!m.contains("is merged"));

        // A reply in a thread on lines: the thread first, then the answer there.
        let mut reply = on_lines_of(456, Some(9), "@fctl no, keep it as it was");
        reply.in_reply_to = Some(450);
        reply.start_line = Some(40);
        let m = message(&follow_up(vec![FollowUpEvent::Comment(reply)]));
        assert!(m.starts_with("On GitHub, @ada replied on src/login.rs lines 40–42 of your pull request acme/web#12, “Fix the login redirect” (https://github.com/acme/web/pull/12#discussion_r456):\n\n> @fctl no, keep it as it was\nIt answers comment 450 in its thread (`gh api repos/acme/web/pulls/comments/450`).\n"), "{m}");
        assert!(m.contains("in a comment's thread (`gh api repos/acme/web/pulls/12/comments/450/replies -f body='…'`)"));
        assert!(!m.contains("gh pr comment"));

        // On a merged pull request: changes go in a new one.
        let mut f = follow_up(vec![FollowUpEvent::Comment(comment(
            5,
            "ada",
            "@fctl add a test",
        ))]);
        f.state = Some("merged".into());
        let m = message(&f);
        assert!(m.contains(
            "reach you this way. The pull request is merged: do what it asks in a new pull request from the updated main, not on its branch; answer questions;"
        ), "{m}");
        assert!(!m.contains("and push"), "{m}");
        f.state = Some("closed".into());
        assert!(message(&f).contains("The pull request is closed: do what it asks if it still makes sense, without opening it again unless they ask you to;"));
    }

    #[test]
    fn a_review_brings_its_comments_on_lines() {
        let mut review = comment(789, "ada", "@fctl please address these before we merge.");
        review.review = Some("changes_requested".into());
        review.url = "https://github.com/acme/web/pull/12#pullrequestreview-789".into();
        let mut second = on_lines_of(457, Some(789), "This needs a test.");
        second.path = Some("src/api.rs".into());
        second.start_line = Some(10);
        second.line = Some(12);
        second.deleted = true;
        let events = vec![
            FollowUpEvent::Comment(review),
            FollowUpEvent::Comment(on_lines_of(456, Some(789), "Use the shared helper here.")),
            FollowUpEvent::Comment(second),
        ];
        let m = message(&follow_up(events));
        assert!(m.starts_with("On GitHub, @ada reviewed your pull request acme/web#12, “Fix the login redirect”, and requested changes (https://github.com/acme/web/pull/12#pullrequestreview-789):\n\n> @fctl please address these before we merge.\n\nIts comments on lines:\n\n1. src/login.rs line 42 · comment 456 (https://github.com/acme/web/pull/12#discussion_r456):\n   > Use the shared helper here.\n\n2. src/api.rs lines 10–12 of the old version · comment 457 (https://github.com/acme/web/pull/12#discussion_r457):\n   > This needs a test.\n"), "{m}");
        assert!(m.contains("on the pull request (`gh pr comment 12"));
        assert!(m.contains(
            "and in a comment's thread (`gh api repos/acme/web/pulls/12/comments/456/replies"
        ));
    }

    #[test]
    fn comments_of_several_people_come_together() {
        let m = message(&follow_up(vec![
            FollowUpEvent::Comment(comment(1, "ada", "@fctl rename it")),
            FollowUpEvent::Comment(on_lines_of(2, None, "agent: and here")),
            FollowUpEvent::Comment(comment(3, "bob", "@agent and the docs")),
        ]));
        assert!(m.starts_with("On GitHub, people wrote to you on your pull request acme/web#12, “Fix the login redirect”:\n\n1. @ada commented (https://github.com/acme/web/pull/12#issuecomment-1):\n   > @fctl rename it\n\n2. @ada commented on src/login.rs line 42 (https://github.com/acme/web/pull/12#discussion_r2):\n   > agent: and here\n\n3. @bob commented (https://github.com/acme/web/pull/12#issuecomment-3):\n   > @agent and the docs\n"), "{m}");
        assert!(
            m.contains(
                "\nOnly people who can write to acme/web reach you this way. Do what they ask"
            ),
            "{m}"
        );
    }

    #[test]
    fn failed_checks_say_which_and_what_still_runs() {
        let checks = |names: &[&str], passed, pending| FollowUpEvent::ChecksFailed {
            sha: "3f2a1c9d0e".into(),
            names: names.iter().map(|n| (*n).to_owned()).collect(),
            passed,
            failed: names.len() as u32,
            pending,
        };
        let m = message(&follow_up(vec![checks(&["Rust tests", "Clippy"], 7, 1)]));
        assert!(m.starts_with("On GitHub, 2 checks of your pull request acme/web#12, “Fix the login redirect”, failed at commit 3f2a1c9: Rust tests and Clippy. 1 is still running, and 7 passed.\n\n"), "{m}");
        assert!(m.contains("`gh pr checks 12 --repo acme/web`"));
        assert!(m.contains("`gh run view RUN --repo acme/web --log-failed`"));
        assert!(m.contains(
            "Wait for the checks still running first (`gh pr checks 12 --repo acme/web --watch`)"
        ));
        let m = message(&follow_up(vec![checks(&["Lint"], 0, 0)]));
        assert!(m.starts_with("On GitHub, a check of your pull request acme/web#12, “Fix the login redirect”, failed at commit 3f2a1c9: Lint.\n\n"), "{m}");
        assert!(!m.contains("--watch"));
        let many = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let m = message(&follow_up(vec![checks(&many, 0, 2)]));
        assert!(m.contains("8 checks of your pull request acme/web#12, “Fix the login redirect”, failed at commit 3f2a1c9: a, b, c, d, e, f and 2 more. 2 are still running.\n"), "{m}");
        let m = message(&follow_up(vec![checks(&["Lint"], 5, 0)]));
        assert!(m.contains(": Lint. The other 5 passed.\n"), "{m}");
        assert_eq!(
            held_back(
                &follow_up(vec![checks(&["Rust tests", "Clippy"], 7, 1)]),
                Repeats::Checks
            )
            .as_deref(),
            Some(
                "Checks failed on acme/web#12 at commit 3f2a1c9: Rust tests and Clippy. fctl didn't pass it on: the agent heard of failed checks 3 times in a row, with no message from a person in between. Message the agent to have it look."
            )
        );
        assert_eq!(
            held_back(
                &follow_up(vec![FollowUpEvent::Closed { by: None }]),
                Repeats::Checks
            ),
            None
        );
        assert_eq!(
            held_back(
                &follow_up(vec![checks(&["Lint"], 0, 0)]),
                Repeats::Conflicts
            ),
            None
        );
    }

    #[test]
    fn conflicts_say_how_to_resolve_them() {
        let conflicts = |base: Option<&str>| FollowUpEvent::Conflicts {
            sha: "8e1d2b7c6a".into(),
            base: base.map(str::to_owned),
        };
        let m = message(&follow_up(vec![conflicts(Some("main"))]));
        assert_eq!(
            m,
            "On GitHub, your pull request acme/web#12, “Fix the login redirect”, has conflicts with main at commit 8e1d2b7: it can't merge until they're resolved, and GitHub runs none of its `pull_request` workflows until then.\n\nMerge the updated main into its branch (`git fetch origin main && git merge origin/main`), resolve each conflict so that both sides' changes still do what they were for, run the tests that cover them, and push. Don't rebase or force-push: that rewrites commits people may have reviewed. If a conflict takes a decision that the code and the pull request don't answer, ask on the pull request (`gh pr comment 12 --repo acme/web --body '…'`) instead of guessing.\n"
        );
        // The pull request's base when the event doesn't say.
        let m = message(&follow_up(vec![conflicts(None)]));
        assert!(
            m.contains(" has conflicts with main at commit 8e1d2b7: "),
            "{m}"
        );
        // Nobody said which branch.
        let mut nowhere = follow_up(vec![conflicts(None)]);
        nowhere.base = None;
        let m = message(&nowhere);
        assert!(
            m.contains(" has conflicts with its base branch at commit 8e1d2b7: "),
            "{m}"
        );
        assert!(m.contains("Merge its base branch, updated, into its branch (`gh pr view 12 --repo acme/web --json baseRefName` names it), "), "{m}");
        // With failed checks of the same pull request: both, in the order
        // they came.
        let m = message(&follow_up(vec![
            conflicts(Some("main")),
            FollowUpEvent::ChecksFailed {
                sha: "8e1d2b7c6a".into(),
                names: vec!["Lint".into()],
                passed: 0,
                failed: 1,
                pending: 0,
            },
        ]));
        let (first, second) = m.split_once("\n\nOn GitHub, ").unwrap_or_default();
        assert!(first.starts_with("On GitHub, your pull request acme/web#12, “Fix the login redirect”, has conflicts with main"));
        assert!(second.starts_with("a check of your pull request"), "{m}");
        assert_eq!(
            held_back(
                &follow_up(vec![conflicts(Some("main"))]),
                Repeats::Conflicts
            )
            .as_deref(),
            Some(
                "acme/web#12 has conflicts with main at commit 8e1d2b7. fctl didn't pass it on: the agent heard of conflicts 3 times in a row, with no message from a person in between. Message the agent to have it resolve them."
            )
        );
        assert_eq!(
            held_back(&nowhere, Repeats::Conflicts)
                .as_deref()
                .map(|h| h.split(" at ").next()),
            Some(Some("acme/web#12 has conflicts with its base branch"))
        );
        assert_eq!(
            held_back(&follow_up(vec![conflicts(None)]), Repeats::Checks),
            None
        );
    }

    #[test]
    fn merges_and_closes_say_what_is_left() {
        let m = message(&follow_up(vec![FollowUpEvent::Merged {
            by: Some("ada".into()),
            base: Some("main".into()),
        }]));
        assert!(m.starts_with("On GitHub, your pull request acme/web#12, “Fix the login redirect”, was merged into main by @ada.\n\nDo what is left to do now that it merged"), "{m}");
        assert!(m.contains("more changes go in a new pull request from the updated main."));
        let m = message(&follow_up(vec![FollowUpEvent::Closed {
            by: Some("bob".into()),
        }]));
        assert!(m.starts_with("On GitHub, your pull request acme/web#12, “Fix the login redirect”, was closed without being merged, by @bob.\n\n"), "{m}");
        assert!(m.contains("`gh pr view 12 --repo acme/web --comments`"));
        let mut untitled = follow_up(vec![FollowUpEvent::Merged {
            by: None,
            base: None,
        }]);
        untitled.title = None;
        untitled.base = None;
        let m = message(&untitled);
        assert!(
            m.starts_with("On GitHub, your pull request acme/web#12 was merged.\n"),
            "{m}"
        );
        assert!(
            m.contains("a new pull request from its base branch, updated."),
            "{m}"
        );
    }

    fn looked(state: &str, failing: &[&str]) -> Looked {
        Looked {
            pr: PullRequestRef {
                number: 12,
                url: "https://github.com/acme/web/pull/12".into(),
                state: Some(state.into()),
                checks: Some(PullRequestChecks {
                    passed: 3,
                    failed: failing.len() as u32,
                    pending: 1,
                    sha: Some("abc".into()),
                    at: 1,
                }),
                ..Default::default()
            },
            head: Some("abc".into()),
            failing: failing.iter().map(|n| (*n).to_owned()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn failed_checks_of_open_pull_requests_are_news() {
        assert_eq!(
            checks_failed(&looked("draft", &["Lint"])),
            Some(FollowUpEvent::ChecksFailed {
                sha: "abc".into(),
                names: vec!["Lint".into()],
                passed: 3,
                failed: 1,
                pending: 1,
            })
        );
        assert_eq!(checks_failed(&looked("open", &[])), None);
        assert_eq!(checks_failed(&looked("merged", &["Lint"])), None);
        assert_eq!(checks_failed(&looked("closed", &["Lint"])), None);
        let mut other = looked("open", &["Lint"]);
        other.head = Some("def".into());
        assert_eq!(checks_failed(&other), None);
    }

    #[test]
    fn conflicts_of_open_pull_requests_are_news() {
        let with = |state: &str, mergeable: Option<Mergeable>| {
            let mut l = looked(state, &[]);
            l.base_ref = Some("main".into());
            l.mergeable = mergeable;
            l
        };
        assert_eq!(
            conflicts(&with("open", Some(Mergeable::Conflicts))),
            Some(FollowUpEvent::Conflicts {
                sha: "abc".into(),
                base: Some("main".into()),
            })
        );
        // Drafts and pull requests in a merge queue too.
        for state in ["draft", "queued"] {
            assert!(conflicts(&with(state, Some(Mergeable::Conflicts))).is_some());
        }
        for (state, mergeable) in [
            ("open", Some(Mergeable::Yes)),
            ("open", Some(Mergeable::Unknown)),
            ("open", None),
            ("merged", Some(Mergeable::Conflicts)),
            ("closed", Some(Mergeable::Conflicts)),
        ] {
            assert_eq!(
                conflicts(&with(state, mergeable)),
                None,
                "{state} {mergeable:?}"
            );
        }
        // Without its head commit, nothing to say it of.
        let mut headless = with("open", Some(Mergeable::Conflicts));
        headless.head = None;
        assert_eq!(conflicts(&headless), None);
        // GitHub doesn't know yet: worth asking again, while it can merge.
        assert!(unsure(&with("open", Some(Mergeable::Unknown))));
        assert!(unsure(&with("draft", Some(Mergeable::Unknown))));
        assert!(!unsure(&with("merged", Some(Mergeable::Unknown))));
        assert!(!unsure(&with("open", Some(Mergeable::Conflicts))));
        assert!(!unsure(&with("open", None)));
    }

    #[test]
    fn conflicts_are_old_news_once_a_later_lookup_sees_another_commit() {
        let listed = |state: &str, head: Option<&str>, at: Millis| PullRequestRef {
            number: 12,
            url: "https://github.com/acme/web/pull/12".into(),
            state: Some(state.into()),
            checks: head.map(|h| PullRequestChecks {
                sha: Some(h.into()),
                at,
                ..Default::default()
            }),
            ..Default::default()
        };
        // Still the head GitHub last said, or the session knows no checks.
        assert!(conflicts_still(
            Some(&listed("open", Some("abc"), 10)),
            "abc",
            Some(20)
        ));
        assert!(conflicts_still(
            Some(&listed("draft", None, 0)),
            "abc",
            Some(20)
        ));
        assert!(conflicts_still(None, "abc", Some(20)));
        // The agent pushed since: a later lookup says another commit.
        assert!(!conflicts_still(
            Some(&listed("open", Some("def"), 30)),
            "abc",
            Some(20)
        ));
        assert!(!conflicts_still(
            Some(&listed("open", Some("def"), 20)),
            "abc",
            Some(20)
        ));
        // Another commit's checks from before the lookup that found the
        // conflicts: GitHub had a newer head by then.
        assert!(conflicts_still(
            Some(&listed("open", Some("def"), 10)),
            "abc",
            Some(20)
        ));
        // Without a time to go by, another commit makes them old news.
        assert!(!conflicts_still(
            Some(&listed("open", Some("def"), 10)),
            "abc",
            None
        ));
        // Merged or closed: nothing to resolve.
        for state in ["merged", "closed"] {
            assert!(!conflicts_still(
                Some(&listed(state, Some("abc"), 10)),
                "abc",
                Some(20)
            ));
        }
    }

    #[test]
    fn sessions_ask_again_sooner_then_later_then_not() {
        assert_eq!(ask_again_in(0), None);
        assert_eq!(ask_again_in(1), Some(15_000));
        assert_eq!(ask_again_in(2), Some(30_000));
        assert_eq!(ask_again_in(5), Some(240_000));
        assert_eq!(ask_again_in(6), None);
        assert_eq!(ask_again_in(u32::MAX), None);
        // About eight minutes in all, once GitHub was first asked.
        assert_eq!(ASK_AGAIN_MS.iter().sum::<i64>(), 465_000);
    }

    #[test]
    fn closing_deliveries_say_who_merged_or_closed() {
        let delivery = |merged: bool, by: Value| {
            let mut pr = pull_request();
            pr["state"] = json!("closed");
            pr["closed_at"] = json!("2026-10-23T11:00:00Z");
            if merged {
                pr["merged"] = json!(true);
                pr["merged_at"] = json!("2026-10-23T11:00:00Z");
                pr["merged_by"] = by.clone();
            }
            let d = webhooks::delivery(
                "pull_request",
                &json!({ "action": "closed", "pull_request": pr, "sender": by,
                    "repository": repository(), "installation": { "id": 42 } }),
            );
            match d {
                Delivery::Hook(h) => some(h.payload),
                other => panic!("not a hook: {other:?}"),
            }
        };
        assert_eq!(
            closed(&delivery(true, user("ada", "User"))),
            Some(FollowUpEvent::Merged {
                by: Some("ada".into()),
                base: Some("main".into()),
            })
        );
        // Merged by a merge queue: nobody in particular.
        assert_eq!(
            closed(&delivery(true, user("github-merge-queue[bot]", "Bot"))),
            Some(FollowUpEvent::Merged {
                by: None,
                base: Some("main".into()),
            })
        );
        assert_eq!(
            closed(&delivery(false, user("bob", "User"))),
            Some(FollowUpEvent::Closed {
                by: Some("bob".into())
            })
        );
        let mut opened = delivery(false, user("bob", "User"));
        opened.action = "reopened".into();
        assert_eq!(closed(&opened), None);
        let _ = Hints::default();
    }
}
