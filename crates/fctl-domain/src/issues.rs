//! GitHub's issues as [`Ticket`]s, for `fctl ticket` in projects whose
//! tracker is GitHub Issues of one repository (`crate::tickets`): their
//! agents' tokens reach no issues, so the control plane reads and writes
//! that repository's for them, with a token of its own for it alone. This
//! is what that decides without I/O: the queries, and GitHub's answers as
//! tickets.

use crate::pulls::github_time;
use crate::validate::Result;
use fctl_proto::ApiError;
use fctl_proto::session::{RepoRef, Tracker};
use fctl_proto::tickets::{
    StateFilter, StateKind, Ticket, TicketComment, TicketField, TicketLink, TicketPerson,
    TicketState,
};
use serde_json::{Value, json};

/// One issue, with its parent, sub-issues and latest comments.
pub const ISSUE: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { issue(number: $number) { \
    id databaseId number title body url state stateReason createdAt updatedAt \
    author { login avatarUrl __typename } labels(first: 20) { nodes { name } } \
    assignees(first: 5) { nodes { login name avatarUrl } } milestone { title dueOn } issueType { name } \
    parent { number title url state stateReason repository { nameWithOwner } } \
    subIssues(first: 50) { nodes { number title url state stateReason repository { nameWithOwner } } } \
    comments(last: 30) { totalCount nodes { id databaseId body url createdAt author { login avatarUrl __typename } } } } } }";

/// GitHub's states, as `fctl ticket states` lists them.
pub fn states() -> Vec<TicketState> {
    vec![
        TicketState {
            name: "open".into(),
            kind: StateKind::Unstarted,
            id: None,
        },
        TicketState {
            name: "closed".into(),
            kind: StateKind::Completed,
            id: None,
        },
        TicketState {
            name: "not planned".into(),
            kind: StateKind::Canceled,
            id: None,
        },
    ]
}

/// What a state's name comes to on GitHub: open, or closed with a reason
/// (`completed`, `not_planned`, `duplicate`).
pub fn state_change(name: &str) -> Result<(&'static str, Option<&'static str>)> {
    match name
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-'], " ")
        .as_str()
    {
        "open" | "reopen" | "reopened" | "todo" => Ok(("open", None)),
        "closed" | "close" | "done" | "completed" | "complete" => Ok(("closed", Some("completed"))),
        "not planned" | "canceled" | "cancelled" | "won't do" | "wont do" => {
            Ok(("closed", Some("not_planned")))
        }
        "duplicate" => Ok(("closed", Some("duplicate"))),
        _ => Err(ApiError::validation(
            "state",
            "GitHub issues are open or closed: use open, closed, not planned or duplicate",
        )),
    }
}

/// What a list of issues asks GitHub's search (`GET /search/issues?q=…`):
/// the repository's issues in that state, with those words and label.
pub fn search_q(
    repo: &RepoRef,
    state: StateFilter,
    query: Option<&str>,
    label: Option<&str>,
) -> String {
    let mut q = format!("repo:{} is:issue", repo.full_name());
    match state {
        StateFilter::Open => q.push_str(" is:open"),
        StateFilter::Closed => q.push_str(" is:closed"),
        StateFilter::All => {}
    }
    if let Some(l) = label.map(str::trim).filter(|l| !l.is_empty()) {
        q.push_str(&format!(" label:\"{}\"", l.replace('"', "")));
    }
    // Qualifiers that would reach other repositories go: their issues would
    // come back as this one's.
    let words: Vec<&str> = query
        .unwrap_or_default()
        .split_whitespace()
        .filter(|w| {
            let w = w.to_ascii_lowercase();
            !["repo:", "org:", "user:", "owner:"]
                .iter()
                .any(|q| w.starts_with(q))
        })
        .collect();
    if !words.is_empty() {
        q.push(' ');
        q.push_str(&words.join(" "));
    }
    q
}

