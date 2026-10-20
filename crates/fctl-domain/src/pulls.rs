//! The pull requests of a session: every one its agent found for the
//! branches it worked on, in all of its project's repositories, each with
//! the repository it was opened in.
//!
//! After each turn fctl-agent looks up the pull request of the branch each
//! repository has checked out (`GitState`), so it reports only the latest
//! one per repository. The control plane keeps the others: a session that
//! opened one in each of three repositories, or a second one after the
//! first was merged, lists them all.

use fctl_proto::event::GitState;
use fctl_proto::session::{PullRequestRef, RepoRef, Session};

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

/// The pull request with its repository: the one its URL names, or else
/// `repo`, the repository whose state it came in.
fn with_repo(pr: &PullRequestRef, repo: Option<RepoRef>) -> PullRequestRef {
    let mut pr = pr.clone();
    if pr.repo.is_none() {
        pr.repo = repo_of(&pr.url).or(repo);
    }
    pr
}

/// The pull requests in a git state, the primary repository's first, each
/// with its repository.
pub fn found(git: &GitState) -> Vec<PullRequestRef> {
    let primary = git.pull_request.iter().map(|pr| with_repo(pr, None));
    let others = git.repos.iter().flatten().filter_map(|r| {
        let repo = RepoRef {
            owner: r.owner.clone(),
            name: r.name.clone(),
        };
        r.pull_request.as_ref().map(|pr| with_repo(pr, Some(repo)))
    });
    primary.chain(others).collect()
}

/// Adds the pull requests an agent found to those its session lists, after
/// them, and takes the newer title and state of the ones it lists already.
/// Returns whether the list changed.
pub fn merge(known: &mut Vec<PullRequestRef>, found: &[PullRequestRef]) -> bool {
    let mut changed = false;
    for pr in found {
        if let Some(k) = known.iter_mut().find(|k| k.url == pr.url) {
            if k != pr {
                *k = pr.clone();
                changed = true;
            }
        } else if known.len() < MAX_PULL_REQUESTS {
            known.push(pr.clone());
            changed = true;
        }
    }
    changed
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
            repo: None,
        }
    }

    fn repo(full: &str) -> Option<RepoRef> {
        RepoRef::parse(full)
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
    fn sessions_keep_every_pull_request_they_found() {
        let mut known = Vec::new();
        let first = found(&GitState {
            pull_request: Some(pr("acme/app", 12)),
            ..Default::default()
        });
        assert!(merge(&mut known, &first));
        assert!(!merge(&mut known, &first), "nothing new");
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
        assert!(merge(&mut known, &next));
        // Then a second one in the primary repository, from a new branch:
        // the agent no longer reports the first.
        let later = found(&GitState {
            pull_request: Some(pr("acme/app", 13)),
            ..Default::default()
        });
        assert!(merge(&mut known, &later));
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
        assert!(!merge(&mut known, &[pr("acme/app", 100)]));
        let mut closed = pr("acme/app", 1);
        closed.state = Some("closed".into());
        assert!(
            merge(&mut known, &[closed]),
            "the ones it lists still change"
        );
        assert_eq!(known.len(), MAX_PULL_REQUESTS);
        assert_eq!(known[0].state.as_deref(), Some("closed"));
    }
}
