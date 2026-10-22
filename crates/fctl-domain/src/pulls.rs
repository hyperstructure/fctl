//! The pull requests of a session: every one its agent found, in all of its
//! project's repositories, each with the repository it was opened in.
//!
//! fctl-agent follows the pull requests of the branches its repositories
//! have checked out (`GitState::pull_request`), and those `gh pr create`
//! printed, whichever branch, worktree or clone they came from. It reports
//! them all (`GitState::pull_requests`) and looks them up again after each
//! turn until they merge. Older agents report only the checked-out
//! branches' latest one per repository. The control plane keeps the
//! others: a session that opened one in each of three repositories, or a
//! second one after the first was merged, lists them all.
//!
//! GitHub tells the control plane when they change, through its GitHub
//! App's webhook (`crate::webhooks`): their state, merge queues included,
//! and their checks. What the agent and GitHub say comes together in
//! [`update`]: a lookup older than the one a pull request's state came from
//! doesn't undo it.

use fctl_proto::Millis;
use fctl_proto::event::GitState;
use fctl_proto::session::{PullRequestChecks, PullRequestRef, RepoRef, Session};

/// Most pull requests a session lists: the first its agent found.
pub const MAX_PULL_REQUESTS: usize = 20;

/// The repository a pull request's URL names
/// (`https://github.com/<owner>/<name>/pull/<number>`).
pub fn repo_of(url: &str) -> Option<RepoRef> {
    let mut parts = url.trim().strip_prefix("https://github.com/")?.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if parts.next()? != "pull" {
        return None;
    }
    RepoRef::parse(&format!("{owner}/{name}"))
}

/// The pull request a URL names (`https://github.com/<owner>/<name>/pull/<number>`,
/// with or without more after the number), as a session lists it before
/// anyone looked it up: its number and repository, and the URL without
/// the rest.
pub fn named_by(url: &str) -> Option<PullRequestRef> {
    let repo = repo_of(url)?;
    let number = url.trim().splitn(7, '/').nth(6)?;
    let digits = number.split(|c: char| !c.is_ascii_digit()).next()?;
    let number = digits.parse::<u64>().ok().filter(|n| *n > 0)?;
    Some(PullRequestRef {
        number,
        url: format!("{}/pull/{number}", repo.html_url()),
        repo: Some(repo),
        ..Default::default()
    })
}

/// What an agent says of a pull request, as the control plane keeps it: a
/// GitHub pull request's own URL, rebuilt from it with its repository and
/// number, and the title, state and times `gh pr view` gives, none of them
/// later than `now`. Anything in a session's sandbox can pose as its agent,
/// and the apps show these as the session's own links and checks, so a
/// link anywhere else, a number its URL doesn't have, or checks (which only
/// GitHub tells the control plane) are refused or left out; a time in the
/// future would keep what GitHub says later from replacing it ([`update`]).
pub fn from_agent(pr: &PullRequestRef, now: Millis) -> Option<PullRequestRef> {
    let named = named_by(&pr.url)?;
    if named.number != pr.number {
        return None;
    }
    let state = pr
        .state
        .as_deref()
        .filter(|s| matches!(*s, "open" | "draft" | "merged" | "closed"));
    Some(PullRequestRef {
        title: pr.title.clone(),
        state: state.map(str::to_owned),
        state_changed_at: pr.state_changed_at.map(|t| t.min(now)),
        updated_at: pr.updated_at.map(|t| t.min(now)),
        checks: None,
        ..named
    })
}

/// A git state from an agent, with its pull requests as the control plane
/// keeps them ([`from_agent`]).
pub fn from_agent_git(git: &mut GitState, now: Millis) {
    git.pull_request = git.pull_request.as_ref().and_then(|p| from_agent(p, now));
    for r in git.repos.iter_mut().flatten() {
        r.pull_request = r.pull_request.as_ref().and_then(|p| from_agent(p, now));
    }
    if let Some(list) = &mut git.pull_requests {
        *list = list.iter().filter_map(|p| from_agent(p, now)).collect();
    }
}

