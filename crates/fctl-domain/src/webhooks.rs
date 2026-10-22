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
        })
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
            let head = pr
                .pointer("/head/sha")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let numbers = vec![seen.number];
            let payload = Payload {
                action,
                seen,
                head: head.clone(),
                mergeable_state: text(pr, "mergeable_state"),
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

/// The most pull requests one lookup asks about. GitHub counts a query of
/// up to 100 as one point of the installation's rate limit (5,000 an hour),
/// and answers one of 50 in about two seconds; it gives up after ten.
pub const MAX_LOOKUP: usize = 50;

/// What a lookup asks of each pull request: its title, state and times,
/// whether it is in a merge queue, and the checks of its head commit
/// counted by their state.
const FIELDS: &str = "fragment pr on PullRequest {
  number url title state isDraft createdAt updatedAt mergedAt closedAt headRefOid
  mergeStateStatus
  mergeQueueEntry { enqueuedAt }
  commits(last: 1) {
    nodes {
      commit {
        oid
        statusCheckRollup {
          contexts {
            checkRunCountsByState { state count }
            statusContextCountsByState { state count }
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Looked {
    /// Its title, state, times and checks.
    pub pr: PullRequestRef,
    /// Its head commit.
    pub head: Option<String>,
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
            let count = u32::try_from(count).unwrap_or(u32::MAX);
            match check_state(state) {
                Some(Check::Passed) => checks.passed = checks.passed.saturating_add(count),
                Some(Check::Failed) => checks.failed = checks.failed.saturating_add(count),
                Some(Check::Pending) => checks.pending = checks.pending.saturating_add(count),
                None => {}
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
    })
}

enum Check {
    Passed,
    Failed,
    Pending,
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
}
