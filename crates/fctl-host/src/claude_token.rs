//! The Claude token a host keeps for its runner's owner (`fctl host claude
//! set-token`): a token from `claude setup-token` in `claude-token`, next
//! to `host.toml` (or where `claude_token_file` there says), that only its
//! owner may read (mode 0600).
//!
//! The control plane never gets it. Hellos and heartbeats say only whether
//! the host keeps one and since when ([`announce`]); the control plane
//! sends the sessions of a Claude connection whose token its owner keeps on
//! their runners only to those of their runners that keep one, with an
//! empty token that the host fills in ([`fill`]). What agents send to the
//! control plane goes through [`redact`] first, so that a token an agent
//! prints (`printenv`, say) doesn't reach the transcript.

use anyhow::{Context, Result, bail};
use fctl_proto::Millis;
use fctl_proto::runner::{ClaudeCredential, RunnerClaudeToken};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

/// What a redacted token reads as.
pub const REDACTED: &str = "[Claude token, redacted by fctl]";

/// Shorter secrets aren't redacted: they could be words. Claude's tokens
/// are about a hundred characters long.
const MIN_REDACTED: usize = 32;

/// What a redacted GitHub token reads as.
pub const REDACTED_GITHUB: &str = "[GitHub token, redacted by fctl]";

/// A kind of token redacted by its shape, whoever's it is: one this host
/// never knew of too, such as the one a session found running after the
/// host restarted started with, or another session's.
struct Shape {
    /// How each starts.
    prefix: &'static str,
    /// The fewest characters one has, its prefix included: shorter ones
    /// stay.
    min: usize,
    /// The characters it goes on with.
    body: fn(u8) -> bool,
    /// What it reads as, redacted.
    redacted: &'static str,
}

/// Claude's OAuth access tokens (`sk-ant-oat01-…`, from a full login or
/// `claude setup-token`), and GitHub's installation tokens (`ghs_…`), which
/// sessions' agents have.
const SHAPES: [Shape; 2] = [
    Shape {
        prefix: "sk-ant-oat",
        min: 40,
        body: base64url,
        redacted: REDACTED,
    },
    Shape {
        prefix: "ghs_",
        min: 40,
        body: alphanumeric,
        redacted: REDACTED_GITHUB,
    },
];

