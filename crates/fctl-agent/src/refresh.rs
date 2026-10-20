//! When fctl-agent refreshes the repositories' state while Claude works.
//!
//! The state (changed files, commits, whether the branch is pushed, pull
//! requests) goes out in a `git` event when a turn ends and when the
//! session stops, and in between soon after Claude changes the
//! repositories: once the result of a tool call that can change them
//! comes (`Bash`, `Write`, `Edit`, `NotebookEdit`, a subagent's too), or a
//! background task ends. A burst of them costs one refresh, [`QUIET`]
//! after the last, and refreshes come at most every [`EVERY`]. The pull
//! requests `gh pr create` prints go to the next one, which follows them
//! whichever branch they came from.

use fctl_domain::pulls;
use fctl_proto::session::PullRequestRef;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The least time between two refreshes.
pub const EVERY: Duration = Duration::from_secs(10);

/// How long a refresh waits for more tool results, unless the first came
/// [`EVERY`] ago: a burst of tool calls costs one refresh.
pub const QUIET: Duration = Duration::from_secs(2);

/// The tools that can change the repositories: Claude Code's, and one that
/// older versions of it have (`MultiEdit`).
const CHANGERS: [&str; 5] = ["Bash", "Write", "Edit", "MultiEdit", "NotebookEdit"];

/// Most tool calls followed at a time: the results of interrupted ones
/// never come.
const MAX_CALLS: usize = 1000;

#[derive(Debug, Default)]
pub struct Refresh {
    /// The tool calls that can change the repositories whose results
    /// haven't come, by id: whether each opens a pull request.
    calls: HashMap<String, bool>,
    /// When the first tool result since the last refresh came, and the
    /// latest.
    changed: Option<(Instant, Instant)>,
    /// What `gh pr create` did since the last refresh.
    opened: Opened,
    /// When the last refresh started.
    last: Option<Instant>,
}

/// What `gh pr create` did since the last refresh.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Opened {
    /// It ran.
    pub ran: bool,
    /// The pull requests it printed, as their URLs name them.
    pub pull_requests: Vec<PullRequestRef>,
}

impl Refresh {
    /// Follows a Claude message that came at `now`.
    pub fn observe(&mut self, v: &Value, now: Instant) {
        match v.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                for block in blocks(v, "tool_use") {
                    let (Some(id), Some(name)) = (text(block, "id"), text(block, "name")) else {
                        continue;
                    };
                    if !CHANGERS.contains(&name) {
                        continue;
                    }
                    if self.calls.len() >= MAX_CALLS {
                        self.calls.clear();
                    }
                    let command = block.pointer("/input/command").and_then(Value::as_str);
                    let opens_pr = name == "Bash" && command.is_some_and(opens_pr);
                    self.calls.insert(id.to_owned(), opens_pr);
                }
            }
            Some("user") => {
                for block in blocks(v, "tool_result") {
                    let Some(opens_pr) =
                        text(block, "tool_use_id").and_then(|id| self.calls.remove(id))
                    else {
                        continue;
                    };
                    if opens_pr {
                        self.opened.ran = true;
                        for pr in pulls::named_in(&result_text(block)) {
                            if !self.opened.pull_requests.iter().any(|o| o.url == pr.url) {
                                self.opened.pull_requests.push(pr);
                            }
                        }
                    }
                    self.touch(now);
                }
            }
            Some("system") if text(v, "subtype") == Some("task_notification") => self.touch(now),
            _ => {}
        }
    }

    fn touch(&mut self, now: Instant) {
        let first = self.changed.map_or(now, |(first, _)| first);
        self.changed = Some((first, now));
    }

    /// Whether a refresh is due at `now`.
    pub fn due(&self, now: Instant) -> bool {
        let Some((first, latest)) = self.changed else {
            return false;
        };
        let settled = now.duration_since(latest) >= QUIET || now.duration_since(first) >= EVERY;
        settled && self.last.is_none_or(|t| now.duration_since(t) >= EVERY)
    }

    /// A refresh starts at `now`, while Claude works or once a turn has
    /// ended: returns what `gh pr create` did since the last one.
    pub fn start(&mut self, now: Instant) -> Opened {
        self.changed = None;
        self.last = Some(now);
        std::mem::take(&mut self.opened)
    }

    /// As if `by` had passed since what it remembers.
    #[cfg(test)]
    pub fn backdate(&mut self, by: Duration) {
        let back = |t: Instant| t.checked_sub(by).unwrap_or(t);
        self.changed = self
            .changed
            .map(|(first, latest)| (back(first), back(latest)));
        self.last = self.last.map(back);
    }
}

