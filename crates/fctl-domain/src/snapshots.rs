//! Session snapshots (docs/architecture.md#session-snapshots): a session's
//! conversation and uncommitted work, saved as its sandbox stops, so that
//! it resumes on any runner as it was. What fctl-agent and the control
//! plane agree on: how a snapshot is sealed with its session's key, which
//! Claude Code may read the conversation another wrote, what a
//! conversation that is too large leaves out, how long one stays, and what
//! the transcript says.
//!
//! A snapshot's bytes are a gzipped tar, sealed in segments of
//! [`SEGMENT`] bytes with AES-256-GCM (the STREAM construction): the file
//! starts with [`MAGIC`] and a random 7-byte prefix, and each segment's
//! nonce is that prefix, the segment's number and whether it is the last,
//! so that segments can't be reordered, dropped or cut off unnoticed.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use serde_json::Value;

/// What a sealed snapshot starts with: its format's name and version.
pub const MAGIC: &[u8; 8] = b"FCTLSNP1";
const PREFIX: usize = 7;
/// The bytes before its first segment: [`MAGIC`] and the nonce prefix.
pub const HEADER: usize = MAGIC.len() + PREFIX;
/// A segment's plaintext: every one but the last is this long.
pub const SEGMENT: usize = 1 << 20;
/// What sealing adds to a segment: its tag.
pub const TAG: usize = 16;

/// Why a snapshot couldn't be sealed or opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The key isn't 32 bytes.
    Key,
    /// It isn't a snapshot this version reads.
    Format,
    /// A segment doesn't open: another key, or bytes changed, cut short or
    /// reordered.
    Tampered,
    /// Sealing failed.
    Seal,
    /// More segments than a counter numbers.
    TooLong,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Key => "the key isn't 32 bytes",
            Self::Format => "it isn't a snapshot this fctl reads",
            Self::Tampered => "it doesn't open with the session's key: changed or cut short",
            Self::Seal => "sealing it failed",
            Self::TooLong => "it is too long",
        })
    }
}

fn cipher(key: &[u8]) -> Result<Aes256Gcm, Error> {
    if key.len() != 32 {
        return Err(Error::Key);
    }
    Aes256Gcm::new_from_slice(key).map_err(|_| Error::Key)
}

fn nonce(prefix: &[u8; PREFIX], counter: u32, last: bool) -> [u8; 12] {
    let mut n = [0u8; 12];
    if let Some(p) = n.get_mut(..PREFIX) {
        p.copy_from_slice(prefix);
    }
    if let Some(c) = n.get_mut(PREFIX..PREFIX + 4) {
        c.copy_from_slice(&counter.to_be_bytes());
    }
    if let Some(l) = n.get_mut(11) {
        *l = u8::from(last);
    }
    n
}

/// Seals a snapshot segment by segment.
pub struct Sealer {
    cipher: Aes256Gcm,
    prefix: [u8; PREFIX],
    counter: u32,
}

impl Sealer {
    /// A sealer for `key`, with the header the sealed file starts with.
    pub fn new(key: &[u8]) -> Result<(Self, Vec<u8>), Error> {
        let prefix: [u8; PREFIX] = crate::random_bytes();
        let mut header = MAGIC.to_vec();
        header.extend_from_slice(&prefix);
        Ok((
            Self {
                cipher: cipher(key)?,
                prefix,
                counter: 0,
            },
            header,
        ))
    }

    /// Seals the next segment, at most [`SEGMENT`] bytes; `last` for the
    /// final one, which may be empty.
    pub fn seal(&mut self, plain: &[u8], last: bool) -> Result<Vec<u8>, Error> {
        let n = nonce(&self.prefix, self.counter, last);
        self.counter = self.counter.checked_add(1).ok_or(Error::TooLong)?;
        self.cipher
            .encrypt(&Nonce::from(n), plain)
            .map_err(|_| Error::Seal)
    }
}

