//! The checks on a base branch after a pull request of an autopilot's
//! sessions merged into it: they say whether the work holds there, before
//! the autopilot builds on it, deploys it or tells people it shipped. fctl
//! wakes the autopilot once they end, passed or not, so that it needs no
//! script to wait for them.
//!
//! What tells is the first commit on the base branch that holds the merge
//! (its merge commit, or one after it) whose checks all end
//! ([`webhooks::looked_up_commit`]). One whose checks were cancelled, with
//! none failing, while its branch moved on doesn't: a newer merge's run of
//! the same workflows took over, as a workflow's `concurrency` makes it,
//! and that newer commit, which holds the merge too, tells instead.

use crate::follow_ups::names;
use crate::reviews::short;
use crate::webhooks::Commit;

/// How the checks on a base branch ended after a merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// None failed or was cancelled.
    Passed,
    /// One or more failed, errored, timed out or couldn't start.
    Failed,
    /// One or more were cancelled, none failed, and its branch is still at
    /// that commit: nothing newer will tell.
    Cancelled,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
            Outcome::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Outcome::Passed, Outcome::Failed, Outcome::Cancelled]
            .into_iter()
            .find(|o| o.as_str() == s)
    }
}

/// How the checks of a commit on a base branch ended: `None` while any is
/// still to finish, of one GitHub knows no checks of, of one whose checks
/// were cancelled, none failing, while its branch moved on since (the
/// newer commit's tell instead), and of one that isn't on the branch, such
/// as a fork's whose branch has the base branch's name.
pub fn outcome(c: &Commit) -> Option<Outcome> {
    let checks = &c.checks;
    if !c.on_branch || checks.total() == 0 || checks.pending > 0 {
        return None;
    }
    if checks.failed > 0 {
        return Some(Outcome::Failed);
    }
    if checks.cancelled > 0 {
        return (!c.superseded()).then_some(Outcome::Cancelled);
    }
    Some(Outcome::Passed)
}

/// The merges that commit `c`'s checks settle: of those still waiting,
/// `merges` (their merge commits), those `c` holds, by their place in
/// `merges`. None while its checks don't tell ([`outcome`]).
pub fn settles(c: &Commit, merges: &[&str]) -> Option<(Outcome, Vec<usize>)> {
    let outcome = outcome(c)?;
    let held: Vec<usize> = merges
        .iter()
        .enumerate()
        .filter(|(_, m)| c.holds(m))
        .map(|(i, _)| i)
        .collect();
    (!held.is_empty()).then_some((outcome, held))
}

/// A merge whose checks on its base branch ended, for what fctl tells the
/// autopilot.
#[derive(Clone, Debug)]
pub struct View<'a> {
    /// The pull request: `romanesco/shop#67`.
    pub pull: String,
    pub title: Option<&'a str>,
    /// The session that opened it: `shop#12`.
    pub session: String,
    /// The branch it went into, and its commit there.
    pub base: &'a str,
    pub merge_sha: &'a str,
    /// The commit whose checks ended: its own, or a later one that holds
    /// it.
    pub sha: &'a str,
    pub outcome: Outcome,
    pub passed: u32,
    pub failed: u32,
    pub cancelled: u32,
    /// Those that failed, by name.
    pub failing: &'a [String],
}

/// What fctl tells the autopilot of a merge whose checks ended:
/// "romanesco/shop#67 “Add Apple Pay” (shop#12), into main: its 24 checks
/// passed at 3f2a1c9."
pub fn line(v: &View<'_>) -> String {
    let title = v
        .title
        .map(|t| format!(" “{}”", t.trim()))
        .unwrap_or_default();
    let total = v
        .passed
        .saturating_add(v.failed)
        .saturating_add(v.cancelled);
    let at = if v.sha.eq_ignore_ascii_case(v.merge_sha) {
        format!("at {}", short(v.sha))
    } else {
        format!("at {}, a later commit that holds it", short(v.sha))
    };
    let ended = match v.outcome {
        Outcome::Passed if total == 1 => format!("its check passed {at}"),
        Outcome::Passed => format!("its {total} checks passed {at}"),
        Outcome::Failed => {
            let mut s = format!("{} of {total} {} failed {at}", v.failed, plural(total));
            if !v.failing.is_empty() {
                s.push_str(&format!(": {}", names(v.failing)));
            }
            if v.cancelled > 0 {
                s.push_str(&format!(" ({} cancelled)", v.cancelled));
            }
            s
        }
        Outcome::Cancelled => format!(
            "{} of {total} {} {} cancelled {at}, still {}'s newest commit",
            v.cancelled,
            plural(total),
            if v.cancelled == 1 { "was" } else { "were" },
            v.base
        ),
    };
    format!(
        "{}{title} ({}), into {}: {ended}.",
        v.pull, v.session, v.base
    )
}

fn plural(n: u32) -> &'static str {
    if n == 1 { "check" } else { "checks" }
}

