//! GitHub's webhooks to the control plane's GitHub App, and what fctl asks
//! GitHub when one comes: whether a delivery is GitHub's, which pull
//! requests it concerns, and what a lookup of one says of it.
//!
//! A webhook only says that something changed: a pull request was opened,
//! pushed to, queued, merged or closed (`pull_request`), or the checks of a
//! commit moved on (`check_run`, `check_suite`, `status`). For each pull
//! request a session lists among those, the control plane looks it up
//! ([`query`]), which tells its state and all of its checks at once, and
//! the sessions take that in (`crate::pulls::observe`). When the lookup
//! fails, a `pull_request` event's payload tells the state.

use crate::pulls::{self, Times, github_time as time};
use fctl_proto::Millis;
use fctl_proto::session::{PullRequestChecks, PullRequestRef, RepoRef};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Map, Value, json};
use sha2::Sha256;

/// Whether `signature`, a delivery's `X-Hub-Signature-256` (`sha256=` and
/// the HMAC-SHA256 of its body in hex), is GitHub's, with the webhook's
/// `secret`. The comparison takes as long whatever the signature.
pub fn signed(secret: &str, body: &[u8], signature: &str) -> bool {
    let Some(tag) = signature
        .trim()
        .strip_prefix("sha256=")
        .and_then(|h| hex::decode(h).ok())
    else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&tag).is_ok()
}

/// What a delivery asks of the control plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// GitHub checks that the webhook works (`ping`).
    Ping,
    /// Look up the pull requests of a repository it names.
    Hook(Hook),
    /// Nothing about pull requests' state or checks.
    Other,
}

/// Pull requests whose state or checks may have changed: those of `repo`
/// numbered `numbers`, and those whose head commit is `sha`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hook {
    /// The app's installation that sent it, which mints the token that
    /// looks them up.
    pub installation: Option<u64>,
    /// GitHub's id of the account the repository belongs to: what GitHub
    /// says reaches only the sessions of workspaces connected to it.
    pub owner_id: Option<u64>,
    pub repo: RepoRef,
    pub numbers: Vec<u64>,
    pub sha: Option<String>,
    /// The `pull_request` event's own pull request.
    pub payload: Option<Payload>,
    /// It says that checks changed (`check_run`, `check_suite`, `status`):
    /// failed ones are news to pass on (`crate::follow_ups`).
    pub checks: bool,
}

/// What a `pull_request` event says of its pull request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload {
    /// `opened`, `synchronize`, `enqueued`, `closed`…
    pub action: String,
    /// Its title, state and times, as a lookup would have them: what the
    /// control plane takes when it can't look it up.
    pub seen: PullRequestRef,
    /// Its head commit.
    pub head: Option<String>,
    /// GitHub's `mergeable_state` in the payload, which lookups take when
    /// GitHub gives no `mergeStateStatus` (`pulls::queued`).
    pub mergeable_state: Option<String>,
    /// Its author's GitHub login, and the branches it comes from and goes
    /// into.
    pub author: Option<String>,
    pub head_ref: Option<String>,
    pub base_ref: Option<String>,
    /// Who merged it, and the person whose action the event is: their
    /// GitHub logins. An app's bot (a merge queue's) counts as nobody.
    pub merged_by: Option<String>,
    pub sender: Option<String>,
}

impl Payload {
    /// What the payload says to a lookup.
    pub fn hints(&self) -> Hints<'_> {
        Hints {
            action: Some(&self.action),
            mergeable_state: self.mergeable_state.as_deref(),
        }
    }
}

/// What a webhook says that a lookup of its pull request can't: that it
/// was added to the merge queue (`action`), and its `mergeable_state`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Hints<'a> {
    pub action: Option<&'a str>,
    pub mergeable_state: Option<&'a str>,
}

/// What a delivery of `event` (its `X-GitHub-Event`) with `payload` asks.
pub fn delivery(event: &str, payload: &Value) -> Delivery {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    let repo = || {
        let r = payload.get("repository")?;
        RepoRef::parse(&text(r, "full_name")?)
    };
    let installation = payload.pointer("/installation/id").and_then(Value::as_u64);
    let owner_id = payload
        .pointer("/repository/owner/id")
        .and_then(Value::as_u64);
    let hook = |repo, numbers, sha, payload| {
        Delivery::Hook(Hook {
            installation,
            owner_id,
            repo,
            numbers,
            sha,
            payload,
            checks: matches!(event, "check_run" | "check_suite" | "status"),
        })
    };
    let at = |v: &Value, path: &str| v.pointer(path).and_then(Value::as_str).map(str::to_owned);
    // A person's login: an app's bot, such as a merge queue's, is nobody's.
    let person = |v: Option<&Value>| {
        v.filter(|u| crate::follow_ups::is_person(u))
            .and_then(|u| at(u, "/login"))
    };
    match event {
        "ping" => Delivery::Ping,
        "pull_request" => {
            let (Some(repo), Some(pr)) = (repo(), payload.get("pull_request")) else {
                return Delivery::Other;
            };
            let action = text(payload, "action").unwrap_or_default();
            let Some(seen) = seen_in(pr, &action) else {
                return Delivery::Other;
            };
            let head = at(pr, "/head/sha");
            let numbers = vec![seen.number];
            let payload = Payload {
                action,
                seen,
                head: head.clone(),
                mergeable_state: text(pr, "mergeable_state"),
                author: at(pr, "/user/login"),
                head_ref: at(pr, "/head/ref"),
                base_ref: at(pr, "/base/ref"),
                merged_by: person(pr.get("merged_by")),
                sender: person(payload.get("sender")),
            };
            hook(repo, numbers, head, Some(payload))
        }
        "check_run" | "check_suite" => {
            let (Some(repo), Some(check)) = (repo(), payload.get(event)) else {
                return Delivery::Other;
            };
            // A check run's pull requests are its suite's.
            let numbers = [
                check.get("pull_requests"),
                check.pointer("/check_suite/pull_requests"),
            ]
            .into_iter()
            .flatten()
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(|p| p.get("number").and_then(Value::as_u64))
            .fold(Vec::new(), |mut all, n| {
                if !all.contains(&n) {
                    all.push(n);
                }
                all
            });
            hook(repo, numbers, text(check, "head_sha"), None)
        }
        "status" => {
            let Some(repo) = repo() else {
                return Delivery::Other;
            };
            hook(repo, Vec::new(), text(payload, "sha"), None)
        }
        _ => Delivery::Other,
    }
}

