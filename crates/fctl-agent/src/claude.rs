//! The Claude Code process: `claude -p` in stream-json mode.
//!
//! stdout carries Claude's messages and its control requests to us (for
//! example `oauth_token_refresh`); stdin carries user turns and our control
//! requests (`initialize`, `interrupt`, `remote_control`).

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

/// Environment variables that would move Claude off the subscription or
/// redirect its traffic. They never reach the process.
pub const FORBIDDEN_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_BEDROCK_BASE_URL",
    "ANTHROPIC_VERTEX_BASE_URL",
    "ANTHROPIC_UNIX_SOCKET",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "AWS_BEARER_TOKEN_BEDROCK",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
];

pub enum ClaudeOut {
    /// A stream-json message.
    Message(Value),
    /// Claude asks us something (`oauth_token_refresh`, `can_use_tool`, …).
    ControlRequest {
        request_id: String,
        request: Value,
    },
    Stderr(String),
    Exited(Option<i32>),
}

pub struct LaunchSpec<'a> {
    pub bin: &'a str,
    pub cwd: &'a Path,
    pub home: &'a Path,
    pub claude_session_id: &'a str,
    pub resume: bool,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub append_system_prompt: &'a str,
    /// `--setting-sources` (user only when project settings are unsafe).
    pub setting_sources: &'a str,
    pub extra_args: &'a [String],
    pub env: BTreeMap<String, String>,
}

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<std::result::Result<Value, String>>>>>;

pub struct Claude {
    child: Child,
    stdin: mpsc::UnboundedSender<String>,
    pending: Pending,
    next_id: u64,
}

/// Claude's user settings in the sandbox: no API key helper, ever, and no
/// "Generated with Claude Code" footer on pull requests (their descriptions
/// become commit messages; the session link is in the system prompt).
/// `attribution.pr` is the long-supported form: a Claude Code that rejects a
/// key skips the whole file, permissions included.
pub fn user_settings() -> Value {
    json!({
        "$schema": "https://json.schemastore.org/claude-code-settings.json",
        "permissions": { "defaultMode": "bypassPermissions" },
        "env": { "DISABLE_AUTOUPDATER": "1" },
        "attribution": { "pr": "" }
    })
}

/// The line in `~/.claude/CLAUDE.md`, Claude's user memory, that loads
/// fctl's `~/AGENTS.md`. User memory loads in every repository (also when
/// fctl ignores the repository's settings), and unlike a CLAUDE.md beside
/// the checkout it never makes Claude skip a repository's own AGENTS.md.
pub const AGENTS_IMPORT: &str = "@~/AGENTS.md";

/// Writes `~/AGENTS.md` and imports it into Claude's user memory, keeping
/// whatever else the image put there.
pub fn write_instructions(home: &Path, agents_md: &str) -> Result<()> {
    std::fs::write(home.join("AGENTS.md"), agents_md)?;
    let memory = home.join(".claude/CLAUDE.md");
    let mut text = std::fs::read_to_string(&memory).unwrap_or_default();
    if !text.lines().any(|l| l.trim() == AGENTS_IMPORT) {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(AGENTS_IMPORT);
        text.push('\n');
        std::fs::create_dir_all(home.join(".claude"))?;
        std::fs::write(&memory, text)?;
    }
    Ok(())
}

/// Whether Claude Code has this conversation under `home`: it keeps them as
/// `~/.claude/projects/<working directory>/<session id>.jsonl`. A sandbox
/// created for a resumed session (the stopped one expired) has none.
pub fn conversation_exists(home: &Path, session_id: &str) -> bool {
    let file = format!("{session_id}.jsonl");
    std::fs::read_dir(home.join(".claude/projects"))
        .map(|dirs| dirs.flatten().any(|d| d.path().join(&file).is_file()))
        .unwrap_or(false)
}

