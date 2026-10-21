//! What changed on a session's branches from one of its git states to the
//! next. fctl-agent reports the state while Claude works, after each turn
//! and when the session stops, and most reports only move files; the
//! transcript shows one when a branch gets a commit, is pushed, or has a
//! pull request the session hadn't had.

use fctl_proto::event::GitState;
use std::collections::HashSet;

/// A repository's branch in a git state: the primary repository's (`name`
/// `None`) or another's (`owner/name`).
struct Branch<'a> {
    name: Option<String>,
    head: Option<&'a str>,
    ahead: u32,
    pushed: bool,
    pull_request: Option<&'a str>,
}

fn branches(git: &GitState) -> Vec<Branch<'_>> {
    let primary = Branch {
        name: None,
        head: git.head.as_deref(),
        ahead: git.ahead,
        pushed: git.pushed,
        pull_request: git.pull_request.as_ref().map(|pr| pr.url.as_str()),
    };
    let others = git.repos.iter().flatten().map(|r| Branch {
        name: Some(r.full_name()),
        head: r.head.as_deref(),
        ahead: r.ahead,
        pushed: r.pushed,
        pull_request: r.pull_request.as_ref().map(|pr| pr.url.as_str()),
    });
    std::iter::once(primary).chain(others).collect()
}

/// The repositories whose branches changed since `before` (`None` for the
/// primary one, `owner/name` for the others): a commit of their own (the
/// head moved, with commits the base lacks), a push of them, or a pull
/// request not in `seen`, which keeps the pull requests' URLs from one call
/// to the next.
pub fn changed(
    before: Option<&GitState>,
    after: &GitState,
    seen: &mut HashSet<String>,
) -> Vec<Option<String>> {
    let was = before.map(branches).unwrap_or_default();
    branches(after)
        .into_iter()
        .filter_map(|b| {
            let w = was.iter().find(|w| w.name == b.name);
            let committed = b.ahead > 0 && b.head.is_some() && b.head != w.and_then(|w| w.head);
            let pushed = b.ahead > 0 && b.pushed && !w.is_some_and(|w| w.pushed);
            let opened = b
                .pull_request
                .is_some_and(|url| seen.insert(url.to_owned()));
            (committed || pushed || opened).then_some(b.name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::event::{FileChange, RepoGitState};
    use fctl_proto::session::PullRequestRef;

    fn state(head: &str, ahead: u32, pushed: bool) -> GitState {
        GitState {
            branch: Some("fctl/1".into()),
            head: Some(head.into()),
            ahead,
            pushed,
            ..Default::default()
        }
    }

    fn pr(n: u64) -> PullRequestRef {
        PullRequestRef {
            number: n,
            url: format!("https://github.com/acme/widgets/pull/{n}"),
            ..Default::default()
        }
    }

    #[test]
    fn commits_pushes_and_new_pull_requests_change_a_branch() {
        let mut seen = HashSet::new();
        let mut steps = vec![];
        let mut before: Option<GitState> = None;
        let edited = GitState {
            files: vec![FileChange {
                path: "a.rs".into(),
                status: "modified".into(),
                additions: Some(1),
                deletions: Some(0),
            }],
            uncommitted: 1,
            ..state("base", 0, false)
        };
        let with_pr = GitState {
            pull_request: Some(pr(12)),
            ..state("c2", 2, true)
        };
        for (what, after) in [
            ("a new session's branch", state("base", 0, false)),
            ("edits", edited),
            ("a commit", state("c1", 1, false)),
            ("the same state again", state("c1", 1, false)),
            ("another commit, pushed", state("c2", 2, true)),
            ("a pull request", with_pr.clone()),
            ("the same pull request", with_pr.clone()),
            ("a lookup that found none", state("c2", 2, true)),
            ("the pull request again", with_pr),
            ("a rebase", state("c3", 2, false)),
            ("a push", state("c3", 2, true)),
            ("a reset to the base", state("base", 0, false)),
        ] {
            let changed = changed(before.as_ref(), &after, &mut seen);
            steps.push((what, !changed.is_empty()));
            before = Some(after);
        }
        assert_eq!(
            steps,
            [
                ("a new session's branch", false),
                ("edits", false),
                ("a commit", true),
                ("the same state again", false),
                ("another commit, pushed", true),
                ("a pull request", true),
                ("the same pull request", false),
                ("a lookup that found none", false),
                ("the pull request again", false),
                ("a rebase", true),
                ("a push", true),
                ("a reset to the base", false),
            ]
        );
        // The first state seen, of a branch with commits, shows them.
        assert_eq!(
            changed(None, &state("c3", 2, true), &mut HashSet::new()),
            [None]
        );
    }

    #[test]
    fn names_the_repositories_that_changed() {
        let lib = |head: &str, ahead: u32| RepoGitState {
            owner: "acme".into(),
            name: "lib".into(),
            head: Some(head.into()),
            ahead,
            ..Default::default()
        };
        let docs = RepoGitState {
            owner: "acme".into(),
            name: "docs".into(),
            pull_request: Some(pr(3)),
            ..Default::default()
        };
        let before = GitState {
            repos: Some(vec![lib("l0", 0)]),
            ..state("c1", 1, false)
        };
        let after = GitState {
            repos: Some(vec![lib("l1", 1), docs]),
            ..state("c1", 1, false)
        };
        let mut seen = HashSet::new();
        assert_eq!(
            changed(Some(&before), &after, &mut seen),
            [Some("acme/lib".to_owned()), Some("acme/docs".to_owned())]
        );
    }
}
