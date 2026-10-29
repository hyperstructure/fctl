//! The GitHub App review agents post their reviews as where it's installed
//! (docs/architecture.md#review-agents): `fctl-review` on app.fctl.sh,
//! beside the instance's GitHub App, `fctl-sh`, which its agents act as. On
//! GitHub, people then tell a review agent's findings apart from what the
//! agents answer.
//!
//! A workspace connects nothing for it: fctl uses it only on repositories
//! the workspace connected through the GitHub App, whose connection decides
//! what the workspace may use. Installing it on an account is all it takes,
//! and where it isn't installed, or doesn't reach a repository, the GitHub
//! App posts the reviews as before. This module says how it stands on each
//! of a workspace's accounts ([`state`]) and where on GitHub people install
//! it or choose its repositories ([`install_url`], [`settings_url`]).

use crate::totp::percent;
use fctl_proto::api::ReviewAppState;
use serde_json::{Value, json};
use std::collections::HashSet;

/// The review app's installation on an account, as GitHub describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// GitHub's id of the account it's on.
    pub account_id: Option<u64>,
    /// It reaches every repository of the account (its repository
    /// selection is `all`).
    pub all: bool,
}

/// How the review app stands on an account a workspace connected, whose
/// id the connection keeps (`connected`, `None` from before ids were
/// kept), and the repositories there its projects may write to
/// (`writable`) that it doesn't reach, whose reviews post as the GitHub
/// App. `found`: its installation there; `reached`: the names of the
/// repositories it reaches, in lowercase, where it reaches only some
/// (`None` when GitHub listed too many to tell). An installation on
/// another account than the one connected, which has the login now, isn't
/// the workspace's.
pub fn state(
    found: Option<&Installed>,
    connected: Option<u64>,
    reached: Option<&HashSet<String>>,
    writable: &[String],
) -> (ReviewAppState, Vec<String>) {
    let Some(found) = found else {
        return (ReviewAppState::NotInstalled, Vec::new());
    };
    if connected.is_some() && found.account_id != connected {
        return (ReviewAppState::NotInstalled, Vec::new());
    }
    if found.all {
        return (ReviewAppState::Installed, Vec::new());
    }
    let Some(reached) = reached else {
        return (ReviewAppState::Installed, Vec::new());
    };
    let mut unreached: Vec<String> = Vec::new();
    for name in writable {
        let new = !unreached.iter().any(|u| u.eq_ignore_ascii_case(name));
        if new && !reached.contains(&name.to_lowercase()) {
            unreached.push(name.clone());
        }
    }
    unreached.sort_by_key(|n| n.to_lowercase());
    match unreached.is_empty() {
        true => (ReviewAppState::Installed, unreached),
        false => (ReviewAppState::Partly, unreached),
    }
}

/// Where GitHub installs app `slug` (the review app) on the account whose
/// id is `target`, chosen already, on the repositories `repos` (GitHub's
/// ids, at most 100) or, without them, on every repository; with
/// `state`, which GitHub sends back to the app's setup URL. Without
/// `target`, people choose the account on GitHub.
pub fn install_url(slug: &str, target: Option<u64>, repos: &[u64], state: &str) -> String {
    let slug = percent(slug);
    let state = percent(state);
    let Some(target) = target else {
        return format!("https://github.com/apps/{slug}/installations/new?state={state}");
    };
    let repos: String = repos
        .iter()
        .take(MAX_PRESELECTED)
        .map(|id| format!("&repository_ids%5B%5D={id}"))
        .collect();
    format!(
        "https://github.com/apps/{slug}/installations/new/permissions?suggested_target_id={target}{repos}&state={state}"
    )
}

/// The most repositories GitHub's installation page takes preselected.
pub const MAX_PRESELECTED: usize = 100;

/// Where an account's admins choose the repositories installation `id` of
/// an app reaches: an organization's settings, or a user's own.
pub fn settings_url(account: &str, account_type: Option<&str>, id: u64) -> String {
    let account = percent(account);
    match account_type {
        Some("User") => format!("https://github.com/settings/installations/{id}"),
        _ => format!("https://github.com/organizations/{account}/settings/installations/{id}"),
    }
}

/// The GraphQL query for GitHub's ids of repositories `names` of `owner`
/// (at most [`MAX_PRESELECTED`]), which [`repository_ids`] reads.
pub fn repository_ids_query(owner: &str, names: &[String]) -> Value {
    let names: Vec<&String> = names.iter().take(MAX_PRESELECTED).collect();
    let params: Vec<String> = (0..names.len())
        .map(|i| format!(", $n{i}: String!"))
        .collect();
    let fields: Vec<String> = (0..names.len())
        .map(|i| format!("r{i}: repository(owner: $owner, name: $n{i}) {{ databaseId }}"))
        .collect();
    let mut variables = serde_json::Map::new();
    variables.insert("owner".into(), json!(owner));
    for (i, name) in names.iter().enumerate() {
        variables.insert(format!("n{i}"), json!(name));
    }
    json!({
        "query": format!("query($owner: String!{}) {{ {} }}", params.concat(), fields.join(" ")),
        "variables": variables,
    })
}

