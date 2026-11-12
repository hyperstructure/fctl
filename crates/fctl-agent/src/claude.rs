//! The Claude Code process: `claude -p` in stream-json mode.
//!
//! stdout carries Claude's messages and its control requests to us (for
//! example `oauth_token_refresh`); stdin carries user turns and our control
//! requests (`initialize`, `interrupt`, `remote_control`).

use anyhow::{Context, Result};
use fctl_proto::runner::GitIdentity;
use fctl_proto::session::ProjectSettings;
use fctl_proto::uplink::ContextWindow;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Child;
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
    let run = crate::user::command(bin)
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

/// How Claude Code's process ended, when it ended on its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Exit {
    /// Its exit code, when it exited.
    pub code: Option<i32>,
    /// The signal that killed it, when one did.
    pub signal: Option<i32>,
    /// The kernel killed a process of the sandbox for want of memory while
    /// it ran, and a signal killed it ([`oom_kills`]).
    #[serde(default)]
    pub out_of_memory: bool,
}

/// SIGKILL, which the kernel's out-of-memory killer sends.
const SIGKILL: i32 = 9;

impl Exit {
    /// Whether the kernel killed it for want of memory, as far as the
    /// sandbox's counts say.
    pub fn killed_for_memory(self, kills_before: Option<u64>, kills_now: Option<u64>) -> Self {
        let more = matches!((kills_before, kills_now), (Some(b), Some(n)) if n > b);
        Self {
            out_of_memory: more && self.signal == Some(SIGKILL),
            ..self
        }
    }

    /// How it ended, for the transcript and Claude: "ran out of memory",
    /// "was killed (signal 9)", "exited with code 1".
    pub fn words(self) -> String {
        match (self.out_of_memory, self.signal, self.code) {
            (true, ..) => "was killed because the sandbox ran out of memory".to_owned(),
            (false, Some(signal), _) => format!("was killed (signal {signal})"),
            (false, None, Some(code)) => format!("exited with code {code}"),
            (false, None, None) => "exited".to_owned(),
        }
    }
}

/// Where the kernel counts the processes it killed for want of memory
/// (`oom_kill`), the sandbox's own first: its cgroup's, with cgroup v2 or
/// v1, as in a container; then the system's, as in a VM sandbox, whose
/// processes are in the root cgroup, which has no `memory.events`.
const OOM_COUNTS: [&str; 3] = [
    "/sys/fs/cgroup/memory.events",
    "/sys/fs/cgroup/memory/memory.oom_control",
    "/proc/vmstat",
];

/// How many times the kernel killed a process of the sandbox for want of
/// memory, where the sandbox shows it: compared before and after, it says
/// whether a process that a signal killed ran out of memory.
pub fn oom_kills() -> Option<u64> {
    OOM_COUNTS
        .iter()
        .find_map(|path| oom_kills_in(&std::fs::read_to_string(path).ok()?))
}

fn oom_kills_in(counts: &str) -> Option<u64> {
    counts.lines().find_map(|line| {
        let (key, n) = line.split_once(' ')?;
        (key == "oom_kill").then(|| n.trim().parse().ok())?
    })
}

/// How often [`OomCount`] reads the count again.
const OOM_EVERY: Duration = Duration::from_secs(10);

/// The sandbox's out-of-memory kills as counted lately ([`oom_kills`]):
/// read again every ten seconds while Claude Code runs, it keeps the count
/// from ten to twenty seconds before. A kill since then, as the kernel
/// counts it when it picks a process, before that one has ended, was
/// likely Claude Code's own when a SIGKILL ended it; one long before, of a
/// build say, is no clue.
#[derive(Clone, Copy, Debug, Default)]
pub struct OomCount {
    before: Option<u64>,
    last: Option<u64>,
    at: Option<std::time::Instant>,
}

impl OomCount {
    /// Claude Code starts: counted from now.
    pub fn start(now: std::time::Instant, count: Option<u64>) -> Self {
        Self {
            before: count,
            last: count,
            at: Some(now),
        }
    }

    /// Reads the count again, through `count`, once ten seconds have
    /// passed since it last did.
    pub fn tick(&mut self, now: std::time::Instant, count: impl FnOnce() -> Option<u64>) {
        if self
            .at
            .is_some_and(|at| now.saturating_duration_since(at) >= OOM_EVERY)
        {
            self.before = self.last;
            self.last = count();
            self.at = Some(now);
        }
    }

    /// The count to compare with at Claude Code's exit.
    pub fn before(&self) -> Option<u64> {
        self.before
    }
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
    /// `--setting-sources` (user only when project settings are unsafe,
    /// and for reviews).
    pub setting_sources: &'a str,
    /// `--strict-mcp-config`: no MCP server from the repository's
    /// `.mcp.json` or anywhere but the command line, for reviews.
    pub strict_mcp: bool,
    pub extra_args: &'a [String],
    pub env: BTreeMap<String, String>,
}

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<std::result::Result<Value, AskError>>>>>;