/// The REST answer's `state` and `state_reason` as a state.
fn state(state: &str, reason: Option<&str>) -> TicketState {
    let open = state.eq_ignore_ascii_case("open");
    match (open, reason.map(str::to_ascii_lowercase).as_deref()) {
        (true, _) => TicketState {
            name: "open".into(),
            kind: StateKind::Unstarted,
            id: None,
        },
        (false, Some("not_planned" | "duplicate")) => TicketState {
            name: if reason.is_some_and(|r| r.eq_ignore_ascii_case("duplicate")) {
                "duplicate".into()
            } else {
                "not planned".into()
            },
            kind: StateKind::Canceled,
            id: None,
        },
        (false, _) => TicketState {
            name: "closed".into(),
            kind: StateKind::Completed,
            id: None,
        },
    }
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn time(v: &Value, key: &str) -> Option<i64> {
    github_time(v.get(key)?.as_str()?)
}

fn person(v: &Value) -> Option<TicketPerson> {
    let login = s(v, "login")?;
    let app = v
        .get("type")
        .or_else(|| v.get("__typename"))
        .and_then(Value::as_str)
        .is_some_and(|t| t == "Bot");
    Some(TicketPerson {
        name: s(v, "name").unwrap_or_else(|| login.clone()),
        handle: Some(login),
        avatar_url: s(v, "avatar_url").or_else(|| s(v, "avatarUrl")),
        app,
    })
}

/// An issue's key in `repo`, or in the repository GraphQL names it in.
fn key_in(repo: &RepoRef, v: &Value) -> Option<String> {
    let number = v.get("number").and_then(Value::as_u64)?;
    let full = v
        .get("repository")
        .and_then(|r| s(r, "nameWithOwner"))
        .unwrap_or_else(|| repo.full_name());
    Some(format!("{full}#{number}"))
}

fn link(repo: &RepoRef, v: &Value) -> Option<TicketLink> {
    Some(TicketLink {
        key: key_in(repo, v)?,
        title: s(v, "title").unwrap_or_default(),
        url: s(v, "url").or_else(|| s(v, "html_url")).unwrap_or_default(),
        state: v.get("state").and_then(Value::as_str).map(|st| {
            state(
                st,
                v.get("stateReason")
                    .or_else(|| v.get("state_reason"))
                    .and_then(Value::as_str),
            )
        }),
    })
}

/// An issue as GitHub's REST API gives it (`GET /repos/{o}/{n}/issues…`,
/// search) as a [`Ticket`]; `None` for a pull request, which the issues
/// endpoints list too.
pub fn ticket(repo: &RepoRef, v: &Value) -> Option<Ticket> {
    if v.get("pull_request").is_some_and(|p| !p.is_null()) {
        return None;
    }
    // Another repository's issue, as a search could find, isn't this one's.
    if let Some(url) = v.get("repository_url").and_then(Value::as_str)
        && !url
            .to_ascii_lowercase()
            .ends_with(&format!("/repos/{}", repo.full_name().to_ascii_lowercase()))
    {
        return None;
    }
    let number = v.get("number").and_then(Value::as_u64)?;
    let key = format!("{}#{number}", repo.full_name());
    let mut fields = Vec::new();
    if let Some(m) = v.get("milestone").and_then(|m| s(m, "title")) {
        fields.push(TicketField {
            name: "Milestone".into(),
            value: m,
        });
    }
    if let Some(n) = v.get("comments").and_then(Value::as_u64).filter(|n| *n > 0) {
        fields.push(TicketField {
            name: "Comments".into(),
            value: n.to_string(),
        });
    }
    Some(Ticket {
        tracker: Tracker::Github,
        id: key.clone(),
        key,
        title: s(v, "title").unwrap_or_default(),
        url: s(v, "html_url").unwrap_or_default(),
        description: s(v, "body"),
        state: Some(state(
            v.get("state").and_then(Value::as_str).unwrap_or("open"),
            v.get("state_reason").and_then(Value::as_str),
        )),
        kind: v.get("type").and_then(|t| s(t, "name")),
        labels: v
            .get("labels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|l| s(l, "name"))
            .collect(),
        priority: None,
        assignee: v.get("assignee").and_then(person),
        delegate: None,
        place: Some(repo.full_name()),
        parent: None,
        children: Vec::new(),
        fields,
        comments: Vec::new(),
        earlier_comments: 0,
        created_at: time(v, "created_at"),
        updated_at: time(v, "updated_at"),
    })
}

/// An issue as [`ISSUE`] asks GraphQL for it (the `issue` object) as a
/// [`Ticket`], with its parent, sub-issues and latest comments.
pub fn ticket_from_graphql(repo: &RepoRef, v: &Value) -> Option<Ticket> {
    let number = v.get("number").and_then(Value::as_u64)?;
    let key = format!("{}#{number}", repo.full_name());
    let nodes = |k: &str| -> Vec<Value> {
        v.get(k)
            .and_then(|c| c.get("nodes"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let comments: Vec<TicketComment> = nodes("comments")
        .iter()
        .filter_map(|c| {
            Some(TicketComment {
                id: c
                    .get("databaseId")
                    .and_then(Value::as_u64)
                    .map(|n| n.to_string())
                    .or_else(|| s(c, "id"))?,
                author: c.get("author").and_then(person),
                body: s(c, "body").unwrap_or_default(),
                url: s(c, "url"),
                created_at: time(c, "createdAt"),
                parent: None,
            })
        })
        .collect();
    let total = v
        .get("comments")
        .and_then(|c| c.get("totalCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut fields = Vec::new();
    if let Some(m) = v.get("milestone").filter(|m| !m.is_null()) {
        if let Some(title) = s(m, "title") {
            fields.push(TicketField {
                name: "Milestone".into(),
                value: title,
            });
        }
        if let Some(due) = s(m, "dueOn") {
            fields.push(TicketField {
                name: "Due".into(),
                value: due.chars().take(10).collect(),
            });
        }
    }
    let assignees: Vec<TicketPerson> = nodes("assignees").iter().filter_map(person).collect();
    if assignees.len() > 1 {
        fields.push(TicketField {
            name: "Assignees".into(),
            value: assignees
                .iter()
                .filter_map(|a| a.handle.clone())
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    Some(Ticket {
        tracker: Tracker::Github,
        id: key.clone(),
        key,
        title: s(v, "title").unwrap_or_default(),
        url: s(v, "url").unwrap_or_default(),
        description: s(v, "body"),
        state: Some(state(
            v.get("state").and_then(Value::as_str).unwrap_or("OPEN"),
            v.get("stateReason").and_then(Value::as_str),
        )),
        kind: v.get("issueType").and_then(|t| s(t, "name")),
        labels: nodes("labels")
            .iter()
            .filter_map(|l| s(l, "name"))
            .collect(),
        priority: None,
        assignee: assignees.into_iter().next(),
        delegate: None,
        place: Some(repo.full_name()),
        parent: v
            .get("parent")
            .filter(|p| !p.is_null())
            .and_then(|p| link(repo, p)),
        children: nodes("subIssues")
            .iter()
            .filter_map(|c| link(repo, c))
            .collect(),
        fields,
        earlier_comments: u32::try_from(total.saturating_sub(comments.len() as u64))
            .unwrap_or(u32::MAX),
        comments,
        created_at: time(v, "createdAt"),
        updated_at: time(v, "updatedAt"),
    })
}

/// A comment as GitHub's REST API gives it, on an issue of `repo`.
pub fn comment(v: &Value) -> Option<TicketComment> {
    Some(TicketComment {
        id: v.get("id").and_then(Value::as_u64)?.to_string(),
        author: v.get("user").and_then(person),
        body: s(v, "body").unwrap_or_default(),
        url: s(v, "html_url"),
        created_at: time(v, "created_at"),
        parent: None,
    })
}

/// The body of `PATCH /repos/{o}/{n}/issues/{number}` for what an update
/// changes: its title, description and state.
pub fn patch(title: Option<&str>, body: Option<&str>, state: Option<&str>) -> Result<Value> {
    let mut v = json!({});
    if let Some(t) = title {
        v["title"] = json!(t);
    }
    if let Some(b) = body {
        v["body"] = json!(b);
    }
    if let Some(st) = state {
        let (state, reason) = state_change(st)?;
        v["state"] = json!(state);
        if let Some(r) = reason {
            v["state_reason"] = json!(r);
        }
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> RepoRef {
        RepoRef {
            owner: "acme".into(),
            name: "web".into(),
        }
    }

    #[test]
    fn rest_issues_become_tickets_and_pull_requests_dont() {
        let v = json!({
            "number": 12, "title": "Fix login", "body": "It loops", "html_url": "https://github.com/acme/web/issues/12",
            "state": "closed", "state_reason": "not_planned", "labels": [{ "name": "bug" }],
            "assignee": { "login": "ada", "avatar_url": "https://a", "type": "User" },
            "milestone": { "title": "v2" }, "comments": 3, "type": { "name": "Bug" },
            "created_at": "2026-10-20T10:00:00Z", "updated_at": "2026-10-21T10:00:00Z"
        });
        let t = ticket(&repo(), &v).unwrap_or_else(|| unreachable!());
        assert_eq!(t.key, "acme/web#12");
        assert_eq!(t.id, "acme/web#12");
        assert_eq!(
            t.state.as_ref().map(|s| (s.name.as_str(), s.kind)),
            Some(("not planned", StateKind::Canceled))
        );
        assert_eq!(t.kind.as_deref(), Some("Bug"));
        assert_eq!(
            t.assignee.as_ref().and_then(|a| a.handle.as_deref()),
            Some("ada")
        );
        assert!(
            t.fields
                .iter()
                .any(|f| f.name == "Milestone" && f.value == "v2")
        );
        assert!(
            ticket(
                &repo(),
                &json!({ "number": 4, "pull_request": { "url": "x" } })
            )
            .is_none()
        );
    }

    #[test]
    fn graphql_issues_bring_their_parent_children_and_comments() {
        let v = json!({
            "number": 7, "title": "Auth", "body": "", "url": "https://github.com/acme/web/issues/7",
            "state": "OPEN", "stateReason": null, "labels": { "nodes": [{ "name": "epic" }] },
            "assignees": { "nodes": [{ "login": "ada" }, { "login": "grace", "name": "Grace" }] },
            "issueType": { "name": "Epic" },
            "parent": null,
            "subIssues": { "nodes": [{ "number": 8, "title": "Login", "url": "u8", "state": "CLOSED", "stateReason": "COMPLETED", "repository": { "nameWithOwner": "acme/web" } }] },
            "comments": { "totalCount": 40, "nodes": [{ "databaseId": 991, "body": "Ping", "url": "c", "createdAt": "2026-10-20T10:00:00Z", "author": { "login": "fctl-sh", "__typename": "Bot" } }] }
        });
        let t = ticket_from_graphql(&repo(), &v).unwrap_or_else(|| unreachable!());
        assert_eq!(t.description, None);
        assert_eq!(t.kind.as_deref(), Some("Epic"));
        assert_eq!(
            t.children.first().map(|c| c.key.as_str()),
            Some("acme/web#8")
        );
        assert_eq!(
            t.children[0].state.as_ref().map(|s| s.kind),
            Some(StateKind::Completed)
        );
        assert_eq!(t.comments[0].id, "991");
        assert!(t.comments[0].author.as_ref().is_some_and(|a| a.app));
        assert_eq!(t.earlier_comments, 39);
        assert!(
            t.fields
                .iter()
                .any(|f| f.name == "Assignees" && f.value == "ada, grace")
        );
    }

    #[test]
    fn states_as_people_name_them() {
        assert_eq!(state_change("Open").ok(), Some(("open", None)));
        assert_eq!(
            state_change("done").ok(),
            Some(("closed", Some("completed")))
        );
        assert_eq!(
            state_change("not_planned").ok(),
            Some(("closed", Some("not_planned")))
        );
        assert_eq!(
            state_change("Duplicate").ok(),
            Some(("closed", Some("duplicate")))
        );
        assert!(state_change("In Review").is_err());
        assert_eq!(
            patch(Some("New"), None, Some("closed")).ok(),
            Some(json!({ "title": "New", "state": "closed", "state_reason": "completed" }))
        );
    }

    #[test]
    fn searches_one_repository() {
        assert_eq!(
            search_q(&repo(), StateFilter::Open, Some("login loop"), Some("bug")),
            "repo:acme/web is:issue is:open label:\"bug\" login loop"
        );
        assert_eq!(
            search_q(&repo(), StateFilter::All, None, None),
            "repo:acme/web is:issue"
        );
        // Nothing reaches beyond the repository.
        assert_eq!(
            search_q(
                &repo(),
                StateFilter::All,
                Some("crash repo:other/app ORG:big user:x"),
                None
            ),
            "repo:acme/web is:issue crash"
        );
    }

    #[test]
    fn search_results_from_other_repositories_are_dropped() {
        let item = |url: &str| {
            json!({ "number": 3, "title": "T", "state": "open", "html_url": "u",
                    "repository_url": url, "created_at": "2026-10-20T10:00:00Z" })
        };
        assert!(ticket(&repo(), &item("https://api.github.com/repos/acme/web")).is_some());
        assert!(ticket(&repo(), &item("https://api.github.com/repos/ACME/Web")).is_some());
        assert!(ticket(&repo(), &item("https://api.github.com/repos/other/app")).is_none());
    }
}