/// What a `pull_request` event's pull request says of it.
fn seen_in(pr: &Value, action: &str) -> Option<PullRequestRef> {
    let text = |key: &str| pr.get(key).and_then(Value::as_str);
    let flag = |key: &str| pr.get(key).and_then(Value::as_bool).unwrap_or(false);
    let url = text("html_url")?;
    let merged = flag("merged") || text("merged_at").is_some();
    let closed = text("state") == Some("closed");
    let queued = pulls::queued(Some(action), false, None, text("mergeable_state"));
    let state = pulls::state_of(merged, closed, queued, flag("draft"));
    let times = Times {
        created: text("created_at").and_then(time),
        merged: text("merged_at").and_then(time),
        closed: text("closed_at").and_then(time),
        queued: None,
    };
    Some(PullRequestRef {
        number: pr.get("number").and_then(Value::as_u64)?,
        url: url.to_owned(),
        title: text("title").map(str::to_owned),
        state: Some(state.to_owned()),
        repo: pulls::repo_of(url),
        state_changed_at: times.of(state),
        updated_at: text("updated_at").and_then(time),
        checks: None,
    })
}

/// The most pull requests one lookup asks about. GitHub counts a query by
/// the pages of connections it could fetch, a hundred to a point of the
/// installation's rate limit (5,000 an hour): two for each pull request
/// (its last commit, and that commit's checks), so one of 50 costs one
/// point. It answers one of 50 in two to three seconds, and gives up after
/// ten.
pub const MAX_LOOKUP: usize = 50;

/// What a lookup asks of each pull request: its title, state and times,
/// its author and branches, whether it is in a merge queue, and the checks
/// of its head commit counted by their state, with the first 100 checks
/// among them, in which [`replaced`] finds the runs that a later run of the
/// same check replaced, and [`failing`] the checks that failed.
const FIELDS: &str = "fragment pr on PullRequest {
  number url title state isDraft createdAt updatedAt mergedAt closedAt headRefOid
  headRefName baseRefName author { login __typename }
  mergeStateStatus
  mergeQueueEntry { enqueuedAt }
  commits(last: 1) {
    nodes {
      commit {
        oid
        statusCheckRollup {
          contexts(first: 100) {
            checkRunCountsByState { state count }
            statusContextCountsByState { state count }
            nodes {
              ... on CheckRun {
                name status conclusion databaseId
                checkSuite { app { databaseId } workflowRun { event workflow { databaseId } } }
              }
              ... on StatusContext { context state }
            }
          }
        }
      }
    }
  }
}";

/// The GraphQL request, its `query` and `variables`, that looks up `prs`
/// (one to [`MAX_LOOKUP`]) at once: [`looked_up`] reads each by its place
/// in `prs`, and [`remaining`] what is left of the rate limit.
pub fn query(prs: &[(RepoRef, u64)]) -> Value {
    let mut params = Vec::new();
    let mut fields = vec!["rateLimit { remaining }".to_owned()];
    let mut variables = Map::new();
    for (i, (repo, number)) in prs.iter().enumerate() {
        params.push(format!(
            "$owner{i}: String!, $name{i}: String!, $number{i}: Int!"
        ));
        fields.push(format!(
            "pr{i}: repository(owner: $owner{i}, name: $name{i}) {{ pullRequest(number: $number{i}) {{ ...pr }} }}"
        ));
        variables.insert(format!("owner{i}"), json!(repo.owner));
        variables.insert(format!("name{i}"), json!(repo.name));
        // GraphQL's Int has 32 bits: a larger number, which no pull request
        // has, asks for one GitHub doesn't have instead of failing them all.
        variables.insert(format!("number{i}"), json!((*number).min(i32::MAX as u64)));
    }
    json!({
        "query": format!(
            "query({}) {{\n  {}\n}}\n{FIELDS}",
            params.join(", "),
            fields.join("\n  ")
        ),
        "variables": variables,
    })
}

/// How many points of the installation's rate limit are left, as GitHub's
/// answer to [`query`] says.
pub fn remaining(answer: &Value) -> Option<u64> {
    answer.pointer("/data/rateLimit/remaining")?.as_u64()
}

/// A pull request as a lookup found it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Looked {
    /// Its title, state, times and checks.
    pub pr: PullRequestRef,
    /// Its head commit.
    pub head: Option<String>,
    /// Its author's GitHub login (`fctl[bot]`), and the branches it comes
    /// from and goes into.
    pub author: Option<String>,
    pub head_ref: Option<String>,
    pub base_ref: Option<String>,
    /// The checks of its head commit that failed, by name ([`failing`]).
    pub failing: Vec<String>,
}

