//! Claude's background work, as Claude Code reports it in stream-json: a
//! shell command with `run_in_background`, a subagent, a Monitor. Each task
//! starts with `system/task_started` and ends, for any reason, with one
//! `system/task_notification` (from 2.1.101, the oldest Claude Code fctl
//! runs, to 2.1.302 at least). A session whose agent ended its turn to wait
//! for them isn't idle until they end.
//!
//! When background work ends, Claude Code can start a turn on its own to
//! hand Claude the result. `system/session_state_changed` says so when
//! [`SESSION_STATE_ENV`] is set; without it, the first message of the
//! conversation after a `result` does.

use fctl_proto::runner::SessionSpec;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Makes Claude Code report each turn starting and ending
/// (`session_state_changed`: `running`, `requires_action`, `idle`).
pub const SESSION_STATE_ENV: &str = "CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS";

/// How long background work may keep a session up after its last turn,
/// unless the project's idle timeout is longer, so that a forgotten dev
/// server or `tail -f` doesn't hold its runner's slot for ever.
pub const LIMIT: Duration = Duration::from_secs(60 * 60);

/// How long a session waits for a message, with nothing running, before
/// it stops: its project's idle timeout, a minute at least.
pub fn idle_timeout(spec: &SessionSpec) -> Duration {
    Duration::from_secs(spec.idle_timeout_secs.max(60))
}

/// How long background work may keep a session up after its last turn,
/// with this idle timeout: [`LIMIT`], or the idle timeout when it is
/// longer, so that background work never stops a session sooner than
/// waiting would.
pub fn limit(idle_timeout: Duration) -> Duration {
    LIMIT.max(idle_timeout)
}

/// A task Claude Code started and hasn't reported ending.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    /// What it does, in Claude Code's words: the command's description, or
    /// the subagent's.
    #[serde(default)]
    pub description: String,
    /// Claude Code's `task_type`: `local_bash`, `local_agent`, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl Task {
    /// The task for a person or for Claude: `Start the dev stack (shell
    /// command)`.
    pub fn label(&self) -> String {
        let description: String = self
            .description
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let kind = self.kind.as_deref().and_then(kind_label);
        match (description.is_empty(), kind) {
            (false, Some(kind)) => {
                format!("{} ({kind})", crate::claude::truncate(&description, 80))
            }
            (false, None) => crate::claude::truncate(&description, 80),
            (true, Some(kind)) => format!("a {kind}"),
            (true, None) => "a task without a description".to_owned(),
        }
    }
}

fn kind_label(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "local_bash" => "shell command",
        "local_agent" => "subagent",
        "remote_agent" => "remote agent",
        "in_process_teammate" => "teammate",
        "local_workflow" => "workflow",
        "mcp_task" => "MCP task",
        k if k.starts_with("monitor") => "monitor",
        _ => return None,
    })
}

/// The tasks that still run, in the order they started.
#[derive(Debug, Default)]
pub struct Tasks(Vec<Task>);

