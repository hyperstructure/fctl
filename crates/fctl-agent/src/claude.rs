//! The Claude Code process: `claude -p` in stream-json mode.
//!
//! stdout carries Claude's messages and its control requests to us (for
//! example `oauth_token_refresh`); stdin carries user turns and our control
//! requests (`initialize`, `interrupt`, `remote_control`).

use anyhow::{Context, Result};
use fctl_proto::runner::GitIdentity;
use fctl_proto::session::ProjectSettings;
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

/// The oldest Claude Code fctl works with. 2.1.101 is the first that asks
/// its host for a fresh OAuth access token (the `oauth_token_refresh`
/// control request), which sessions on a full Claude login need once
/// theirs expires; Remote Control's `remote_control` request came in
/// 2.1.63.
pub const MIN_VERSION: Version = Version(2, 1, 101);

/// A Claude Code version: major, minor, patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// The first `X.Y.Z` in `claude --version`'s output, `2.1.302 (Claude
    /// Code)`; a pre-release suffix doesn't count.
    pub fn parse(text: &str) -> Option<Self> {
        text.split_whitespace().find_map(|word| {
            let word = word.trim_start_matches('v');
            let mut parts = word.splitn(3, '.');
            let major = parts.next()?.parse().ok()?;
            let minor = parts.next()?.parse().ok()?;
            let patch = parts.next()?;
            let digits = patch.chars().take_while(char::is_ascii_digit).count();
            let patch = patch[..digits].parse().ok()?;
            Some(Self(major, minor, patch))
        })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Asks `bin --version` which Claude Code the sandbox has. `Err` says why
/// sessions can't run with it: it is missing, or older than
/// [`MIN_VERSION`]. `Ok(None)` when it doesn't say.
pub async fn check_version(bin: &str) -> std::result::Result<Option<Version>, String> {
    let run = Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = match tokio::time::timeout(Duration::from_secs(30), run).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "This sandbox has no Claude Code (`{bin}` is not on the PATH): install it in the image, or build the image from ghcr.io/hyperstructure/sandbox, which has it"
            ));
        }
        Ok(Err(e)) => {
            tracing::warn!("running {bin} --version: {e}");
            return Ok(None);
        }
        Err(_) => {
            tracing::warn!("{bin} --version did not answer within 30 seconds");
            return Ok(None);
        }
    };
    let Some(version) = Version::parse(&String::from_utf8_lossy(&out.stdout)) else {
        return Ok(None);
    };
    if version < MIN_VERSION {
        return Err(format!(
            "This sandbox has Claude Code {version}, and fctl needs {MIN_VERSION} or later: update it in the image (ghcr.io/hyperstructure/sandbox has the newest)"
        ));
    }
    Ok(Some(version))
}

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
///
/// `attribution.commit` ends commits with the trailers the project wants
/// ([`commit_attribution`]); it came with `attribution.pr`, in Claude Code
/// 2.0.62. A project that doesn't credit Claude also gets
/// `attribution.sessionUrl: false`, so Remote Control sessions leave out
/// the claude.ai link (Claude Code 2.1.183 and later; were an older one to
/// skip the file over it, it would credit Claude as by default, and the
/// command line still sets the permission mode). A repository's Claude
/// settings or its CLAUDE.md can still replace them.
///
/// Without `claude_ai_connectors`, `disableClaudeAiConnectors` too: a
/// repository's settings can set `ENABLE_CLAUDEAI_MCP_SERVERS` back on, but
/// no settings source can undo this one. Claude Code skips keys it doesn't
/// know (2.1.302 does), so one from before the key only has the variable.
pub fn user_settings(project: &ProjectSettings, co_authors: &[GitIdentity]) -> Value {
    let mut settings = json!({
        "$schema": "https://json.schemastore.org/claude-code-settings.json",
        "permissions": { "defaultMode": "bypassPermissions" },
        "env": { "DISABLE_AUTOUPDATER": "1" },
        "attribution": { "pr": "" }
    });
    if let Some(commit) = commit_attribution(project.credit_claude, co_authors) {
        settings["attribution"]["commit"] = json!(commit);
    }
    if !project.credit_claude {
        settings["attribution"]["sessionUrl"] = json!(false);
    }
    if !project.claude_ai_connectors {
        settings["disableClaudeAiConnectors"] = json!(true);
    }
    settings
}