/// Why a control request got no answer to use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskError {
    /// Claude answered with an error: it refused.
    Refused(String),
    /// Claude exited before it answered.
    Exited,
    /// Claude didn't answer in time; it may still act on the request.
    TimedOut,
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(e) => f.write_str(e),
            Self::Exited => f.write_str("Claude exited"),
            Self::TimedOut => f.write_str("Claude did not answer in time"),
        }
    }
}

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

/// Turns Claude Code's auto memory off. It lives in the sandbox's
/// `~/.claude/projects/`, so it would last only as long as the sandbox: in
/// projects that keep a memory, the agent keeps that one instead (`fctl
/// memory`), and Claude Code's system prompt doesn't offer another.
pub const DISABLE_AUTO_MEMORY_ENV: &str = "CLAUDE_CODE_DISABLE_AUTO_MEMORY";

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

/// The arguments Claude Code runs with, for `spec`.
fn command_line(spec: &LaunchSpec<'_>) -> Vec<String> {
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
    if spec.strict_mcp {
        args.push("--strict-mcp-config".into());
    }
    args.extend(spec.extra_args.iter().cloned());
    args
}

impl Claude {
    pub fn spawn(spec: LaunchSpec<'_>, out: mpsc::UnboundedSender<ClaudeOut>) -> Result<Self> {
        let args = command_line(&spec);
        // As the session's user, leading a process group of its own: a
        // supervisor that takes over from this one stops that group
        // (crate::orphan).
        let mut cmd = crate::user::command_in_own_group(spec.bin);
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
            if let Some(v) = crate::user::var(key) {
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
                                    Err(AskError::Refused(
                                        resp.get("error")
                                            .and_then(Value::as_str)
                                            .unwrap_or("control request failed")
                                            .to_owned(),
                                    ))
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
        self.ask(request, timeout).await.map_err(|e| e.to_string())
    }

    /// Sends a control request. The answer comes from the future, which
    /// doesn't hold on to the process, so another task can wait for it.
    pub fn ask(
        &mut self,
        request: Value,
        timeout: Duration,
    ) -> impl Future<Output = std::result::Result<Value, AskError>> + Send + use<> {
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
                Ok(Err(_)) => Err(AskError::Exited),
                Err(_) => {
                    if let Ok(mut p) = pending.lock() {
                        p.remove(&id);
                    }
                    Err(AskError::TimedOut)
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

    /// Waits for the process to exit and reports how it ended.
    pub async fn wait(&mut self) -> Option<Exit> {
        let status = self.child.wait().await.ok()?;
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(&status);
        #[cfg(not(unix))]
        let signal = None;
        Some(Exit {
            code: status.code(),
            signal,
            out_of_memory: false,
        })
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

/// The context window Claude works in, from its answer to
/// `get_context_usage`: `maxTokens`, and `autoCompactThreshold`, where it
/// compacts the conversation, which it leaves out when auto-compact is off.
/// Claude doesn't compact there when it leaves that to the API
/// (`autocompactSource` `auto`: it compacts once the API says the prompt is
/// too long, and `/context` shows no buffer for it). `None` when the answer
/// doesn't say.
pub fn context_window(usage: &Value) -> Option<ContextWindow> {
    let tokens = usage.get("maxTokens")?.as_u64().filter(|&t| t > 0)?;
    let compacts = usage.get("isAutoCompactEnabled").and_then(Value::as_bool) != Some(false)
        && usage.get("autocompactSource").and_then(Value::as_str) != Some("auto");
    let compact_at = usage
        .get("autoCompactThreshold")
        .and_then(Value::as_u64)
        .filter(|&t| compacts && t > 0 && t < tokens);
    Some(ContextWindow { tokens, compact_at })
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

    #[test]
    fn says_how_claude_code_ended_and_whether_memory_ran_out() {
        // cgroup v2's memory.events, v1's memory.oom_control, /proc/vmstat.
        let events = "low 0\nhigh 0\nmax 4\noom 2\noom_kill 2\noom_group_kill 0\n";
        assert_eq!(oom_kills_in(events), Some(2));
        let control = "oom_kill_disable 0\nunder_oom 0\noom_kill 3\n";
        assert_eq!(oom_kills_in(control), Some(3));
        let vmstat = "nr_free_pages 1\npgfault 9\noom_kill 4\nnr_unstable 0\n";
        assert_eq!(oom_kills_in(vmstat), Some(4));
        assert_eq!(oom_kills_in("low 0\n"), None);

        let killed = Exit {
            code: None,
            signal: Some(SIGKILL),
            out_of_memory: false,
        };
        // The kernel killed a process for want of memory meanwhile.
        let oom = killed.killed_for_memory(Some(1), Some(2));
        assert!(oom.out_of_memory);
        assert_eq!(
            oom.words(),
            "was killed because the sandbox ran out of memory"
        );
        // It didn't, or the sandbox doesn't say.
        for (before, now) in [(Some(2), Some(2)), (None, Some(2)), (Some(1), None)] {
            let exit = killed.killed_for_memory(before, now);
            assert_eq!(exit.words(), "was killed (signal 9)");
        }
        // A process that exited itself didn't run out of memory, whatever
        // else the kernel killed.
        let crashed = Exit {
            code: Some(1),
            ..Exit::default()
        };
        assert_eq!(
            crashed.killed_for_memory(Some(1), Some(2)).words(),
            "exited with code 1"
        );
        assert_eq!(Exit::default().words(), "exited");
    }

    #[test]
    fn counts_out_of_memory_kills_from_shortly_before() {
        use std::time::Instant;
        let start = Instant::now();
        let s = |n: u64| start + Duration::from_secs(n);
        let mut count = OomCount::start(start, Some(1));
        // Within ten seconds, it reads nothing again.
        count.tick(s(5), || panic!("read too soon"));
        assert_eq!(count.before(), Some(1));
        // A build ran out of memory early on: once twenty seconds have
        // passed, that kill is no clue.
        count.tick(s(10), || Some(2));
        assert_eq!(count.before(), Some(1));
        count.tick(s(20), || Some(2));
        assert_eq!(count.before(), Some(2));
        // The kernel counts Claude Code's kill as it picks it: a reading
        // then leaves the count of ten seconds before to compare with.
        count.tick(s(30), || Some(3));
        assert_eq!(count.before(), Some(2));
        let killed = Exit {
            signal: Some(SIGKILL),
            ..Exit::default()
        };
        assert!(
            killed
                .killed_for_memory(count.before(), Some(3))
                .out_of_memory
        );
        assert_eq!(OomCount::default().before(), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reports_the_signal_that_killed_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let bin = dir.path().join("claude");
        write_script(&bin, "#!/bin/sh\nkill -9 $$\n")?;
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut c = Claude::spawn(
            LaunchSpec {
                bin: &bin.display().to_string(),
                cwd: dir.path(),
                home: dir.path(),
                claude_session_id: "s",
                resume: false,
                model: None,
                effort: None,
                append_system_prompt: "",
                setting_sources: "user",
                strict_mcp: false,
                extra_args: &[],
                env: Default::default(),
            },
            tx,
        )?;
        let exit = c.wait().await;
        assert_eq!(
            exit,
            Some(Exit {
                code: None,
                signal: Some(SIGKILL),
                out_of_memory: false
            })
        );
        Ok(())
    }

    fn project(credit_claude: bool) -> ProjectSettings {
        ProjectSettings {
            credit_claude,
            ..Default::default()
        }
    }

    #[test]
    fn reviews_take_no_mcp_server_of_the_repository() {
        let dir = Path::new("/work");
        let launch = |setting_sources, strict_mcp| LaunchSpec {
            bin: "claude",
            cwd: dir,
            home: dir,
            claude_session_id: "s",
            resume: false,
            model: None,
            effort: None,
            append_system_prompt: "",
            setting_sources,
            strict_mcp,
            extra_args: &[],
            env: Default::default(),
        };
        let review = command_line(&launch("user", true));
        let sources = review.iter().position(|a| a == "--setting-sources");
        assert_eq!(
            sources.and_then(|i| review.get(i + 1)).map(String::as_str),
            Some("user")
        );
        assert!(review.iter().any(|a| a == "--strict-mcp-config"));
        let session = command_line(&launch("user,project,local", false));
        assert!(!session.iter().any(|a| a == "--strict-mcp-config"));
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
    fn claude_says_which_context_window_it_works_in() {
        // As Claude Code 2.1.308 answers `get_context_usage` with
        // `detail: "summary"` for Opus 5.5 (categories cut short).
        let answer = json!({
            "categories": [{ "name": "System prompt", "tokens": 1873, "color": "promptBorder", "kind": "used" }],
            "totalTokens": 16480, "maxTokens": 1_000_000, "rawMaxTokens": 1_000_000, "percentage": 2,
            "model": "claude-opus-5-5", "autoCompactThreshold": 967_000, "isAutoCompactEnabled": true,
            "autocompactSource": "model-default",
            "apiUsage": { "input_tokens": 3, "output_tokens": 7, "cache_creation_input_tokens": 5723, "cache_read_input_tokens": 5724 }
        });
        assert_eq!(
            context_window(&answer),
            Some(ContextWindow {
                tokens: 1_000_000,
                compact_at: Some(967_000)
            })
        );
        // Auto-compact off: no threshold.
        let off = json!({ "maxTokens": 200_000, "isAutoCompactEnabled": false });
        assert_eq!(
            context_window(&off),
            Some(ContextWindow {
                tokens: 200_000,
                compact_at: None
            })
        );
        // Left to the API, which says when the prompt is too long.
        let mut auto = answer.clone();
        auto["autocompactSource"] = json!("auto");
        assert_eq!(context_window(&auto).and_then(|w| w.compact_at), None);
        // A Claude Code from before `isAutoCompactEnabled` gives the
        // threshold only when auto-compact is on.
        let older = json!({ "maxTokens": 200_000, "autoCompactThreshold": 167_000 });
        assert_eq!(
            context_window(&older).and_then(|w| w.compact_at),
            Some(167_000)
        );
        // An answer without the window says nothing.
        assert_eq!(context_window(&json!({ "totalTokens": 9 })), None);
        assert_eq!(context_window(&json!({ "maxTokens": 0 })), None);
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