/// Opens a sealed snapshot segment by segment.
pub struct Opener {
    cipher: Aes256Gcm,
    prefix: [u8; PREFIX],
    counter: u32,
}

impl Opener {
    /// An opener for `key`, from the file's first [`HEADER`] bytes.
    pub fn new(key: &[u8], header: &[u8]) -> Result<Self, Error> {
        let (magic, prefix) = header.split_at_checked(MAGIC.len()).ok_or(Error::Format)?;
        if magic != MAGIC || prefix.len() != PREFIX {
            return Err(Error::Format);
        }
        let mut p = [0u8; PREFIX];
        p.copy_from_slice(prefix);
        Ok(Self {
            cipher: cipher(key)?,
            prefix: p,
            counter: 0,
        })
    }

    /// Opens the next sealed segment ([`SEGMENT`] + [`TAG`] bytes, but the
    /// last); `last` for the one the file ends with.
    pub fn open(&mut self, sealed: &[u8], last: bool) -> Result<Vec<u8>, Error> {
        let n = nonce(&self.prefix, self.counter, last);
        self.counter = self.counter.checked_add(1).ok_or(Error::TooLong)?;
        self.cipher
            .decrypt(&Nonce::from(n), sealed)
            .map_err(|_| Error::Tampered)
    }
}

/// Seals `plain` whole.
pub fn seal(key: &[u8], plain: &[u8]) -> Result<Vec<u8>, Error> {
    let (mut sealer, mut out) = Sealer::new(key)?;
    let mut chunks = plain.chunks(SEGMENT).peekable();
    if chunks.peek().is_none() {
        out.extend(sealer.seal(&[], true)?);
    }
    while let Some(chunk) = chunks.next() {
        out.extend(sealer.seal(chunk, chunks.peek().is_none())?);
    }
    Ok(out)
}

/// Opens a snapshot [`seal`] sealed.
pub fn open(key: &[u8], sealed: &[u8]) -> Result<Vec<u8>, Error> {
    let (header, body) = sealed.split_at_checked(HEADER).ok_or(Error::Format)?;
    let mut opener = Opener::new(key, header)?;
    let mut out = Vec::with_capacity(body.len());
    let mut chunks = body.chunks(SEGMENT + TAG).peekable();
    if chunks.peek().is_none() {
        return Err(Error::Tampered);
    }
    while let Some(chunk) = chunks.next() {
        out.extend(opener.open(chunk, chunks.peek().is_none())?);
    }
    Ok(out)
}

// ───────────────────────────── Claude Code's versions ─────────────────────────────