/// How the autopilot looks into checks that failed or were cancelled on a
/// base branch, after the lines that say so.
pub const LOOK_INTO: &str = "`gh run list --repo OWNER/NAME --commit SHA` lists a commit's workflow runs, and `gh run view RUN --repo OWNER/NAME --log-failed` shows what failed.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webhooks::CommitChecks;

    fn commit(sha: &str, history: &[&str], head: &str, checks: CommitChecks) -> Commit {
        Commit {
            sha: sha.to_owned(),
            history: history.iter().map(|h| (*h).to_owned()).collect(),
            head: Some(head.to_owned()),
            on_branch: true,
            checks,
        }
    }

    fn checks(passed: u32, failed: u32, cancelled: u32, pending: u32) -> CommitChecks {
        CommitChecks {
            passed,
            failed,
            cancelled,
            pending,
            failing: Vec::new(),
        }
    }

    #[test]
    fn a_commits_checks_tell_once_all_ended() {
        let at = |c| outcome(&commit("bbb", &["bbb", "aaa"], "bbb", c));
        assert_eq!(at(checks(24, 0, 0, 0)), Some(Outcome::Passed));
        assert_eq!(at(checks(22, 2, 0, 0)), Some(Outcome::Failed));
        assert_eq!(at(checks(20, 2, 2, 0)), Some(Outcome::Failed));
        assert_eq!(at(checks(21, 0, 3, 0)), Some(Outcome::Cancelled));
        // Still running, or none at all yet.
        assert_eq!(at(checks(20, 2, 0, 2)), None);
        assert_eq!(at(checks(0, 0, 0, 0)), None);
    }

    #[test]
    fn cancelled_checks_tell_only_while_their_branch_is_still_there() {
        // A newer merge's run took over: the newer commit tells.
        let moved = commit("aaa", &["aaa"], "bbb", checks(21, 0, 3, 0));
        assert_eq!(outcome(&moved), None);
        // But checks that failed before the cancel tell at once.
        let failed = commit("aaa", &["aaa"], "bbb", checks(20, 1, 3, 0));
        assert_eq!(outcome(&failed), Some(Outcome::Failed));
        // And checks that passed don't wait for the newer commit's.
        let passed = commit("aaa", &["aaa"], "bbb", checks(24, 0, 0, 0));
        assert_eq!(outcome(&passed), Some(Outcome::Passed));
        // A branch GitHub didn't say where it is: nothing says the commit
        // is on it.
        let unknown = Commit {
            head: None,
            on_branch: false,
            ..moved
        };
        assert_eq!(outcome(&unknown), None);
    }

    #[test]
    fn a_commit_off_the_base_branch_settles_nothing() {
        // A fork's pull request from its own main, up to date with the
        // upstream main: its commit holds the merge, but main's checks are
        // those of main's commits.
        let fork = Commit {
            on_branch: false,
            ..commit("fff", &["fff", "bbb", "aaa"], "bbb", checks(24, 0, 0, 0))
        };
        assert_eq!(outcome(&fork), None);
        assert_eq!(settles(&fork, &["bbb"]), None);
    }

    #[test]
    fn a_commit_settles_the_merges_it_holds() {
        let c = commit("ccc", &["ccc", "bbb", "aaa"], "ccc", checks(24, 0, 0, 0));
        assert_eq!(
            settles(&c, &["aaa", "ccc", "ddd"]),
            Some((Outcome::Passed, vec![0, 1]))
        );
        // GitHub writes commits in lowercase; fctl may have one otherwise.
        assert_eq!(settles(&c, &["AAA"]), Some((Outcome::Passed, vec![0])));
        // A commit before the merge, or beside it, doesn't hold it.
        assert_eq!(settles(&c, &["ddd"]), None);
        // Nor does one whose checks still run.
        let running = commit("ccc", &["ccc", "bbb"], "ccc", checks(20, 0, 0, 4));
        assert_eq!(settles(&running, &["bbb"]), None);
    }

    #[test]
    fn says_how_a_merges_checks_ended() {
        let failing = ["Rust tests".to_owned(), "Clippy".to_owned()];
        let v = View {
            pull: "romanesco/shop#67".into(),
            title: Some("Add Apple Pay"),
            session: "shop#12".into(),
            base: "main",
            merge_sha: "3f2a1c9d0e",
            sha: "3f2a1c9d0e",
            outcome: Outcome::Passed,
            passed: 24,
            failed: 0,
            cancelled: 0,
            failing: &[],
        };
        assert_eq!(
            line(&v),
            "romanesco/shop#67 “Add Apple Pay” (shop#12), into main: its 24 checks passed at 3f2a1c9."
        );
        let later = View {
            sha: "77aa00bb11",
            ..v.clone()
        };
        assert_eq!(
            line(&later),
            "romanesco/shop#67 “Add Apple Pay” (shop#12), into main: its 24 checks passed at 77aa00b, a later commit that holds it."
        );
        let failed = View {
            outcome: Outcome::Failed,
            passed: 21,
            failed: 2,
            cancelled: 1,
            failing: &failing,
            title: None,
            ..v.clone()
        };
        assert_eq!(
            line(&failed),
            "romanesco/shop#67 (shop#12), into main: 2 of 24 checks failed at 3f2a1c9: Rust tests and Clippy (1 cancelled)."
        );
        let cancelled = View {
            outcome: Outcome::Cancelled,
            passed: 23,
            cancelled: 1,
            ..v.clone()
        };
        assert_eq!(
            line(&cancelled),
            "romanesco/shop#67 “Add Apple Pay” (shop#12), into main: 1 of 24 checks was cancelled at 3f2a1c9, still main's newest commit."
        );
        for o in [Outcome::Passed, Outcome::Failed, Outcome::Cancelled] {
            assert_eq!(Outcome::parse(o.as_str()), Some(o));
        }
    }
}