fn base64url(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn alphanumeric(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

impl Shape {
    /// Where the token of this shape that starts at `at` in `text` ends.
    fn end(&self, text: &[u8], at: usize) -> usize {
        let from = at + self.prefix.len();
        from + text.get(from..).map_or(0, |rest| {
            rest.iter().take_while(|b| (self.body)(**b)).count()
        })
    }
}

/// `text` with every token of a [`Shape`] in it replaced, whoever's it is.
pub fn redact_shapes(text: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(text);
    for shape in &SHAPES {
        if !out.contains(shape.prefix) {
            continue;
        }
        let bytes = out.as_bytes();
        let mut redacted = String::with_capacity(out.len());
        let (mut copied, mut from, mut changed) = (0, 0, false);
        while let Some(at) = out.get(from..).and_then(|s| s.find(shape.prefix)) {
            let at = from + at;
            let end = shape.end(bytes, at);
            if end - at >= shape.min {
                // Prefixes and bodies are ASCII: these are characters' starts.
                redacted.push_str(out.get(copied..at).unwrap_or_default());
                redacted.push_str(shape.redacted);
                copied = end;
                changed = true;
            }
            from = end;
        }
        if changed {
            redacted.push_str(out.get(copied..).unwrap_or_default());
            out = Cow::Owned(redacted);
        }
    }
    out
}

/// Whether `line`, a frame from fctl-agent, is a stream's
/// (`AgentToHost::Stream`), whose deltas may cut a token in two: the
/// [`StreamRedactor`] redacts those by their shape, as they come whole.
pub fn is_stream_frame(line: &str) -> bool {
    line.trim_start().starts_with(r#"{"type":"stream""#)
}

/// A token the host keeps, and when it was saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub token: String,
    /// When the file was last written (ms since the epoch).
    pub saved_at: Millis,
}

/// The token kept at `path`: `None` without one. A file that holds
/// something other than a token from `claude setup-token` is an error, so
/// that the host doesn't say it keeps one.
pub fn load(path: &Path) -> Result<Option<Kept>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let token = text.trim();
    if token.is_empty() {
        return Ok(None);
    }
    if let Some(problem) = fctl_domain::claude::setup_token_problem(token) {
        bail!(
            "{} doesn't hold a token from `claude setup-token`: {problem}",
            path.display()
        );
    }
    let saved_at = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    Ok(Some(Kept {
        token: token.to_owned(),
        saved_at,
    }))
}

/// Keeps `token` at `path`, readable only by its owner, in place of the
/// one kept there before.
pub fn save(path: &Path, token: &str) -> Result<()> {
    let token = token.trim();
    if let Some(problem) = fctl_domain::claude::setup_token_problem(token) {
        bail!("{problem}");
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::config::write_private(path, &format!("{token}\n"))
        .with_context(|| format!("writing {}", path.display()))
}

/// Removes the token kept at `path`: whether there was one.
pub fn remove(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Whether only its owner may read the file at `path`, as `save` makes it.
/// A file put there otherwise (a copy, a secrets manager's mount) may not
/// be; the host still uses it, and `fctl host doctor` says so.
pub fn private(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o077 == 0)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

/// What hellos and heartbeats say of the token kept at `path`: when it was
/// saved, or nothing saved without one. A file that can't be read, or
/// holds no token, counts as none, and says why in the log.
pub fn announce(path: &Path) -> RunnerClaudeToken {
    match load(path) {
        Ok(kept) => RunnerClaudeToken {
            saved_at: kept.map(|k| k.saved_at),
        },
        Err(e) => {
            tracing::warn!("{e:#}; this runner says it keeps no Claude token");
            RunnerClaudeToken::default()
        }
    }
}

/// Puts the token kept at `path` into `cred` when it is one whose owner
/// keeps it on their runners (`on_runners`, which the control plane sends
/// without a token), or says why it can't: `runner` names this runner.
/// Other credentials stay as they are.
pub fn fill(path: &Path, cred: &mut ClaudeCredential, runner: &str) -> Result<(), String> {
    if !cred.on_runners {
        return Ok(());
    }
    match load(path) {
        Ok(Some(kept)) => {
            cred.access_token = kept.token;
            Ok(())
        }
        Ok(None) => Err(format!(
            "{runner} keeps no Claude token: run `fctl host claude set-token` on it"
        )),
        Err(e) => Err(format!("{e:#}")),
    }
}

/// `text` without the `secrets` in it: each becomes [`REDACTED`]. Secrets
/// shorter than [`MIN_REDACTED`] stay, since they could be words.
pub fn redact<'a>(text: &'a str, secrets: &[String]) -> Cow<'a, str> {
    let mut out = Cow::Borrowed(text);
    for secret in secrets.iter().filter(|s| s.len() >= MIN_REDACTED) {
        if out.contains(secret.as_str()) {
            out = Cow::Owned(out.replace(secret.as_str(), REDACTED));
        }
    }
    out
}

/// The Claude tokens a session's agent has, which [`redact`] keeps out of
/// what it sends to the control plane. Shared with the task that reads its
/// frames.
#[derive(Clone, Debug, Default)]
pub struct Secrets(Arc<Mutex<Vec<String>>>);

impl Secrets {
    /// Adds `secret`, if it is long enough to redact and new.
    pub fn add(&self, secret: &str) {
        if secret.len() < MIN_REDACTED {
            return;
        }
        let mut all = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !all.iter().any(|s| s == secret) {
            all.push(secret.to_owned());
        }
    }

    /// `text` without any of them.
    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let all = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if all.is_empty() {
            return Cow::Borrowed(text);
        }
        match redact(text, &all) {
            Cow::Borrowed(_) => Cow::Borrowed(text),
            Cow::Owned(s) => Cow::Owned(s),
        }
    }

    /// A frame from fctl-agent without any of them, nor any token of a
    /// [`Shape`] but in a stream's deltas, which the [`StreamRedactor`]
    /// redacts.
    pub fn redact_frame<'a>(&self, line: &'a str) -> Cow<'a, str> {
        let known = self.redact(line);
        if is_stream_frame(&known) {
            return known;
        }
        match redact_shapes(&known) {
            Cow::Borrowed(_) => known,
            Cow::Owned(s) => Cow::Owned(s),
        }
    }

    fn all(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The fields of Claude's streamed deltas that carry what it writes: text,
/// thoughts and tool calls' input.
const DELTA_FIELDS: [&str; 3] = ["text", "thinking", "partial_json"];

/// Most content blocks it follows at once: the held text of those a stream
/// that broke off left behind goes, rather than pile up.
const MAX_BLOCKS: usize = 64;

/// Keeps the [`Secrets`] out of Claude's streamed deltas (`stream_event`s,
/// which viewers get live), which split what Claude writes anywhere, so
/// that a token it repeats comes in pieces that each look like nothing.
/// Of each content block's text, the end that could start a secret waits
/// for the next delta; a secret that comes whole becomes [`REDACTED`], and
/// what waited goes out before the block ends.
#[derive(Debug, Default)]
pub struct StreamRedactor {
    /// What each block held back, by the subagent call it is in (`None`
    /// for the agent's own) and its index.
    held: HashMap<(Option<String>, u64), Held>,
}

#[derive(Debug)]
struct Held {
    text: String,
    /// The delta it came in, to send it in when it goes out.
    delta: Value,
    field: &'static str,
}

impl StreamRedactor {
    /// What to send viewers in place of `event`, a message that goes to
    /// them live (`AgentToHost::Stream`): none, while all of a delta waits;
    /// the delta without what waits; or, at a block's end, what waited and
    /// then the end. Other messages pass as they are.
    pub fn apply(&mut self, secrets: &Secrets, event: Value) -> Vec<Value> {
        let mut out = self.held_back(secrets, event);
        // Tokens of a shape whole in any message, held back or not: a
        // block's start, a delta without a text, a tool's progress.
        for event in &mut out {
            redact_shapes_in(event);
        }
        out
    }

    /// [`Self::apply`] for the known secrets, and the tokens of a
    /// [`Shape`] a delta's text may cut in two.
    fn held_back(&mut self, secrets: &Secrets, mut event: Value) -> Vec<Value> {
        if event.get("type").and_then(Value::as_str) != Some("stream_event") {
            return vec![event];
        }
        let parent = event
            .get("parent_tool_use_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let kind = event
            .pointer("/event/type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let index = event.pointer("/event/index").and_then(Value::as_u64);
        match (kind.as_str(), index) {
            ("content_block_delta", Some(index)) => {
                let secrets = secrets.all();
                let Some((field, text)) = DELTA_FIELDS.iter().find_map(|f| {
                    event
                        .pointer(&format!("/event/delta/{f}"))
                        .and_then(Value::as_str)
                        .map(|t| (*f, t.to_owned()))
                }) else {
                    return vec![event];
                };
                let key = (parent, index);
                let before = self.held.remove(&key).map(|h| h.text).unwrap_or_default();
                let (out, kept) = hold_back(&format!("{before}{text}"), &secrets);
                if !kept.is_empty() {
                    if self.held.len() >= MAX_BLOCKS {
                        self.held.clear();
                    }
                    let delta = event.pointer("/event/delta").cloned().unwrap_or_default();
                    self.held.insert(
                        key,
                        Held {
                            text: kept,
                            delta,
                            field,
                        },
                    );
                }
                if out.is_empty() {
                    return Vec::new();
                }
                if let Some(t) = event.pointer_mut(&format!("/event/delta/{field}")) {
                    *t = Value::String(out);
                }
                vec![event]
            }
            ("content_block_stop", Some(index)) => {
                let mut out: Vec<Value> = self
                    .held
                    .remove(&(parent.clone(), index))
                    .map(|h| flushed(&parent, index, h))
                    .into_iter()
                    .collect();
                out.push(event);
                out
            }
            ("message_start" | "message_stop", _) => {
                // A message that ends without its blocks' ends: what they
                // held goes out before it does.
                let keys: Vec<(Option<String>, u64)> = self
                    .held
                    .keys()
                    .filter(|(p, _)| *p == parent)
                    .cloned()
                    .collect();
                let mut out: Vec<Value> = keys
                    .into_iter()
                    .filter_map(|k| {
                        let index = k.1;
                        self.held.remove(&k).map(|h| flushed(&parent, index, h))
                    })
                    .collect();
                out.push(event);
                out
            }
            _ => vec![event],
        }
    }
}

/// A delta that sends what a block held back: a token of a [`Shape`] it
/// ends with is whole now.
fn flushed(parent: &Option<String>, index: u64, held: Held) -> Value {
    let mut delta = held.delta;
    delta[held.field] = Value::String(redact_shapes(&held.text).into_owned());
    serde_json::json!({
        "type": "stream_event",
        "parent_tool_use_id": parent,
        "event": { "type": "content_block_delta", "index": index, "delta": delta },
    })
}

/// `text` with whole secrets replaced, split into what can go now and the
/// end that could start one of `secrets`, or be a token of a [`Shape`]
/// that may go on, which waits for more. What goes now has the tokens of
/// a shape in it replaced, which end there.
fn hold_back(text: &str, secrets: &[String]) -> (String, String) {
    let (now, later) = hold_back_known(text, secrets);
    let wait = shaped_wait(&now);
    let cut = now.len() - wait;
    match (now.get(..cut), now.get(cut..)) {
        (Some(go), Some(held)) => (redact_shapes(go).into_owned(), format!("{held}{later}")),
        _ => (redact_shapes(&now).into_owned(), later),
    }
}

/// How much of the end of `text` could be a token of a [`Shape`] that goes
/// on in the next delta: one that runs to its end, or the start of a
/// prefix.
fn shaped_wait(text: &str) -> usize {
    let bytes = text.as_bytes();
    // Every shape's prefix and body are base64url characters.
    let run = bytes.iter().rev().take_while(|b| base64url(**b)).count();
    let tail = text.get(text.len() - run..).unwrap_or_default();
    SHAPES
        .iter()
        .map(|shape| {
            // One that runs to the end, wherever in the word it starts
            // (`key_sk-ant-oat01-…`).
            if let Some(at) = tail.find(shape.prefix) {
                return run - at;
            }
            // The start of one, where a token may start: not the `s` of
            // `tokens`, which would hold back most words.
            (1..shape.prefix.len())
                .rev()
                .find(|&k| {
                    tail.ends_with(&shape.prefix[..k]) && token_may_start(bytes, bytes.len() - k)
                })
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0)
}

/// Whether a token may start at `at` in `text`: at its start, or after a
/// character that is neither a letter nor a digit (a space, `=`, a quote,
/// and the `-` and `_` that join words, as in `key_sk-ant-oat01-…`).
fn token_may_start(text: &[u8], at: usize) -> bool {
    at == 0 || text.get(at - 1).is_none_or(|b| !b.is_ascii_alphanumeric())
}

/// `value` with every token of a [`Shape`] in its strings replaced.
fn redact_shapes_in(value: &mut Value) {
    match value {
        Value::String(s) => {
            if let Cow::Owned(redacted) = redact_shapes(s) {
                *s = redacted;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_shapes_in),
        Value::Object(fields) => fields.values_mut().for_each(redact_shapes_in),
        _ => {}
    }
}

/// [`hold_back`] for the known `secrets` alone.
fn hold_back_known(text: &str, secrets: &[String]) -> (String, String) {
    let text = redact(text, secrets).into_owned();
    let bytes = text.as_bytes();
    // What starts a secret where a token may start, not inside a word
    // that ends like its start ([`token_may_start`]); one that runs to the
    // end inside a word, its whole prefix there, waits by its shape
    // ([`shaped_wait`]).
    let wait = secrets
        .iter()
        .filter(|s| s.len() >= MIN_REDACTED)
        .map(|s| {
            let s = s.as_bytes();
            (1..s.len().min(bytes.len() + 1))
                .rev()
                .find(|&k| {
                    bytes[bytes.len() - k..] == s[..k] && token_may_start(bytes, bytes.len() - k)
                })
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0);
    // Secrets are ASCII, so what matches one starts at a character.
    let cut = bytes.len() - wait;
    match (text.get(..cut), text.get(cut..)) {
        (Some(now), Some(later)) => (now.to_owned(), later.to_owned()),
        _ => (text, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::CredentialId;
    use fctl_proto::runner::ClaudeCredentialKind;

    const TOKEN: &str = "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJ-Xy3Q";

    fn credential(on_runners: bool) -> ClaudeCredential {
        ClaudeCredential {
            id: CredentialId("crd_1".into()),
            kind: ClaudeCredentialKind::SetupToken,
            access_token: String::new(),
            scopes: vec!["user:inference".into()],
            expires_at: None,
            subscription_type: None,
            account: None,
            on_runners,
        }
    }

    #[test]
    fn keeps_a_token_only_its_owner_reads() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("claude-token");
        assert_eq!(load(&path)?, None);
        assert!(!announce(&path).kept());
        save(&path, &format!("  {TOKEN}\n"))?;
        let kept = load(&path)?.context("a token")?;
        assert_eq!(kept.token, TOKEN);
        assert!(kept.saved_at > 0);
        assert_eq!(announce(&path).saved_at, Some(kept.saved_at));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)?.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(private(&path));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
            assert!(!private(&path));
            // Saving again makes it private again.
            save(&path, TOKEN)?;
            assert!(private(&path));
        }
        assert!(remove(&path)?);
        assert!(!remove(&path)?);
        assert_eq!(load(&path)?, None);
        Ok(())
    }

    #[test]
    fn keeps_only_tokens_from_claude_setup_token() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("claude-token");
        let key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF";
        let refused = save(&path, key).err().map(|e| e.to_string());
        assert!(refused.is_some_and(|e| e.contains("API key")));
        assert!(!path.exists());
        // Something else written there by hand isn't announced.
        std::fs::write(&path, "not a token\n")?;
        assert!(load(&path).is_err());
        assert!(!announce(&path).kept());
        std::fs::write(&path, "\n")?;
        assert_eq!(load(&path)?, None);
        Ok(())
    }

    #[test]
    fn fills_in_only_tokens_kept_on_runners() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("claude-token");
        let mut theirs = credential(true);
        assert_eq!(
            fill(&path, &mut theirs, "studio-mac"),
            Err("studio-mac keeps no Claude token: run `fctl host claude set-token` on it".into())
        );
        assert_eq!(theirs.access_token, "");
        save(&path, TOKEN)?;
        assert_eq!(fill(&path, &mut theirs, "studio-mac"), Ok(()));
        assert_eq!(theirs.access_token, TOKEN);
        // A credential the control plane sent with its token keeps it.
        let mut stored = ClaudeCredential {
            access_token: "sk-ant-oat01-from-the-control-plane".into(),
            ..credential(false)
        };
        assert_eq!(fill(&path, &mut stored, "studio-mac"), Ok(()));
        assert_eq!(stored.access_token, "sk-ant-oat01-from-the-control-plane");
        Ok(())
    }

    fn delta(parent: Option<&str>, index: u64, field: &str, text: &str) -> Value {
        let kind = match field {
            "thinking" => "thinking_delta",
            "partial_json" => "input_json_delta",
            _ => "text_delta",
        };
        serde_json::json!({"type": "stream_event", "parent_tool_use_id": parent,
            "event": {"type": "content_block_delta", "index": index, "delta": {"type": kind, field: text}}})
    }

    fn stop(parent: Option<&str>, index: u64) -> Value {
        serde_json::json!({"type": "stream_event", "parent_tool_use_id": parent,
            "event": {"type": "content_block_stop", "index": index}})
    }

    /// What viewers would put together from `events`, a block's deltas.
    fn shown(events: &[Value], field: &str) -> String {
        events
            .iter()
            .filter_map(|e| {
                e.pointer(&format!("/event/delta/{field}"))
                    .and_then(Value::as_str)
            })
            .collect()
    }

    fn streamed(r: &mut StreamRedactor, secrets: &Secrets, events: Vec<Value>) -> Vec<Value> {
        events
            .into_iter()
            .flat_map(|e| r.apply(secrets, e))
            .collect()
    }

    #[test]
    fn redacts_a_token_streamed_in_pieces() {
        let secrets = Secrets::default();
        secrets.add(TOKEN);
        let said = format!("Your token is {TOKEN}, as printenv showed.");
        for size in [1, 2, 3, 5, 7, 13, 40] {
            let chars: Vec<char> = said.chars().collect();
            let mut events: Vec<Value> = chars
                .chunks(size)
                .map(|c| delta(None, 0, "text", &c.iter().collect::<String>()))
                .collect();
            events.push(stop(None, 0));
            let mut r = StreamRedactor::default();
            let out = streamed(&mut r, &secrets, events);
            let text = shown(&out, "text");
            assert_eq!(
                text,
                format!("Your token is {REDACTED}, as printenv showed."),
                "{size}"
            );
            assert!(
                out.last()
                    .is_some_and(|e| e["event"]["type"] == "content_block_stop")
            );
        }
    }

    #[test]
    fn what_only_looks_like_a_token_goes_out_by_the_blocks_end() {
        let secrets = Secrets::default();
        secrets.add(TOKEN);
        let mut r = StreamRedactor::default();
        // A tool call's input, streamed: it starts like the token, then
        // isn't, and goes out as soon as it differs.
        let out = streamed(
            &mut r,
            &secrets,
            vec![
                delta(None, 1, "partial_json", r#"{"command": "echo sk-ant-o"#),
                delta(None, 1, "partial_json", r#"at01-abc"}"#),
            ],
        );
        assert_eq!(
            shown(&out, "partial_json"),
            r#"{"command": "echo sk-ant-oat01-abc"}"#
        );
        // One that is still like the token when its block ends goes out then.
        let out = streamed(
            &mut r,
            &secrets,
            vec![
                delta(None, 2, "partial_json", r#"{"key": "sk-ant-oat01-abc"#),
                delta(None, 2, "partial_json", "defgh"),
            ],
        );
        assert_eq!(shown(&out, "partial_json"), r#"{"key": ""#);
        let out = streamed(&mut r, &secrets, vec![stop(None, 2)]);
        assert_eq!(shown(&out, "partial_json"), "sk-ant-oat01-abcdefgh");
        assert_eq!(out[0]["event"]["delta"]["type"], "input_json_delta");
        assert_eq!(out[1]["event"]["type"], "content_block_stop");
        // Without secrets, nothing changes but where a token's shape may
        // start, and for other messages, nothing at all.
        let mut quiet = StreamRedactor::default();
        let plain = delta(None, 0, "text", "skip it");
        assert_eq!(quiet.apply(&Secrets::default(), plain.clone()), [plain]);
        assert!(
            quiet
                .apply(&Secrets::default(), delta(None, 0, "text", "sk-ant-o"))
                .is_empty()
        );
        let progress = serde_json::json!({"type": "tool_progress", "tool_use_id": "t1"});
        assert_eq!(quiet.apply(&secrets, progress.clone()), [progress]);
    }

    /// A Claude token this host never knew of: the one a session found
    /// running after a restart started with, say.
    const UNKNOWN: &str = "sk-ant-oat01-ZYXWVUTSRQPONMLKJIHGFEDCBA9876543210zyxwvutsrqp-_AbC";
    /// A session's GitHub installation token.
    const GITHUB: &str = "ghs_abcdefghijklmnopqrstuvwxyz0123456789";

    #[test]
    fn redacts_tokens_by_their_shape_whoever_s_they_are() {
        let text = format!("CLAUDE_CODE_OAUTH_TOKEN={UNKNOWN}\nGH_TOKEN={GITHUB} and {UNKNOWN}.");
        assert_eq!(
            redact_shapes(&text),
            format!(
                "CLAUDE_CODE_OAUTH_TOKEN={REDACTED}\nGH_TOKEN={REDACTED_GITHUB} and {REDACTED}."
            )
        );
        // What only starts like one, or is too short to be one, stays.
        for plain in [
            "sk-ant-oat01-short",
            "ghs_short",
            "the ghs_ prefix and sk-ant-oat",
            "ghs_snake_case_name_that_goes_on_and_on_and_on",
        ] {
            assert!(matches!(redact_shapes(plain), Cow::Borrowed(_)), "{plain}");
        }
        // In frames from fctl-agent, but in a stream's deltas, which the
        // stream's redactor takes as they come whole.
        let secrets = Secrets::default();
        let events = format!(r#"{{"type":"events","events":[{{"text":"{UNKNOWN}"}}]}}"#);
        assert_eq!(
            secrets.redact_frame(&events),
            format!(r#"{{"type":"events","events":[{{"text":"{REDACTED}"}}]}}"#)
        );
        let stream = format!(r#"{{"type":"stream","event":{{"text":"{UNKNOWN}"}}}}"#);
        assert_eq!(secrets.redact_frame(&stream), stream);
        assert!(is_stream_frame(&stream) && !is_stream_frame(&events));
    }

    #[test]
    fn words_that_end_like_a_token_starts_go_out_at_once() {
        let secrets = Secrets::default();
        secrets.add(TOKEN);
        let mut r = StreamRedactor::default();
        for text in ["two tokens", "a string", "the hosts", "a long thing"] {
            let plain = delta(None, 0, "text", text);
            assert_eq!(r.apply(&secrets, plain.clone()), [plain], "{text}");
        }
        // A word that could still be one waits.
        assert!(
            r.apply(&secrets, delta(None, 1, "text", "see sk"))
                .iter()
                .all(|e| { e["event"]["delta"]["text"].as_str() == Some("see ") })
        );
        assert!(
            r.apply(&secrets, delta(None, 2, "text", "GH_TOKEN=ghs_ab"))
                .iter()
                .all(|e| { e["event"]["delta"]["text"].as_str() == Some("GH_TOKEN=") })
        );
        // Also one that a `-` or `_` joins to a word.
        for (index, text, now) in [
            (3, "key_sk-a", "key_"),
            (4, "x-ghs_ab", "x-"),
            (5, "key_s", "key_"),
        ] {
            assert!(
                r.apply(&secrets, delta(None, index, "text", text))
                    .iter()
                    .all(|e| e["event"]["delta"]["text"].as_str() == Some(now)),
                "{text}"
            );
        }
    }

    #[test]
    fn redacts_tokens_by_their_shape_in_any_streamed_message() {
        let secrets = Secrets::default();
        let mut r = StreamRedactor::default();
        // A block that starts with text, a delta without one, and a
        // message of another kind.
        let start = serde_json::json!({"type": "stream_event", "parent_tool_use_id": null,
            "event": {"type": "content_block_start", "index": 0,
                "content_block": {"type": "text", "text": format!("Here: {UNKNOWN}")}}});
        let out = r.apply(&secrets, start);
        assert_eq!(
            out[0]["event"]["content_block"]["text"],
            format!("Here: {REDACTED}")
        );
        let no_index = serde_json::json!({"type": "stream_event", "parent_tool_use_id": null,
            "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": GITHUB}}});
        assert_eq!(
            r.apply(&secrets, no_index)[0]["event"]["delta"]["text"],
            REDACTED_GITHUB
        );
        let progress = serde_json::json!({"type": "tool_progress", "output": [UNKNOWN]});
        assert_eq!(r.apply(&secrets, progress)[0]["output"][0], REDACTED);
    }

    #[test]
    fn redacts_a_token_it_never_knew_streamed_in_pieces() {
        let secrets = Secrets::default();
        let said = format!("Here: {UNKNOWN}, and {GITHUB}, key_{UNKNOWN} and x-{GITHUB}.");
        for size in [1, 2, 3, 5, 7, 13, 40, 200] {
            let chars: Vec<char> = said.chars().collect();
            let mut events: Vec<Value> = chars
                .chunks(size)
                .map(|c| delta(None, 0, "text", &c.iter().collect::<String>()))
                .collect();
            events.push(stop(None, 0));
            let mut r = StreamRedactor::default();
            let out = streamed(&mut r, &secrets, events);
            assert_eq!(
                shown(&out, "text"),
                format!(
                    "Here: {REDACTED}, and {REDACTED_GITHUB}, key_{REDACTED} and x-{REDACTED_GITHUB}."
                ),
                "{size}"
            );
            // No piece of either shows on its way.
            for e in &out {
                let piece = e["event"]["delta"]["text"].as_str().unwrap_or_default();
                assert!(
                    !piece.contains("ZYXWVUTSRQ") && !piece.contains("abcdefghij"),
                    "{size}: {piece}"
                );
            }
        }
    }

    #[test]
    fn subagents_and_blocks_hold_back_their_own() {
        let secrets = Secrets::default();
        secrets.add(TOKEN);
        let (head, tail) = TOKEN.split_at(20);
        let mut r = StreamRedactor::default();
        let out = streamed(
            &mut r,
            &secrets,
            vec![
                delta(Some("toolu_1"), 0, "thinking", head),
                delta(None, 0, "thinking", "fine"),
                delta(Some("toolu_1"), 0, "thinking", tail),
                stop(Some("toolu_1"), 0),
            ],
        );
        assert_eq!(shown(&out, "thinking"), format!("fine{REDACTED}"));
        // A message that starts anew lets out what an unfinished one held.
        let out = streamed(
            &mut r,
            &secrets,
            vec![
                delta(None, 2, "text", "sk-"),
                serde_json::json!({"type": "stream_event", "parent_tool_use_id": null,
                    "event": {"type": "message_start", "message": {"id": "m2"}}}),
            ],
        );
        assert_eq!(shown(&out, "text"), "sk-");
        assert_eq!(
            out.last().map(|e| &e["event"]["type"]),
            Some(&serde_json::json!("message_start"))
        );
    }

    #[test]
    fn redacts_tokens_wherever_they_show() {
        let secrets = Secrets::default();
        let line = format!(
            r#"{{"type":"events","events":[{{"text":"CLAUDE_CODE_OAUTH_TOKEN={TOKEN}\nPATH=/usr/bin"}}]}}"#
        );
        assert_eq!(secrets.redact(&line), line);
        secrets.add(TOKEN);
        secrets.add(TOKEN);
        // Short ones could be words.
        secrets.add("a");
        let redacted = secrets.redact(&line);
        assert_eq!(
            redacted,
            format!(
                r#"{{"type":"events","events":[{{"text":"CLAUDE_CODE_OAUTH_TOKEN={REDACTED}\nPATH=/usr/bin"}}]}}"#
            )
        );
        assert!(serde_json::from_str::<serde_json::Value>(&redacted).is_ok());
        assert!(matches!(secrets.redact("nothing here"), Cow::Borrowed(_)));
        assert_eq!(
            redact(&format!("{TOKEN} and {TOKEN}"), &[TOKEN.to_owned()]),
            format!("{REDACTED} and {REDACTED}")
        );
    }
}
