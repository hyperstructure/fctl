//! Review threads of pull requests, which agents that may comment but not
//! push resolve through the control plane (`fctl resolve`): GitHub lets
//! only tokens that can write a repository's contents resolve its threads,
//! and such a token could push too. The control plane checks that the
//! thread is in a repository the session may write to, then resolves it
//! with a token of its own ([`crate::repos::resolve_permissions`]).

use crate::validate::Result;
use fctl_proto::ApiError;
use fctl_proto::session::RepoRef;
use serde_json::{Value, json};

/// A thread's id as GitHub gives it (`PRRT_` and base64), checked so that
/// it goes to GitHub as an id and nothing else.
pub fn check_id(input: &str) -> Result<String> {
    let id = input.trim();
    let ok = id.starts_with("PRRT_")
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if ok {
        Ok(id.to_owned())
    } else {
        Err(ApiError::validation(
            "thread",
            "Name a review thread by its id, like PRRT_kwDOABCD1234. `gh api graphql` lists a pull request's threads with their ids.",
        ))
    }
}

/// Asks GitHub which pull request thread `id` is on, and whether it is
/// resolved.
pub fn lookup(id: &str) -> Value {
    json!({
        "query": "query($id: ID!) { node(id: $id) { ... on PullRequestReviewThread { id isResolved pullRequest { url repository { name owner { login } } } } } }",
        "variables": { "id": id },
    })
}

/// A review thread as [`lookup`] found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub resolved: bool,
    /// The repository of its pull request.
    pub repo: RepoRef,
    pub pull_request_url: String,
}

/// What GitHub's answer to [`lookup`] says: `None` when it has no review
/// thread by that id that the token can see.
pub fn found(answer: &Value) -> Option<Found> {
    let node = answer.pointer("/data/node")?;
    let pr = node.get("pullRequest")?;
    let repo = pr.get("repository")?;
    Some(Found {
        resolved: node.get("isResolved")?.as_bool()?,
        repo: RepoRef {
            owner: repo.pointer("/owner/login")?.as_str()?.to_owned(),
            name: repo.get("name")?.as_str()?.to_owned(),
        },
        pull_request_url: pr.get("url")?.as_str()?.to_owned(),
    })
}

/// Resolves thread `id`, or with `unresolve` opens it again.
pub fn resolve(id: &str, unresolve: bool) -> Value {
    let mutation = if unresolve {
        "unresolveReviewThread"
    } else {
        "resolveReviewThread"
    };
    json!({
        "query": format!("mutation($id: ID!) {{ {mutation}(input: {{ threadId: $id }}) {{ thread {{ id isResolved }} }} }}"),
        "variables": { "id": id },
    })
}

/// Whether the thread is resolved after [`resolve`], or GitHub's message
/// when it refused.
pub fn resolved(answer: &Value) -> std::result::Result<bool, String> {
    let data = answer.get("data");
    let state = ["resolveReviewThread", "unresolveReviewThread"]
        .iter()
        .find_map(|m| data?.get(*m)?.pointer("/thread/isResolved")?.as_bool());
    if let Some(state) = state {
        return Ok(state);
    }
    let message = answer
        .pointer("/errors/0/message")
        .and_then(Value::as_str)
        .or_else(|| answer.get("message").and_then(Value::as_str))
        .unwrap_or("GitHub gave no answer");
    Err(message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_threads_and_nothing_else() {
        assert_eq!(
            check_id(" PRRT_kwDOABCD12-_x ").ok(),
            Some("PRRT_kwDOABCD12-_x".to_owned())
        );
        for bad in [
            "",
            "PRRC_kwDO",
            "PRRT_a b",
            "PRRT_\"}) { x }",
            &format!("PRRT_{}", "a".repeat(100)),
        ] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn lookups_name_the_pull_request_and_its_repository() {
        let answer = json!({ "data": { "node": {
            "id": "PRRT_1", "isResolved": false,
            "pullRequest": { "url": "https://github.com/acme/web/pull/12",
                "repository": { "name": "web", "owner": { "login": "acme" } } }
        } } });
        assert_eq!(
            found(&answer),
            Some(Found {
                resolved: false,
                repo: RepoRef {
                    owner: "acme".into(),
                    name: "web".into()
                },
                pull_request_url: "https://github.com/acme/web/pull/12".into(),
            })
        );
        // Another kind of node, or none.
        assert_eq!(found(&json!({ "data": { "node": {} } })), None);
        assert_eq!(found(&json!({ "data": { "node": null } })), None);
        assert_eq!(found(&json!({ "errors": [{ "message": "x" }] })), None);
        assert_eq!(lookup("PRRT_1")["variables"]["id"], "PRRT_1");
    }

    #[test]
    fn resolving_says_what_github_did() {
        let q = resolve("PRRT_1", false);
        assert!(
            q["query"]
                .as_str()
                .is_some_and(|s| s.contains("resolveReviewThread("))
        );
        let q = resolve("PRRT_1", true);
        assert!(
            q["query"]
                .as_str()
                .is_some_and(|s| s.contains("unresolveReviewThread("))
        );
        let done = json!({ "data": { "resolveReviewThread": { "thread": { "id": "PRRT_1", "isResolved": true } } } });
        assert_eq!(resolved(&done), Ok(true));
        let opened = json!({ "data": { "unresolveReviewThread": { "thread": { "id": "PRRT_1", "isResolved": false } } } });
        assert_eq!(resolved(&opened), Ok(false));
        let refused = json!({ "data": { "resolveReviewThread": null }, "errors": [{ "type": "FORBIDDEN", "message": "Resource not accessible by integration" }] });
        assert_eq!(
            resolved(&refused),
            Err("Resource not accessible by integration".to_owned())
        );
    }
}