/// The pull requests a command's output names, each once, in order: the
/// URL `gh pr create` prints, or the one it says already exists.
pub fn named_in(text: &str) -> Vec<PullRequestRef> {
    let mut out: Vec<PullRequestRef> = Vec::new();
    for (start, _) in text.match_indices("https://github.com/") {
        let rest = &text[start..];
        let end = rest
            .find(|c: char| c.is_whitespace() || "\"'`<>()[]{},;".contains(c))
            .unwrap_or(rest.len());
        if let Some(pr) = named_by(&rest[..end])
            && !out.iter().any(|o| o.url.eq_ignore_ascii_case(&pr.url))
        {
            out.push(pr);
        }
    }
    out
}

/// The pull request with its repository: the one its URL names, or else
/// `repo`, the repository whose state it came in.
fn with_repo(pr: &PullRequestRef, repo: Option<RepoRef>) -> PullRequestRef {
    let mut pr = pr.clone();
    if pr.repo.is_none() {
        pr.repo = repo_of(&pr.url).or(repo);
    }
    pr
}

/// The pull requests in a git state, each once and with its repository:
/// the checked-out branches', the primary repository's first, then the
/// others the agent follows.
pub fn found(git: &GitState) -> Vec<PullRequestRef> {
    let primary = git.pull_request.iter().map(|pr| with_repo(pr, None));
    let others = git.repos.iter().flatten().filter_map(|r| {
        let repo = RepoRef {
            owner: r.owner.clone(),
            name: r.name.clone(),
        };
        r.pull_request.as_ref().map(|pr| with_repo(pr, Some(repo)))
    });
    let followed = git.pull_requests.iter().flatten();
    let mut out: Vec<PullRequestRef> = Vec::new();
    for pr in primary
        .chain(others)
        .chain(followed.map(|pr| with_repo(pr, None)))
    {
        if !out.iter().any(|o| o.url == pr.url) {
            out.push(pr);
        }
    }
    out
}

/// Whether two URLs name the same pull request: GitHub's names don't care
/// about case.
fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Adds the pull requests an agent found to those its session lists, after
/// them, and takes what it says of the ones it lists already ([`update`]).
/// Returns whether the list changed.
pub fn merge(known: &mut Vec<PullRequestRef>, found: &[PullRequestRef], now: Millis) -> bool {
    let mut changed = false;
    for pr in found {
        if let Some(k) = known.iter_mut().find(|k| same(&k.url, &pr.url)) {
            changed |= update(k, pr, Source::Agent, now);
        } else if known.len() < MAX_PULL_REQUESTS {
            known.push(pr.clone());
            changed = true;
        }
    }
    changed
}

/// Takes what GitHub says of pull requests into those a session lists, and
/// leaves the others out: a session lists what its agent found. Returns
/// whether any changed.
pub fn observe(known: &mut [PullRequestRef], seen: &[PullRequestRef], now: Millis) -> bool {
    let mut changed = false;
    for s in seen {
        for k in known.iter_mut().filter(|k| same(&k.url, &s.url)) {
            changed |= update(k, s, Source::Github, now);
        }
    }
    changed
}

/// Who says what of a pull request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The session's agent, from `gh pr view`, which doesn't say whether
    /// it is in a merge queue: it reports a queued one as `open`.
    Agent,
    /// GitHub, to the control plane: a lookup after a webhook, or the
    /// webhook's payload.
    Github,
}

/// Takes what `seen` says of the pull request a session lists as `k`, at
/// `now`, and returns whether `k` changed:
///
/// - its title and state, unless `k`'s come from a newer lookup (by
///   GitHub's `updated_at`; a lookup without one counts as older than one
///   with it, as agents before it report what they last looked up). An
///   agent's `open` leaves a `queued` one queued: agents can't see merge
///   queues.
/// - when its state changed: the time GitHub gives for the new state (when
///   it merged, closed or joined the queue), unless that is from before
///   the old state's, as when it is reopened or marked ready (GitHub gives
///   when it was opened); then when fctl saw it change.
/// - its checks, unless `k`'s are from a later lookup.
pub fn update(k: &mut PullRequestRef, seen: &PullRequestRef, source: Source, now: Millis) -> bool {
    let before = k.clone();
    if k.repo.is_none() {
        k.repo = seen.repo.clone();
    }
    let newer = match (k.updated_at, seen.updated_at) {
        (Some(known), Some(seen)) => seen >= known,
        (Some(_), None) => false,
        (None, _) => true,
    };
    if newer {
        if seen.title.is_some() {
            k.title = seen.title.clone();
        }
        if seen.updated_at.is_some() {
            k.updated_at = seen.updated_at;
        }
        if let Some(state) = seen.state.as_deref() {
            let unseen_queue =
                source == Source::Agent && state == "open" && k.state.as_deref() == Some("queued");
            if k.state.as_deref() == Some(state) || unseen_queue {
                k.state_changed_at = k.state_changed_at.or(seen.state_changed_at);
            } else {
                k.state_changed_at = match (seen.state_changed_at, k.state_changed_at) {
                    (Some(at), Some(was)) if at > was => Some(at),
                    (Some(_), Some(_)) => Some(now),
                    (Some(at), None) => Some(at),
                    (None, _) if k.state.is_none() => None,
                    (None, _) => Some(now),
                };
                k.state = Some(state.to_owned());
            }
        }
    }
    if let Some(checks) = &seen.checks
        && k.checks.as_ref().is_none_or(|c| checks.at >= c.at)
    {
        k.checks = Some(checks.clone());
    }
    *k != before
}