/// `2.1.140` from what `claude --version` says (`2.1.140 (Claude Code)`).
fn version(text: &str) -> Option<(u32, u32, u32)> {
    let v = text.split_whitespace().next()?;
    let mut parts = v.split('.').map(|p| p.parse::<u32>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

/// Why the Claude Code here (`here`) may not read a conversation that
/// `saved` wrote: another major version, or an older one, which may not
/// know what it wrote. `None` when it may, and when either is unknown.
pub fn unreadable_by(saved: Option<&str>, here: Option<&str>) -> Option<String> {
    let (s, h) = (version(saved?)?, version(here?)?);
    if h.0 != s.0 || h < s {
        Some(format!(
            "it was saved by Claude Code {}.{}.{}, which this sandbox's {}.{}.{} may not read",
            s.0, s.1, s.2, h.0, h.1, h.2
        ))
    } else {
        None
    }
}

// ───────────────────────────── conversations too large ─────────────────────────────

/// What stands in for an image a snapshot leaves out.
pub const IMAGE_LEFT_OUT: &str = "[An image fctl left out of this saved conversation to keep it within the size the instance allows]";

/// A conversation (Claude Code's JSON lines) without its images: each
/// base64 image in a message's content, a tool result's too, becomes a line
/// of text that says so. Lines that aren't JSON stay as they are. Returns
/// how many it left out.
pub fn without_images(jsonl: &str) -> (String, usize) {
    let mut left = 0;
    let mut out = String::with_capacity(jsonl.len());
    for line in jsonl.split_inclusive('\n') {
        let (body, end) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        match serde_json::from_str::<Value>(body) {
            Ok(mut v) => {
                let n = strip(&mut v);
                if n == 0 {
                    out.push_str(line);
                } else {
                    left += n;
                    out.push_str(&v.to_string());
                    out.push_str(end);
                }
            }
            Err(_) => out.push_str(line),
        }
    }
    (out, left)
}

/// Replaces the base64 images in `v`'s content, at any depth.
fn strip(v: &mut Value) -> usize {
    match v {
        Value::Object(map) => {
            let image = map.get("type").and_then(Value::as_str) == Some("image")
                && map
                    .get("source")
                    .and_then(|s| s.get("type"))
                    .and_then(Value::as_str)
                    == Some("base64");
            if image {
                *v = serde_json::json!({ "type": "text", "text": IMAGE_LEFT_OUT });
                return 1;
            }
            map.values_mut().map(strip).sum()
        }
        Value::Array(items) => items.iter_mut().map(strip).sum(),
        _ => 0,
    }
}

// ───────────────────────────── retention ─────────────────────────────

/// Days a workspace keeps a session's snapshot after it was saved, unless
/// it chooses otherwise.
pub const DEFAULT_DAYS: u32 = 30;
/// The longest it may keep one.
pub const MAX_DAYS: u32 = 365;
const DAY_MS: i64 = 86_400_000;

/// Whether `days` is a retention a workspace may choose.
pub fn valid_days(days: u32) -> bool {
    (1..=MAX_DAYS).contains(&days)
}

/// When a snapshot saved at `saved_at` goes, kept `days` (the default
/// when `None`).
pub fn expires_at(saved_at: i64, days: Option<u32>) -> i64 {
    saved_at + i64::from(days.unwrap_or(DEFAULT_DAYS)) * DAY_MS
}

// ───────────────────────────── what the transcript says ─────────────────────────────

/// `12 MB`, `840 KB`.
pub fn size_words(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000 => format!("{} MB", b.div_ceil(1_000_000)),
        b if b >= 1_000 => format!("{} KB", b.div_ceil(1_000)),
        b => format!("{b} bytes"),
    }
}

/// What a session that resumed with its snapshot says: `Resumed with its
/// conversation and uncommitted work (12 MB), saved on studio-mac`.
pub fn restored_words(
    conversation: bool,
    changes: bool,
    size: u64,
    saved_on: Option<&str>,
) -> String {
    let what = match (conversation, changes) {
        (true, true) => "its conversation and uncommitted work",
        (true, false) => "its conversation",
        (false, true) => "its uncommitted work",
        (false, false) => "its snapshot",
    };
    let on = saved_on
        .map(|r| format!(", saved on {r}"))
        .unwrap_or_default();
    format!("Resumed with {what} ({}){on}", size_words(size))
}

/// What a session that resumed without its snapshot says, and why
/// (`why`): `Resumed without its conversation: it was saved by an older
/// Claude Code. The agent has the task and the branch.`
pub fn unusable_words(why: &str) -> String {
    format!(
        "Resumed without its conversation: {}. The agent has the task and the branch.",
        why.trim().trim_end_matches('.')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn a_snapshot_opens_with_its_key_alone_and_whole() -> Result<(), Error> {
        for len in [0, 1, SEGMENT - 1, SEGMENT, SEGMENT + 1, 2 * SEGMENT + 17] {
            let plain: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let sealed = seal(&KEY, &plain)?;
            assert_eq!(open(&KEY, &sealed)?, plain, "{len}");
            assert!(sealed.starts_with(MAGIC));
        }
        let plain = vec![1u8; 2 * SEGMENT + 5];
        let sealed = seal(&KEY, &plain)?;
        // Another key.
        assert_eq!(open(&[8; 32], &sealed), Err(Error::Tampered));
        // A byte changed.
        let mut changed = sealed.clone();
        if let Some(b) = changed.get_mut(HEADER + 10) {
            *b ^= 1;
        }
        assert_eq!(open(&KEY, &changed), Err(Error::Tampered));
        // Cut short at a segment's end: the one before wasn't the last.
        let cut = sealed.get(..HEADER + SEGMENT + TAG).unwrap_or_default();
        assert_eq!(open(&KEY, cut), Err(Error::Tampered));
        // Segments swapped.
        let segment = SEGMENT + TAG;
        let mut swapped = sealed.get(..HEADER).unwrap_or_default().to_vec();
        swapped.extend_from_slice(
            sealed
                .get(HEADER + segment..HEADER + 2 * segment)
                .unwrap_or_default(),
        );
        swapped.extend_from_slice(sealed.get(HEADER..HEADER + segment).unwrap_or_default());
        swapped.extend_from_slice(sealed.get(HEADER + 2 * segment..).unwrap_or_default());
        assert_eq!(open(&KEY, &swapped), Err(Error::Tampered));
        // Not a snapshot, or a short key.
        assert_eq!(open(&KEY, b"PK\x03\x04 a zip"), Err(Error::Format));
        assert!(matches!(seal(&[1; 16], b"x"), Err(Error::Key)));
        // Two seals of the same bytes differ.
        assert_ne!(seal(&KEY, b"same")?, seal(&KEY, b"same")?);
        Ok(())
    }

    #[test]
    fn a_newer_claude_code_of_the_same_major_reads_a_conversation() {
        assert_eq!(
            unreadable_by(Some("2.1.140"), Some("2.1.140 (Claude Code)")),
            None
        );
        assert_eq!(unreadable_by(Some("2.1.140"), Some("2.3.1")), None);
        assert_eq!(unreadable_by(None, Some("2.1.140")), None);
        assert_eq!(
            unreadable_by(Some("2.1.150"), Some("2.1.140")).as_deref(),
            Some("it was saved by Claude Code 2.1.150, which this sandbox's 2.1.140 may not read")
        );
        assert!(unreadable_by(Some("2.1.140"), Some("3.0.0")).is_some());
    }

    #[test]
    fn images_go_and_say_so() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"look"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}]}}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":[{"type":"image","source":{"type":"base64","data":"BBBB"}}]}]}}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"a url image stays"},{"type":"image","source":{"type":"url","url":"https://x"}}]}}"#,
            "\n",
            "not json\n",
        );
        let (out, left) = without_images(jsonl);
        assert_eq!(left, 2);
        assert!(!out.contains("AAAA") && !out.contains("BBBB"));
        assert_eq!(out.matches(IMAGE_LEFT_OUT).count(), 2);
        assert!(out.contains("https://x") && out.ends_with("not json\n"));
        assert_eq!(out.lines().count(), 4);
        // Without images, as it was, byte for byte.
        let plain = "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n";
        assert_eq!(without_images(plain), (plain.to_owned(), 0));
    }

    #[test]
    fn words_and_retention() {
        assert_eq!(size_words(12_345_678), "13 MB");
        assert_eq!(size_words(840_100), "841 KB");
        assert_eq!(
            restored_words(true, true, 12_000_000, Some("studio-mac")),
            "Resumed with its conversation and uncommitted work (12 MB), saved on studio-mac"
        );
        assert_eq!(
            unusable_words("it was saved by an older Claude Code."),
            "Resumed without its conversation: it was saved by an older Claude Code. The agent has the task and the branch."
        );
        assert_eq!(expires_at(0, None), 30 * DAY_MS);
        assert_eq!(expires_at(0, Some(7)), 7 * DAY_MS);
        assert!(valid_days(1) && valid_days(365) && !valid_days(0) && !valid_days(366));
    }
}
