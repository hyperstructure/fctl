//! A scripted stand-in for Claude Code's stream-json mode.
//!
//! It speaks the same protocol as `claude -p --input-format stream-json
//! --output-format stream-json --include-partial-messages`: control requests
//! (`initialize`, `interrupt`, `remote_control`), user turns on stdin, and
//! `system`, `stream_event`, `assistant`, `user`, `result` and
//! `rate_limit_event` messages on stdout. A user turn that arrives during
//! another waits for it, and each `result` says how many turns still wait
//! (`queued_turn_count`), as Claude Code's does.
//!
//! What it does depends on the words in a user turn:
//!
//! | Word | Behavior |
//! |---|---|
//! | (anything) | reports plan usage, streams a short answer, runs a fake `Bash` tool, answers again |
//! | `write <name>` | creates `<name>` in the working directory and commits it, ending the message as Claude Code does: with `attribution.commit` from `~/.claude/settings.json`, or Claude's trailer when that is unset |
//! | `rename <title>` | names the session `<title>` with `fctl edit`, as agents do |
//! | `screenshot` | draws a small PNG and publishes it with `fctl publish`, as agents do for pull requests |
//! | `slow` | streams for about ten seconds (try interrupting) |
//! | `limit` | reports a five-hour usage limit that resets in 20 seconds, unless the turn also says `reset`, as fctl-agent's message that continues the session does |
//! | `overage` | reports paid extra usage starting |
//! | `refresh` | asks its host for a fresh OAuth token |
//! | `crash` | exits with status 3 |
//! | (attached files) | says how many images the message holds and checks that each listed file is on disk |
//!
//! Plan usage comes as Claude Code reports it after a response
//! (`unifiedWindows` in a `rate_limit_event`): the five-hour and weekly
//! windows follow the clock, so every session reports the same numbers.
//!
//! Environment: `FAKE_CLAUDE_API_KEY_SOURCE` overrides the reported
//! `apiKeySource` (default `none`); `FAKE_CLAUDE_DELAY_MS` sets the delay
//! between streamed chunks (default 25). `system/init` lists a claude.ai
//! connector (`claude.ai Fake Docs`), as Claude Code does on a full login,
//! unless `ENABLE_CLAUDEAI_MCP_SERVERS` is `false`, `0`, `no` or `off`.

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

struct Session {
    id: String,
    model: String,
    turns: u64,
    input_tokens: u64,
    output_tokens: u64,
    delay: Duration,
}

impl Session {
    fn sleep(&self) {
        std::thread::sleep(self.delay);
    }

    fn stream_text(&self, text: &str) -> String {
        let msg_id = format!("msg_{}", &uuid()[..8]);
        out(
            &json!({"type": "stream_event", "event": {"type": "message_start", "message": {"id": msg_id, "role": "assistant", "model": self.model, "content": []}}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
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
            &json!({"type": "assistant", "message": {"id": msg_id, "type": "message", "role": "assistant", "model": self.model, "content": [{"type": "text", "text": written}], "stop_reason": null, "usage": {"input_tokens": 12, "output_tokens": written.len() / 4}}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
        msg_id
    }

    fn tool(&self, name: &str, input: Value, output: &str, is_error: bool) {
        let tool_id = format!("toolu_{}", &uuid()[..12]);
        out(
            &json!({"type": "assistant", "message": {"id": format!("msg_{}", &uuid()[..8]), "type": "message", "role": "assistant", "model": self.model, "content": [{"type": "tool_use", "id": tool_id, "name": name, "input": input}], "stop_reason": "tool_use", "usage": {"input_tokens": 20, "output_tokens": 30}}, "parent_tool_use_id": null, "session_id": self.id, "uuid": uuid()}),
        );
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

    fn result(&mut self, started: Instant, text: &str, subtype: &str) {
        self.turns += 1;
        self.input_tokens += 120;
        self.output_tokens += 80 + text.len() as u64 / 4;
        let ms = started.elapsed().as_millis() as u64;
        out(&json!({
            "type": "result", "subtype": subtype, "is_error": subtype != "success", "duration_ms": ms, "duration_api_ms": ms,
            "num_turns": 3, "result": text, "stop_reason": "end_turn", "total_cost_usd": 0.0, "session_id": self.id, "uuid": uuid(),
            "usage": {"input_tokens": 120, "output_tokens": 80, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0},
            "modelUsage": {self.model.clone(): {"inputTokens": self.input_tokens, "outputTokens": self.output_tokens, "cacheReadInputTokens": self.turns * 1000, "cacheCreationInputTokens": 0, "webSearchRequests": 0, "costUSD": 0.0, "contextWindow": 200000, "maxOutputTokens": 32000}},
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
            let req_id = format!("req_{}", &uuid()[..8]);
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
        let done = "Done. Everything looks good: the change is committed and ready for review.";
        self.stream_text(done);
        self.result(started, done, "success");
    }
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
        turns: 0,
        input_tokens: 0,
        output_tokens: 0,
        delay: Duration::from_millis(
            std::env::var("FAKE_CLAUDE_DELAY_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(25),
        ),
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
    for v in rx.iter() {
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
                // Drain anything that arrived meanwhile into the turn's control channel.
                session.turn(&text, images, &turn_rx);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{adler32, asks_for_limit, crc32, rename_to, screenshot_png};

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
}