/// What GitHub's answer to [`query`], asked at `at`, says of the `i`th pull
/// request it asked about, with what the webhook that asked says (`hints`);
/// `None` when GitHub has none.
pub fn looked_up(answer: &Value, i: usize, at: Millis, hints: Hints<'_>) -> Option<Looked> {
    let pr = answer.pointer(&format!("/data/pr{i}/pullRequest"))?;
    let text = |key: &str| pr.get(key).and_then(Value::as_str);
    let url = text("url")?;
    let state = text("state")?;
    let entry = pr.get("mergeQueueEntry").filter(|e| !e.is_null());
    let queued = pulls::queued(
        hints.action,
        entry.is_some(),
        text("mergeStateStatus"),
        hints.mergeable_state,
    );
    let draft = pr.get("isDraft").and_then(Value::as_bool).unwrap_or(false);
    let state = pulls::state_of(state == "MERGED", state == "CLOSED", queued, draft);
    let times = Times {
        created: text("createdAt").and_then(time),
        merged: text("mergedAt").and_then(time),
        closed: text("closedAt").and_then(time),
        queued: entry
            .and_then(|e| e.get("enqueuedAt"))
            .and_then(Value::as_str)
            .and_then(time),
    };
    let commit = pr.pointer("/commits/nodes/0/commit");
    let head = text("headRefOid")
        .or_else(|| commit?.get("oid")?.as_str())
        .map(str::to_owned);
    let failing = failing(commit.and_then(|c| c.pointer("/statusCheckRollup/contexts")));
    let checks = commit.map(|c| {
        let contexts = c.pointer("/statusCheckRollup/contexts");
        let counts = |key: &str| {
            contexts
                .and_then(|x| x.get(key))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|s| Some((s.get("state")?.as_str()?, s.get("count")?.as_u64()?)))
        };
        let mut checks = PullRequestChecks {
            sha: head.clone(),
            at,
            ..Default::default()
        };
        for (state, count) in
            counts("checkRunCountsByState").chain(counts("statusContextCountsByState"))
        {
            if let Some(check) = check_state(state) {
                let n = check.of(&mut checks);
                *n = n.saturating_add(u32::try_from(count).unwrap_or(u32::MAX));
            }
        }
        // GitHub counts every run of a check; the pull request shows its latest.
        for state in replaced(contexts) {
            if let Some(check) = check_state(state) {
                let n = check.of(&mut checks);
                *n = n.saturating_sub(1);
            }
        }
        checks
    });
    Some(Looked {
        pr: PullRequestRef {
            number: pr.get("number").and_then(Value::as_u64)?,
            url: url.to_owned(),
            title: text("title").map(str::to_owned),
            state: Some(state.to_owned()),
            repo: pulls::repo_of(url),
            state_changed_at: times.of(state),
            updated_at: text("updatedAt").and_then(time),
            checks,
        },
        head,
        // As REST names an app's bot, and pull requests' authors elsewhere:
        // `fctl[bot]`, where GraphQL says `fctl`.
        author: pr
            .pointer("/author/login")
            .and_then(Value::as_str)
            .map(|login| {
                if pr.pointer("/author/__typename").and_then(Value::as_str) == Some("Bot") {
                    format!("{login}[bot]")
                } else {
                    login.to_owned()
                }
            }),
        head_ref: text("headRefName").map(str::to_owned),
        base_ref: text("baseRefName").map(str::to_owned),
        failing,
    })
}