/// The repositories' ids in GitHub's answer to [`repository_ids_query`],
/// in order: those GitHub has, of the first `n`.
pub fn repository_ids(answer: &Value, n: usize) -> Vec<u64> {
    (0..n.min(MAX_PRESELECTED))
        .filter_map(|i| {
            answer
                .pointer(&format!("/data/r{i}/databaseId"))
                .and_then(Value::as_u64)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn set(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| s.to_lowercase()).collect()
    }

    #[test]
    fn it_stands_by_what_it_reaches_of_the_workspaces_repositories() {
        let writable = names(&["web", "API", "docs", "web"]);
        let all = Installed {
            account_id: Some(7),
            all: true,
        };
        let some = Installed {
            account_id: Some(7),
            all: false,
        };
        assert_eq!(
            state(None, Some(7), None, &writable),
            (ReviewAppState::NotInstalled, vec![])
        );
        assert_eq!(
            state(Some(&all), Some(7), None, &writable),
            (ReviewAppState::Installed, vec![])
        );
        // On some repositories: those it doesn't reach, once each.
        let reached = set(&["Web", "api", "other"]);
        assert_eq!(
            state(Some(&some), Some(7), Some(&reached), &writable),
            (ReviewAppState::Partly, names(&["docs"]))
        );
        let none = set(&[]);
        assert_eq!(
            state(Some(&some), Some(7), Some(&none), &writable),
            (ReviewAppState::Partly, names(&["API", "docs", "web"]))
        );
        let every = set(&["web", "api", "docs"]);
        assert_eq!(
            state(Some(&some), Some(7), Some(&every), &writable),
            (ReviewAppState::Installed, vec![])
        );
        // Too many to tell, or no repository of the workspace's there.
        assert_eq!(
            state(Some(&some), Some(7), None, &writable),
            (ReviewAppState::Installed, vec![])
        );
        assert_eq!(
            state(Some(&some), Some(7), Some(&none), &[]),
            (ReviewAppState::Installed, vec![])
        );
        // Another account has the login now: not the workspace's. A
        // connection from before ids were kept takes what it finds.
        assert_eq!(
            state(Some(&all), Some(8), None, &writable),
            (ReviewAppState::NotInstalled, vec![])
        );
        assert_eq!(
            state(Some(&all), None, None, &writable),
            (ReviewAppState::Installed, vec![])
        );
    }

    #[test]
    fn installing_goes_to_the_account_with_its_repositories_chosen() {
        assert_eq!(
            install_url("fctl-review", Some(42), &[], "s.t"),
            "https://github.com/apps/fctl-review/installations/new/permissions?suggested_target_id=42&state=s.t"
        );
        assert_eq!(
            install_url("fctl-review", Some(42), &[1, 2], "a+b/c="),
            "https://github.com/apps/fctl-review/installations/new/permissions?suggested_target_id=42&repository_ids%5B%5D=1&repository_ids%5B%5D=2&state=a%2Bb%2Fc%3D"
        );
        let many: Vec<u64> = (1..=150).collect();
        let url = install_url("fctl-review", Some(42), &many, "s");
        assert_eq!(url.matches("repository_ids").count(), MAX_PRESELECTED);
        // Without the account's id, people choose it on GitHub.
        assert_eq!(
            install_url("fctl-review", None, &[1], "s"),
            "https://github.com/apps/fctl-review/installations/new?state=s"
        );
        assert_eq!(
            settings_url("acme", Some("Organization"), 9),
            "https://github.com/organizations/acme/settings/installations/9"
        );
        assert_eq!(
            settings_url("ada", Some("User"), 9),
            "https://github.com/settings/installations/9"
        );
        assert_eq!(
            settings_url("acme", None, 9),
            "https://github.com/organizations/acme/settings/installations/9"
        );
    }

    #[test]
    fn repositories_are_looked_up_by_name_in_one_query() {
        let q = repository_ids_query("acme", &names(&["web", "api"]));
        assert_eq!(
            q["query"],
            "query($owner: String!, $n0: String!, $n1: String!) { r0: repository(owner: $owner, name: $n0) { databaseId } r1: repository(owner: $owner, name: $n1) { databaseId } }"
        );
        assert_eq!(q["variables"]["owner"], "acme");
        assert_eq!(q["variables"]["n1"], "api");
        let answer = json!({ "data": { "r0": { "databaseId": 11 }, "r1": null } });
        assert_eq!(repository_ids(&answer, 2), vec![11]);
        assert_eq!(
            repository_ids(&json!({ "errors": [] }), 2),
            Vec::<u64>::new()
        );
    }
}