impl Tasks {
    /// Follows a Claude message: whether it started or ended a task.
    pub fn observe(&mut self, v: &Value) -> bool {
        if v.get("type").and_then(Value::as_str) != Some("system") {
            return false;
        }
        let Some(id) = v.get("task_id").and_then(Value::as_str) else {
            return false;
        };
        match v.get("subtype").and_then(Value::as_str) {
            // Claude Code's own housekeeping (`ambient`, like consolidating
            // its memory) isn't the agent's work.
            Some("task_started")
                if v.get("ambient").and_then(Value::as_bool) != Some(true)
                    && !self.0.iter().any(|t| t.id == id) =>
            {
                let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
                self.0.push(Task {
                    id: id.to_owned(),
                    description: text("description").unwrap_or_default(),
                    kind: text("task_type"),
                });
                true
            }
            // Tasks of an earlier Claude Code process end here too, once
            // it resumes: there is nothing to forget about them.
            Some("task_notification") => {
                let before = self.0.len();
                self.0.retain(|t| t.id != id);
                self.0.len() != before
            }
            _ => false,
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Forgets them all, for a Claude Code that no longer runs them.
    pub fn take(&mut self) -> Vec<Task> {
        std::mem::take(&mut self.0)
    }
}

/// What Claude Code says about its turns in `session_state_changed`:
/// `running` or `requires_action` while one is under way, `idle` after.
pub fn session_state(v: &Value) -> Option<&str> {
    if v.get("type").and_then(Value::as_str) != Some("system")
        || v.get("subtype").and_then(Value::as_str) != Some("session_state_changed")
    {
        return None;
    }
    Some(v.get("state").and_then(Value::as_str).unwrap_or_default())
}

/// Whether a message belongs to a turn of the conversation: Claude streams,
/// writes or gets a tool's result in it. A subagent's messages name the
/// tool use that started it in `parent_tool_use_id`, and arrive while the
/// conversation itself may wait.
pub fn in_turn(v: &Value) -> bool {
    matches!(
        v.get("type").and_then(Value::as_str),
        Some("stream_event" | "assistant" | "user")
    ) && v.get("parent_tool_use_id").is_none_or(Value::is_null)
}

/// Why an idle session waits: for its background tasks while any run,
/// then for a message.
pub fn waiting_reason(running: usize) -> String {
    match running {
        0 => "Waiting for a message".to_owned(),
        1 => "Waiting for a background task".to_owned(),
        n => format!("Waiting for {n} background tasks"),
    }
}

/// What the transcript says when the session stops while tasks run.
pub fn stopped_notice(tasks: &[Task]) -> String {
    let what = match tasks.len() {
        1 => "a background task that was still running".to_owned(),
        n => format!("{n} background tasks that were still running"),
    };
    format!("Stopped {what}: {}", list(tasks))
}

/// The status of a session that background work kept up for `limit`.
pub fn limit_reason(limit: Duration) -> String {
    format!(
        "Stopped after {} waiting for background tasks",
        duration(limit)
    )
}

/// At most ten tasks, then how many more.
fn list(tasks: &[Task]) -> String {
    let mut labels: Vec<String> = tasks.iter().take(10).map(Task::label).collect();
    if tasks.len() > 10 {
        labels.push(format!("and {} more", tasks.len() - 10));
    }
    labels.join(", ")
}

/// `a minute`, `10 minutes`, `an hour`, `90 minutes`, `2 hours`.
pub fn duration(d: Duration) -> String {
    let minutes = ((d.as_secs() + 30) / 60).max(1);
    match (minutes % 60, minutes / 60) {
        (0, 1) => "an hour".to_owned(),
        (0, hours) => format!("{hours} hours"),
        _ if minutes == 1 => "a minute".to_owned(),
        _ => format!("{minutes} minutes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn started(id: &str, description: &str, kind: &str) -> Value {
        json!({"type": "system", "subtype": "task_started", "task_id": id, "tool_use_id": "toolu_1",
               "description": description, "task_type": kind, "is_backgrounded": true,
               "session_id": "s", "uuid": "u"})
    }

    fn ended(id: &str, status: &str) -> Value {
        json!({"type": "system", "subtype": "task_notification", "task_id": id, "tool_use_id": "toolu_1",
               "status": status, "output_file": "", "summary": "", "session_id": "s", "uuid": "u"})
    }

    #[test]
    fn follows_tasks_from_start_to_end() {
        let mut tasks = Tasks::default();
        assert!(tasks.observe(&started(
            "a1",
            "Review the images branch diff",
            "local_agent"
        )));
        assert!(tasks.observe(&started("b1", "Start the dev stack", "local_bash")));
        // Reported twice, counted once.
        assert!(!tasks.observe(&started("b1", "Start the dev stack", "local_bash")));
        assert_eq!(tasks.len(), 2);
        assert!(tasks.observe(&ended("a1", "completed")));
        assert!(!tasks.observe(&ended("a1", "completed")));
        assert_eq!(tasks.len(), 1);
        // A task of an earlier process, which Claude Code reports once it resumes.
        assert!(!tasks.observe(&ended("gone", "stopped")));
        assert!(tasks.observe(&ended("b1", "stopped")));
        assert!(tasks.is_empty());
    }

    #[test]
    fn leaves_out_claude_codes_housekeeping_and_other_messages() {
        let mut tasks = Tasks::default();
        let mut dream = started("d1", "dreaming", "dream");
        dream["ambient"] = json!(true);
        assert!(!tasks.observe(&dream));
        assert!(!tasks.observe(&json!({"type": "system", "subtype": "status", "status": null})));
        assert!(!tasks.observe(&json!({"type": "system", "subtype": "task_started"})));
        assert!(!tasks.observe(&json!({"type": "user", "task_id": "x"})));
        assert!(tasks.is_empty());
    }

    #[test]
    fn names_tasks_for_people() {
        let mut tasks = Tasks::default();
        tasks.observe(&started(
            "a1",
            "Review the images\nbranch diff",
            "local_agent",
        ));
        tasks.observe(&started("b1", "Start the dev stack", "local_bash"));
        tasks.observe(&started("m1", "Watch the deploy", "monitor_mcp"));
        tasks.observe(&started("x1", "", "local_bash"));
        tasks.observe(&started("y1", "Something new", "future_kind"));
        let all = tasks.take();
        assert!(tasks.is_empty());
        assert_eq!(
            stopped_notice(&all),
            "Stopped 5 background tasks that were still running: Review the images branch diff (subagent), Start the dev stack (shell command), Watch the deploy (monitor), a shell command, Something new"
        );
        assert_eq!(
            stopped_notice(&all[1..2]),
            "Stopped a background task that was still running: Start the dev stack (shell command)"
        );
        let many: Vec<Task> = (0..12)
            .map(|i| Task {
                id: format!("t{i}"),
                description: format!("Task {i}"),
                kind: None,
            })
            .collect();
        assert!(stopped_notice(&many).ends_with("Task 9, and 2 more"));
        let long = Task {
            id: "l".into(),
            description: "x".repeat(200),
            kind: None,
        };
        assert_eq!(long.label().chars().count(), 81);
    }

    #[test]
    fn says_why_the_session_waits() {
        assert_eq!(waiting_reason(0), "Waiting for a message");
        assert_eq!(waiting_reason(1), "Waiting for a background task");
        assert_eq!(waiting_reason(2), "Waiting for 2 background tasks");
    }

    #[test]
    fn background_work_has_an_hour_or_the_idle_timeout() {
        let minutes = |m: u64| Duration::from_secs(m * 60);
        assert_eq!(limit(minutes(10)), LIMIT);
        assert_eq!(limit(minutes(120)), minutes(120));
        assert_eq!(
            limit_reason(limit(minutes(10))),
            "Stopped after an hour waiting for background tasks"
        );
        assert_eq!(duration(minutes(1)), "a minute");
        assert_eq!(duration(Duration::from_secs(20)), "a minute");
        assert_eq!(duration(minutes(10)), "10 minutes");
        assert_eq!(duration(minutes(90)), "90 minutes");
        assert_eq!(duration(minutes(120)), "2 hours");
    }

    #[test]
    fn tells_turns_from_messages() {
        let state =
            |s: &str| json!({"type": "system", "subtype": "session_state_changed", "state": s});
        assert_eq!(session_state(&state("running")), Some("running"));
        assert_eq!(session_state(&state("idle")), Some("idle"));
        assert_eq!(session_state(&ended("a1", "completed")), None);

        assert!(in_turn(
            &json!({"type": "stream_event", "parent_tool_use_id": null})
        ));
        assert!(in_turn(&json!({"type": "assistant", "message": {}})));
        assert!(in_turn(
            &json!({"type": "user", "parent_tool_use_id": null})
        ));
        // A background subagent's work, and messages around turns.
        assert!(!in_turn(
            &json!({"type": "assistant", "parent_tool_use_id": "toolu_1"})
        ));
        assert!(!in_turn(&ended("a1", "completed")));
        assert!(!in_turn(&json!({"type": "result"})));
        assert!(!in_turn(&json!({"type": "rate_limit_event"})));
    }
}
