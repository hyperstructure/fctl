//! The pull requests fctl-agent follows for its session: those of the
//! branches its repositories have checked out, those `gh pr create`
//! printed, whichever branch, worktree or clone they came from, and those
//! the session listed when the sandbox started. It reports them all with
//! their state as it last looked them up, and looks each up again when a
//! turn ends until it is merged. What it knows of a merged one's commits
//! shows the work it merged, and keeps fctl-agent from pushing its branch
//! again.

use crate::workspace::{Moment, Paths, run};
use fctl_domain::{pulls, repos};
use fctl_proto::runner::SessionSpec;
use fctl_proto::session::{PullRequestRef, RepoGrant, RepoRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// What fctl-agent asks `gh pr view` for.
const FIELDS: &str = "number,url,title,state,isDraft,headRefName,headRefOid,baseRefOid";

/// The fields every gh that fctl-agent supports has.
const OLD_FIELDS: &str = "number,url,title,state,isDraft";

/// A pull request fctl-agent follows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Followed {
    /// As git states report it, without its repository, which its URL
    /// names: until a lookup tells more, all its URL says.
    pub pr: PullRequestRef,
    /// The branch it is from (`headRefName`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The commit its branch was at (`headRefOid`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// The commit of its base it is compared with (`baseRefOid`): once it
    /// is merged, the base's last before the merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

impl Followed {
    /// Whether it was merged when fctl-agent last looked it up.
    pub fn merged(&self) -> bool {
        self.pr.state.as_deref() == Some("merged")
    }

    /// The repository it is in.
    fn repo(&self) -> Option<RepoRef> {
        pulls::repo_of(&self.pr.url)
    }
}

/// The pull requests fctl-agent follows, in the order it found them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Pulls(Vec<Followed>);

impl Pulls {
    /// Follows these too: pull requests `gh pr create` printed, or that the
    /// session lists, in repositories the session may use, at most
    /// [`pulls::MAX_PULL_REQUESTS`] of them. Returns whether it follows more.
    pub fn follow(
        &mut self,
        prs: impl IntoIterator<Item = PullRequestRef>,
        spec: &SessionSpec,
    ) -> bool {
        let grants: Vec<RepoGrant> = spec
            .project
            .repos
            .iter()
            .flatten()
            .filter(|r| r.unavailable.is_none())
            .map(|r| r.grant())
            .collect();
        let mut more = false;
        for pr in prs {
            let Some(repo) = pulls::repo_of(&pr.url) else {
                continue;
            };
            let usable = repos::access(spec.project.repo.as_ref(), &grants, &repo).is_some();
            if !usable || self.get(&pr.url).is_some() || self.0.len() >= pulls::MAX_PULL_REQUESTS {
                continue;
            }
            self.0.push(Followed {
                pr: PullRequestRef { repo: None, ..pr },
                branch: None,
                head: None,
                base: None,
            });
            more = true;
        }
        more
    }

    fn get(&self, url: &str) -> Option<&Followed> {
        self.0.iter().find(|f| f.pr.url.eq_ignore_ascii_case(url))
    }

    /// The pull request of `branch` in `repo`: an open one before the
    /// others, and the latest of those.
    pub fn of_branch(&self, repo: &RepoRef, branch: &str) -> Option<&Followed> {
        self.0
            .iter()
            .filter(|f| f.branch.as_deref() == Some(branch))
            .filter(|f| f.repo().is_some_and(|r| r.same(repo)))
            .max_by_key(|f| {
                let open = matches!(f.pr.state.as_deref(), Some("open" | "draft"));
                (open, f.pr.number)
            })
    }

    /// The merged pull request of `branch` in `repo`, if that is its pull
    /// request.
    pub fn merged(&self, repo: Option<&RepoRef>, branch: Option<&str>) -> Option<&Followed> {
        self.of_branch(repo?, branch?).filter(|f| f.merged())
    }

    /// Takes what a lookup found: newer facts of one it follows, keeping
    /// those an older gh doesn't tell, or one more to follow.
    pub fn found(&mut self, found: Followed) {
        match self
            .0
            .iter_mut()
            .find(|f| f.pr.url.eq_ignore_ascii_case(&found.pr.url))
        {
            Some(f) => {
                *f = Followed {
                    branch: found.branch.or(f.branch.take()),
                    head: found.head.or(f.head.take()),
                    base: found.base.or(f.base.take()),
                    pr: found.pr,
                }
            }
            None => self.0.push(found),
        }
    }

    /// The URLs to look up at `moment` besides the checked-out branches'
    /// (`looked`): after `gh pr create`, the ones never looked up; when a
    /// turn ends, every one that isn't merged.
    pub fn due(&self, moment: Moment, looked: &[String]) -> Vec<String> {
        self.0
            .iter()
            .filter(|f| !looked.iter().any(|u| u.eq_ignore_ascii_case(&f.pr.url)))
            .filter(|f| match moment {
                Moment::Working { opened_pr } => opened_pr && f.pr.state.is_none(),
                Moment::TurnEnded => !f.merged(),
                Moment::Stopping => false,
            })
            .map(|f| f.pr.url.clone())
            .collect()
    }

    /// Every one, as git states report them.
    pub fn listed(&self) -> Vec<PullRequestRef> {
        self.0.iter().map(|f| f.pr.clone()).collect()
    }
}

/// Looks up a pull request with `gh pr view`: a branch's, in the
/// repository of the clone in `dir`, or the one a URL names.
pub async fn look_up(p: &Paths, dir: &Path, selector: &str) -> Option<Followed> {
    let view = |fields: &'static str| ["pr", "view", selector, "--json", fields];
    let mut o = run(dir, &p.home, &p.gh, &view(FIELDS)).await.ok()?;
    if !o.ok && o.stderr.contains("Unknown JSON field") {
        o = run(dir, &p.home, &p.gh, &view(OLD_FIELDS)).await.ok()?;
    }
    if !o.ok {
        return None;
    }
    let branch = (!selector.starts_with("https://")).then_some(selector);
    parse(&o.stdout, branch)
}

/// What `gh pr view --json` says of a pull request; `branch` is the one
/// it was looked up for, if it was.
fn parse(json: &str, branch: Option<&str>) -> Option<Followed> {
    let v: Value = serde_json::from_str(json).ok()?;
    let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    let draft = v.get("isDraft").and_then(Value::as_bool).unwrap_or(false);
    let state = text("state").map(|s| {
        if draft && s == "OPEN" {
            "draft".to_owned()
        } else {
            s.to_lowercase()
        }
    });
    Some(Followed {
        pr: PullRequestRef {
            number: v.get("number").and_then(Value::as_u64)?,
            url: text("url")?,
            title: text("title"),
            state,
            repo: None,
        },
        branch: text("headRefName").or(branch.map(str::to_owned)),
        head: text("headRefOid"),
        base: text("baseRefOid"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::tests::spec_with;
    use anyhow::{Context, Result};

    fn followed(repo: &str, number: u64, state: &str, branch: &str) -> Followed {
        Followed {
            pr: PullRequestRef {
                number,
                url: format!("https://github.com/{repo}/pull/{number}"),
                title: None,
                state: Some(state.into()),
                repo: None,
            },
            branch: Some(branch.into()),
            head: None,
            base: None,
        }
    }

    fn named(url: &str) -> Option<PullRequestRef> {
        pulls::named_by(url)
    }

    #[test]
    fn follows_pull_requests_in_the_repositories_the_session_may_use() -> Result<()> {
        let spec = spec_with(serde_json::json!([
            { "owner": "acme", "name": "lib", "access": "write" },
            { "owner": "acme", "name": "gone", "access": "write", "unavailable": "The GitHub App can't reach it" },
            { "owner": "other", "access": "read" }
        ]))?;
        let mut pulls = Pulls::default();
        let urls = [
            "https://github.com/acme/widgets/pull/1",
            "https://github.com/Acme/Lib/pull/2",
            "https://github.com/other/thing/pull/3",
            "https://github.com/acme/gone/pull/4",
            "https://github.com/stranger/app/pull/5",
        ];
        assert!(pulls.follow(urls.iter().filter_map(|u| named(u)), &spec));
        let listed: Vec<String> = pulls.listed().into_iter().map(|p| p.url).collect();
        assert_eq!(listed, &urls[..3]);
        // As agents report them: their URLs name their repositories.
        assert!(pulls.listed().iter().all(|p| p.repo.is_none()));
        // Each once.
        let again = named("https://github.com/acme/widgets/pull/1");
        assert!(!pulls.follow(again, &spec));
        Ok(())
    }

    #[test]
    fn a_branch_has_its_open_pull_request_or_its_latest() -> Result<()> {
        let widgets = RepoRef::parse("acme/widgets").context("a repository")?;
        let number = |pulls: &Pulls, b: &str| pulls.of_branch(&widgets, b).map(|f| f.pr.number);
        let mut pulls = Pulls::default();
        for f in [
            followed("acme/widgets", 1, "merged", "fctl/1-test"),
            followed("acme/widgets", 2, "open", "fctl/1-test"),
            followed("acme/widgets", 3, "closed", "fctl/1-test"),
            followed("acme/lib", 4, "open", "fctl/1-second"),
        ] {
            pulls.found(f);
        }
        assert_eq!(number(&pulls, "fctl/1-test"), Some(2));
        // Another repository's branch of that name is another's.
        assert_eq!(number(&pulls, "fctl/1-second"), None);
        assert_eq!(pulls.merged(Some(&widgets), Some("fctl/1-test")), None);
        // Once the open one merges too, the latest.
        pulls.found(followed("acme/widgets", 2, "merged", "fctl/1-test"));
        assert_eq!(number(&pulls, "fctl/1-test"), Some(3));
        pulls.found(followed("acme/widgets", 3, "merged", "fctl/1-test"));
        let merged = pulls.merged(Some(&widgets), Some("fctl/1-test"));
        assert_eq!(merged.map(|f| f.pr.number), Some(3));
        Ok(())
    }

    #[test]
    fn lookups_keep_what_older_gh_does_not_tell() {
        let mut pulls = Pulls::default();
        let mut first = followed("acme/widgets", 1, "open", "fctl/1-test");
        first.head = Some("a1".into());
        pulls.found(first);
        // From a gh without headRefName and headRefOid.
        let old = parse(
            r#"{"number":1,"url":"https://github.com/acme/widgets/pull/1","title":"First","state":"MERGED","isDraft":false}"#,
            None,
        );
        let Some(old) = old else {
            panic!("a pull request");
        };
        assert_eq!((old.branch.as_deref(), old.head.as_deref()), (None, None));
        pulls.found(old);
        let f = &pulls.0[0];
        assert_eq!(f.pr.state.as_deref(), Some("merged"));
        assert_eq!(f.pr.title.as_deref(), Some("First"));
        assert_eq!(
            (f.branch.as_deref(), f.head.as_deref()),
            (Some("fctl/1-test"), Some("a1"))
        );
        // A draft, looked up for its branch.
        let draft = parse(
            r#"{"number":2,"url":"https://github.com/acme/widgets/pull/2","state":"OPEN","isDraft":true}"#,
            Some("fctl/1-draft"),
        );
        assert_eq!(
            draft.map(|f| (f.pr.state, f.branch)),
            Some((Some("draft".into()), Some("fctl/1-draft".into())))
        );
        assert_eq!(parse("not json", None), None);
    }

    #[test]
    fn looks_up_new_ones_after_gh_pr_create_and_every_one_not_merged_after_a_turn() {
        let mut pulls = Pulls::default();
        pulls.found(followed("acme/widgets", 1, "merged", "fctl/1-test"));
        pulls.found(followed("acme/widgets", 2, "open", "fctl/1-second"));
        pulls.found(followed("acme/widgets", 3, "closed", "fctl/1-third"));
        let mut stub = followed("acme/widgets", 4, "open", "x");
        stub.pr.state = None;
        stub.branch = None;
        pulls.found(stub);
        let due = |moment: Moment, looked: &[String]| -> Vec<u64> {
            let urls = pulls.due(moment, looked);
            urls.iter()
                .filter_map(|u| pulls.get(u).map(|f| f.pr.number))
                .collect()
        };
        let opened = Moment::Working { opened_pr: true };
        assert_eq!(due(opened, &[]), [4]);
        assert!(due(Moment::Working { opened_pr: false }, &[]).is_empty());
        assert_eq!(due(Moment::TurnEnded, &[]), [2, 3, 4]);
        // Not twice: the checked-out branches' were looked up already.
        let looked = ["https://github.com/ACME/widgets/pull/2".to_owned()];
        assert_eq!(due(Moment::TurnEnded, &looked), [3, 4]);
        assert!(due(Moment::Stopping, &[]).is_empty());
    }
}