/// Claude's user settings: the image's own `~/.claude/settings.json`, with
/// fctl's ([`user_settings`]) over it. fctl decides the permission mode,
/// `attribution`, `disableClaudeAiConnectors` and the auto-updater, and
/// drops what could move Claude off the subscription or out of bypass mode:
/// `apiKeyHelper`, `ANTHROPIC_*` and provider variables in `env` (as
/// [`project_settings_are_unsafe`] finds them in a repository), and
/// `permissions.disableBypassPermissionsMode`. Everything else the image
/// ships stays: plugins and marketplaces, hooks, permission rules, other
/// variables. A sandbox that resumes merges into what it has.
pub fn merge_user_settings(image: &Value, ours: &Value) -> Value {
    let mut merged = match image {
        Value::Object(o) => o.clone(),
        _ => serde_json::Map::new(),
    };
    for key in ["apiKeyHelper", "attribution", "disableClaudeAiConnectors"] {
        merged.remove(key);
    }
    let mut env = take_object(&mut merged, "env");
    env.retain(|k, _| !k.starts_with("ANTHROPIC_") && !FORBIDDEN_ENV.contains(&k.as_str()));
    let mut permissions = take_object(&mut merged, "permissions");
    permissions.remove("disableBypassPermissionsMode");
    for (key, value) in ours.as_object().into_iter().flatten() {
        match (key.as_str(), value) {
            ("env", Value::Object(o)) => env.extend(o.clone()),
            ("permissions", Value::Object(o)) => permissions.extend(o.clone()),
            _ => {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    merged.insert("env".into(), Value::Object(env));
    merged.insert("permissions".into(), Value::Object(permissions));
    Value::Object(merged)
}

/// Removes `key` from `map`: its object, or an empty one.
fn take_object(
    map: &mut serde_json::Map<String, Value>,
    key: &str,
) -> serde_json::Map<String, Value> {
    match map.remove(key) {
        Some(Value::Object(o)) => o,
        _ => serde_json::Map::new(),
    }
}

/// Claude's trailer in commits that credit people too. Claude Code's own
/// names the model, which isn't known before Claude starts; Claude Code
/// writes this one itself when it can't tell the model's version.
const CLAUDE_TRAILER: &str = "Co-authored-by: Claude <noreply@anthropic.com>";

/// The text Claude ends commits with: Claude's trailer if the project
/// credits Claude, then a `Co-authored-by` trailer for each person the
/// session credits; empty for no trailers at all. `None` when only Claude
/// is credited, which keeps Claude Code's default trailer, naming the model.
pub fn commit_attribution(credit_claude: bool, co_authors: &[GitIdentity]) -> Option<String> {
    let people: Vec<String> = co_authors.iter().filter_map(co_author_trailer).collect();
    if credit_claude && people.is_empty() {
        return None;
    }
    let claude = credit_claude.then(|| CLAUDE_TRAILER.to_owned());
    Some(
        claude
            .into_iter()
            .chain(people)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// `Co-authored-by: Name <email>`, on one line with one address. The
/// control plane checks both; this keeps an odd one from adding lines.
fn co_author_trailer(person: &GitIdentity) -> Option<String> {
    let name: String = person
        .name
        .chars()
        .map(|c| {
            if c.is_control() || c == '<' || c == '>' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let email = person.email.trim();
    let odd = |c: char| c.is_whitespace() || c.is_control() || c == '<' || c == '>';
    if name.is_empty() || !email.contains('@') || email.chars().any(odd) {
        return None;
    }
    Some(format!("Co-authored-by: {name} <{email}>"))
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
        self.ask(request, timeout).await
    }

    /// Sends a control request. The answer comes from the future, which
    /// doesn't hold on to the process, so another task can wait for it.
    pub fn ask(
        &mut self,
        request: Value,
        timeout: Duration,
    ) -> impl Future<Output = std::result::Result<Value, String>> + Send + use<> {
        self.next_id += 1;
        let id = format!("fctl_{}", self.next_id);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id.clone(), tx);
        }
        self.write(&json!({ "type": "control_request", "request_id": id, "request": request }));
        let pending = self.pending.clone();
        async move {
            match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(r)) => r,
                Ok(Err(_)) => Err("Claude exited".into()),
                Err(_) => {
                    if let Ok(mut p) = pending.lock() {
                        p.remove(&id);
                    }
                    Err("Claude did not answer in time".into())
                }
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

/// The effort level Claude works at, from its answer to `get_settings`:
/// `applied` holds what it sends the API, after the session's choice
/// (`--effort`, or a change from the Claude apps over Remote Control),
/// settings, environment and the model's default. Empty when the model
/// takes no effort level; `None` when the answer doesn't say (a Claude Code
/// from before `applied`).
pub fn applied_effort(settings: &Value) -> Option<String> {
    match settings.get("applied")?.get("effort")? {
        Value::String(level) => Some(level.clone()),
        Value::Null => Some(String::new()),
        _ => None,
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
pub(crate) mod tests {
    use super::*;

    /// Writes `script` to `path`, executable, for a test to run right away.
    /// A process that another test starts holds copies of this process's open
    /// files until it execs, and running a script while such a copy is open for
    /// writing fails with "Text file busy". So `sh` writes the script, and it is
    /// never open here.
    pub(crate) fn write_script(path: &Path, script: &str) -> Result<()> {
        let status = std::process::Command::new("sh")
            .args(["-c", r#"printf %s "$2" > "$1" && chmod 755 "$1""#, "sh"])
            .arg(path)
            .arg(script)
            .status()?;
        anyhow::ensure!(status.success(), "writing {}: {status}", path.display());
        Ok(())
    }

    fn project(credit_claude: bool) -> ProjectSettings {
        ProjectSettings {
            credit_claude,
            ..Default::default()
        }
    }

    #[test]
    fn claude_says_which_effort_it_works_at() {
        // As Claude Code 2.1.304 answers `get_settings`, without `--effort`.
        let answer = json!({
            "effective": { "permissions": { "defaultMode": "bypassPermissions" } },
            "sources": [],
            "applied": {
                "model": "claude-opus-5-5", "effort": "medium", "advisor": null,
                "ultracode": false, "ultracodeRequested": false, "ultracodeAvailable": true
            }
        });
        assert_eq!(applied_effort(&answer).as_deref(), Some("medium"));
        // A model that takes no effort level.
        let none = json!({ "applied": { "model": "claude-haiku-4-5", "effort": null } });
        assert_eq!(applied_effort(&none).as_deref(), Some(""));
        // Older Claude Code doesn't say.
        assert_eq!(
            applied_effort(&json!({ "effective": {}, "sources": [] })),
            None
        );
        assert_eq!(
            applied_effort(&json!({ "applied": { "model": "x" } })),
            None
        );
    }

    #[test]
    fn user_settings_keep_pull_requests_plain() {
        let s = user_settings(&project(true), &[]);
        assert_eq!(s["attribution"]["pr"], "");
        assert_eq!(s["permissions"]["defaultMode"], "bypassPermissions");
        assert!(s.get("apiKeyHelper").is_none());
    }

    #[test]
    fn the_images_settings_stay_under_fctls() {
        let image = json!({
            "enabledPlugins": { "rust@acme": true },
            "extraKnownMarketplaces": { "acme": { "source": { "source": "github", "repo": "acme/plugins" } } },
            "hooks": { "PostToolUse": [] },
            "permissions": {
                "allow": ["Bash(cargo:*)"],
                "defaultMode": "plan",
                "disableBypassPermissionsMode": "disable"
            },
            "env": {
                "RUST_LOG": "info",
                "ANTHROPIC_API_KEY": "sk-ant-x",
                "ANTHROPIC_BASE_URL": "https://proxy",
                "CLAUDE_CODE_USE_BEDROCK": "1",
                "DISABLE_AUTOUPDATER": "0"
            },
            "apiKeyHelper": "echo sk-ant-x",
            "attribution": { "commit": "Made by acme" },
            "disableClaudeAiConnectors": true,
            "model": "sonnet"
        });
        let s = merge_user_settings(&image, &user_settings(&project(true), &[]));
        assert_eq!(s["enabledPlugins"], image["enabledPlugins"]);
        assert_eq!(s["extraKnownMarketplaces"], image["extraKnownMarketplaces"]);
        assert_eq!(s["hooks"], image["hooks"]);
        assert_eq!(s["model"], "sonnet");
        assert_eq!(s["permissions"]["allow"], json!(["Bash(cargo:*)"]));
        assert_eq!(s["permissions"]["defaultMode"], "bypassPermissions");
        assert!(
            s["permissions"]
                .get("disableBypassPermissionsMode")
                .is_none()
        );
        assert_eq!(
            s["env"],
            json!({ "RUST_LOG": "info", "DISABLE_AUTOUPDATER": "1" })
        );
        assert!(s.get("apiKeyHelper").is_none());
        // The project's settings decide credits and connectors.
        assert_eq!(s["attribution"], json!({ "pr": "" }));
        assert!(s.get("disableClaudeAiConnectors").is_none());
        // A resume merges into what the last run wrote: the same again.
        let again = merge_user_settings(&s, &user_settings(&project(true), &[]));
        assert_eq!(again, s);
        // A turned-off setting comes back on once the project says so.
        let off = merge_user_settings(&s, &user_settings(&project(false), &[]));
        assert_eq!(off["attribution"]["sessionUrl"], false);
        let on = merge_user_settings(&off, &user_settings(&project(true), &[]));
        assert!(on["attribution"].get("sessionUrl").is_none());
        // No settings, or ones that aren't an object: fctl's alone.
        for nothing in [Value::Null, json!([1])] {
            let ours = user_settings(&project(true), &[]);
            let s = merge_user_settings(&nothing, &ours);
            assert_eq!(s, ours);
        }
    }

    #[test]
    fn user_settings_turn_connectors_off() {
        assert!(
            user_settings(&project(true), &[])
                .get("disableClaudeAiConnectors")
                .is_none()
        );
        let off = ProjectSettings {
            claude_ai_connectors: false,
            ..Default::default()
        };
        let s = user_settings(&off, &[]);
        assert_eq!(s["disableClaudeAiConnectors"], true);
        assert_eq!(s["permissions"]["defaultMode"], "bypassPermissions");
    }

    fn person(name: &str, email: &str) -> GitIdentity {
        GitIdentity {
            name: name.into(),
            email: email.into(),
        }
    }

    #[test]
    fn commits_credit_the_people_the_session_names() {
        // Nobody to credit: Claude Code's own trailer, which names the model.
        let s = user_settings(&project(true), &[]);
        assert!(s["attribution"].get("commit").is_none());
        assert!(s["attribution"].get("sessionUrl").is_none());
        let s = user_settings(
            &project(true),
            &[person("Ada Lovelace", "1+ada@users.noreply.github.com")],
        );
        assert_eq!(
            s["attribution"]["commit"],
            "Co-authored-by: Claude <noreply@anthropic.com>\n\
             Co-authored-by: Ada Lovelace <1+ada@users.noreply.github.com>"
        );
        assert_eq!(s["attribution"]["pr"], "");
    }

    #[test]
    fn projects_that_dont_credit_claude_leave_its_trailer_out() {
        // No trailers at all, and no claude.ai link from Remote Control.
        let s = user_settings(&project(false), &[]);
        assert_eq!(s["attribution"]["commit"], "");
        assert_eq!(s["attribution"]["sessionUrl"], false);
        assert_eq!(s["attribution"]["pr"], "");
        // The people the session credits stay.
        let s = user_settings(
            &project(false),
            &[person("Ada Lovelace", "1+ada@users.noreply.github.com")],
        );
        assert_eq!(
            s["attribution"]["commit"],
            "Co-authored-by: Ada Lovelace <1+ada@users.noreply.github.com>"
        );
        assert_eq!(s["attribution"]["sessionUrl"], false);
    }

    #[test]
    fn co_author_trailers_stay_one_line() {
        assert_eq!(
            commit_attribution(true, &[person(" Ada\n<b>Lovelace ", "ada@example.com")]).as_deref(),
            Some(
                "Co-authored-by: Claude <noreply@anthropic.com>\n\
                 Co-authored-by: Ada b Lovelace <ada@example.com>"
            )
        );
        for email in ["ada", "ada@example.com>\nFixes: #1", "a b@example.com", ""] {
            assert_eq!(
                commit_attribution(true, &[person("Ada", email)]),
                None,
                "{email:?}"
            );
        }
        assert_eq!(
            commit_attribution(true, &[person(" <> ", "ada@example.com")]),
            None
        );
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
    fn reads_claude_codes_version() {
        assert_eq!(
            Version::parse("2.1.302 (Claude Code)\n"),
            Some(Version(2, 1, 302))
        );
        assert_eq!(
            Version::parse("2.1.999 (fake-claude)"),
            Some(Version(2, 1, 999))
        );
        assert_eq!(
            Version::parse("claude v2.1.101-rc.1"),
            Some(Version(2, 1, 101))
        );
        assert_eq!(Version::parse("Claude Code"), None);
        assert!(Version(2, 1, 100) < MIN_VERSION && MIN_VERSION < Version(2, 2, 0));
        assert_eq!(MIN_VERSION.to_string(), "2.1.101");
    }

    #[tokio::test]
    async fn sessions_need_a_recent_claude_code() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let claude = |version: &str| -> anyhow::Result<String> {
            let bin = dir.path().join(format!("claude-{version}"));
            write_script(
                &bin,
                &format!("#!/bin/sh\necho '{version} (Claude Code)'\n"),
            )?;
            Ok(bin.display().to_string())
        };
        assert_eq!(
            check_version(&claude("2.1.302")?).await,
            Ok(Some(Version(2, 1, 302)))
        );
        assert_eq!(
            check_version(&claude("2.1.90")?).await,
            Err("This sandbox has Claude Code 2.1.90, and fctl needs 2.1.101 or later: update it in the image (ghcr.io/hyperstructure/sandbox has the newest)".into())
        );
        let missing = check_version("fctl-test-no-claude").await;
        assert!(
            missing
                .as_ref()
                .is_err_and(|e| e.starts_with("This sandbox has no Claude Code")),
            "{missing:?}"
        );
        assert_eq!(check_version(&claude("unknown")?).await, Ok(None));
        Ok(())
    }

    #[test]
    fn truncates_on_char_boundaries() {
        assert_eq!(truncate("héllo", 2), "h…");
        assert_eq!(truncate("abc", 5), "abc");
    }
}