/// A pull request's state as fctl names it, from what GitHub says of it.
pub fn state_of(merged: bool, closed: bool, queued: bool, draft: bool) -> &'static str {
    if merged {
        "merged"
    } else if closed {
        "closed"
    } else if queued {
        "queued"
    } else if draft {
        "draft"
    } else {
        "open"
    }
}

/// Whether a pull request is in a merge queue: the webhook's `action` is
/// `enqueued` (or `queued`), GitHub has its entry in the queue
/// (`merge_queue_entry`), or its merge state (`merge_state_status`, else
/// `mergeable_state`) is `queued` or `in_merge_queue`.
pub fn queued(
    action: Option<&str>,
    merge_queue_entry: bool,
    merge_state_status: Option<&str>,
    mergeable_state: Option<&str>,
) -> bool {
    let in_queue =
        |s: &str| s.eq_ignore_ascii_case("queued") || s.eq_ignore_ascii_case("in_merge_queue");
    action.is_some_and(|a| a == "enqueued" || a == "queued")
        || merge_queue_entry
        || merge_state_status.or(mergeable_state).is_some_and(in_queue)
}

/// When GitHub says a pull request took each state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Times {
    pub created: Option<Millis>,
    pub merged: Option<Millis>,
    pub closed: Option<Millis>,
    pub queued: Option<Millis>,
}

impl Times {
    /// When it took `state`: for an open one or a draft, when it was opened,
    /// which [`update`] doesn't take for one marked ready or reopened.
    pub fn of(&self, state: &str) -> Option<Millis> {
        match state {
            "merged" => self.merged,
            "closed" => self.closed,
            "queued" => self.queued,
            _ => self.created,
        }
    }
}

