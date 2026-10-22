//! Printing a session transcript in the terminal, live.

use crate::style;
use fctl_domain::{branches, usage};
use fctl_proto::event::{EventBody, FileChange, GitState, NoticeLevel, SessionEvent};
use fctl_proto::session::PullRequestRef;
use serde_json::Value;
use std::collections::HashSet;
use std::io::Write;

#[derive(Default)]
pub struct Renderer {
    /// Message ids whose text was streamed (so the stored copy is not printed again).
    streamed: HashSet<String>,
    current_stream: Option<String>,
    in_text: bool,
    pub last_seq: u64,
    pub quiet_logs: bool,
    /// The repositories' state seen last, and the pull requests seen: a
    /// state shows when a branch changed (`fctl_domain::branches`).
    git: Option<GitState>,
    pull_requests: HashSet<String>,
    /// Print each stored event as one line of JSON instead (for scripts).
    pub json: bool,
}

/// Prints what a session said, without what a terminal would act on
/// (`style::terminal_safe`).
fn out(s: &str) {
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(style::terminal_safe(s).as_bytes());
    let _ = o.flush();
}

/// A one-line description of a tool call.
pub fn tool_summary(name: &str, input: &Value) -> String {
    let s = |k: &str| {
        input
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let arg = match name {
        "Bash" => s("command"),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => s("file_path"),
        "Grep" | "Glob" => s("pattern"),
        "WebFetch" => s("url"),
        "WebSearch" => s("query"),
        "Task" | "Agent" => s("description"),
        "TodoWrite" => "update the plan".into(),
        _ => input
            .as_object()
            .and_then(|o| o.values().find_map(Value::as_str))
            .unwrap_or_default()
            .to_owned(),
    };
    let arg: String = arg
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(100)
        .collect();
    format!("{name}({arg})")
}

/// `± 3 files (+10 −2) · 1 commits · pushed · PR #12 <url>`, with the
/// repository's name for the ones besides the primary.
fn git_line(
    repo: Option<&str>,
    files: &[FileChange],
    ahead: u32,
    pushed: bool,
    pr: Option<&PullRequestRef>,
) -> String {
    let adds: u32 = files.iter().filter_map(|f| f.additions).sum();
    let dels: u32 = files.iter().filter_map(|f| f.deletions).sum();
    let mut line = match repo {
        Some(name) => format!("± {name}: {} files (+{adds} −{dels})", files.len()),
        None => format!("± {} files (+{adds} −{dels})", files.len()),
    };
    if ahead > 0 {
        line.push_str(&format!(" · {ahead} commits"));
    }
    if pushed {
        line.push_str(" · pushed");
    }
    if let Some(pr) = pr {
        line.push_str(&format!(" · PR #{} {}", pr.number, pr.url));
    }
    line
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

impl Renderer {
    fn end_text(&mut self) {
        if self.in_text {
            out("\n");
            self.in_text = false;
        }
    }

    /// A line about the session that isn't an event (left out of `--json`).
    pub fn note(&mut self, line: &str) {
        if self.json {
            return;
        }
        self.end_text();
        out(&format!("{line}\n"));
    }

    pub fn event(&mut self, ev: &SessionEvent) {
        self.last_seq = self.last_seq.max(ev.seq);
        if self.json {
            if let Ok(line) = serde_json::to_string(ev) {
                out(&format!("{line}\n"));
            }
            return;
        }
        match &ev.body {
            EventBody::UserMessage(m) => {
                self.end_text();
                let who = m
                    .author
                    .as_ref()
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "fctl".into());
                out(&format!("\n{} {}\n", style::blue("▍"), style::bold(&who)));
                for line in m.text.lines() {
                    out(&format!("{} {line}\n", style::blue("▍")));
                }
                for a in &m.attachments {
                    out(&format!(
                        "{} {} {}\n",
                        style::blue("▍"),
                        a.name,
                        style::dim(&fctl_domain::files::human_size(a.size))
                    ));
                }
                out("\n");
            }
            EventBody::Status(s) => {
                self.end_text();
                let reason = s
                    .reason
                    .as_deref()
                    .map(|r| format!(" — {r}"))
                    .unwrap_or_default();
                out(&format!(
                    "{}{}\n",
                    style::status(s.status),
                    style::dim(&reason)
                ));
            }
            EventBody::Log(l) => {
                if !self.quiet_logs {
                    self.end_text();
                    out(&format!("{}\n", style::dim(&format!("  {}", l.text))));
                }
            }
            EventBody::Notice(n) if n.retrospective.is_some() => {
                self.end_text();
                if let Some(r) = &n.retrospective {
                    out(&format!(
                        "{} {}\n",
                        style::accent("◆"),
                        style::bold(&format!(
                            "Retrospective: {}",
                            fctl_domain::retrospectives::outcome_label(r.outcome).to_lowercase()
                        ))
                    ));
                    for (label, text) in [
                        ("Worked on", Some(&r.worked_on)),
                        ("Approach", r.approach.as_ref()),
                        ("Challenges", r.challenges.as_ref()),
                        ("Insights", r.insights.as_ref()),
                    ] {
                        if let Some(text) = text {
                            out(&format!("  {} {text}\n", style::dim(&format!("{label}:"))));
                        }
                    }
                }
            }
            EventBody::Notice(n) if n.memory.is_some() => {
                self.end_text();
                let line = if n.code == "memory_stale" {
                    style::dim(&format!("◇ {}", n.message))
                } else {
                    format!("{} {}", style::accent("◆"), n.message)
                };
                out(&format!("{line}\n"));
            }
            EventBody::Notice(n) => {
                self.end_text();
                let text = format!("! {}", n.message);
                out(&format!(
                    "{}\n",
                    match n.level {
                        NoticeLevel::Error => style::red(&text),
                        NoticeLevel::Warn => style::amber(&text),
                        NoticeLevel::Info => style::blue(&text),
                    }
                ));
            }
            EventBody::Git(g) => {
                // A line for each repository whose branch changed: a
                // commit, a push or a new pull request.
                let changed = branches::changed(self.git.as_ref(), g, &mut self.pull_requests);
                self.git = Some(g.clone());
                if changed.is_empty() {
                    return;
                }
                self.end_text();
                if changed.contains(&None) {
                    let pr = g.pull_request.as_ref();
                    let line = git_line(None, &g.files, g.ahead, g.pushed, pr);
                    out(&format!("{}\n", style::dim(&line)));
                }
                for r in g.repos.iter().flatten() {
                    let name = r.full_name();
                    if !changed.contains(&Some(name.clone())) {
                        continue;
                    }
                    let line = git_line(
                        Some(&name),
                        &r.files,
                        r.ahead,
                        r.pushed,
                        r.pull_request.as_ref(),
                    );
                    out(&format!("{}\n", style::dim(&line)));
                }
            }
            EventBody::RemoteControl(rc) => {
                self.end_text();
                match (&rc.session_url, &rc.unavailable_reason) {
                    (Some(url), _) => out(&format!(
                        "{} {}\n",
                        style::accent("◆ Follow in the Claude apps:"),
                        url
                    )),
                    (None, Some(why)) => out(&format!(
                        "{}\n",
                        style::dim(&format!("◇ Remote Control unavailable: {why}"))
                    )),
                    _ => {}
                }
            }
            EventBody::Interrupt(i) => {
                self.end_text();
                let who = i.by.as_ref().map(|u| u.name.clone()).unwrap_or_default();
                out(&format!(
                    "{}\n",
                    style::amber(&format!("■ interrupted by {who}"))
                ));
            }
            EventBody::Claude(v) => self.claude(v),
        }
    }

    fn claude(&mut self, v: &Value) {
        match v.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                let id = v
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let streamed = self.streamed.contains(&id);
                let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
                    return;
                };
                for b in blocks {
                    match b.get("type").and_then(Value::as_str) {
                        Some("text") if !streamed => {
                            self.end_text();
                            out(&format!(
                                "{}\n",
                                b.get("text").and_then(Value::as_str).unwrap_or_default()
                            ));
                        }
                        Some("tool_use") => {
                            self.end_text();
                            let name = b.get("name").and_then(Value::as_str).unwrap_or("tool");
                            let input = b.get("input").cloned().unwrap_or(Value::Null);
                            out(&format!(
                                "{} {}\n",
                                style::accent("⏺"),
                                tool_summary(name, &input)
                            ));
                        }
                        _ => {}
                    }
                }
            }
            Some("user") => {
                let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
                    return;
                };
                for b in blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
                {
                    let text = result_text(b.get("content").unwrap_or(&Value::Null));
                    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
                    let is_error = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                    for (i, l) in lines.iter().take(3).enumerate() {
                        let prefix = if i == 0 { "  ⎿ " } else { "    " };
                        let l: String = l.chars().take(140).collect();
                        let line = format!("{prefix}{l}");
                        out(&format!(
                            "{}\n",
                            if is_error {
                                style::red(&line)
                            } else {
                                style::dim(&line)
                            }
                        ));
                    }
                    if lines.len() > 3 {
                        out(&format!(
                            "{}\n",
                            style::dim(&format!("    … {} more lines", lines.len() - 3))
                        ));
                    }
                }
            }
            Some("result") => {
                self.end_text();
                let secs = v.get("duration_ms").and_then(Value::as_f64).unwrap_or(0.0) / 1000.0;
                let ok = v.get("subtype").and_then(Value::as_str) == Some("success");
                let line = format!("{} turn finished in {secs:.1}s", if ok { "✓" } else { "✕" });
                out(&format!(
                    "\n{}\n\n",
                    if ok {
                        style::accent(&line)
                    } else {
                        style::amber(&line)
                    }
                ));
            }
            Some("rate_limit_event") => {
                // Claude Code reports usage as it moves, and warns
                // (`allowed_warning`) when a limit goes faster than its
                // window passes: only a refusal means a limit is reached.
                let status = v.pointer("/rate_limit_info/status").and_then(Value::as_str);
                if status == Some("rejected") {
                    let limit = v
                        .pointer("/rate_limit_info/rateLimitType")
                        .and_then(Value::as_str)
                        .map_or_else(|| "Usage limit".into(), usage::label);
                    self.end_text();
                    out(&format!(
                        "{}\n",
                        style::amber(&format!("◍ {limit} reached"))
                    ));
                }
            }
            _ => {}
        }
    }

    /// An ephemeral `stream_event` (token deltas).
    pub fn stream(&mut self, v: &Value) {
        if self.json {
            return; // streaming deltas are not events; the stored message follows
        }
        let Some(ev) = v.get("event") else { return };
        if v.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
            return; // subagent output stays out of the main transcript
        }
        match ev.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                self.current_stream = ev
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(id) = &self.current_stream {
                    self.streamed.insert(id.clone());
                }
            }
            Some("content_block_delta") => {
                if let Some(t) = ev.pointer("/delta/text").and_then(Value::as_str) {
                    if !self.in_text {
                        self.in_text = true;
                    }
                    out(t);
                }
            }
            Some("content_block_stop") => self.end_text(),
            _ => {}
        }
    }
}
