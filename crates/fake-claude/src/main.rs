//! A scripted stand-in for Claude Code's stream-json mode.
//!
//! It speaks the same protocol as `claude -p --input-format stream-json
//! --output-format stream-json --include-partial-messages`: control requests
//! (`initialize`, `interrupt`, `remote_control`, `get_settings`), user turns
//! on stdin, and `system`, `stream_event`, `assistant`, `user`, `result` and
//! `rate_limit_event` messages on stdout. A user turn that arrives during
//! another waits for it, and each `result` says how many turns still wait
//! (`queued_turn_count`), as Claude Code's does. With
//! `CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS` set, it reports each turn
//! starting and ending (`system/session_state_changed`), as Claude Code
//! does.
//!
//! What it does depends on the words in a user turn:
//!
//! | Word | Behavior |
//! |---|---|
//! | (anything) | reports plan usage, streams a short answer, runs a fake `Bash` tool, answers again |
//! | `write <name>` | creates `<name>` in the working directory and commits it, ending the message as Claude Code does: with `attribution.commit` from `~/.claude/settings.json`, or Claude's trailer when that is unset |
//! | `push` | pushes the branch to `origin` with `git push`; a repository without one (the dev instance's projects have no repository) gets a bare one in `~/.fake-github` to stand in for GitHub |
//! | `watch <seconds>` | watches the pull request's checks in one tool call that lasts that many seconds (at most ten minutes), as agents wait for CI before they end their turn |
//! | `rename <title>` | names the session `<title>` with `fctl edit`, as agents do |
//! | `screenshot` | draws a small PNG and publishes it with `fctl publish`, as agents do for pull requests |
//! | `remember <text>` | adds `<text>` (to the end of its line) to the project's memory with `fctl memory add`, as agents do |
//! | `recall` | lists the project's memory with `fctl memory` |
//! | `retro` | records a retrospective of the turn's work with `fctl retro`, as agents do when they finish a piece of work |
//! | `slow` | streams for about ten seconds (try interrupting) |
//! | `background <seconds>` | starts a shell command in the background that ends after that many seconds (`system/task_started`), and ends its turn; when the command ends (`system/task_notification`), takes its result in a turn of its own, as Claude Code does |
//! | `delegate` | delegates a look around the working directory to a subagent with the `Agent` tool, as Claude Code does: the subagent's messages name the call (`parent_tool_use_id`), it runs two commands about a second each, and the call returns its report |
//! | `delegate … in the background` | the same subagent in the background (`run_in_background`): its call returns at once and the turn ends; it takes a step every two seconds, and when it ends, a turn of its own takes its report |
//! | `limit` | reports a five-hour usage limit that resets in 20 seconds, unless the turn also says `reset`, as fctl-agent's message that continues the session does |
//! | `overage` | reports paid extra usage starting |
//! | `refresh` | asks its host for a fresh OAuth token |
//! | `crash` | exits with status 3 |
//! | `effort <level>` | works at that effort level from then on (`low` … `max`, or `none` as a model that takes none), as Claude Code does after `/effort <level>`, which it takes too; `get_settings` says so |
//! | (attached files) | says how many images the message holds and checks that each listed file is on disk |
//!
//! Plan usage comes as Claude Code reports it after a response
//! (`unifiedWindows` in a `rate_limit_event`): the five-hour and weekly
//! windows follow the clock, so every session reports the same numbers.
//! Each API call's tokens come as Claude Code's do: its input as its
//! response starts (`message_start`, when it streams) and with each of its
//! assistant messages, its output in the `message_delta` that ends it, and
//! the process's totals with each `result` (`modelUsage`).
//!
//! Environment: `FAKE_CLAUDE_API_KEY_SOURCE` overrides the reported
//! `apiKeySource` (default `none`); `FAKE_CLAUDE_DELAY_MS` sets the delay
//! between streamed chunks (default 25). `system/init` lists a claude.ai
//! connector (`claude.ai Fake Docs`), as Claude Code does on a full login,
//! unless `ENABLE_CLAUDEAI_MCP_SERVERS` is `false`, `0`, `no` or `off`.
//! `get_settings` answers with the model and the effort level it works at
//! (`applied`): `--effort`, else `medium`.

use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
/// User turns read from stdin that haven't started yet: each `result`
/// reports them as `queued_turn_count`, and fctl-agent keeps the session
/// running while there are any.
static QUEUED_TURNS: AtomicUsize = AtomicUsize::new(0);

fn out(v: &Value) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{v}");
    let _ = stdout.flush();
}

fn uuid() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let h = format!("{:032x}", n ^ 0x5a5a_1234_9876_fedc_0123_4567_89ab_cdef);
    format!(
        "{}-{}-4{}-a{}-{}",
        &h[0..8],
        &h[8..12],
        &h[13..16],
        &h[17..20],
        &h[20..32]
    )
}

/// A short id for a message, tool use, task or request, as Claude's have:
/// the end of a [`uuid`], which is what changes from one to the next, and a
/// count, so that no two are the same.
fn short_id(len: usize) -> String {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let u = uuid();
    format!(
        "{}{:x}",
        &u[u.len() - len..],
        COUNT.fetch_add(1, Ordering::SeqCst)
    )
}

struct Session {
    id: String,
    model: String,
    /// The effort level it works at; `None` as a model that takes none.
    effort: Option<String>,
    turns: u64,
    /// What its API calls used, for `modelUsage`.
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    delay: Duration,
    /// Shell commands and subagents working in the background, in the
    /// order they end.
    background: Vec<Background>,
    /// Whether to report turns starting and ending
    /// (`CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS`), and whether one runs.
    reports_turns: bool,
    running: bool,
}

/// A shell command Claude Code runs with `run_in_background`, or a
/// subagent it runs in the background.
struct Background {
    id: String,
    tool_use_id: String,
    description: String,
    ends: Instant,
    /// A subagent, with the steps it still takes, each when it is due.
    subagent: Option<(Subagent, Vec<(Instant, Step)>)>,
}