impl Claude {
    pub fn spawn(spec: LaunchSpec<'_>, out: mpsc::UnboundedSender<ClaudeOut>) -> Result<Self> {
        let mut args: Vec<String> = vec![
            "-p".into(),
            "--input-format".into(),
            "stream-json".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--include-partial-messages".into(),
            "--permission-mode".into(),
            "bypassPermissions".into(),
            "--allow-dangerously-skip-permissions".into(),
            "--setting-sources".into(),
            spec.setting_sources.into(),
            "--append-system-prompt".into(),
            spec.append_system_prompt.into(),
        ];
        if spec.resume {
            args.extend(["--resume".into(), spec.claude_session_id.into()]);
        } else {
            args.extend(["--session-id".into(), spec.claude_session_id.into()]);
        }
        if let Some(m) = spec.model {
            args.extend(["--model".into(), m.into()]);
        }
        if let Some(e) = spec.effort {
            args.extend(["--effort".into(), e.into()]);
        }
        args.extend(spec.extra_args.iter().cloned());

        let mut cmd = Command::new(spec.bin);
        cmd.args(&args)
            .current_dir(spec.cwd)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for key in [
            "PATH", "LANG", "LC_ALL", "TZ", "TMPDIR", "USER", "LOGNAME", "SHELL", "TERM",
        ] {
            if let Ok(v) = std::env::var(key) {
                cmd.env(key, v);
            }
        }
        cmd.env("HOME", spec.home);
        for (k, v) in &spec.env {
            if !FORBIDDEN_ENV.contains(&k.as_str()) && !k.starts_with("ANTHROPIC_") {
                cmd.env(k, v);
            }
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("starting {} (is Claude Code installed?)", spec.bin))?;

        let mut stdin = child.stdin.take().context("claude stdin")?;
        let stdout = child.stdout.take().context("claude stdout")?;
        let stderr = child.stderr.take().context("claude stderr")?;
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(mut line) = stdin_rx.recv().await {
                line.push('\n');
                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
            // Dropping stdin closes it: Claude finishes and exits.
        });

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let pending_reader = pending.clone();
        let out_stdout = out.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::with_capacity(256 * 1024, stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let text = line.trim();
                if text.is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(text) else {
                    let _ = out_stdout.send(ClaudeOut::Stderr(format!(
                        "non-JSON output: {}",
                        truncate(text, 500)
                    )));
                    continue;
                };
                match v.get("type").and_then(Value::as_str) {
                    Some("control_response") => {
                        let resp = v.get("response").cloned().unwrap_or(Value::Null);
                        let id = resp
                            .get("request_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        let tx = pending_reader.lock().ok().and_then(|mut p| p.remove(&id));
                        if let Some(tx) = tx {
                            let result =
                                if resp.get("subtype").and_then(Value::as_str) == Some("success") {
                                    Ok(resp.get("response").cloned().unwrap_or(Value::Null))
                                } else {
                                    Err(resp
                                        .get("error")
                                        .and_then(Value::as_str)
                                        .unwrap_or("control request failed")
                                        .to_owned())
                                };
                            let _ = tx.send(result);
                        }
                    }
                    Some("control_request") => {
                        let request_id = v
                            .get("request_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        let request = v.get("request").cloned().unwrap_or(Value::Null);
                        let _ = out_stdout.send(ClaudeOut::ControlRequest {
                            request_id,
                            request,
                        });
                    }
                    Some("keep_alive")
                    | Some("transcript_mirror")
                    | Some("control_cancel_request") => {}
                    _ => {
                        let _ = out_stdout.send(ClaudeOut::Message(v));
                    }
                }
            }
            // stdout closed: the process is exiting. The supervisor collects the code.
            let _ = out_stdout.send(ClaudeOut::Exited(None));
        });
        let out_stderr = out.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    let _ = out_stderr.send(ClaudeOut::Stderr(truncate(&line, 2000)));
                }
            }
        });
        Ok(Self {
            child,
            stdin: stdin_tx,
            pending,
            next_id: 0,
        })
    }

    fn write(&self, v: &Value) {
        let _ = self.stdin.send(v.to_string());
    }

    /// Sends a control request and waits for the answer.
    pub async fn request(
        &mut self,
        request: Value,
        timeout: Duration,
    ) -> std::result::Result<Value, String> {
        self.next_id += 1;
        let id = format!("fctl_{}", self.next_id);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id.clone(), tx);
        }
        self.write(&json!({ "type": "control_request", "request_id": id, "request": request }));
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("Claude exited".into()),
            Err(_) => {
                if let Ok(mut p) = self.pending.lock() {
                    p.remove(&id);
                }
                Err("Claude did not answer in time".into())
            }
        }
    }

    /// Fire-and-forget control request (answers are ignored).
    pub fn request_nowait(&mut self, request: Value) {
        self.next_id += 1;
        let id = format!("fctl_{}", self.next_id);
        self.write(&json!({ "type": "control_request", "request_id": id, "request": request }));
    }

    /// Answers a control request from Claude.
    pub fn respond(&self, request_id: &str, result: std::result::Result<Value, String>) {
        let response = match result {
            Ok(v) => json!({ "subtype": "success", "request_id": request_id, "response": v }),
            Err(e) => json!({ "subtype": "error", "request_id": request_id, "error": e }),
        };
        self.write(&json!({ "type": "control_response", "response": response }));
    }

    /// A user turn: text and image blocks.
    pub fn send_user(&self, content: Vec<Value>, uuid: &str) {
        self.write(&json!({
            "type": "user",
            "session_id": "",
            "uuid": uuid,
            "message": { "role": "user", "content": content },
            "parent_tool_use_id": null
        }));
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Waits for the process to exit and reports it.
    pub async fn wait(&mut self) -> Option<i32> {
        self.child.wait().await.ok().and_then(|s| s.code())
    }

    pub async fn kill(&mut self) {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

/// Whether the repository's Claude settings could switch to API billing (an
/// `apiKeyHelper`, or `ANTHROPIC_*` / provider variables in `env`).
pub fn project_settings_are_unsafe(repo: &Path) -> Option<String> {
    for name in [".claude/settings.json", ".claude/settings.local.json"] {
        let path = repo.join(name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if v.get("apiKeyHelper").is_some() {
            return Some(format!("{name} sets apiKeyHelper"));
        }
        if let Some(env) = v.get("env").and_then(Value::as_object) {
            for key in env.keys() {
                if key.starts_with("ANTHROPIC_") || FORBIDDEN_ENV.contains(&key.as_str()) {
                    return Some(format!("{name} sets {key}"));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_settings_keep_pull_requests_plain() {
        let s = user_settings();
        assert_eq!(s["attribution"]["pr"], "");
        assert_eq!(s["permissions"]["defaultMode"], "bypassPermissions");
        assert!(s.get("apiKeyHelper").is_none());
    }

    #[test]
    fn user_memory_imports_agents_md_once() -> anyhow::Result<()> {
        let home = tempfile::tempdir()?;
        write_instructions(home.path(), "# fctl\n")?;
        write_instructions(home.path(), "# fctl\n")?;
        let memory = std::fs::read_to_string(home.path().join(".claude/CLAUDE.md"))?;
        assert_eq!(memory, "@~/AGENTS.md\n");
        assert_eq!(
            std::fs::read_to_string(home.path().join("AGENTS.md"))?,
            "# fctl\n"
        );
        // An image's own user memory stays, with the import after it.
        std::fs::write(home.path().join(".claude/CLAUDE.md"), "Use pnpm.")?;
        write_instructions(home.path(), "# fctl\n")?;
        let memory = std::fs::read_to_string(home.path().join(".claude/CLAUDE.md"))?;
        assert_eq!(memory, "Use pnpm.\n@~/AGENTS.md\n");
        Ok(())
    }

    #[test]
    fn finds_conversations() -> anyhow::Result<()> {
        let home = tempfile::tempdir()?;
        let id = "8d3f0c5e-0000-4000-8000-000000000001";
        assert!(!conversation_exists(home.path(), id));
        let project = home.path().join(".claude/projects/-home-agent-workspace");
        std::fs::create_dir_all(&project)?;
        std::fs::write(project.join("another.jsonl"), "{}")?;
        assert!(!conversation_exists(home.path(), id));
        std::fs::write(project.join(format!("{id}.jsonl")), "{}")?;
        assert!(conversation_exists(home.path(), id));
        Ok(())
    }

    #[test]
    fn detects_unsafe_project_settings() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        assert_eq!(project_settings_are_unsafe(dir.path()), None);
        std::fs::create_dir_all(dir.path().join(".claude"))?;
        std::fs::write(dir.path().join(".claude/settings.json"), r#"{"hooks": {}}"#)?;
        assert_eq!(project_settings_are_unsafe(dir.path()), None);
        std::fs::write(
            dir.path().join(".claude/settings.json"),
            r#"{"env": {"ANTHROPIC_API_KEY": "x"}}"#,
        )?;
        assert!(project_settings_are_unsafe(dir.path()).is_some());
        std::fs::write(
            dir.path().join(".claude/settings.json"),
            r#"{"apiKeyHelper": "echo key"}"#,
        )?;
        assert!(project_settings_are_unsafe(dir.path()).is_some());
        Ok(())
    }

    #[test]
    fn truncates_on_char_boundaries() {
        assert_eq!(truncate("héllo", 2), "h…");
        assert_eq!(truncate("abc", 5), "abc");
    }
}