/// The checks among `contexts`' nodes that failed, by name, as the pull
/// request lists them: check runs whose latest run failed, timed out or
/// couldn't start, and commit statuses that failed or errored. A cancelled
/// run, or one that waits for someone to approve it, is no failure of the
/// code.
fn failing(contexts: Option<&Value>) -> Vec<String> {
    let nodes: Vec<&Value> = contexts
        .and_then(|c| c.get("nodes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let id = |run: &Value| run.get("databaseId").and_then(Value::as_u64);
    let mut names: Vec<String> = Vec::new();
    for node in &nodes {
        let (name, failed) = match (node.get("name"), node.get("context")) {
            (Some(name), _) => {
                let Some(check) = check_of(node) else {
                    continue;
                };
                // A later run of the same check replaced this one.
                let later = nodes.iter().any(|other| {
                    check_of(other) == Some(check)
                        && id(other).zip(id(node)).is_some_and(|(o, n)| o > n)
                });
                let failed = matches!(
                    node.get("conclusion").and_then(Value::as_str),
                    Some("FAILURE" | "TIMED_OUT" | "STARTUP_FAILURE")
                );
                (name.as_str(), failed && !later)
            }
            (None, Some(context)) => (
                context.as_str(),
                matches!(
                    node.get("state").and_then(Value::as_str),
                    Some("FAILURE" | "ERROR")
                ),
            ),
            (None, None) => continue,
        };
        if let Some(name) = name.filter(|_| failed)
            && !names.iter().any(|n| n == name)
        {
            names.push(name.to_owned());
        }
    }
    names
}

enum Check {
    Passed,
    Failed,
    Pending,
}

impl Check {
    /// Its count among `checks`.
    fn of(self, checks: &mut PullRequestChecks) -> &mut u32 {
        match self {
            Check::Passed => &mut checks.passed,
            Check::Failed => &mut checks.failed,
            Check::Pending => &mut checks.pending,
        }
    }
}

/// The states of the check runs among `contexts`' nodes that a later run
/// of the same check replaced: a re-run, or a run of its workflow again for
/// the same commit (one that runs when a pull request's title changes, say,
/// and cancels the run before). GitHub counts every run, but a check's
/// latest run is its result, the one `gh pr checks` shows alone. The latest
/// is the run GitHub created last, the highest `databaseId`. Commit
/// statuses need none of this: GitHub gives the latest of each context.
fn replaced(contexts: Option<&Value>) -> Vec<&str> {
    let runs: Vec<_> = contexts
        .and_then(|c| c.get("nodes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|run| {
            // As GitHub counts it: its conclusion once it finished, else its status.
            let state = run
                .get("conclusion")
                .and_then(Value::as_str)
                .or_else(|| run.get("status")?.as_str())?;
            Some((check_of(run)?, run.get("databaseId")?.as_u64()?, state))
        })
        .collect();
    runs.iter()
        .filter(|(check, id, _)| {
            runs.iter()
                .any(|(other, later, _)| other == check && later > id)
        })
        .map(|(_, _, state)| *state)
        .collect()
}

/// Which check a check run is a run of: its name, and the app, workflow and
/// event that ran it, which tell apart checks of the same name, such as a
/// job of a workflow that runs both on pushes and on pull requests.
fn check_of(run: &Value) -> Option<[&Value; 4]> {
    let suite = run.get("checkSuite").unwrap_or(&Value::Null);
    let at = |path| suite.pointer(path).unwrap_or(&Value::Null);
    Some([
        run.get("name").filter(|name| name.is_string())?,
        at("/app/databaseId"),
        at("/workflowRun/workflow/databaseId"),
        at("/workflowRun/event"),
    ])
}

/// What a check run's state (`CheckRunState`) or a commit status's
/// (`StatusState`) counts as; nothing for a state GitHub adds later.
fn check_state(state: &str) -> Option<Check> {
    Some(match state {
        "SUCCESS" | "NEUTRAL" | "SKIPPED" | "COMPLETED" => Check::Passed,
        "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => {
            Check::Failed
        }
        "QUEUED" | "IN_PROGRESS" | "PENDING" | "WAITING" | "REQUESTED" | "STALE" | "EXPECTED" => {
            Check::Pending
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deliveries_are_signed_with_the_secret() {
        // GitHub's example: https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries
        let secret = "It's a Secret to Everybody";
        let body = b"Hello, World!";
        let sig = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert!(signed(secret, body, sig));
        assert!(!signed(secret, b"Hello, World?", sig));
        assert!(!signed("another secret", body, sig));
        for bad in [
            "",
            "sha256=",
            "sha1=757107ea",
            "757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17",
            "sha256=zz",
        ] {
            assert!(!signed(secret, body, bad), "{bad}");
        }
    }

    fn ms(s: &str) -> Option<Millis> {
        time(s)
    }

    /// A `pull_request` event as GitHub sends it, with the fields fctl reads.
    fn pull_request_event(action: &str, pr: Value) -> Value {
        json!({
            "action": action,
            "number": pr["number"],
            "pull_request": pr,
            "repository": { "full_name": "acme/app", "name": "app", "owner": { "login": "acme", "id": 314 } },
            "installation": { "id": 42 },
        })
    }

    fn open_pr() -> Value {
        json!({
            "number": 12,
            "html_url": "https://github.com/acme/app/pull/12",
            "title": "Fix the flaky login test",
            "state": "open",
            "draft": false,
            "merged": false,
            "merged_at": null,
            "closed_at": null,
            "created_at": "2026-10-21T10:00:00Z",
            "updated_at": "2026-10-21T10:05:00Z",
            "mergeable_state": "unknown",
            "head": { "sha": "abc123", "ref": "fctl/1-fix" },
        })
    }

    fn hook(d: Delivery) -> Hook {
        match d {
            Delivery::Hook(h) => h,
            other => panic!("not a hook: {other:?}"),
        }
    }

    #[test]
    fn pull_request_events_say_its_state() {
        let h = hook(delivery(
            "pull_request",
            &pull_request_event("opened", open_pr()),
        ));
        assert_eq!(h.installation, Some(42));
        assert_eq!(h.owner_id, Some(314));
        assert_eq!(h.repo.full_name(), "acme/app");
        assert_eq!(
            (h.numbers.as_slice(), h.sha.as_deref()),
            (&[12][..], Some("abc123"))
        );
        let p = h.payload.unwrap_or_else(|| panic!("no payload"));
        assert_eq!(p.action, "opened");
        assert_eq!(p.head.as_deref(), Some("abc123"));
        assert_eq!(p.mergeable_state.as_deref(), Some("unknown"));
        assert_eq!(
            p.seen,
            PullRequestRef {
                number: 12,
                url: "https://github.com/acme/app/pull/12".into(),
                title: Some("Fix the flaky login test".into()),
                state: Some("open".into()),
                repo: RepoRef::parse("acme/app"),
                state_changed_at: ms("2026-10-21T10:00:00Z"),
                updated_at: ms("2026-10-21T10:05:00Z"),
                checks: None,
            }
        );
        let state = |action: &str, pr: Value| {
            let h = hook(delivery("pull_request", &pull_request_event(action, pr)));
            let seen = h.payload.unwrap_or_else(|| panic!("no payload")).seen;
            (seen.state.unwrap_or_default(), seen.state_changed_at)
        };
        let mut draft = open_pr();
        draft["draft"] = json!(true);
        assert_eq!(
            state("converted_to_draft", draft.clone()),
            ("draft".into(), ms("2026-10-21T10:00:00Z"))
        );
        assert_eq!(state("enqueued", open_pr()), ("queued".into(), None));
        assert_eq!(
            state("dequeued", open_pr()),
            ("open".into(), ms("2026-10-21T10:00:00Z"))
        );
        let mut merged = open_pr();
        merged["state"] = json!("closed");
        merged["merged"] = json!(true);
        merged["merged_at"] = json!("2026-10-21T11:00:00Z");
        merged["closed_at"] = json!("2026-10-21T11:00:00Z");
        assert_eq!(
            state("closed", merged),
            ("merged".into(), ms("2026-10-21T11:00:00Z"))
        );
        let mut closed = open_pr();
        closed["state"] = json!("closed");
        closed["closed_at"] = json!("2026-10-21T12:00:00Z");
        assert_eq!(
            state("closed", closed),
            ("closed".into(), ms("2026-10-21T12:00:00Z"))
        );
        // A pull request without its URL tells nothing.
        let mut broken = open_pr();
        broken.as_object_mut().map(|o| o.remove("html_url"));
        assert_eq!(
            delivery("pull_request", &pull_request_event("opened", broken)),
            Delivery::Other
        );
    }

    #[test]
    fn check_events_name_pull_requests_or_a_commit() {
        let repository = json!({ "full_name": "acme/app" });
        let run = json!({
            "action": "completed",
            "check_run": {
                "head_sha": "abc123", "status": "completed", "conclusion": "success",
                "pull_requests": [{ "number": 12 }],
                "check_suite": { "pull_requests": [{ "number": 12 }, { "number": 14 }] },
            },
            "repository": repository,
            "installation": { "id": 42 },
        });
        let h = hook(delivery("check_run", &run));
        assert_eq!(
            (h.numbers, h.sha.as_deref(), h.payload),
            (vec![12, 14], Some("abc123"), None)
        );
        let suite = json!({
            "action": "completed",
            "check_suite": { "head_sha": "def456", "pull_requests": [] },
            "repository": repository,
        });
        let h = hook(delivery("check_suite", &suite));
        assert_eq!(
            (h.installation, h.numbers.is_empty(), h.sha.as_deref()),
            (None, true, Some("def456"))
        );
        let status = json!({ "sha": "abc123", "state": "pending", "context": "ci", "repository": repository });
        let h = hook(delivery("status", &status));
        assert_eq!(
            (h.numbers.is_empty(), h.sha.as_deref()),
            (true, Some("abc123"))
        );
        assert_eq!(
            delivery("ping", &json!({ "zen": "Keep it simple." })),
            Delivery::Ping
        );
        assert_eq!(
            delivery("issues", &json!({ "repository": repository })),
            Delivery::Other
        );
        assert_eq!(
            delivery("status", &json!({ "sha": "abc" })),
            Delivery::Other
        );
    }

    /// GitHub's answer to a [`query`] of hyperstructure/fctl#170 alone,
    /// while its checks ran (trimmed of states with no checks).
    fn answer() -> Value {
        json!({ "data": { "rateLimit": { "remaining": 4990 }, "pr0": { "pullRequest": {
            "number": 170,
            "url": "https://github.com/hyperstructure/fctl/pull/170",
            "title": "ci: run the checks in parallel jobs and deploy main sooner",
            "state": "OPEN",
            "isDraft": true,
            "createdAt": "2026-10-21T16:20:00Z",
            "updatedAt": "2026-10-21T16:41:12Z",
            "mergedAt": null,
            "closedAt": null,
            "headRefOid": "9f8e7d",
            "mergeStateStatus": "BLOCKED",
            "mergeQueueEntry": null,
            "commits": { "nodes": [{ "commit": {
                "oid": "9f8e7d",
                "statusCheckRollup": { "contexts": {
                    "checkRunCountsByState": [
                        { "state": "IN_PROGRESS", "count": 2 },
                        { "state": "SKIPPED", "count": 1 },
                        { "state": "SUCCESS", "count": 5 },
                        { "state": "FAILURE", "count": 0 }
                    ],
                    "statusContextCountsByState": [
                        { "state": "EXPECTED", "count": 0 },
                        { "state": "SUCCESS", "count": 1 }
                    ]
                } }
            } }] }
        } } } })
    }

    #[test]
    fn lookups_say_the_state_and_count_the_checks() {
        let looked =
            looked_up(&answer(), 0, 1_000, Hints::default()).unwrap_or_else(|| panic!("no found"));
        assert_eq!(looked.head.as_deref(), Some("9f8e7d"));
        let pr = looked.pr;
        assert_eq!((pr.number, pr.state.as_deref()), (170, Some("draft")));
        assert_eq!(pr.repo, RepoRef::parse("hyperstructure/fctl"));
        assert_eq!(pr.state_changed_at, ms("2026-10-21T16:20:00Z"));
        assert_eq!(pr.updated_at, ms("2026-10-21T16:41:12Z"));
        assert_eq!(
            pr.checks,
            Some(PullRequestChecks {
                passed: 7,
                failed: 0,
                pending: 2,
                sha: Some("9f8e7d".into()),
                at: 1_000,
            })
        );
        // Without checks: none counted, still for that commit.
        let mut none = answer();
        none["data"]["pr0"]["pullRequest"]["commits"]["nodes"][0]["commit"]["statusCheckRollup"] =
            Value::Null;
        let c = looked_up(&none, 0, 1_000, Hints::default())
            .and_then(|l| l.pr.checks)
            .unwrap_or_else(|| panic!("no checks"));
        assert_eq!(
            (c.passed, c.failed, c.pending, c.sha.as_deref()),
            (0, 0, 0, Some("9f8e7d"))
        );
        // Nothing found: no such pull request, or repository, or answer.
        for nothing in [
            json!({ "data": { "pr0": { "pullRequest": null } } }),
            json!({ "data": { "pr0": null } }),
            json!({ "errors": [{ "message": "Bad credentials" }] }),
        ] {
            assert_eq!(looked_up(&nothing, 0, 1, Hints::default()), None);
        }
    }

    #[test]
    fn one_lookup_asks_about_many_pull_requests() {
        let repo = |s| RepoRef::parse(s).unwrap_or_else(|| panic!("{s}"));
        let body = query(&[
            (repo("acme/app"), 12),
            (repo("acme/site"), 3),
            (repo("acme/app"), 1 << 40),
        ]);
        assert_eq!(
            body["variables"],
            json!({
                "owner0": "acme", "name0": "app", "number0": 12,
                "owner1": "acme", "name1": "site", "number1": 3,
                // Larger than GraphQL's Int: no pull request is numbered so.
                "owner2": "acme", "name2": "app", "number2": i32::MAX,
            })
        );
        let q = body["query"].as_str().unwrap_or_default();
        assert!(q.starts_with(
            "query($owner0: String!, $name0: String!, $number0: Int!, $owner1: String!, "
        ));
        assert!(q.contains("\n  rateLimit { remaining }\n"));
        for i in 0..3 {
            assert!(q.contains(&format!(
                "\n  pr{i}: repository(owner: $owner{i}, name: $name{i}) {{ pullRequest(number: $number{i}) {{ ...pr }} }}"
            )));
        }
        assert!(q.ends_with(FIELDS));
        // The answer says each by its place, and what is left of the limit.
        let mut a = answer();
        a["data"]["pr1"] = json!({ "pullRequest": null });
        a["data"]["pr2"] = a["data"]["pr0"].clone();
        a["data"]["pr2"]["pullRequest"]["number"] = json!(171);
        a["data"]["pr2"]["pullRequest"]["url"] =
            json!("https://github.com/hyperstructure/fctl/pull/171");
        let number = |i| looked_up(&a, i, 1, Hints::default()).map(|l| l.pr.number);
        assert_eq!(
            [number(0), number(1), number(2), number(3)],
            [Some(170), None, Some(171), None]
        );
        assert_eq!(remaining(&a), Some(4990));
        assert_eq!(remaining(&json!({ "errors": [] })), None);
    }

    #[test]
    fn lookups_see_merge_queues() {
        let mut queued = answer();
        let pr = &mut queued["data"]["pr0"]["pullRequest"];
        pr["isDraft"] = json!(false);
        pr["mergeQueueEntry"] = json!({ "enqueuedAt": "2026-10-21T16:45:00Z" });
        let looked = looked_up(&queued, 0, 1, Hints::default())
            .unwrap_or_else(|| panic!("no found"))
            .pr;
        assert_eq!(looked.state.as_deref(), Some("queued"));
        assert_eq!(looked.state_changed_at, ms("2026-10-21T16:45:00Z"));
        // The webhook says it was queued, before GitHub lists its entry.
        let mut open = answer();
        open["data"]["pr0"]["pullRequest"]["isDraft"] = json!(false);
        let hints = |action| Hints {
            action: Some(action),
            mergeable_state: None,
        };
        let state = |v: &Value, h| looked_up(v, 0, 1, h).and_then(|l| l.pr.state);
        assert_eq!(state(&open, hints("enqueued")).as_deref(), Some("queued"));
        assert_eq!(state(&open, hints("dequeued")).as_deref(), Some("open"));
        // Merged beats queued.
        let mut merged = queued.clone();
        merged["data"]["pr0"]["pullRequest"]["state"] = json!("MERGED");
        merged["data"]["pr0"]["pullRequest"]["mergedAt"] = json!("2026-10-21T16:50:00Z");
        let pr = looked_up(&merged, 0, 1, hints("enqueued"))
            .unwrap_or_else(|| panic!("no found"))
            .pr;
        assert_eq!(
            (pr.state.as_deref(), pr.state_changed_at),
            (Some("merged"), ms("2026-10-21T16:50:00Z"))
        );
        // `mergeable_state` counts only when GitHub gives no `mergeStateStatus`.
        open["data"]["pr0"]["pullRequest"]["mergeStateStatus"] = Value::Null;
        let h = Hints {
            action: Some("edited"),
            mergeable_state: Some("in_merge_queue"),
        };
        assert_eq!(state(&open, h).as_deref(), Some("queued"));
    }

    #[test]
    fn checks_that_fail_or_wait_count_as_such() {
        let counts = |runs: Value, statuses: Value| {
            let mut a = answer();
            a["data"]["pr0"]["pullRequest"]["commits"]["nodes"][0]["commit"]["statusCheckRollup"]
                ["contexts"] =
                json!({ "checkRunCountsByState": runs, "statusContextCountsByState": statuses });
            let c = looked_up(&a, 0, 1, Hints::default())
                .and_then(|l| l.pr.checks)
                .unwrap_or_else(|| panic!("no checks"));
            (c.passed, c.failed, c.pending)
        };
        let runs = json!([
            { "state": "CANCELLED", "count": 1 }, { "state": "TIMED_OUT", "count": 1 },
            { "state": "ACTION_REQUIRED", "count": 1 }, { "state": "STARTUP_FAILURE", "count": 1 },
            { "state": "QUEUED", "count": 1 }, { "state": "WAITING", "count": 1 },
            { "state": "NEUTRAL", "count": 1 }, { "state": "SOMETHING_NEW", "count": 9 }
        ]);
        let statuses = json!([
            { "state": "ERROR", "count": 1 }, { "state": "EXPECTED", "count": 1 }, { "state": "PENDING", "count": 1 }
        ]);
        assert_eq!(counts(runs, statuses), (1, 5, 4));
    }

    /// GitHub's answer to a [`query`] of hyperstructure/fctl#216 (trimmed of
    /// states with no checks): its Commits workflow ran twice for its head
    /// commit as its title changed, and the second run cancelled the first.
    fn answer_216() -> Value {
        let ci = |name: &str, id: u64, conclusion: &str| {
            json!({ "name": name, "status": "COMPLETED", "conclusion": conclusion, "databaseId": id,
                "checkSuite": { "app": { "databaseId": 15368 }, "workflowRun": { "event": "pull_request", "workflow": { "databaseId": 387806428 } } } })
        };
        let commits = |id: u64, conclusion: &str| {
            json!({ "name": "Conventional Commits", "status": "COMPLETED", "conclusion": conclusion, "databaseId": id,
                "checkSuite": { "app": { "databaseId": 15368 }, "workflowRun": { "event": "pull_request", "workflow": { "databaseId": 388057660 } } } })
        };
        json!({ "data": { "rateLimit": { "remaining": 4952 }, "pr0": { "pullRequest": {
            "number": 216,
            "url": "https://github.com/hyperstructure/fctl/pull/216",
            "title": "feat(web): rename Home to Agent Floor and light its row up on hover",
            "state": "MERGED",
            "isDraft": false,
            "createdAt": "2026-10-22T19:52:09Z",
            "updatedAt": "2026-10-22T20:19:41Z",
            "mergedAt": "2026-10-22T20:19:37Z",
            "closedAt": "2026-10-22T20:19:37Z",
            "headRefOid": "e389f64d066714d0c366b8b4b3ae4157ff7bebfd",
            "mergeStateStatus": "UNKNOWN",
            "mergeQueueEntry": null,
            "commits": { "nodes": [{ "commit": {
                "oid": "e389f64d066714d0c366b8b4b3ae4157ff7bebfd",
                "statusCheckRollup": { "contexts": {
                    "checkRunCountsByState": [
                        { "state": "CANCELLED", "count": 1 },
                        { "state": "SKIPPED", "count": 3 },
                        { "state": "SUCCESS", "count": 6 }
                    ],
                    "statusContextCountsByState": [],
                    "nodes": [
                        commits(120062394194, "CANCELLED"),
                        ci("Formatting, generated types and Clippy", 120062394772, "SUCCESS"),
                        commits(120062476880, "SUCCESS"),
                        ci("Rust tests", 120062395217, "SUCCESS"),
                        ci("Web app and standalone server", 120062395530, "SUCCESS"),
                        ci("cargo xtask dev --smoke", 120062395207, "SUCCESS"),
                        ci("Build the Worker", 120062397324, "SKIPPED"),
                        ci("cargo xtask check", 120062830355, "SUCCESS"),
                        ci("Deploy app.fctl.sh", 120063000845, "SKIPPED"),
                        ci("Pin fctl.sh to these docs", 120062889266, "SKIPPED")
                    ]
                } }
            } }] }
        } } } })
    }

    #[test]
    fn lookups_say_who_opened_it_and_which_checks_failed() {
        let mut a = answer_216();
        let pr = &mut a["data"]["pr0"]["pullRequest"];
        pr["state"] = json!("OPEN");
        pr["headRefName"] = json!("fctl/119-agent-floor");
        pr["baseRefName"] = json!("main");
        pr["author"] = json!({ "login": "fctl-sh", "__typename": "Bot" });
        let nodes = pr
            .pointer_mut("/commits/nodes/0/commit/statusCheckRollup/contexts/nodes")
            .and_then(Value::as_array_mut)
            .unwrap_or_else(|| panic!("no nodes"));
        let run = |name: &str, id: u64, conclusion: &str| {
            json!({ "name": name, "status": "COMPLETED", "conclusion": conclusion, "databaseId": id,
                "checkSuite": { "app": { "databaseId": 15368 }, "workflowRun": { "event": "pull_request", "workflow": { "databaseId": 1 } } } })
        };
        nodes.extend([
            run("Rust tests", 7, "FAILURE"),
            // Failed, then passed when someone ran it again.
            run("Flaky", 8, "FAILURE"),
            run("Flaky", 9, "SUCCESS"),
            run("Timed out", 10, "TIMED_OUT"),
            run("Cancelled", 11, "CANCELLED"),
            run("Needs approval", 12, "ACTION_REQUIRED"),
            run("Rust tests", 13, "FAILURE"),
            json!({ "context": "ci/deploy", "state": "ERROR" }),
            json!({ "context": "ci/lint", "state": "SUCCESS" }),
            json!({}),
        ]);
        let looked = looked_up(&a, 0, 1, Hints::default()).unwrap_or_else(|| panic!("no found"));
        // GraphQL names an app's bot without `[bot]`, which REST adds.
        assert_eq!(looked.author.as_deref(), Some("fctl-sh[bot]"));
        let mut person = a.clone();
        person["data"]["pr0"]["pullRequest"]["author"] =
            json!({ "login": "fctl-sh", "__typename": "User" });
        let by = looked_up(&person, 0, 1, Hints::default()).and_then(|l| l.author);
        assert_eq!(by.as_deref(), Some("fctl-sh"));
        assert_eq!(looked.head_ref.as_deref(), Some("fctl/119-agent-floor"));
        assert_eq!(looked.base_ref.as_deref(), Some("main"));
        // In the order of their latest runs.
        assert_eq!(looked.failing, ["Timed out", "Rust tests", "ci/deploy"]);
        // A lookup from before the fields: none of them.
        let looked =
            looked_up(&answer(), 0, 1, Hints::default()).unwrap_or_else(|| panic!("no found"));
        assert_eq!(
            (looked.author, looked.head_ref, looked.failing.len()),
            (None, None, 0)
        );
        assert!(FIELDS.contains("headRefName baseRefName author { login __typename }"));
        assert!(FIELDS.contains("... on StatusContext { context state }"));
    }

    #[test]
    fn deliveries_say_whether_checks_changed_and_who_acted() {
        let mut pr = open_pr();
        pr["user"] = json!({ "login": "fctl[bot]", "type": "Bot" });
        pr["base"] = json!({ "ref": "main" });
        pr["merged_by"] = json!({ "login": "ada", "type": "User" });
        let mut event = pull_request_event("closed", pr);
        event["sender"] = json!({ "login": "ada", "type": "User" });
        let h = hook(delivery("pull_request", &event));
        assert!(!h.checks);
        let p = h.payload.unwrap_or_else(|| panic!("no payload"));
        assert_eq!(p.author.as_deref(), Some("fctl[bot]"));
        assert_eq!(p.head_ref.as_deref(), Some("fctl/1-fix"));
        assert_eq!(p.base_ref.as_deref(), Some("main"));
        assert_eq!(
            (p.merged_by.as_deref(), p.sender.as_deref()),
            (Some("ada"), Some("ada"))
        );
        // An app's bot is nobody.
        event["sender"] = json!({ "login": "github-merge-queue[bot]", "type": "Bot" });
        let p = hook(delivery("pull_request", &event))
            .payload
            .unwrap_or_else(|| panic!("no payload"));
        assert_eq!(p.sender, None);
        let status = json!({ "sha": "abc123", "state": "failure", "repository": { "full_name": "acme/app" } });
        assert!(hook(delivery("status", &status)).checks);
        let suite = json!({ "check_suite": { "head_sha": "abc" }, "repository": { "full_name": "acme/app" } });
        assert!(hook(delivery("check_suite", &suite)).checks);
    }

    #[test]
    fn a_check_that_ran_again_counts_by_its_latest_run() {
        // GitHub counts the cancelled run too; its pull request shows 9
        // checks, all passed or skipped.
        let c = looked_up(&answer_216(), 0, 1, Hints::default())
            .and_then(|l| l.pr.checks)
            .unwrap_or_else(|| panic!("no checks"));
        assert_eq!((c.passed, c.failed, c.pending), (9, 0, 0));
    }

    /// What a lookup counts of check runs (name, `databaseId`, workflow,
    /// event, state), which GitHub counts each: passed, failed, pending.
    /// Workflow 0 is another app's check, without one.
    fn count_runs(runs: &[(&str, Value, u64, &str, &str)]) -> (u32, u32, u32) {
        let mut states = std::collections::BTreeMap::<&str, u64>::new();
        let mut nodes: Vec<Value> = runs
            .iter()
            .map(|(name, id, workflow, event, state)| {
                *states.entry(state).or_default() += 1;
                let done = !matches!(*state, "QUEUED" | "IN_PROGRESS");
                let suite = match workflow {
                    0 => json!({ "app": { "databaseId": 77 }, "workflowRun": null }),
                    w => json!({ "app": { "databaseId": 15368 },
                        "workflowRun": { "event": event, "workflow": { "databaseId": w } } }),
                };
                json!({
                    "name": name,
                    "status": if done { "COMPLETED" } else { state },
                    "conclusion": done.then_some(state),
                    "databaseId": id,
                    "checkSuite": suite,
                })
            })
            .collect();
        // A commit status, which the query asks nothing of.
        nodes.push(json!({}));
        let mut a = answer();
        let contexts = a
            .pointer_mut("/data/pr0/pullRequest/commits/nodes/0/commit/statusCheckRollup/contexts")
            .unwrap_or_else(|| panic!("no contexts"));
        *contexts = json!({
            "checkRunCountsByState": states
                .iter()
                .map(|(state, count)| json!({ "state": state, "count": count }))
                .collect::<Vec<_>>(),
            "statusContextCountsByState": [],
            "nodes": nodes,
        });
        let c = looked_up(&a, 0, 1, Hints::default())
            .and_then(|l| l.pr.checks)
            .unwrap_or_else(|| panic!("no checks"));
        (c.passed, c.failed, c.pending)
    }

    #[test]
    fn runs_replace_only_earlier_runs_of_the_same_check() {
        const CI: u64 = 1;
        const RELEASE: u64 = 2;
        let pr = "pull_request";
        // The latest run counts, wherever GitHub lists it.
        let latest = [
            ("lint", json!(3), CI, pr, "SUCCESS"),
            ("lint", json!(1), CI, pr, "CANCELLED"),
            ("lint", json!(2), CI, pr, "FAILURE"),
        ];
        assert_eq!(count_runs(&latest), (1, 0, 0));
        // A failed check that someone re-runs waits for its new run.
        let rerun = [
            ("test", json!(4), CI, pr, "FAILURE"),
            ("test", json!(9), CI, pr, "QUEUED"),
        ];
        assert_eq!(count_runs(&rerun), (0, 0, 1));
        // A check of the same name in another workflow, on another event,
        // or from another app is another check.
        let others = [
            ("build", json!(1), CI, pr, "FAILURE"),
            ("build", json!(2), RELEASE, pr, "SUCCESS"),
            ("build", json!(3), CI, "push", "SUCCESS"),
            ("build", json!(4), 0, "", "SUCCESS"),
        ];
        assert_eq!(count_runs(&others), (3, 1, 0));
        // A run without its id replaces none, and none replaces it.
        let unknown = [
            ("test", Value::Null, CI, pr, "FAILURE"),
            ("test", json!(2), CI, pr, "SUCCESS"),
        ];
        assert_eq!(count_runs(&unknown), (1, 1, 0));
    }
}