/// A subagent that Claude Code's `Agent` tool started.
struct Subagent {
    /// The `Agent` call, which its messages name (`parent_tool_use_id`).
    tool_use_id: String,
    task_id: String,
    description: String,
    started: Instant,
    tool_uses: u64,
}

impl Subagent {
    /// What it has used so far, as Claude Code reports it.
    fn usage(&self) -> Value {
        json!({"duration_ms": self.started.elapsed().as_millis() as u64, "tool_uses": self.tool_uses, "total_tokens": 1000 * (self.tool_uses + 1)})
    }
}

/// What fake-claude's subagents do: a command, or a few words.
#[derive(Clone, Copy)]
enum Step {
    Bash {
        command: &'static str,
        description: &'static str,
    },
    Text(&'static str),
}

/// A look around the working directory, as fake-claude delegates it.
const SUBAGENT_STEPS: [Step; 3] = [
    Step::Bash {
        command: "ls -la",
        description: "List the files",
    },
    Step::Text("Now the recent commits."),
    Step::Bash {
        command: "git log --oneline -5",
        description: "Read the recent commits",
    },
];
const SUBAGENT_DESCRIPTION: &str = "Look around the working directory";
const SUBAGENT_PROMPT: &str = "List the files in the working directory and read its recent commits. Report in a sentence whether anything needs attention.";
const SUBAGENT_REPORT: &str =
    "The working directory and its recent commits look as expected: nothing needs attention.";

impl Session {
    fn sleep(&self) {
        std::thread::sleep(self.delay);
    }

    /// `system/session_state_changed`, when the turns' state changes.
    fn report_turn(&mut self, running: bool) {
        if self.reports_turns && self.running != running {
            let state = if running { "running" } else { "idle" };
            out(
                &json!({"type": "system", "subtype": "session_state_changed", "state": state, "session_id": self.id, "uuid": uuid()}),
            );
        }
        self.running = running;
    }