/// The content blocks of a message that have the type `kind`.
fn blocks<'a>(v: &'a Value, kind: &'a str) -> impl Iterator<Item = &'a Value> {
    v.pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(move |b| text(b, "type") == Some(kind))
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// What a tool result says: its content, a string or text blocks.
fn result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| text(p, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Whether a shell command opens a pull request: runs `gh pr create`, or
/// its alias `gh pr new`.
fn opens_pr(command: &str) -> bool {
    let words: Vec<&str> = command
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    words
        .windows(3)
        .any(|w| w[0] == "gh" && w[1] == "pr" && matches!(w[2], "create" | "new"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str, name: &str, input: Value) -> Value {
        json!({"type": "assistant", "parent_tool_use_id": null, "message": {"id": "m1", "role": "assistant",
               "content": [{"type": "text", "text": "Next"}, {"type": "tool_use", "id": id, "name": name, "input": input}]}})
    }

    fn result(id: &str) -> Value {
        json!({"type": "user", "parent_tool_use_id": null, "message": {"role": "user",
               "content": [{"type": "tool_result", "tool_use_id": id, "content": "ok", "is_error": false}]}})
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn tool_results_that_can_change_the_repositories_call_for_one() {
        let t0 = Instant::now();
        let mut r = Refresh::default();
        r.observe(&call("t1", "Read", json!({"file_path": "a.rs"})), t0);
        r.observe(&result("t1"), t0);
        r.observe(&call("t2", "Grep", json!({"pattern": "x"})), t0);
        r.observe(&result("t2"), t0);
        assert!(!r.due(t0 + secs(60)));

        // A call is done once its result comes.
        r.observe(&call("t3", "Edit", json!({"file_path": "a.rs"})), t0);
        assert!(!r.due(t0 + secs(60)));
        r.observe(&result("t3"), t0);
        assert!(!r.due(t0 + secs(1)));
        assert!(r.due(t0 + QUIET));
        assert!(!r.start(t0 + QUIET).ran);
        assert!(!r.due(t0 + secs(60)));

        // A subagent's, a background command's end, and the others.
        let mut sub = call("t4", "Bash", json!({"command": "git commit -m x"}));
        sub["parent_tool_use_id"] = json!("toolu_agent");
        r.observe(&sub, t0);
        r.observe(&result("t4"), t0 + secs(20));
        assert!(r.due(t0 + secs(22)));
        r.start(t0 + secs(22));
        r.observe(
            &json!({"type": "system", "subtype": "task_notification", "task_id": "b1", "status": "completed"}),
            t0 + secs(40),
        );
        assert!(r.due(t0 + secs(42)));
        r.start(t0 + secs(42));
        let mut at = t0 + secs(60);
        for (id, tool) in [("w", "Write"), ("m", "MultiEdit"), ("n", "NotebookEdit")] {
            r.observe(&call(id, tool, json!({})), at);
            r.observe(&result(id), at);
            assert!(r.due(at + QUIET), "{tool}");
            r.start(at + QUIET);
            at += secs(20);
        }
    }

    #[test]
    fn a_burst_costs_one_refresh_and_refreshes_come_at_most_every_ten_seconds() {
        let t0 = Instant::now();
        let mut r = Refresh::default();
        // Results a second apart: the refresh waits for a quiet moment…
        for i in 0..5 {
            let id = format!("t{i}");
            r.observe(&call(&id, "Edit", json!({})), t0);
            r.observe(&result(&id), t0 + secs(i));
            assert!(!r.due(t0 + secs(i) + Duration::from_millis(1900)));
        }
        assert!(r.due(t0 + secs(4) + QUIET));
        r.start(t0 + secs(6));
        // …and comes no sooner than ten seconds after the last one.
        r.observe(&call("t5", "Bash", json!({})), t0);
        r.observe(&result("t5"), t0 + secs(7));
        assert!(!r.due(t0 + secs(15)));
        assert!(r.due(t0 + secs(16)));
        r.start(t0 + secs(16));

        // Results that keep coming get one about every ten seconds.
        let mut refreshed = vec![];
        for i in 17..50 {
            let id = format!("u{i}");
            r.observe(&call(&id, "Edit", json!({})), t0);
            r.observe(&result(&id), t0 + secs(i));
            if r.due(t0 + secs(i)) {
                refreshed.push(i);
                r.start(t0 + secs(i));
            }
        }
        assert_eq!(refreshed, [27, 38, 49]);
    }

    #[test]
    fn gh_pr_create_is_news_for_the_next_refresh() {
        let t0 = Instant::now();
        let mut r = Refresh::default();
        let create = json!({"command": "git push -u origin HEAD && gh pr create --fill"});
        r.observe(&call("t1", "Bash", create), t0);
        // Not before it ran.
        assert!(!r.start(t0).ran);
        r.observe(&result("t1"), t0);
        assert!(r.start(t0 + secs(10)).ran);
        assert_eq!(r.start(t0 + secs(20)), Opened::default());
        // Results that come twice, or for calls never seen, count once.
        r.observe(&result("t1"), t0 + secs(30));
        assert!(!r.due(t0 + secs(60)));
    }

    #[test]
    fn the_next_refresh_follows_the_pull_requests_gh_pr_create_printed() {
        let t0 = Instant::now();
        let mut r = Refresh::default();
        let printed = |id: &str, content: Value| {
            json!({"type": "user", "parent_tool_use_id": null, "message": {"role": "user",
                   "content": [{"type": "tool_result", "tool_use_id": id, "content": content, "is_error": false}]}})
        };
        // One from a worktree, and one in another repository, whose
        // output comes as text blocks.
        let worktree =
            "cd ~/second && git push -u origin fctl/1-second && gh pr create --head fctl/1-second";
        r.observe(&call("t1", "Bash", json!({"command": worktree})), t0);
        r.observe(
            &printed("t1", json!("https://github.com/acme/widgets/pull/2\n")),
            t0,
        );
        r.observe(
            &call(
                "t2",
                "Bash",
                json!({"command": "cd ~/repos/acme/lib && gh pr create --fill"}),
            ),
            t0,
        );
        r.observe(
            &printed(
                "t2",
                json!([{"type": "text", "text": "https://github.com/acme/lib/pull/3"}]),
            ),
            t0,
        );
        // Other commands' output names pull requests too: not theirs.
        r.observe(
            &call("t3", "Bash", json!({"command": "gh pr view 1 --json url"})),
            t0,
        );
        r.observe(
            &printed("t3", json!("https://github.com/acme/widgets/pull/1")),
            t0,
        );
        let opened = r.start(t0 + QUIET);
        let urls: Vec<&str> = opened
            .pull_requests
            .iter()
            .map(|p| p.url.as_str())
            .collect();
        assert!(opened.ran);
        assert_eq!(
            urls,
            [
                "https://github.com/acme/widgets/pull/2",
                "https://github.com/acme/lib/pull/3"
            ]
        );
        assert!(r.start(t0 + secs(20)).pull_requests.is_empty());
    }

    #[test]
    fn tells_commands_that_open_pull_requests() {
        assert!(opens_pr("gh pr create --title x --body y"));
        assert!(opens_pr("cd ~/repos/acme/lib && gh  pr new --fill"));
        assert!(opens_pr("url=$(gh pr create --fill)"));
        assert!(!opens_pr("gh pr view 12 --json url"));
        assert!(!opens_pr("gh pr checks 12 --watch"));
        assert!(!opens_pr("git commit -m 'create gh pr'"));
    }
}
