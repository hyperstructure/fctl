//! The branch a session starts from: its project's default branch, or the
//! one it asks for (`fctl run --base`). The repository must have it, unless
//! it is empty: a session there starts its branch from nothing. The control
//! plane asks GitHub when a project's default branch or repository changes
//! ([`lookup`], [`found`], [`check`]), and fctl-agent fails a session whose
//! base branch is missing from a repository with commits ([`missing`]).

use crate::validate::Result;
use fctl_proto::ApiError;
use fctl_proto::session::RepoRef;
use serde_json::{Value, json};

/// Asks GitHub whether `repo` has branch `branch`, whether it is empty,
/// and what its default branch is.
pub fn lookup(repo: &RepoRef, branch: &str) -> Value {
    json!({
        "query": "query($owner: String!, $name: String!, $ref: String!) { repository(owner: $owner, name: $name) { isEmpty defaultBranchRef { name } ref(qualifiedName: $ref) { name } } }",
        "variables": {
            "owner": repo.owner,
            "name": repo.name,
            "ref": format!("refs/heads/{branch}"),
        },
    })
}

/// What GitHub's answer to [`lookup`] says of the branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// The repository has it.
    Branch,
    /// The repository has no commits, so no branches yet.
    Empty,
    /// The repository has commits but not this branch; `default` is its
    /// default branch.
    Missing { default: Option<String> },
    /// GitHub shows no such repository.
    NoRepository,
}

/// What GitHub's answer to [`lookup`] says: `None` when it gave none.
pub fn found(answer: &Value) -> Option<Found> {
    let data = answer.get("data").filter(|d| !d.is_null())?;
    let Some(repo) = data.get("repository").filter(|r| !r.is_null()) else {
        return Some(Found::NoRepository);
    };
    if repo.get("ref").is_some_and(|r| !r.is_null()) {
        return Some(Found::Branch);
    }
    if repo.get("isEmpty")?.as_bool()? {
        return Some(Found::Empty);
    }
    let default = repo
        .pointer("/defaultBranchRef/name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(Found::Missing { default })
}

/// Refuses `branch` as the default branch of `repo` when GitHub found the
/// repository without it: its sessions would have nothing to start from.
/// Any branch will do in an empty repository.
pub fn check(repo: &RepoRef, branch: &str, found: &Found) -> Result<()> {
    let Found::Missing { default } = found else {
        return Ok(());
    };
    let like = default
        .as_deref()
        .filter(|d| *d != branch)
        .map(|d| format!(", like {d}"))
        .unwrap_or_default();
    Err(ApiError::validation(
        "default_branch",
        format!(
            "{} has no branch named {branch}: push it first, or choose one it has{like}",
            repo.full_name()
        ),
    ))
}

/// Why a session fails to start: `repo` has commits, but no branch `base`
/// to start the session's `branch` from. `default` says whether `base` is
/// the project's default branch, which people change for new sessions; the
/// session keeps its own, so pushing `base` lets it start when resumed.
pub fn missing(repo: &RepoRef, base: &str, default: bool, branch: &str) -> String {
    let repo = repo.full_name();
    if default {
        format!(
            "{repo} has no branch named {base}, the project's default branch, to start {branch} from. Choose a branch it has in Settings → Project → Default branch (fctl projects set --default-branch) and start a new session, or push {base} and resume this one."
        )
    } else {
        format!(
            "{repo} has no branch named {base} to start {branch} from. Start a new session from a branch it has (fctl run --base), or push {base} and resume this one."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web() -> RepoRef {
        RepoRef {
            owner: "acme".into(),
            name: "web".into(),
        }
    }

    #[test]
    fn lookups_ask_for_the_branch_by_its_full_name() {
        let q = lookup(&web(), "feature/a#b");
        assert_eq!(q["variables"]["owner"], "acme");
        assert_eq!(q["variables"]["name"], "web");
        // A variable, so that no name changes the query.
        assert_eq!(q["variables"]["ref"], "refs/heads/feature/a#b");
    }

    #[test]
    fn answers_tell_a_branch_from_an_empty_repository_and_a_missing_branch() {
        // As GitHub answered for hyperstructure/fctl, and an empty repository.
        let has = json!({ "data": { "repository": {
            "isEmpty": false, "defaultBranchRef": { "name": "main" }, "ref": { "name": "main" }
        } } });
        assert_eq!(found(&has), Some(Found::Branch));
        let empty = json!({ "data": { "repository": {
            "isEmpty": true, "defaultBranchRef": null, "ref": null
        } } });
        assert_eq!(found(&empty), Some(Found::Empty));
        let lacks = json!({ "data": { "repository": {
            "isEmpty": false, "defaultBranchRef": { "name": "main" }, "ref": null
        } } });
        assert_eq!(
            found(&lacks),
            Some(Found::Missing {
                default: Some("main".into())
            })
        );
        let gone = json!({ "data": { "repository": null }, "errors": [{
            "type": "NOT_FOUND", "message": "Could not resolve to a Repository with the name 'acme/web'."
        }] });
        assert_eq!(found(&gone), Some(Found::NoRepository));
        // No answer: rate limited, say.
        for none in [
            json!({ "errors": [{ "type": "RATE_LIMITED" }] }),
            json!({ "data": null }),
            json!({ "data": { "repository": { "ref": null } } }),
        ] {
            assert_eq!(found(&none), None, "{none}");
        }
    }

    #[test]
    fn only_a_missing_branch_is_refused() {
        assert!(check(&web(), "main", &Found::Branch).is_ok());
        assert!(check(&web(), "develop", &Found::Empty).is_ok());
        let e = check(
            &web(),
            "develpo",
            &Found::Missing {
                default: Some("main".into()),
            },
        )
        .err();
        let e = e.as_ref();
        assert_eq!(e.and_then(|e| e.field.as_deref()), Some("default_branch"));
        assert_eq!(
            e.map(|e| e.message.as_str()),
            Some(
                "acme/web has no branch named develpo: push it first, or choose one it has, like main"
            )
        );
        let e = check(&web(), "develop", &Found::Missing { default: None }).err();
        assert_eq!(
            e.map(|e| e.message),
            Some(
                "acme/web has no branch named develop: push it first, or choose one it has".into()
            )
        );
    }

    #[test]
    fn a_missing_base_says_how_to_start_the_session() {
        let default = missing(&web(), "develop", true, "fctl/12-fix-login");
        assert!(default.starts_with(
            "acme/web has no branch named develop, the project's default branch, to start fctl/12-fix-login from."
        ));
        assert!(default.contains("fctl projects set --default-branch"));
        assert!(default.ends_with("or push develop and resume this one."));
        let asked = missing(&web(), "release", false, "fctl/12-fix-login");
        assert!(
            asked.starts_with(
                "acme/web has no branch named release to start fctl/12-fix-login from."
            )
        );
        assert!(asked.contains("fctl run --base"));
        assert!(!asked.contains("default branch"));
    }
}