    /// Starts a shell command in the background that ends after `secs`, as
    /// Claude Code's Bash tool does with `run_in_background`.
    fn start_background(&mut self, secs: u64) {
        let id = format!("b{}", short_id(8));
        let tool_use_id = format!("toolu_{}", short_id(12));
        let description = format!("Wait {secs} seconds in the background");
        let msg_id = format!("msg_{}", short_id(8));
        let usage = self.call_started();
        out(
            &json!({"type": "assistant", "message": {"id": msg_id, "type": "message", "role": "assistant", "model": self.model, "content": [{"type": "tool_use", "id": tool_use_id, "name": "Bash", "input": {"command": format!("sleep {secs}"), "description": description, "run_in_background": true}}], "stop_reason": "tool_use", "usage": usage}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        self.call_ended(&msg_id, "tool_use", 30);
        out(
            &json!({"type": "system", "subtype": "task_started", "task_id": id, "tool_use_id": tool_use_id, "description": description, "task_type": "local_bash", "is_backgrounded": true, "session_id": self.id, "uuid": uuid()}),
        );
        out(
            &json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_use_id, "content": format!("Command running in background with ID: {id}."), "is_error": false}]}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        self.background.push(Background {
            id,
            tool_use_id,
            description,
            ends: Instant::now() + Duration::from_secs(secs),
            subagent: None,
        });
        self.background.sort_by_key(|b| b.ends);
    }

    /// When background work next takes a step or ends.
    fn next_due(&self) -> Option<Instant> {
        self.background
            .iter()
            .flat_map(|b| {
                let step = b.subagent.as_ref().and_then(|(_, steps)| steps.first());
                step.map(|(at, _)| *at).into_iter().chain([b.ends])
            })
            .min()
    }

    /// Subagents in the background take the steps that are due. The work
    /// that has ended gets its notification, and a turn of fake-claude's
    /// own takes its results.
    fn finish_background(&mut self) {
        let now = Instant::now();
        let mut work = std::mem::take(&mut self.background);
        for b in &mut work {
            if let Some((agent, steps)) = &mut b.subagent {
                let n = steps.iter().take_while(|(at, _)| *at <= now).count();
                for (_, step) in steps.drain(..n) {
                    self.subagent_step(agent, step);
                }
            }
        }
        self.background = work;
        let ended: Vec<Background> = match self.background.iter().position(|b| b.ends > now) {
            Some(n) => self.background.drain(..n).collect(),
            None => std::mem::take(&mut self.background),
        };
        if ended.is_empty() {
            return;
        }
        for b in &ended {
            match &b.subagent {
                Some((agent, _)) => self.end_subagent(agent, "completed", true),
                None => out(
                    &json!({"type": "system", "subtype": "task_notification", "task_id": b.id, "tool_use_id": b.tool_use_id, "status": "completed", "output_file": "", "summary": format!("Background command \"{}\" completed (exit code 0)", b.description), "session_id": self.id, "uuid": uuid()}),
                ),
            }
        }
        INTERRUPTED.store(false, Ordering::SeqCst);
        self.report_turn(true);
        let started = Instant::now();
        let names: Vec<&str> = ended.iter().map(|b| b.description.as_str()).collect();
        let what = if ended.iter().all(|b| b.subagent.is_some()) {
            "The subagent"
        } else {
            "The background command"
        };
        let text = format!(
            "{what} finished: {}. Picking up where I left off.",
            names.join(", ")
        );
        self.stream_text(&text);
        self.result(started, &text, "success");
        if QUEUED_TURNS.load(Ordering::SeqCst) == 0 {
            self.report_turn(false);
        }
    }

    /// An API call's usage as its response starts: Claude Code repeats it
    /// in each of the call's assistant messages, with the output only
    /// begun.
    fn call_started(&mut self) -> Value {
        self.input_tokens += 12;
        self.cache_read_tokens += 1000;
        self.output_tokens += 1;
        json!({"input_tokens": 12, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0, "output_tokens": 1})
    }

    /// The end of an API call's response: its output tokens in all
    /// (`message_delta`), then `message_stop`.
    fn call_ended(&mut self, msg_id: &str, stop_reason: &str, output: u64) {
        let output = output.max(1);
        self.output_tokens += output - 1;
        out(
            &json!({"type": "stream_event", "event": {"type": "message_delta", "delta": {"stop_reason": stop_reason, "stop_sequence": null}, "usage": {"input_tokens": 12, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0, "output_tokens": output}}, "api_message_id": msg_id, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        out(
            &json!({"type": "stream_event", "event": {"type": "message_stop"}, "api_message_id": msg_id, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
    }

    fn stream_text(&mut self, text: &str) -> String {
        let msg_id = format!("msg_{}", short_id(8));
        let usage = self.call_started();
        out(
            &json!({"type": "stream_event", "event": {"type": "message_start", "message": {"id": msg_id, "role": "assistant", "model": self.model, "content": [], "usage": usage}}, "api_message_id": msg_id, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        out(
            &json!({"type": "stream_event", "event": {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        let mut written = String::new();
        for word in text.split_inclusive(' ') {
            if INTERRUPTED.load(Ordering::SeqCst) {
                break;
            }
            written.push_str(word);
            out(
                &json!({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": word}}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
            );
            self.sleep();
        }
        out(
            &json!({"type": "stream_event", "event": {"type": "content_block_stop", "index": 0}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        out(
            &json!({"type": "assistant", "message": {"id": msg_id, "type": "message", "role": "assistant", "model": self.model, "content": [{"type": "text", "text": written}], "stop_reason": null, "usage": usage}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        self.call_ended(&msg_id, "end_turn", written.len() as u64 / 4);
        msg_id
    }

    fn tool(&mut self, name: &str, input: Value, output: &str, is_error: bool) {
        let tool_id = format!("toolu_{}", short_id(12));
        let msg_id = format!("msg_{}", short_id(8));
        let usage = self.call_started();
        out(
            &json!({"type": "assistant", "message": {"id": msg_id, "type": "message", "role": "assistant", "model": self.model, "content": [{"type": "tool_use", "id": tool_id, "name": name, "input": input}], "stop_reason": "tool_use", "usage": usage}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        self.call_ended(&msg_id, "tool_use", 30);
        for i in 0..2 {
            out(
                &json!({"type": "tool_progress", "tool_use_id": tool_id, "tool_name": name, "parent_tool_use_id": null, "elapsed_time_seconds": i, "session_id": self.id, "uuid": uuid()}),
            );
            self.sleep();
        }
        out(
            &json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_id, "content": output, "is_error": is_error}]}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
    }

    /// Runs the sandbox's `fctl` with `args` in a `Bash` tool call, as
    /// agents do.
    fn fctl(&mut self, args: &[&str], description: &str) {
        let (output, failed) = match std::process::Command::new("fctl").args(args).output() {
            Ok(o) => (
                String::from_utf8_lossy(&o.stdout).into_owned()
                    + &String::from_utf8_lossy(&o.stderr),
                !o.status.success(),
            ),
            Err(e) => (format!("fctl: {e}"), true),
        };
        let command = std::iter::once("fctl".to_owned())
            .chain(args.iter().map(|a| {
                if a.contains(' ') {
                    format!("{a:?}")
                } else {
                    (*a).to_owned()
                }
            }))
            .collect::<Vec<_>>()
            .join(" ");
        self.tool(
            "Bash",
            json!({"command": command, "description": description}),
            &output,
            failed,
        );
    }

    /// Pushes the branch, as agents do before they open a pull request:
    /// without an `origin`, a bare repository in `~/.fake-github` stands in
    /// for GitHub.
    fn push(&mut self) {
        let script = r#"git remote get-url origin >/dev/null 2>&1 || {
  remote="$HOME/.fake-github/$(basename "$PWD").git"
  git init -q --bare "$remote" && git remote add origin "$remote"
} && git push -u origin HEAD"#;
        let (output, failed) = sh(script);
        self.tool(
            "Bash",
            json!({"command": "git push -u origin HEAD", "description": "Push the branch"}),
            &output,
            failed,
        );
    }

    /// Watches the pull request's checks for `secs` seconds in one tool
    /// call, as agents do before they end their turn. Returns whether it
    /// was interrupted.
    fn watch(&mut self, secs: u64) -> bool {
        let tool_id = format!("toolu_{}", short_id(12));
        let msg_id = format!("msg_{}", short_id(8));
        let usage = self.call_started();
        out(
            &json!({"type": "assistant", "message": {"id": msg_id, "type": "message", "role": "assistant", "model": self.model, "content": [{"type": "tool_use", "id": tool_id, "name": "Bash", "input": {"command": "gh pr checks --watch", "description": "Watch the checks"}}], "stop_reason": "tool_use", "usage": usage}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        self.call_ended(&msg_id, "tool_use", 30);
        let started = Instant::now();
        let length = Duration::from_secs(secs);
        let mut reported = 0;
        let interrupted = loop {
            if INTERRUPTED.load(Ordering::SeqCst) {
                break true;
            }
            let elapsed = started.elapsed();
            if elapsed >= length {
                break false;
            }
            if elapsed.as_secs() >= reported {
                out(
                    &json!({"type": "tool_progress", "tool_use_id": tool_id, "tool_name": "Bash", "parent_tool_use_id": null, "elapsed_time_seconds": elapsed.as_secs(), "session_id": self.id, "uuid": uuid()}),
                );
                reported = elapsed.as_secs() + 1;
            }
            std::thread::sleep((length - elapsed).min(Duration::from_millis(200)));
        };
        let output = if interrupted {
            "Interrupted"
        } else {
            "All checks were successful"
        };
        out(
            &json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_id, "content": output, "is_error": interrupted}]}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        interrupted
    }

    /// Starts a subagent with the `Agent` tool, as Claude Code does: the
    /// call, `system/task_started`, and the prompt the subagent gets, which
    /// names the call, as all its messages do (`parent_tool_use_id`).
    fn start_subagent(&mut self, background: bool) -> Subagent {
        let agent = Subagent {
            tool_use_id: format!("toolu_{}", short_id(12)),
            task_id: format!("a{}", short_id(16)),
            description: SUBAGENT_DESCRIPTION.to_owned(),
            started: Instant::now(),
            tool_uses: 0,
        };
        let mut input = json!({"description": SUBAGENT_DESCRIPTION, "prompt": SUBAGENT_PROMPT, "subagent_type": "general-purpose"});
        if background {
            input["run_in_background"] = json!(true);
        }
        let msg_id = format!("msg_{}", short_id(8));
        let usage = self.call_started();
        out(
            &json!({"type": "assistant", "message": {"id": msg_id, "type": "message", "role": "assistant", "model": self.model, "content": [{"type": "tool_use", "id": agent.tool_use_id, "name": "Agent", "input": input}], "stop_reason": "tool_use", "usage": usage}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        self.call_ended(&msg_id, "tool_use", 60);
        out(
            &json!({"type": "system", "subtype": "task_started", "task_id": agent.task_id, "tool_use_id": agent.tool_use_id, "description": SUBAGENT_DESCRIPTION, "prompt": SUBAGENT_PROMPT, "subagent_type": "general-purpose", "task_type": "local_agent", "is_backgrounded": background, "session_id": self.id, "uuid": uuid()}),
        );
        out(
            &json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": SUBAGENT_PROMPT}]}, "parent_tool_use_id": agent.tool_use_id, "session_id": self.id, "uuid": uuid()}),
        );
        agent
    }

    /// One of a subagent's assistant messages.
    fn subagent_says(&mut self, agent: &Subagent, content: Value) {
        let msg_id = format!("msg_{}", short_id(8));
        let usage = self.call_started();
        out(
            &json!({"type": "assistant", "message": {"id": msg_id, "type": "message", "role": "assistant", "model": self.model, "content": content, "stop_reason": null, "usage": usage}, "parent_tool_use_id": agent.tool_use_id, "session_id": self.id, "uuid": uuid()}),
        );
    }

    /// A subagent's step: a few words, or a command it runs, which Claude
    /// Code reports first (`system/task_progress`).
    fn subagent_step(&mut self, agent: &mut Subagent, step: Step) {
        let (command, description) = match step {
            Step::Text(text) => {
                self.subagent_says(agent, json!([{"type": "text", "text": text}]));
                return;
            }
            Step::Bash {
                command,
                description,
            } => (command, description),
        };
        agent.tool_uses += 1;
        out(
            &json!({"type": "system", "subtype": "task_progress", "task_id": agent.task_id, "tool_use_id": agent.tool_use_id, "description": format!("Running {description}"), "last_tool_name": "Bash", "usage": agent.usage(), "session_id": self.id, "uuid": uuid()}),
        );
        let tool_id = format!("toolu_{}", short_id(12));
        self.subagent_says(
            agent,
            json!([{"type": "tool_use", "id": tool_id, "name": "Bash", "input": {"command": command, "description": description}}]),
        );
        // Slow enough to watch it work.
        std::thread::sleep(self.delay * 40);
        let (output, failed) = sh(command);
        out(
            &json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_id, "content": output, "is_error": failed}]}, "parent_tool_use_id": agent.tool_use_id, "session_id": self.id, "uuid": uuid()}),
        );
    }

    /// A subagent ends (`system/task_notification`). One in the background
    /// says its last words first; in the foreground, its call returns them.
    fn end_subagent(&mut self, agent: &Subagent, status: &str, background: bool) {
        let report = if status == "completed" {
            SUBAGENT_REPORT
        } else {
            ""
        };
        if background && !report.is_empty() {
            self.subagent_says(agent, json!([{"type": "text", "text": report}]));
        }
        out(
            &json!({"type": "system", "subtype": "task_notification", "task_id": agent.task_id, "tool_use_id": agent.tool_use_id, "status": status, "output_file": "", "summary": report, "usage": agent.usage(), "session_id": self.id, "uuid": uuid()}),
        );
    }

    /// Delegates a look around the working directory to a subagent, whose
    /// call waits for it and returns its report, as Claude Code's `Agent`
    /// tool does. Returns whether it was interrupted.
    fn delegate(&mut self) -> bool {
        let mut agent = self.start_subagent(false);
        for step in SUBAGENT_STEPS {
            if INTERRUPTED.load(Ordering::SeqCst) {
                break;
            }
            self.subagent_step(&mut agent, step);
        }
        let interrupted = INTERRUPTED.load(Ordering::SeqCst);
        self.end_subagent(
            &agent,
            if interrupted { "stopped" } else { "completed" },
            false,
        );
        let mut result = if interrupted {
            json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": agent.tool_use_id, "content": "[Request interrupted by user for tool use]", "is_error": true}]}})
        } else {
            // Claude Code's copy of the result (`tool_use_result`) has the
            // report alone; Claude gets notes about the subagent with it.
            let usage = agent.usage();
            let text = format!(
                "{SUBAGENT_REPORT}\nagentId: {} (use SendMessage with to: '{}' to continue this agent)\n<usage>tool_uses: {}\nduration_ms: {}</usage>",
                agent.task_id, agent.task_id, usage["tool_uses"], usage["duration_ms"]
            );
            json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": agent.tool_use_id, "content": [{"type": "text", "text": text}], "is_error": false}]},
                "tool_use_result": {"status": "completed", "agentId": agent.task_id, "agentType": "general-purpose", "content": [{"type": "text", "text": SUBAGENT_REPORT}], "totalDurationMs": usage["duration_ms"], "totalToolUseCount": agent.tool_uses}})
        };
        result["parent_tool_use_id"] = Value::Null;
        result["session_id"] = json!(self.id);
        result["uuid"] = json!(uuid());
        out(&result);
        interrupted
    }

    /// Delegates the same look around to a subagent in the background, as
    /// Claude Code does with `run_in_background`: its call returns at once,
    /// it takes a step every two seconds (at the usual delay), and when it
    /// ends, a turn of fake-claude's own takes its report.
    fn delegate_in_background(&mut self) {
        let agent = self.start_subagent(true);
        let launched = format!(
            "Async agent launched successfully.\nagentId: {} (internal ID - do not mention to user. Use SendMessage with to: '{}' to continue this agent.)\nThe agent is working in the background. You will be notified automatically when it completes.",
            agent.task_id, agent.task_id
        );
        out(
            &json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": agent.tool_use_id, "content": [{"type": "text", "text": launched}]}]}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid(),
                "tool_use_result": {"isAsync": true, "status": "async_launched", "agentId": agent.task_id, "description": SUBAGENT_DESCRIPTION, "prompt": SUBAGENT_PROMPT, "outputFile": ""}}),
        );
        let now = Instant::now();
        let every = self.delay * 80;
        let steps: Vec<(Instant, Step)> = SUBAGENT_STEPS
            .iter()
            .zip(1u32..)
            .map(|(step, n)| (now + every * n, *step))
            .collect();
        let ends = now + every * (steps.len() as u32 + 1);
        self.background.push(Background {
            id: agent.task_id.clone(),
            tool_use_id: agent.tool_use_id.clone(),
            description: agent.description.clone(),
            ends,
            subagent: Some((agent, steps)),
        });
        self.background.sort_by_key(|b| b.ends);
    }

    fn result(&mut self, started: Instant, text: &str, subtype: &str) {
        self.turns += 1;
        let ms = started.elapsed().as_millis() as u64;
        out(&json!({
            "type": "result", "subtype": subtype, "is_error": subtype != "success", "duration_ms": ms, "duration_api_ms": ms,
            "num_turns": 3, "result": text, "stop_reason": "end_turn", "total_cost_usd": 0.0, "session_id": self.id, "uuid": uuid(),
            "usage": {"input_tokens": 120, "output_tokens": 80, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0},
            "modelUsage": {self.model.clone(): {"inputTokens": self.input_tokens, "outputTokens": self.output_tokens, "cacheReadInputTokens": self.cache_read_tokens, "cacheCreationInputTokens": 0, "webSearchRequests": 0, "costUSD": 0.0, "contextWindow": 200000, "maxOutputTokens": 32000}},
            "permission_denials": [], "queued_turn_count": QUEUED_TURNS.load(Ordering::SeqCst)
        }));
    }

    /// Plan usage, as Claude Code reports it after a response: each window
    /// starts over on the clock (every five hours, every week since the
    /// epoch) and fills up as it goes.
    fn usage(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let window = |secs: u64, start: f64, end: f64| {
            let elapsed = (now % secs) as f64 / secs as f64;
            let used = start + (end - start) * elapsed;
            json!({"utilization": (used * 100.0).round() / 100.0, "resetsAt": now - now % secs + secs})
        };
        out(
            &json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "isUsingOverage": false, "unifiedWindows": {"five_hour": window(5 * 3600, 0.05, 0.85), "seven_day": window(7 * 86_400, 0.1, 0.7)}}, "uuid": uuid(), "session_id": self.id}),
        );
    }

    fn turn(&mut self, text: &str, images: usize, input: &mpsc::Receiver<Value>) {
        INTERRUPTED.store(false, Ordering::SeqCst);
        let started = Instant::now();
        let lower = text.to_lowercase();
        if lower.contains("crash") {
            eprintln!("fake-claude: crashing as asked");
            std::process::exit(3);
        }
        if let Some(effort) = effort_in(&lower) {
            self.effort = effort;
        }
        if asks_for_limit(text) {
            let resets = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                + 20;
            out(
                &json!({"type": "rate_limit_event", "rate_limit_info": {"status": "rejected", "resetsAt": resets, "rateLimitType": "five_hour", "utilization": 1.0}, "uuid": uuid(), "session_id": self.id}),
            );
            out(
                &json!({"type": "assistant", "message": {"id": "msg_limit", "type": "message", "role": "assistant", "model": self.model, "content": [{"type": "text", "text": "Claude usage limit reached."}], "usage": {"input_tokens": 0, "output_tokens": 0}}, "error": "rate_limit", "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
            );
            self.result(
                started,
                "Claude usage limit reached.",
                "error_during_execution",
            );
            return;
        }
        self.usage();
        if lower.contains("overage") {
            out(
                &json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed_warning", "rateLimitType": "five_hour", "isUsingOverage": true, "overageStatus": "allowed"}, "uuid": uuid(), "session_id": self.id}),
            );
        }
        if lower.contains("refresh") {
            let req_id = format!("req_{}", short_id(8));
            out(
                &json!({"type": "control_request", "request_id": req_id, "request": {"subtype": "oauth_token_refresh"}}),
            );
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut token = String::from("(none)");
            while Instant::now() < deadline {
                match input.recv_timeout(Duration::from_millis(200)) {
                    Ok(v) if v.get("type").and_then(Value::as_str) == Some("control_response") => {
                        token = v
                            .pointer("/response/response/accessToken")
                            .and_then(Value::as_str)
                            .map_or("(none)".into(), |t| format!("{}…", &t[..t.len().min(10)]));
                        break;
                    }
                    _ => {}
                }
            }
            self.stream_text(&format!("The host answered the token refresh: {token}"));
            self.result(started, "refreshed", "success");
            return;
        }
        let intro = if lower.contains("slow") {
            "Let me think about this carefully. ".repeat(40)
        } else {
            format!(
                "I'll take a look at that. You asked: \"{}\". Let me inspect the repository first.",
                text.chars().take(80).collect::<String>()
            )
        };
        self.stream_text(&intro);
        if INTERRUPTED.load(Ordering::SeqCst) {
            self.result(started, "Interrupted", "error_during_execution");
            return;
        }
        if let Some(files) = describe_files(text, images) {
            self.stream_text(&files);
        }
        let ls = std::process::Command::new("ls")
            .arg("-la")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        self.tool(
            "Bash",
            json!({"command": "ls -la", "description": "List files"}),
            &ls,
            false,
        );
        if let Some(name) = lower
            .split("write ")
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
        {
            let name: String = name
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c))
                .collect();
            if !name.is_empty() {
                let body = format!("# {name}\n\nWritten by fake-claude.\n");
                let _ = std::fs::write(&name, &body);
                self.tool(
                    "Write",
                    json!({"file_path": name, "content": body}),
                    &format!("File created successfully at: {name}"),
                    false,
                );
                let mut message = format!("Add {name}");
                if let Some(trailers) = commit_attribution() {
                    message = format!("{message}\n\n{trailers}");
                }
                let commit = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!("git add -- '{name}' && git commit -q -m \"$FAKE_CLAUDE_MESSAGE\" && git log -1 --format='%h %s%n%(trailers)'"))
                    .env("FAKE_CLAUDE_MESSAGE", &message)
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr))
                    .unwrap_or_default();
                let command =
                    format!("git add {name} && git commit -m \"$(cat <<'EOF'\n{message}\nEOF\n)\"");
                self.tool(
                    "Bash",
                    json!({"command": command, "description": "Commit the file"}),
                    &commit,
                    false,
                );
            }
        }
        if says(&lower, "push") {
            self.push();
        }
        if let Some(title) = rename_to(text) {
            let description = "Renamed by fake-claude with fctl edit.";
            let out = std::process::Command::new("fctl")
                .args(["edit", "--title", &title, "--description", description])
                .output();
            let (output, failed) = match out {
                Ok(o) => (
                    String::from_utf8_lossy(&o.stdout).into_owned()
                        + &String::from_utf8_lossy(&o.stderr),
                    !o.status.success(),
                ),
                Err(e) => (format!("fctl: {e}"), true),
            };
            self.tool(
                "Bash",
                json!({"command": format!("fctl edit --title {title:?} --description {description:?}"), "description": "Name the session"}),
                &output,
                failed,
            );
        }
        if lower.contains("screenshot") {
            let dir = std::env::temp_dir().join(format!("fake-claude-{}", std::process::id()));
            let path = dir.join("screenshot.png");
            let written =
                std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, screenshot_png()));
            let (output, failed) = match written.and_then(|_| {
                std::process::Command::new("fctl")
                    .arg("publish")
                    .arg(&path)
                    .output()
            }) {
                Ok(o) => (
                    String::from_utf8_lossy(&o.stdout).into_owned()
                        + &String::from_utf8_lossy(&o.stderr),
                    !o.status.success(),
                ),
                Err(e) => (format!("fctl: {e}"), true),
            };
            self.tool(
                "Bash",
                json!({"command": format!("fctl publish {}", path.display()), "description": "Publish a screenshot"}),
                &output,
                failed,
            );
        }
        if let Some(note) = after_word(text, "remember ") {
            let note =
                note.unwrap_or_else(|| "fake-claude remembers this for later sessions.".into());
            self.fctl(&["memory", "add", &note], "Remember it for later sessions");
        }
        if says(&lower, "recall") {
            self.fctl(&["memory"], "Read the project's memory");
        }
        if says(&lower, "retro") {
            self.fctl(
                &[
                    "retro",
                    "--outcome",
                    "completed",
                    "--worked-on",
                    "What the message asked, in fake-claude's sandbox.",
                    "--approach",
                    "Did the steps the message named, one tool call each.",
                    "--challenges",
                    "None: fake-claude knows its script.",
                    "--insights",
                    "fake-claude reacts to words in a message; docs/development.md lists them.",
                ],
                "Record a retrospective",
            );
        }
        if let Some(secs) = watch_for(text)
            && self.watch(secs)
        {
            self.result(started, "Interrupted", "error_during_execution");
            return;
        }
        if says(&lower, "delegate") {
            if lower.contains("in the background") {
                self.delegate_in_background();
                self.stream_text(
                    "It works in the background: I'll pick up its report when it finishes.",
                );
            } else if self.delegate() {
                self.result(started, "Interrupted", "error_during_execution");
                return;
            }
        }
        if let Some(secs) = background_for(text) {
            self.start_background(secs);
            self.stream_text("It runs in the background: I'll pick it up when it finishes.");
        }
        let done = "Done. Everything looks good: the change is committed and ready for review.";
        self.stream_text(done);
        self.result(started, done, "success");
    }
}

/// Runs `command` with `sh -c`: what it printed, and whether it failed.
fn sh(command: &str) -> (String, bool) {
    match std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
    {
        Ok(o) => (
            String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr),
            !o.status.success(),
        ),
        Err(e) => (format!("sh: {e}"), true),
    }
}

/// How long a turn's background command runs: the seconds after
/// `background` (any case), at most an hour.
fn background_for(text: &str) -> Option<u64> {
    seconds_after(text, "background").map(|s| s.min(3600))
}

/// How long a turn watches checks: the seconds after `watch` (any case), at
/// most ten minutes.
fn watch_for(text: &str) -> Option<u64> {
    seconds_after(text, "watch").map(|s| s.min(600))
}

/// The number right after `word` and a space, in any case.
fn seconds_after(text: &str, word: &str) -> Option<u64> {
    let lower = text.to_lowercase();
    let word = format!("{word} ");
    lower.match_indices(&word).find_map(|(i, word)| {
        let digits: String = lower[i + word.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse::<u64>().ok()
    })
}

/// Whether a lowercased turn has `word` as a word of its own.
fn says(lower: &str, word: &str) -> bool {
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w == word)
}

/// Whether a turn asks for the usage limit: it says `limit` but not
/// `reset`. fctl-agent continues a paused session with "The usage limit has
/// reset. Continue where you left off.", which has to get an answer, or the
/// session would hit the limit again every time it resumed.
fn asks_for_limit(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("limit") && !lower.contains("reset")
}

/// What Claude Code ends commit messages with: `attribution.commit` from
/// the user settings fctl-agent writes (`~/.claude/settings.json`), where
/// empty means nothing, or its own trailer when that is unset. Claude
/// Code's names the model; fake-claude's says Claude.
fn commit_attribution() -> Option<String> {
    let configured = std::env::var_os("HOME")
        .and_then(|home| {
            std::fs::read_to_string(std::path::Path::new(&home).join(".claude/settings.json")).ok()
        })
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| {
            v.pointer("/attribution/commit")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    match configured {
        Some(text) => Some(text).filter(|t| !t.trim().is_empty()),
        None => Some("Co-Authored-By: Claude <noreply@anthropic.com>".into()),
    }
}

/// What fake-claude says about a message's files: the images in it, and
/// whether each file the message lists is on disk.
fn describe_files(text: &str, images: usize) -> Option<String> {
    let listed: Vec<&str> = text
        .lines()
        .skip_while(|l| !l.starts_with("Attached files"))
        .skip(1)
        .filter_map(|l| l.strip_prefix("- "))
        .map(|l| l.rsplit_once(" (").map_or(l, |(path, _)| path))
        .collect();
    if listed.is_empty() && images == 0 {
        return None;
    }
    let mut s = format!(
        "The message holds {images} {}.",
        if images == 1 { "image" } else { "images" }
    );
    for path in listed {
        match std::fs::metadata(path) {
            Ok(m) if m.is_file() => s.push_str(&format!(" {path} is {} bytes.", m.len())),
            _ => s.push_str(&format!(" {path} is not on disk.")),
        }
    }
    Some(s)
}

/// A 160×90 PNG in fctl's colors (stripes of yellow over pink and blue),
/// stored without compression, so fake-claude needs no image library.
fn screenshot_png() -> Vec<u8> {
    let (w, h) = (160u32, 90u32);
    let mut raw = Vec::with_capacity(((w * 3 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0); // no filter
        for x in 0..w {
            let rgb: [u8; 3] = if (x + y) % 40 < 20 {
                [0xff, 0xff, 0x5d]
            } else if y < h / 2 {
                [0xff, 0x00, 0xd8]
            } else {
                [0x00, 0x7e, 0xff]
            };
            raw.extend_from_slice(&rgb);
        }
    }
    // zlib: its header, one stored deflate block (the rows fit), Adler-32.
    let len = raw.len() as u16;
    let mut z = vec![0x78, 0x01, 0x01];
    z.extend_from_slice(&len.to_le_bytes());
    z.extend_from_slice(&(!len).to_le_bytes());
    z.extend_from_slice(&raw);
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png_chunk(&mut png, b"IHDR", &ihdr);
    png_chunk(&mut png, b"IDAT", &z);
    png_chunk(&mut png, b"IEND", &[]);
    png
}

fn png_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc = crc32(&png[start..]);
    png.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// The effort level after `effort` (or Claude Code's `/effort`) in a
/// lowercased turn: `Some(None)` for `none`.
fn effort_in(lower: &str) -> Option<Option<String>> {
    let mut words = lower.split_whitespace();
    while let Some(word) = words.next() {
        if word.strip_prefix('/').unwrap_or(word) != "effort" {
            continue;
        }
        match words
            .next()
            .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        {
            Some(level @ ("low" | "medium" | "high" | "xhigh" | "max")) => {
                return Some(Some(level.to_owned()));
            }
            Some("none") => return Some(None),
            _ => {}
        }
    }
    None
}

/// What follows `word` (any case) to the end of its line: `None` inside
/// when nothing does, and `None` without the word.
fn after_word(text: &str, word: &str) -> Option<Option<String>> {
    let at = text.char_indices().find_map(|(i, _)| {
        text.get(i..i + word.len())
            .filter(|w| w.eq_ignore_ascii_case(word))
            .map(|_| i + word.len())
    })?;
    let rest = text[at..].lines().next().unwrap_or_default().trim();
    Some((!rest.is_empty()).then(|| rest.to_owned()))
}

/// The title after `rename` (any case), to the end of its line.
fn rename_to(text: &str) -> Option<String> {
    let at = text.char_indices().find_map(|(i, _)| {
        text.get(i..i + 7)
            .filter(|w| w.eq_ignore_ascii_case("rename "))
            .map(|_| i + 7)
    })?;
    let title = text[at..].lines().next().unwrap_or_default().trim();
    Some(if title.is_empty() {
        "Renamed by fake-claude".to_owned()
    } else {
        title.to_owned()
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("2.1.999 (fake-claude)");
        return;
    }
    let mut session = Session {
        id: arg("--session-id")
            .or_else(|| arg("--resume"))
            .unwrap_or_else(uuid),
        model: arg("--model").unwrap_or_else(|| "claude-fake-1".into()),
        effort: Some(arg("--effort").unwrap_or_else(|| "medium".into())),
        turns: 0,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        delay: Duration::from_millis(
            std::env::var("FAKE_CLAUDE_DELAY_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(25),
        ),
        background: Vec::new(),
        reports_turns: std::env::var("CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        }),
        running: false,
    };
    // stdin is read on its own thread so interrupts arrive during a turn.
    let (tx, rx) = mpsc::channel::<Value>();
    let (turn_tx, turn_rx) = mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let is_interrupt =
                v.pointer("/request/subtype").and_then(Value::as_str) == Some("interrupt");
            if is_interrupt {
                INTERRUPTED.store(true, Ordering::SeqCst);
            }
            // Counted before it is sent, so the main loop can't start its turn
            // (and take it off the count) first.
            if v.get("type").and_then(Value::as_str) == Some("user") {
                QUEUED_TURNS.fetch_add(1, Ordering::SeqCst);
            }
            // Answers to our own control requests go to the running turn.
            if v.get("type").and_then(Value::as_str) == Some("control_response") {
                let _ = turn_tx.send(v);
            } else if tx.send(v).is_err() {
                break;
            }
        }
    });
    let source = std::env::var("FAKE_CLAUDE_API_KEY_SOURCE").unwrap_or_else(|_| "none".into());
    let connectors_off = std::env::var("ENABLE_CLAUDEAI_MCP_SERVERS").is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
    });
    let mcp_servers = if connectors_off {
        json!([])
    } else {
        json!([{"name": "claude.ai Fake Docs", "status": "connected"}])
    };
    let mut announced = false;
    let announce = |s: &Session| {
        out(
            &json!({"type": "system", "subtype": "init", "apiKeySource": source, "claude_code_version": "2.1.999", "cwd": std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default(), "session_id": s.id, "tools": ["Bash", "Read", "Write", "Edit"], "mcp_servers": mcp_servers, "model": s.model, "permissionMode": "bypassPermissions", "slash_commands": [], "output_style": "default", "skills": [], "plugins": [], "uuid": uuid()}),
        );
    };
    loop {
        // Background work takes its steps, and gets a turn of its own when it ends.
        let next = match session.next_due() {
            Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        let v = match next {
            Ok(v) => v,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                session.finish_background();
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match v.get("type").and_then(Value::as_str) {
            Some("control_request") => {
                let id = v.get("request_id").cloned().unwrap_or(Value::Null);
                let subtype = v
                    .pointer("/request/subtype")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let response = match subtype {
                    "initialize" => {
                        json!({"subtype": "success", "request_id": id, "response": {"commands": [], "agents": [], "output_style": "default", "available_output_styles": ["default"], "models": [], "account": {"email": "fake@example.com"}}})
                    }
                    "interrupt" => json!({"subtype": "success", "request_id": id, "response": {}}),
                    "get_settings" => {
                        json!({"subtype": "success", "request_id": id, "response": {"effective": {}, "sources": [], "applied": {"model": session.model, "effort": session.effort}}})
                    }
                    "remote_control" => {
                        let scopes = std::env::var("CLAUDE_CODE_OAUTH_SCOPES").unwrap_or_default();
                        if scopes.contains("user:profile") {
                            json!({"subtype": "success", "request_id": id, "response": {"session_url": format!("https://claude.ai/code/session_fake{}", &session.id[..8]), "bridge_session_id": "session_fake", "environment_id": "env_fake"}})
                        } else {
                            json!({"subtype": "error", "request_id": id, "error": "Remote Control requires a full-scope login token. Long-lived tokens (from `claude setup-token` or CLAUDE_CODE_OAUTH_TOKEN) are limited to inference-only for security reasons. Run `claude auth login` to use Remote Control."})
                        }
                    }
                    other => {
                        json!({"subtype": "error", "request_id": id, "error": format!("fake-claude does not support {other}")})
                    }
                };
                out(&json!({"type": "control_response", "response": response}));
            }
            Some("user") => {
                QUEUED_TURNS.fetch_sub(1, Ordering::SeqCst);
                if !announced {
                    announce(&session);
                    announced = true;
                }
                let text = v
                    .pointer("/message/content")
                    .map(|c| match c {
                        Value::String(s) => s.clone(),
                        Value::Array(blocks) => blocks
                            .iter()
                            .filter_map(|b| b.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    })
                    .unwrap_or_default();
                let images = v
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .map_or(0, |blocks| {
                        blocks
                            .iter()
                            .filter(|b| b.get("type").and_then(Value::as_str) == Some("image"))
                            .count()
                    });
                session.report_turn(true);
                // Drain anything that arrived meanwhile into the turn's control channel.
                session.turn(&text, images, &turn_rx);
                if QUEUED_TURNS.load(Ordering::SeqCst) == 0 {
                    session.report_turn(false);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        adler32, asks_for_limit, background_for, crc32, effort_in, rename_to, says, screenshot_png,
        short_id, watch_for,
    };

    #[test]
    fn short_ids_differ() {
        let ids: std::collections::HashSet<String> = (0..1000).map(|_| short_id(8)).collect();
        assert_eq!(ids.len(), 1000);
    }

    #[test]
    fn draws_a_png() {
        // The checksums PNG and zlib define, against known values.
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
        let png = screenshot_png();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR"));
        assert_eq!(&png[16..24], &[0, 0, 0, 160, 0, 0, 0, 90]);
        assert!(png.ends_with(&[0xae, 0x42, 0x60, 0x82]));
    }

    #[test]
    fn limit_hits_the_limit_until_it_has_reset() {
        assert!(asks_for_limit("limit"));
        assert!(asks_for_limit("Hit the usage LIMIT, please"));
        assert!(!asks_for_limit(
            "The usage limit has reset. Continue where you left off."
        ));
        assert!(!asks_for_limit("write notes.md"));
    }

    #[test]
    fn background_takes_the_seconds_after_it() {
        assert_eq!(
            background_for("Run it in the BACKGROUND 20, please"),
            Some(20)
        );
        assert_eq!(background_for("background 99999"), Some(3600));
        // fctl-agent's own words about background tasks don't start one.
        assert_eq!(
            background_for("these background tasks were still running"),
            None
        );
        assert_eq!(background_for("write notes.md"), None);
    }

    #[test]
    fn watch_takes_the_seconds_after_it_and_push_is_a_word() {
        assert_eq!(
            watch_for("push it and WATCH 12 seconds of checks"),
            Some(12)
        );
        assert_eq!(watch_for("watch 9999"), Some(600));
        assert_eq!(watch_for("watch the checks"), None);
        assert!(says("push it, then watch 12", "push"));
        // fctl-agent's words when a session resumes in a new sandbox.
        assert!(!says("everything that was pushed to it", "push"));
    }

    #[test]
    fn rename_takes_the_rest_of_the_line() {
        assert_eq!(
            rename_to("Please rename Fix the login test\nthanks").as_deref(),
            Some("Fix the login test")
        );
        assert_eq!(
            rename_to("RENAME  ").as_deref(),
            Some("Renamed by fake-claude")
        );
        assert_eq!(rename_to("write notes.md"), None);
    }

    #[test]
    fn effort_takes_the_level_after_it() {
        assert_eq!(
            effort_in("work at effort high, please"),
            Some(Some("high".into()))
        );
        assert_eq!(effort_in("effort xhigh."), Some(Some("xhigh".into())));
        assert_eq!(effort_in("/effort max"), Some(Some("max".into())));
        assert_eq!(effort_in("effort none"), Some(None));
        // Other words after it change nothing.
        assert_eq!(effort_in("put some effort into it"), None);
        assert_eq!(effort_in("write notes.md"), None);
    }
}