/// A time as GitHub writes them (`2026-10-21T15:43:25Z`, RFC 3339), in
/// milliseconds since the epoch.
pub fn github_time(s: &str) -> Option<Millis> {
    let num = |s: &str| -> Option<i64> {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let (date, rest) = s.trim().split_once(['T', 't', ' '])?;
    let mut d = date.splitn(3, '-');
    let (y, m, day) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    // The offset: `Z`, or `+hh:mm` / `-hh:mm`.
    let i = rest.find(['Z', 'z', '+', '-'])?;
    let (clock, offset) = (&rest[..i], &rest[i..]);
    let offset_min = match offset {
        "Z" | "z" => 0,
        o => {
            let sign = if o.starts_with('-') { -1 } else { 1 };
            let (h, m) = o.get(1..)?.split_once(':')?;
            sign * (num(h)? * 60 + num(m)?)
        }
    };
    let (hms, frac) = clock.split_once('.').unwrap_or((clock, ""));
    let mut t = hms.splitn(3, ':');
    let (h, min, sec) = (num(t.next()?)?, num(t.next()?)?, num(t.next()?)?);
    let ms = match frac {
        "" => 0,
        f => num(&format!("{f:0<3}")[..3])?,
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || min > 59 || sec > 60 {
        return None;
    }
    let days = days_from_civil(y, m, day);
    Some(((days * 86_400 + h * 3600 + min * 60 + sec - offset_min * 60) * 1000) + ms)
}

/// Days since 1970-01-01 of a date, after Howard Hinnant's
/// `days_from_civil`.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Whether a pull request's checks all passed (`Some(true)`), any failed
/// (`Some(false)`), or neither yet (`None`): some still run, or there are
/// none.
pub fn checks_passed(c: &PullRequestChecks) -> Option<bool> {
    if c.failed > 0 {
        Some(false)
    } else if c.pending > 0 || c.passed == 0 {
        None
    } else {
        Some(true)
    }
}

/// Gives a session from before `pull_requests` its one pull request in the
/// list too, and the repositories they were opened in.
pub fn backfill(s: &mut Session) {
    if let Some(pr) = &mut s.pull_request
        && pr.repo.is_none()
    {
        pr.repo = repo_of(&pr.url);
    }
    if s.pull_requests.is_empty()
        && let Some(pr) = &s.pull_request
    {
        s.pull_requests.push(pr.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::event::RepoGitState;
    use fctl_proto::session::RepoAccess;

    fn pr(repo: &str, number: u64) -> PullRequestRef {
        PullRequestRef {
            number,
            url: format!("https://github.com/{repo}/pull/{number}"),
            title: Some(format!("Change {number}")),
            state: Some("open".into()),
            ..Default::default()
        }
    }

    const NOW: Millis = 1_000_000;

    fn repo(full: &str) -> Option<RepoRef> {
        RepoRef::parse(full)
    }

    #[test]
    fn agents_report_only_github_pull_requests_and_never_their_checks() {
        let now = 1_000;
        let honest = PullRequestRef {
            title: Some("Fix login".into()),
            state: Some("open".into()),
            updated_at: Some(900),
            ..pr("acme/widgets", 42)
        };
        let kept = from_agent(&honest, now);
        assert_eq!(
            kept.as_ref().map(|p| p.url.as_str()),
            Some("https://github.com/acme/widgets/pull/42")
        );
        assert_eq!(
            kept.as_ref().and_then(|p| p.repo.clone()),
            repo("acme/widgets")
        );
        assert_eq!(kept.as_ref().and_then(|p| p.updated_at), Some(900));
        // A link to another site, or a number its URL doesn't have.
        for url in [
            "https://evil.example/acme/widgets/pull/42",
            "https://github.com.evil.example/acme/widgets/pull/42",
            "javascript:alert(1)",
            "https://github.com/acme/widgets/pull/41",
        ] {
            let forged = PullRequestRef {
                url: url.into(),
                ..honest.clone()
            };
            assert_eq!(from_agent(&forged, now), None, "{url}");
        }
        // Its repository is the URL's, checks are GitHub's to say, and
        // nothing is from the future.
        let claims = PullRequestRef {
            url: "https://github.com/acme/widgets/pull/42#discussion".into(),
            repo: repo("victim/private"),
            state: Some("<b>merged</b>".into()),
            updated_at: Some(9_000_000),
            state_changed_at: Some(9_000_000),
            checks: Some(PullRequestChecks {
                passed: 12,
                ..Default::default()
            }),
            ..honest.clone()
        };
        let kept = from_agent(&claims, now);
        assert_eq!(
            kept.as_ref().map(|p| p.url.as_str()),
            Some("https://github.com/acme/widgets/pull/42")
        );
        assert_eq!(
            kept.as_ref().and_then(|p| p.repo.clone()),
            repo("acme/widgets")
        );
        assert_eq!(kept.as_ref().and_then(|p| p.state.clone()), None);
        assert_eq!(kept.as_ref().and_then(|p| p.updated_at), Some(now));
        assert_eq!(kept.as_ref().and_then(|p| p.state_changed_at), Some(now));
        assert_eq!(kept.as_ref().and_then(|p| p.checks.clone()), None);
        let mut git = GitState {
            pull_request: Some(claims.clone()),
            pull_requests: Some(vec![
                claims,
                PullRequestRef {
                    url: "https://evil.example/x/pull/1".into(),
                    number: 1,
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        from_agent_git(&mut git, now);
        assert_eq!(
            git.pull_request.as_ref().and_then(|p| p.checks.clone()),
            None
        );
        assert_eq!(git.pull_requests.as_ref().map(Vec::len), Some(1));
    }

    #[test]
    fn urls_name_their_repository() {
        assert_eq!(
            repo_of("https://github.com/hyperstructure/fctl-site/pull/30"),
            repo("hyperstructure/fctl-site")
        );
        assert_eq!(
            repo_of("https://github.com/acme/app/pull/7/files"),
            repo("acme/app")
        );
        for other in [
            "https://github.com/acme/app/issues/7",
            "https://github.com/acme/app",
            "https://example.com/acme/app/pull/7",
            "https://github.com/acme//pull/7",
            "",
        ] {
            assert_eq!(repo_of(other), None, "{other}");
        }
    }

    #[test]
    fn commands_name_the_pull_requests_they_open() {
        let stub = |repo: &str, number: u64| PullRequestRef {
            title: None,
            state: None,
            repo: super::repo_of(&format!("https://github.com/{repo}/pull/{number}")),
            ..pr(repo, number)
        };
        assert_eq!(
            named_by("https://github.com/hyperstructure/fctl/pull/152"),
            Some(stub("hyperstructure/fctl", 152))
        );
        assert_eq!(
            named_by("https://github.com/acme/app/pull/7/files#diff-1"),
            Some(stub("acme/app", 7))
        );
        for other in [
            "https://github.com/acme/app/pull/",
            "https://github.com/acme/app/pull/0",
            "https://github.com/acme/app/issues/7",
            "https://example.com/acme/app/pull/7",
        ] {
            assert_eq!(named_by(other), None, "{other}");
        }
        // What `gh pr create` prints, in a command that also pushed, and
        // what it says when the branch has one already.
        let printed = "To https://github.com/acme/app.git\n * [new branch] fctl/1-x -> fctl/1-x\n\
            https://github.com/acme/app/pull/12\n";
        assert_eq!(named_in(printed), [stub("acme/app", 12)]);
        let exists = "a pull request for branch \"fctl/1-x\" into branch \"main\" already exists:\n\
            https://github.com/acme/app/pull/12.";
        assert_eq!(named_in(exists), [stub("acme/app", 12)]);
        let two = "(https://github.com/acme/app/pull/12) and \"https://github.com/acme/lib/pull/3\", \
            again https://github.com/acme/app/pull/12, see https://github.com/acme/app/issues/1";
        assert_eq!(named_in(two), [stub("acme/app", 12), stub("acme/lib", 3)]);
        assert!(named_in("Created a pull request").is_empty());
    }

    #[test]
    fn a_git_state_names_the_repository_of_each() {
        let git = GitState {
            pull_request: Some(pr("acme/app", 12)),
            repos: Some(vec![
                RepoGitState {
                    owner: "acme".into(),
                    name: "lib".into(),
                    access: RepoAccess::Write,
                    pull_request: Some(pr("acme/lib", 3)),
                    ..Default::default()
                },
                RepoGitState {
                    owner: "acme".into(),
                    name: "docs".into(),
                    access: RepoAccess::Read,
                    ..Default::default()
                },
                // A URL that names no repository: the state's says it.
                RepoGitState {
                    owner: "acme".into(),
                    name: "site".into(),
                    access: RepoAccess::Write,
                    pull_request: Some(PullRequestRef {
                        url: "https://ghe.example.com/pr/5".into(),
                        ..pr("acme/site", 5)
                    }),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        let found = found(&git);
        let names: Vec<_> = found
            .iter()
            .map(|p| (p.repo.as_ref().map(RepoRef::full_name), p.number))
            .collect();
        assert_eq!(
            names,
            [
                (Some("acme/app".into()), 12),
                (Some("acme/lib".into()), 3),
                (Some("acme/site".into()), 5)
            ]
        );
        assert!(super::found(&GitState::default()).is_empty());
    }

    #[test]
    fn a_git_state_lists_the_pull_requests_of_other_branches_too() {
        let mut merged = pr("acme/app", 12);
        merged.state = Some("merged".into());
        let git = GitState {
            pull_request: Some(pr("acme/app", 13)),
            // Every one the agent follows, the checked-out branch's too.
            pull_requests: Some(vec![merged.clone(), pr("acme/app", 13), pr("acme/lib", 4)]),
            ..Default::default()
        };
        let found: Vec<_> = found(&git)
            .into_iter()
            .map(|p| (p.number, p.state, p.repo.map(|r| r.full_name())))
            .collect();
        assert_eq!(
            found,
            [
                (13, Some("open".into()), Some("acme/app".into())),
                (12, Some("merged".into()), Some("acme/app".into())),
                (4, Some("open".into()), Some("acme/lib".into())),
            ]
        );
        // The session keeps the merged one's state current, and lists the
        // one from another branch.
        let mut known = vec![pr("acme/app", 12)];
        assert!(merge(&mut known, &super::found(&git), NOW));
        assert_eq!(known[0].state.as_deref(), Some("merged"));
        assert_eq!(known.len(), 3);
    }

    #[test]
    fn sessions_keep_every_pull_request_they_found() {
        let mut known = Vec::new();
        let first = found(&GitState {
            pull_request: Some(pr("acme/app", 12)),
            ..Default::default()
        });
        assert!(merge(&mut known, &first, NOW));
        assert!(!merge(&mut known, &first, NOW), "nothing new");
        // The next turn: merged, and a follow-up in another repository.
        let mut merged = pr("acme/app", 12);
        merged.state = Some("merged".into());
        let next = found(&GitState {
            pull_request: Some(merged),
            repos: Some(vec![RepoGitState {
                owner: "acme".into(),
                name: "lib".into(),
                pull_request: Some(pr("acme/lib", 3)),
                ..Default::default()
            }]),
            ..Default::default()
        });
        assert!(merge(&mut known, &next, NOW));
        // Then a second one in the primary repository, from a new branch:
        // the agent no longer reports the first.
        let later = found(&GitState {
            pull_request: Some(pr("acme/app", 13)),
            ..Default::default()
        });
        assert!(merge(&mut known, &later, NOW));
        let listed: Vec<_> = known
            .iter()
            .map(|p| (p.url.as_str(), p.state.as_deref()))
            .collect();
        assert_eq!(
            listed,
            [
                ("https://github.com/acme/app/pull/12", Some("merged")),
                ("https://github.com/acme/lib/pull/3", Some("open")),
                ("https://github.com/acme/app/pull/13", Some("open")),
            ]
        );
        assert!(known.iter().all(|p| p.repo.is_some()));
    }

    #[test]
    fn sessions_from_before_the_list_list_their_one() -> Result<(), serde_json::Error> {
        let mut s: Session = serde_json::from_value(serde_json::json!({
            "id": "ses_1", "project_id": "prj_1", "project_slug": "app", "number": 1,
            "title": "Fix it", "status": "stopped",
            "created_by": { "id": "usr_1", "name": "Ada" },
            "created_at": 1, "updated_at": 1, "stats": {}, "archived": false,
            "pull_request": { "number": 12, "url": "https://github.com/acme/app/pull/12" }
        }))?;
        assert!(s.pull_requests.is_empty());
        backfill(&mut s);
        let repos: Vec<_> = s.pull_requests.iter().map(|p| p.repo.clone()).collect();
        assert_eq!(repos, [repo("acme/app")]);
        assert_eq!(s.pull_request.and_then(|p| p.repo), repo("acme/app"));
        let mut s: Session = serde_json::from_value(serde_json::json!({
            "id": "ses_2", "project_id": "prj_1", "project_slug": "app", "number": 2,
            "title": "Fix it", "status": "running",
            "created_by": { "id": "usr_1", "name": "Ada" },
            "created_at": 1, "updated_at": 1, "stats": {}, "archived": false
        }))?;
        backfill(&mut s);
        assert!(s.pull_requests.is_empty());
        Ok(())
    }

    #[test]
    fn the_list_has_room_for_the_first_ones() {
        let mut known: Vec<_> = (1..=MAX_PULL_REQUESTS as u64)
            .map(|n| pr("acme/app", n))
            .collect();
        assert!(!merge(&mut known, &[pr("acme/app", 100)], NOW));
        let mut closed = pr("acme/app", 1);
        closed.state = Some("closed".into());
        assert!(
            merge(&mut known, &[closed], NOW),
            "the ones it lists still change"
        );
        assert_eq!(known.len(), MAX_PULL_REQUESTS);
        assert_eq!(known[0].state.as_deref(), Some("closed"));
    }

    /// What GitHub or an agent says of a pull request: its state, when
    /// GitHub last changed it, and when it took that state.
    fn said(
        n: u64,
        state: &str,
        updated_at: Option<Millis>,
        since: Option<Millis>,
    ) -> PullRequestRef {
        PullRequestRef {
            state: Some(state.into()),
            updated_at,
            state_changed_at: since,
            ..pr("acme/app", n)
        }
    }

    fn state(known: &[PullRequestRef]) -> (Option<&str>, Option<Millis>) {
        (known[0].state.as_deref(), known[0].state_changed_at)
    }

    #[test]
    fn github_says_what_agents_cannot() {
        // An agent from before `updated_at` found it open.
        let mut known = vec![said(12, "open", None, None)];
        // GitHub: it joined the merge queue at 9.
        assert!(observe(
            &mut known,
            &[said(12, "queued", Some(10), Some(9))],
            NOW
        ));
        assert_eq!(state(&known), (Some("queued"), Some(9)));
        // The agent looks it up after someone labels it: `gh pr view` says
        // open, as it can't see the queue.
        merge(&mut known, &[said(12, "open", Some(11), None)], NOW);
        assert_eq!(state(&known), (Some("queued"), Some(9)));
        // The queue merges it.
        assert!(observe(
            &mut known,
            &[said(12, "merged", Some(13), Some(12))],
            NOW
        ));
        assert_eq!(state(&known), (Some("merged"), Some(12)));
        // What agents last looked up doesn't undo it: an older lookup, or
        // one from an agent before `updated_at`.
        assert!(!merge(&mut known, &[said(12, "open", Some(11), None)], NOW));
        assert!(!merge(&mut known, &[said(12, "open", None, None)], NOW));
        assert_eq!(state(&known), (Some("merged"), Some(12)));
        // Nor does a webhook that comes late.
        assert!(!observe(
            &mut known,
            &[said(12, "queued", Some(10), Some(9))],
            NOW
        ));
        assert_eq!(known[0].updated_at, Some(13));
    }

    #[test]
    fn agents_still_say_what_changed() {
        // Where GitHub's webhook is off, agents' lookups are all there is.
        let mut known = vec![said(12, "draft", None, None)];
        assert!(merge(&mut known, &[said(12, "open", None, None)], 20));
        assert_eq!(state(&known), (Some("open"), Some(20)), "when fctl saw it");
        // Agents that say when it merged.
        assert!(merge(
            &mut known,
            &[said(12, "merged", Some(30), Some(29))],
            40
        ));
        assert_eq!(state(&known), (Some("merged"), Some(29)));
        // One that GitHub took out of the queue.
        let mut known = vec![said(13, "queued", Some(10), Some(9))];
        assert!(observe(
            &mut known,
            &[said(13, "open", Some(11), Some(1))],
            12
        ));
        assert_eq!(state(&known), (Some("open"), Some(12)));
    }

    #[test]
    fn it_says_when_the_state_changed() {
        // Named by `gh pr create`: no state yet, so no time either.
        let mut known = vec![PullRequestRef {
            title: None,
            state: None,
            ..pr("acme/app", 12)
        }];
        assert!(observe(
            &mut known,
            &[said(12, "draft", Some(5), Some(5))],
            NOW
        ));
        assert_eq!(
            state(&known),
            (Some("draft"), Some(5)),
            "opened as a draft at 5"
        );
        // Marked ready: GitHub still gives when it was opened.
        assert!(observe(
            &mut known,
            &[said(12, "open", Some(7), Some(5))],
            7
        ));
        assert_eq!(state(&known), (Some("open"), Some(7)));
        // Closed at 20, reopened at 25, merged at 30.
        assert!(observe(
            &mut known,
            &[said(12, "closed", Some(20), Some(20))],
            21
        ));
        assert_eq!(state(&known), (Some("closed"), Some(20)));
        assert!(observe(
            &mut known,
            &[said(12, "open", Some(25), Some(5))],
            25
        ));
        assert_eq!(state(&known), (Some("open"), Some(25)));
        assert!(observe(
            &mut known,
            &[said(12, "merged", Some(31), Some(30))],
            32
        ));
        assert_eq!(state(&known), (Some("merged"), Some(30)));
        // The same state again keeps its time, or gets one it lacked.
        assert!(!observe(
            &mut known,
            &[said(12, "merged", Some(31), Some(40))],
            50
        ));
        let mut known = vec![said(13, "merged", None, None)];
        assert!(observe(
            &mut known,
            &[said(13, "merged", Some(31), Some(30))],
            50
        ));
        assert_eq!(state(&known), (Some("merged"), Some(30)));
    }

    #[test]
    fn checks_come_from_the_latest_lookup() {
        let checks = |at: Millis, passed: u32, pending: u32| PullRequestChecks {
            passed,
            failed: 0,
            pending,
            sha: Some("abc".into()),
            at,
        };
        let mut known = vec![pr("acme/app", 12)];
        let with = |c: PullRequestChecks| PullRequestRef {
            checks: Some(c),
            ..pr("acme/app", 12)
        };
        assert!(observe(&mut known, &[with(checks(10, 1, 2))], NOW));
        // An answer to an earlier lookup that came late.
        assert!(!observe(&mut known, &[with(checks(8, 3, 0))], NOW));
        // Agents don't report checks: the session keeps them.
        assert!(!merge(&mut known, &[pr("acme/app", 12)], NOW));
        assert_eq!(known[0].checks, Some(checks(10, 1, 2)));
        assert!(observe(&mut known, &[with(checks(12, 3, 0))], NOW));
        assert_eq!(known[0].checks, Some(checks(12, 3, 0)));
    }

    #[test]
    fn github_changes_only_what_sessions_list() {
        let mut known = vec![pr("acme/app", 12)];
        let other = said(13, "merged", Some(10), Some(10));
        assert!(!observe(&mut known, &[other], NOW));
        assert_eq!(known, [pr("acme/app", 12)]);
        // GitHub's names don't care about case.
        let merged = PullRequestRef {
            url: "https://github.com/Acme/App/pull/12".into(),
            ..said(12, "merged", Some(10), Some(10))
        };
        assert!(observe(&mut known, &[merged], NOW));
        assert_eq!(known[0].state.as_deref(), Some("merged"));
        assert_eq!(known[0].url, "https://github.com/acme/app/pull/12");
    }

    #[test]
    fn states_as_github_says_them() {
        assert_eq!(state_of(true, true, false, false), "merged");
        assert_eq!(state_of(false, true, false, false), "closed");
        assert_eq!(state_of(false, false, true, false), "queued");
        assert_eq!(state_of(false, false, false, true), "draft");
        assert_eq!(state_of(false, false, false, false), "open");
    }

    #[test]
    fn merge_queues_show_in_any_of_their_signs() {
        assert!(queued(Some("enqueued"), false, None, None));
        assert!(queued(Some("queued"), false, None, None));
        assert!(!queued(Some("dequeued"), false, None, None));
        assert!(queued(None, true, None, None));
        assert!(queued(None, false, Some("QUEUED"), None));
        assert!(queued(None, false, Some("in_merge_queue"), None));
        assert!(queued(None, false, None, Some("in_merge_queue")));
        assert!(queued(None, false, None, Some("queued")));
        // `mergeable_state` only counts without `merge_state_status`.
        assert!(!queued(None, false, Some("BLOCKED"), Some("queued")));
        assert!(!queued(
            Some("synchronize"),
            false,
            Some("CLEAN"),
            Some("clean")
        ));
    }

    #[test]
    fn checks_pass_once_every_one_did() {
        let c = |passed, failed, pending| PullRequestChecks {
            passed,
            failed,
            pending,
            ..Default::default()
        };
        assert_eq!(checks_passed(&c(3, 0, 0)), Some(true));
        assert_eq!(
            checks_passed(&c(3, 1, 2)),
            Some(false),
            "a failure shows at once"
        );
        assert_eq!(checks_passed(&c(3, 0, 2)), None);
        assert_eq!(checks_passed(&c(0, 0, 0)), None, "no checks");
    }

    #[test]
    fn times_read_as_github_writes_them() {
        use crate::account::iso8601;
        let at = |s: &str| github_time(s).map(iso8601);
        assert_eq!(
            at("2026-10-21T15:43:25Z").as_deref(),
            Some("2026-10-21T15:43:25.000Z")
        );
        assert_eq!(
            at("2026-10-21T15:43:25.5Z").as_deref(),
            Some("2026-10-21T15:43:25.500Z")
        );
        assert_eq!(
            at("2026-10-21T17:43:25+02:00").as_deref(),
            Some("2026-10-21T15:43:25.000Z")
        );
        assert_eq!(
            at("2024-02-29T00:00:00Z").as_deref(),
            Some("2024-02-29T00:00:00.000Z")
        );
        assert_eq!(github_time("1970-01-01T00:00:00Z"), Some(0));
        for bad in [
            "",
            "2026-10-21",
            "2026-13-01T00:00:00Z",
            "2026-10-21T15:43:25",
            "yesterday",
        ] {
            assert_eq!(github_time(bad), None, "{bad}");
        }
    }
}
