//! Understanding Claude Code's stream-json messages: what to store, what to
//! count, and what must stop a session to keep it on the subscription.

use fctl_proto::Millis;
use fctl_proto::session::SessionStats;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;

/// Strings longer than this are truncated before an event is stored.
pub const MAX_STORED_STRING_BYTES: usize = 128 * 1024;
/// Stored events larger than this (serialized) are reduced further.
pub const MAX_STORED_EVENT_BYTES: usize = 1_500_000;
/// Live messages [`Live`] keeps at most: a response that streams more starts
/// over.
pub const LIVE_MAX_EVENTS: usize = 6_000;

/// The kind of a Claude stream-json message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaudeKind {
    /// `system/init`: the session's configuration, once per process.
    Init,
    Assistant,
    /// Usually tool results.
    User,
    /// End of a turn.
    Result,
    RateLimit,
    /// Token deltas (`--include-partial-messages`).
    Stream,
    /// Tool heartbeats.
    ToolProgress,
    /// How far Claude's thinking got (`system/thinking_tokens`): its
    /// estimate of the block's tokens so far, about once a second while it
    /// thinks.
    ThinkingProgress,
    /// Other `system` messages (`status`, `api_retry`, `compact_boundary`, …).
    System,
    Other,
}

pub fn classify(v: &Value) -> ClaudeKind {
    match v.get("type").and_then(Value::as_str) {
        Some("system") => match v.get("subtype").and_then(Value::as_str) {
            Some("init") => ClaudeKind::Init,
            Some("thinking_tokens") => ClaudeKind::ThinkingProgress,
            _ => ClaudeKind::System,
        },
        Some("assistant") => ClaudeKind::Assistant,
        Some("user") => ClaudeKind::User,
        Some("result") => ClaudeKind::Result,
        Some("rate_limit_event") => ClaudeKind::RateLimit,
        Some("stream_event") => ClaudeKind::Stream,
        Some("tool_progress") => ClaudeKind::ToolProgress,
        _ => ClaudeKind::Other,
    }
}

/// Messages that are shown live but not stored.
pub fn is_ephemeral(kind: ClaudeKind) -> bool {
    matches!(
        kind,
        ClaudeKind::Stream | ClaudeKind::ToolProgress | ClaudeKind::ThinkingProgress
    )
}

/// The live messages of the response Claude writes, which SessionDO keeps
/// so that a viewer who joins in the middle of it sees it so far: its
/// stream events and tool heartbeats, and the newest of its thinking
/// progress, which comes about once a second and each time counts the
/// whole block.
#[derive(Clone, Debug, Default)]
pub struct Live {
    /// As JSON, in the order they came.
    messages: Vec<String>,
    /// The newest thinking progress, as JSON.
    thought: Option<String>,
}

impl Live {
    /// Takes a live message `v`, serialized as `text`.
    pub fn push(&mut self, v: &Value, text: &str) {
        // The agent's own, not a subagent's, which viewers don't show.
        let own = own(v);
        if classify(v) == ClaudeKind::ThinkingProgress {
            if own {
                self.thought = Some(text.to_owned());
            }
            return;
        }
        // The agent's next response.
        if own && v.pointer("/event/type").and_then(Value::as_str) == Some("message_start") {
            self.clear();
        }
        if self.messages.len() >= LIVE_MAX_EVENTS {
            self.messages.clear();
        }
        self.messages.push(text.to_owned());
    }

    /// Forgets them: what they streamed is stored, or the turn is over.
    pub fn clear(&mut self) {
        self.messages.clear();
        self.thought = None;
    }

    /// The elements of a JSON array of them (`a,b,c`), the thinking
    /// progress last.
    pub fn to_json_elements(&self) -> String {
        let all: Vec<&str> = self
            .messages
            .iter()
            .chain(&self.thought)
            .map(String::as_str)
            .collect();
        all.join(",")
    }
}

/// Whether a Claude message is the conversation's own, not one of a
/// subagent's (which name the tool call that runs them).
fn own(v: &Value) -> bool {
    v.get("parent_tool_use_id").is_none_or(Value::is_null)
}

/// A reason the session must not continue as is.
#[derive(Clone, Debug, PartialEq)]
pub enum BillingGuard {
    /// Claude authenticated with an API key (pay per token). Refuse to run.
    ApiKey { source: String },
    /// Claude started drawing on paid extra usage. Interrupt and pause.
    Overage,
    /// The subscription limit is reached; pause until `resets_at` (ms).
    LimitReached {
        resets_at: Option<i64>,
        limit: Option<String>,
    },
}

/// Checks a Claude message against the subscription-only rules.
pub fn billing_guard(v: &Value) -> Option<BillingGuard> {
    match classify(v) {
        ClaudeKind::Init => {
            let source = v
                .get("apiKeySource")
                .and_then(Value::as_str)
                .unwrap_or("none");
            // 'none' means no API key (claude.ai OAuth). 'oauth' is a legacy
            // spelling of the same. Everything else bills an API account.
            if source == "none" || source == "oauth" {
                None
            } else {
                Some(BillingGuard::ApiKey {
                    source: source.to_owned(),
                })
            }
        }
        ClaudeKind::RateLimit => {
            let info = v.get("rate_limit_info")?;
            let flag = |k: &str| info.get(k).and_then(Value::as_bool).unwrap_or(false);
            let limit_type = info.get("rateLimitType").and_then(Value::as_str);
            if flag("isUsingOverage") || flag("overageInUse") || limit_type == Some("overage") {
                return Some(BillingGuard::Overage);
            }
            if info.get("status").and_then(Value::as_str) == Some("rejected") {
                let resets_at = info
                    .get("resetsAt")
                    .and_then(Value::as_f64)
                    // The CLI reports seconds; accept milliseconds too.
                    .map(|t| {
                        if t < 1e11 {
                            (t * 1000.0) as i64
                        } else {
                            t as i64
                        }
                    });
                return Some(BillingGuard::LimitReached {
                    resets_at,
                    limit: limit_type.map(str::to_owned),
                });
            }
            None
        }
        _ => None,
    }
}

/// How many API calls [`Counted`] remembers: more than run at once (a
/// session's main thread and its subagents).
const COUNTED_CALLS: usize = 32;

/// The API calls whose tokens the session's counters hold so far. Claude
/// reports a call's usage several times: as its response starts
/// (`message_start`, a stream event) and again in each of its stored content
/// blocks, with its input in full and its output only begun, and as it ends
/// (`message_delta`, a stream event), with its output. Stream events can
/// overtake stored messages or go missing, and the blocks of subagents that
/// run at once interleave.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counted {
    calls: VecDeque<Call>,
    /// The call whose response streams now (the latest `message_start`),
    /// for a `message_delta` that doesn't say which call it ends.
    #[serde(default)]
    streaming: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Call {
    /// The API message id.
    id: String,
    /// Whether its input and cache reads are counted.
    input: bool,
    /// Its output tokens counted.
    output: u64,
}

impl Counted {
    /// The call with this id, remembered from now on if it is new.
    fn call(&mut self, id: &str) -> &mut Call {
        let i = match self.calls.iter().position(|c| c.id == id) {
            Some(i) => i,
            None => {
                if self.calls.len() >= COUNTED_CALLS {
                    self.calls.pop_front();
                }
                self.calls.push_back(Call {
                    id: id.to_owned(),
                    input: false,
                    output: 0,
                });
                self.calls.len() - 1
            }
        };
        &mut self.calls[i]
    }
}

/// Counts what Claude reports of an API call's `usage`: its input and
/// cache reads once (`input`), its output as the running total it is. A
/// call without an id counts each time. The input of a call of the
/// conversation's own (`own`) is what its context holds, as the call
/// starts.
fn count_call(
    stats: &mut SessionStats,
    counted: &mut Counted,
    id: Option<&str>,
    usage: &Value,
    input: bool,
    own: bool,
) {
    let n = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let mut call = id.map(|id| counted.call(id));
    if input && !call.as_ref().is_some_and(|c| c.input) {
        stats.input_tokens = stats.input_tokens.saturating_add(n("input_tokens"));
        stats.cache_read_tokens = stats
            .cache_read_tokens
            .saturating_add(n("cache_read_input_tokens"));
        let context = n("input_tokens")
            .saturating_add(n("cache_creation_input_tokens"))
            .saturating_add(n("cache_read_input_tokens"));
        // Claude Code's messages for failed calls report none.
        if own && context > 0 {
            stats.context_tokens = Some(context);
        }
        if let Some(c) = call.as_mut() {
            c.input = true;
        }
    }
    let so_far = n("output_tokens");
    let before = call.as_ref().map_or(0, |c| c.output);
    if so_far > before {
        stats.output_tokens = stats.output_tokens.saturating_add(so_far - before);
        if let Some(c) = call {
            c.output = so_far;
        }
    }
}

/// Updates session counters from a stored Claude message sent at `ts`:
/// tool calls, each API call's tokens and the context as they come,
/// compactions, and a turn's time and Claude's own token totals when it
/// ends. While the session runs
/// (`running`) a turn is under way: one that no status change started
/// starts at `ts`, as does the next when Claude goes on with a message that
/// came during the last.
pub fn apply_stats(
    stats: &mut SessionStats,
    counted: &mut Counted,
    v: &Value,
    ts: Millis,
    running: bool,
) {
    match classify(v) {
        ClaudeKind::Assistant => {
            let tools = v
                .pointer("/message/content")
                .and_then(Value::as_array)
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                        .count()
                })
                .unwrap_or(0);
            stats.tool_calls = stats.tool_calls.saturating_add(tools as u32);
            if let Some(usage) = v.pointer("/message/usage") {
                let id = v.pointer("/message/id").and_then(Value::as_str);
                count_call(stats, counted, id, usage, true, own(v));
            }
            if running && stats.turn_started_at.is_none() {
                stats.turn_started_at = Some(ts);
            }
        }
        ClaudeKind::Result => {
            stats.turns = stats.turns.saturating_add(1);
            if let Some(ms) = v.get("duration_ms").and_then(Value::as_u64) {
                stats.busy_ms = stats.busy_ms.saturating_add(ms);
            }
            // modelUsage is cumulative for the process, and has the calls
            // that stream nothing too (compaction, small models). A resumed
            // session's process knows only part of it: the larger count wins.
            if let Some(models) = v.get("modelUsage").and_then(Value::as_object) {
                let sum = |key: &str| -> u64 {
                    models
                        .values()
                        .filter_map(|m| m.get(key).and_then(Value::as_u64))
                        .sum()
                };
                stats.input_tokens = stats.input_tokens.max(sum("inputTokens"));
                stats.output_tokens = stats.output_tokens.max(sum("outputTokens"));
                stats.cache_read_tokens = stats.cache_read_tokens.max(sum("cacheReadInputTokens"));
            }
            // Claude goes on with a message that came during the turn.
            let queued = v
                .get("queued_turn_count")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0;
            stats.turn_started_at = (running && queued).then_some(ts);
        }
        // Claude compacted the conversation: its context holds what the
        // compaction left (absent from Claude Code before `post_tokens`)
        // until the next call says.
        ClaudeKind::System
            if v.get("subtype").and_then(Value::as_str) == Some("compact_boundary") && own(v) =>
        {
            stats.compactions = stats.compactions.saturating_add(1);
            if let Some(left) = v
                .pointer("/compact_metadata/post_tokens")
                .and_then(Value::as_u64)
            {
                stats.context_tokens = Some(left);
            }
        }
        _ => {}
    }
}

/// Whether a stream event reports an API call's tokens: the one that starts
/// its response and the one that ends it. The others come by the dozen a
/// second and count nothing.
pub fn reports_usage(v: &Value) -> bool {
    matches!(
        v.pointer("/event/type").and_then(Value::as_str),
        Some("message_start" | "message_delta")
    )
}

/// Updates session counters from a stream event ([`reports_usage`]): an API
/// call's input, and the context, as its response starts, before any of it
/// is stored, and its output, which only the end has. Returns whether they
/// changed.
pub fn apply_stream_stats(stats: &mut SessionStats, counted: &mut Counted, v: &Value) -> bool {
    let before = stats.clone();
    match v.pointer("/event/type").and_then(Value::as_str) {
        Some("message_start") => {
            let id = v.pointer("/event/message/id").and_then(Value::as_str);
            counted.streaming = id.map(str::to_owned);
            if let Some(usage) = v.pointer("/event/message/usage") {
                count_call(stats, counted, id, usage, true, own(v));
            }
        }
        // It repeats the input, which the call's start or its stored blocks
        // count. Without the call's id, it ends the call that streams.
        Some("message_delta") => {
            let id = v
                .get("api_message_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| counted.streaming.clone());
            if let (Some(id), Some(usage)) = (id, v.pointer("/event/usage")) {
                count_call(stats, counted, Some(&id), usage, false, own(v));
            }
        }
        _ => {}
    }
    *stats != before
}

/// Follows the session's status: a turn is under way while it runs.
pub fn apply_status(stats: &mut SessionStats, running: bool, now: Millis) {
    if !running {
        stats.turn_started_at = None;
    } else if stats.turn_started_at.is_none() {
        stats.turn_started_at = Some(now);
    }
}

/// A kind and weight for the activity visualization.
pub fn activity(v: &Value) -> Option<(&'static str, u32)> {
    match classify(v) {
        ClaudeKind::Assistant => {
            let blocks = v.pointer("/message/content").and_then(Value::as_array)?;
            let kind = if blocks
                .iter()
                .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
            {
                "tool_use"
            } else if blocks
                .iter()
                .any(|b| b.get("type").and_then(Value::as_str) == Some("thinking"))
            {
                "thinking"
            } else {
                "text"
            };
            let weight = blocks
                .iter()
                .map(|b| {
                    b.get("text")
                        .or_else(|| b.get("thinking"))
                        .and_then(Value::as_str)
                        .map_or(40, |s| s.len() as u32)
                })
                .sum::<u32>()
                .clamp(1, 10_000);
            Some((kind, weight))
        }
        ClaudeKind::User => Some(("tool_result", 20)),
        ClaudeKind::Result => Some(("result", 200)),
        ClaudeKind::Stream => Some(("delta", 1)),
        _ => None,
    }
}

/// Shrinks a Claude message so it can be stored: long strings are cut, and
/// if it is still too large, the structured `tool_use_result` copy goes.
/// Returns whether anything was removed.
pub fn shape_for_storage(v: &mut Value) -> bool {
    let mut changed = omit_base64(v);
    changed |= omit_read_copy(v);
    changed |= truncate_strings(v, MAX_STORED_STRING_BYTES);
    if serialized_len(v) > MAX_STORED_EVENT_BYTES
        && let Some(obj) = v.as_object_mut()
        && obj.remove("tool_use_result").is_some()
    {
        changed = true;
    }
    let mut limit = MAX_STORED_STRING_BYTES / 2;
    while serialized_len(v) > MAX_STORED_EVENT_BYTES && limit >= 256 {
        changed |= truncate_strings(v, limit);
        limit /= 2;
    }
    changed
}

/// Empties the data of base64 sources (images and documents Claude read):
/// the session keeps the images apart (`kept_images`), and truncated they
/// are useless. `omitted_bytes` says how much there was.
fn omit_base64(v: &mut Value) -> bool {
    match v {
        Value::Object(map) => {
            let source = map.get("type").and_then(Value::as_str) == Some("base64");
            match map.get_mut("data") {
                Some(Value::String(data)) if source && data.len() > 1024 => {
                    let n = data.len();
                    data.clear();
                    map.insert("omitted_bytes".into(), Value::from(n));
                    true
                }
                _ => map.values_mut().fold(false, |acc, x| omit_base64(x) | acc),
            }
        }
        Value::Array(items) => items.iter_mut().fold(false, |acc, x| omit_base64(x) | acc),
        _ => false,
    }
}

/// Empties Claude Code's own copy of an image or a document its `Read`
/// read (`tool_use_result.file.base64`), as [`omit_base64`] does the copy
/// Claude got. Other fields named `base64` stay, within the usual limits.
fn omit_read_copy(v: &mut Value) -> bool {
    let Some(file) = v
        .pointer_mut("/tool_use_result/file")
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    match file.get_mut("base64") {
        Some(Value::String(data)) if data.len() > 1024 => {
            let n = data.len();
            data.clear();
            file.insert("omitted_bytes".into(), Value::from(n));
            true
        }
        _ => false,
    }
}

fn serialized_len(v: &Value) -> usize {
    serde_json::to_vec(v).map(|b| b.len()).unwrap_or(usize::MAX)
}

fn truncate_strings(v: &mut Value, max: usize) -> bool {
    match v {
        Value::String(s) if s.len() > max => {
            let removed = s.len() - max;
            let mut cut = max;
            while !s.is_char_boundary(cut) {
                cut -= 1;
            }
            s.truncate(cut);
            s.push_str(&format!("\n… [{removed} bytes truncated by fctl]"));
            true
        }
        Value::Array(items) => items
            .iter_mut()
            .fold(false, |acc, i| truncate_strings(i, max) | acc),
        Value::Object(map) => map
            .values_mut()
            .fold(false, |acc, i| truncate_strings(i, max) | acc),
        _ => false,
    }
}

/// The text of a user message's string or text-block content.
pub fn user_text(v: &Value) -> Option<String> {
    let content = v.pointer("/message/content")?;
    if let Some(s) = content.as_str() {
        return Some(s.to_owned());
    }
    let parts: Vec<&str> = content
        .as_array()?
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn api_keys_are_refused() {
        let ok = json!({"type": "system", "subtype": "init", "apiKeySource": "none"});
        assert_eq!(billing_guard(&ok), None);
        let bad = json!({"type": "system", "subtype": "init", "apiKeySource": "ANTHROPIC_API_KEY"});
        assert!(matches!(
            billing_guard(&bad),
            Some(BillingGuard::ApiKey { .. })
        ));
        let helper = json!({"type": "system", "subtype": "init", "apiKeySource": "apiKeyHelper"});
        assert!(matches!(
            billing_guard(&helper),
            Some(BillingGuard::ApiKey { .. })
        ));
    }

    #[test]
    fn overage_and_limits() {
        let over = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "isUsingOverage": true}});
        assert_eq!(billing_guard(&over), Some(BillingGuard::Overage));
        let over2 = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed_warning", "rateLimitType": "overage"}});
        assert_eq!(billing_guard(&over2), Some(BillingGuard::Overage));
        let limit = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "rejected", "resetsAt": 1_760_000_000, "rateLimitType": "five_hour"}});
        assert_eq!(
            billing_guard(&limit),
            Some(BillingGuard::LimitReached {
                resets_at: Some(1_760_000_000_000),
                limit: Some("five_hour".into())
            })
        );
        let fine = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed_warning", "utilization": 0.9}});
        assert_eq!(billing_guard(&fine), None);
    }

    #[test]
    fn thinking_progress_is_live_only() {
        let progress = json!({"type": "system", "subtype": "thinking_tokens", "estimated_tokens": 900,
                              "estimated_tokens_delta": 100, "session_id": "s", "uuid": "u"});
        assert_eq!(classify(&progress), ClaudeKind::ThinkingProgress);
        assert!(is_ephemeral(classify(&progress)));
        // Claude's other word is kept.
        let status = json!({"type": "system", "subtype": "status", "status": null});
        assert_eq!(classify(&status), ClaudeKind::System);
        assert!(!is_ephemeral(classify(&status)));
        let init = json!({"type": "system", "subtype": "init", "model": "claude-opus-5-5"});
        assert!(!is_ephemeral(classify(&init)));
    }

    /// Hands `live` a stream event of the agent's, or of the subagent that
    /// `parent` started.
    fn stream(live: &mut Live, event: Value, parent: Option<&str>) {
        let v = json!({"type": "stream_event", "event": event, "parent_tool_use_id": parent});
        live.push(&v, &v.to_string());
    }

    fn thinking_progress(live: &mut Live, tokens: u64) {
        let v = json!({"type": "system", "subtype": "thinking_tokens", "estimated_tokens": tokens});
        live.push(&v, &v.to_string());
    }

    /// What a snapshot gives a viewer that joins now.
    fn snapshot(live: &Live) -> Vec<Value> {
        serde_json::from_str(&format!("[{}]", live.to_json_elements())).unwrap_or_default()
    }

    #[test]
    fn live_keeps_a_response_and_its_newest_thinking_progress() {
        let mut live = Live::default();
        let start = |id: &str| json!({"type": "message_start", "message": {"id": id}});
        stream(&mut live, start("m1"), None);
        let thought = json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking"}});
        stream(&mut live, thought, None);
        // A long think keeps one, the newest.
        for n in 1..=600 {
            thinking_progress(&mut live, n * 50);
        }
        let seen = snapshot(&live);
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0]["event"]["type"], "message_start");
        assert_eq!(seen[2]["estimated_tokens"], 30_000);
        // A subagent's response leaves the agent's, and so does its
        // thinking progress, if Claude Code ever sends that.
        stream(&mut live, start("m2"), Some("toolu_1"));
        let theirs = json!({"type": "system", "subtype": "thinking_tokens", "estimated_tokens": 5,
                            "parent_tool_use_id": "toolu_1"});
        live.push(&theirs, &theirs.to_string());
        let seen = snapshot(&live);
        assert_eq!(seen.len(), 4);
        assert_eq!(seen[3]["estimated_tokens"], 30_000);
        // The agent's next response starts over.
        stream(&mut live, start("m3"), None);
        let seen = snapshot(&live);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["event"]["message"]["id"], "m3");
        // A response that streams too much starts over too, its thinking
        // progress kept.
        thinking_progress(&mut live, 50);
        for _ in 0..LIVE_MAX_EVENTS {
            stream(
                &mut live,
                json!({"type": "content_block_delta", "index": 0}),
                None,
            );
        }
        let seen = snapshot(&live);
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["event"]["type"], "content_block_delta");
        assert_eq!(seen[1]["estimated_tokens"], 50);
        live.clear();
        assert!(snapshot(&live).is_empty());
    }

    #[test]
    fn stats() {
        let mut s = SessionStats::default();
        let mut c = Counted::default();
        apply_stats(
            &mut s,
            &mut c,
            &json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "x"}, {"type": "tool_use", "id": "1"}, {"type": "tool_use", "id": "2"}]}}),
            1,
            true,
        );
        apply_stats(
            &mut s,
            &mut c,
            &json!({"type": "result", "duration_ms": 1500, "modelUsage": {"a": {"inputTokens": 10, "outputTokens": 5, "cacheReadInputTokens": 100}, "b": {"inputTokens": 1, "outputTokens": 1, "cacheReadInputTokens": 0}}}),
            2,
            true,
        );
        apply_stats(
            &mut s,
            &mut c,
            &json!({"type": "result", "duration_ms": 500, "modelUsage": {"a": {"inputTokens": 20, "outputTokens": 9, "cacheReadInputTokens": 150}}}),
            3,
            true,
        );
        assert_eq!((s.tool_calls, s.turns, s.busy_ms), (2, 2, 2000));
        assert_eq!(
            (s.input_tokens, s.output_tokens, s.cache_read_tokens),
            (20, 9, 150)
        );
    }

    /// One content block of an API call, with the usage Claude Code puts in
    /// each: the call's input, and the output when the response started.
    fn block(id: &str, parent: Option<&str>, input: u64, cache_read: u64) -> Value {
        json!({"type": "assistant", "parent_tool_use_id": parent, "message": {"id": id, "content": [{"type": "tool_use", "id": "t"}],
            "usage": {"input_tokens": input, "cache_read_input_tokens": cache_read, "cache_creation_input_tokens": 500, "output_tokens": 8}}})
    }

    fn delta(id: Option<&str>, output: u64) -> Value {
        let mut v = json!({"type": "stream_event", "parent_tool_use_id": null, "event": {"type": "message_delta",
            "delta": {"stop_reason": "tool_use"}, "usage": {"input_tokens": 2, "cache_read_input_tokens": 10_000, "output_tokens": output}}});
        if let Some(id) = id {
            v["api_message_id"] = json!(id);
        }
        v
    }

    #[test]
    fn tokens_count_as_calls_come() {
        let mut s = SessionStats::default();
        let mut c = Counted::default();
        // Each block of a call repeats its usage; tool results come between.
        apply_stats(&mut s, &mut c, &block("a", None, 2, 10_000), 1, true);
        apply_stats(&mut s, &mut c, &json!({"type": "user"}), 2, true);
        apply_stats(&mut s, &mut c, &block("a", None, 2, 10_000), 3, true);
        assert_eq!(
            (s.input_tokens, s.output_tokens, s.cache_read_tokens),
            (2, 8, 10_000)
        );
        assert_eq!(s.tool_calls, 2);
        // The output so far comes with `message_delta`, once or more.
        assert!(apply_stream_stats(&mut s, &mut c, &delta(Some("a"), 300)));
        assert!(!apply_stream_stats(&mut s, &mut c, &delta(Some("a"), 300)));
        assert_eq!(s.output_tokens, 300);
        // A call's `message_delta` can overtake its stored messages.
        assert!(apply_stream_stats(&mut s, &mut c, &delta(Some("b"), 40)));
        apply_stats(&mut s, &mut c, &block("b", None, 3, 12_000), 4, true);
        assert_eq!(
            (s.input_tokens, s.output_tokens, s.cache_read_tokens),
            (5, 340, 22_000)
        );
        // A call's input counts as its response starts, before any of it is
        // stored...
        let start = json!({"type": "stream_event", "event": {"type": "message_start", "message": {"id": "c",
            "usage": {"input_tokens": 1, "cache_read_input_tokens": 3_000, "output_tokens": 1}}}});
        assert!(reports_usage(&start));
        assert!(apply_stream_stats(&mut s, &mut c, &start));
        assert_eq!(
            (s.input_tokens, s.output_tokens, s.cache_read_tokens),
            (6, 341, 25_000)
        );
        // ...and once: its stored blocks repeat it.
        apply_stats(&mut s, &mut c, &block("c", None, 1, 3_000), 5, true);
        assert_eq!(
            (s.input_tokens, s.output_tokens, s.cache_read_tokens),
            (6, 348, 25_000)
        );
        // A `message_delta` that doesn't name its call (older Claude Code)
        // ends the one that streams.
        assert!(apply_stream_stats(&mut s, &mut c, &delta(None, 50)));
        assert_eq!(s.output_tokens, 390);
        // The other stream events count nothing.
        let text = json!({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "hi"}}});
        assert!(!reports_usage(&text));
        assert!(!apply_stream_stats(&mut s, &mut c, &text));
        // Claude's totals settle the counts at the end of the turn...
        let result = |input: u64, output: u64, cache: u64| json!({"type": "result", "duration_ms": 9000, "modelUsage": {"m": {"inputTokens": input, "outputTokens": output, "cacheReadInputTokens": cache}}});
        apply_stats(&mut s, &mut c, &result(7, 420, 30_000), 6, true);
        assert_eq!(
            (s.input_tokens, s.output_tokens, s.cache_read_tokens),
            (7, 420, 30_000)
        );
        // ...unless a resumed session's process knows less than was counted.
        apply_stats(&mut s, &mut c, &block("d", None, 2, 5_000), 7, true);
        apply_stats(&mut s, &mut c, &result(2, 8, 5_000), 8, true);
        assert_eq!(
            (s.input_tokens, s.output_tokens, s.cache_read_tokens),
            (9, 428, 35_000)
        );
    }

    #[test]
    fn the_context_is_the_input_of_the_conversations_latest_call() {
        let mut s = SessionStats::default();
        let mut c = Counted::default();
        // Its input, cache reads and cache writes.
        apply_stats(&mut s, &mut c, &block("a", None, 2, 10_000), 1, true);
        assert_eq!(s.context_tokens, Some(10_502));
        // Subagents' calls have contexts of their own.
        apply_stats(&mut s, &mut c, &block("x", Some("t1"), 1, 90_000), 2, true);
        let start = |id: &str, parent: Option<&str>, cache_read: u64| {
            json!({"type": "stream_event", "parent_tool_use_id": parent, "event": {"type": "message_start", "message": {"id": id,
                "usage": {"input_tokens": 1, "cache_read_input_tokens": cache_read, "cache_creation_input_tokens": 300, "output_tokens": 1}}}})
        };
        assert!(apply_stream_stats(
            &mut s,
            &mut c,
            &start("y", Some("t1"), 95_000)
        ));
        assert_eq!(s.context_tokens, Some(10_502));
        // The next call's, as its response starts...
        assert!(apply_stream_stats(
            &mut s,
            &mut c,
            &start("b", None, 12_000)
        ));
        assert_eq!(s.context_tokens, Some(12_301));
        // ...which its output and stored blocks don't change, nor an
        // earlier call's block that comes late.
        assert!(apply_stream_stats(&mut s, &mut c, &delta(Some("b"), 900)));
        apply_stats(&mut s, &mut c, &block("b", None, 1, 12_000), 3, true);
        apply_stats(&mut s, &mut c, &block("a", None, 2, 10_000), 4, true);
        assert_eq!(s.context_tokens, Some(12_301));
        // A call that failed reports no usage.
        let failed = json!({"type": "assistant", "parent_tool_use_id": null, "message": {"id": "e", "content": [{"type": "text", "text": "API Error"}],
            "usage": {"input_tokens": 0, "output_tokens": 0}}});
        apply_stats(&mut s, &mut c, &failed, 5, true);
        assert_eq!(s.context_tokens, Some(12_301));
    }

    #[test]
    fn compactions_leave_a_smaller_context() {
        let mut s = SessionStats::default();
        let mut c = Counted::default();
        apply_stats(&mut s, &mut c, &block("a", None, 2, 160_000), 1, true);
        let boundary = |metadata: Value| json!({"type": "system", "subtype": "compact_boundary", "compact_metadata": metadata});
        apply_stats(
            &mut s,
            &mut c,
            &boundary(json!({"trigger": "auto", "pre_tokens": 160_502, "post_tokens": 9_000})),
            2,
            true,
        );
        assert_eq!((s.compactions, s.context_tokens), (1, Some(9_000)));
        // The next call says what the context holds.
        apply_stats(&mut s, &mut c, &block("b", None, 3, 20_000), 3, true);
        assert_eq!(s.context_tokens, Some(20_503));
        // Claude Code before `post_tokens` doesn't say what was left.
        apply_stats(
            &mut s,
            &mut c,
            &boundary(json!({"trigger": "manual", "pre_tokens": 20_503})),
            4,
            true,
        );
        assert_eq!((s.compactions, s.context_tokens), (2, Some(20_503)));
        // Only the conversation's own count.
        let mut theirs = boundary(json!({"trigger": "auto", "pre_tokens": 100, "post_tokens": 10}));
        theirs["parent_tool_use_id"] = json!("t1");
        apply_stats(&mut s, &mut c, &theirs, 5, true);
        assert_eq!((s.compactions, s.context_tokens), (2, Some(20_503)));
    }

    #[test]
    fn subagents_running_at_once_count_once_each() {
        let mut s = SessionStats::default();
        let mut c = Counted::default();
        for v in [
            block("x", Some("t1"), 1, 100),
            block("y", Some("t2"), 1, 200),
            block("x", Some("t1"), 1, 100),
            block("y", Some("t2"), 1, 200),
        ] {
            apply_stats(&mut s, &mut c, &v, 1, true);
        }
        assert_eq!((s.input_tokens, s.cache_read_tokens), (2, 300));
        // It remembers the latest calls only.
        for i in 0..40 {
            apply_stats(
                &mut s,
                &mut c,
                &block(&format!("m{i}"), None, 1, 0),
                2,
                true,
            );
        }
        assert_eq!(c.calls.len(), COUNTED_CALLS);
        assert_eq!(s.input_tokens, 42);
    }

    #[test]
    fn turns_under_way() {
        let mut s = SessionStats::default();
        let mut c = Counted::default();
        // The session runs: a turn starts.
        apply_status(&mut s, true, 1000);
        assert_eq!((s.turn_started_at, s.turns_taken()), (Some(1000), 1));
        // Its messages and status reports while it runs keep its start.
        apply_stats(&mut s, &mut c, &block("a", None, 1, 0), 2000, true);
        apply_status(&mut s, true, 2500);
        assert_eq!(s.turn_started_at, Some(1000));
        // Its result ends it.
        let result = |queued: u64| json!({"type": "result", "duration_ms": 1800, "queued_turn_count": queued});
        apply_stats(&mut s, &mut c, &result(0), 3000, true);
        assert_eq!((s.turn_started_at, s.turns, s.busy_ms), (None, 1, 1800));
        // Claude goes on with a message that came meanwhile.
        apply_status(&mut s, true, 4000);
        apply_stats(&mut s, &mut c, &result(1), 5000, true);
        assert_eq!((s.turn_started_at, s.turns_taken()), (Some(5000), 3));
        // A session that stops running has no turn under way...
        apply_status(&mut s, false, 6000);
        assert_eq!((s.turn_started_at, s.turns_taken()), (None, 2));
        // ...even when what it sent before comes after (a host that
        // reconnects reports its state first).
        apply_stats(&mut s, &mut c, &block("b", None, 1, 0), 5500, false);
        apply_stats(&mut s, &mut c, &result(0), 5900, false);
        assert_eq!((s.turn_started_at, s.turns, s.busy_ms), (None, 3, 5400));
        // A turn that was under way when no status change said so (a
        // control plane that didn't follow turns yet) starts with its next
        // message.
        apply_stats(&mut s, &mut c, &block("c", None, 1, 0), 7000, true);
        assert_eq!(s.turn_started_at, Some(7000));
    }

    #[test]
    fn stats_without_a_turn_under_way_serialize_as_before() {
        let s = SessionStats {
            turns: 2,
            ..Default::default()
        };
        let v = serde_json::to_value(&s).unwrap_or_default();
        assert!(v.get("turn_started_at").is_none());
        assert_eq!(v["turns"], 2);
        // What older control planes stored reads as no turn under way.
        let old: SessionStats =
            serde_json::from_value(json!({"turns": 1, "busy_ms": 5})).unwrap_or_default();
        assert_eq!((old.turns, old.busy_ms, old.turn_started_at), (1, 5, None));
    }

    #[test]
    fn storage_shaping() {
        let mut v = json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "é".repeat(200_000)}]}, "tool_use_result": {"stdout": "x".repeat(2_000_000)}});
        assert!(shape_for_storage(&mut v));
        assert!(serialized_len(&v) <= MAX_STORED_EVENT_BYTES);
        let small = &mut json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "hi"}]}});
        assert!(!shape_for_storage(small));
    }

    #[test]
    fn storage_omits_base64_images() {
        let data = "iVBORw0KGgo".repeat(1000);
        let mut v = json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": data}},
            {"type": "text", "text": "x".repeat(2000)}
        ]}]}, "tool_use_result": {"type": "image", "file": {"base64": data, "type": "image/png", "originalSize": 8250}}});
        assert!(shape_for_storage(&mut v));
        let source = &v["message"]["content"][0]["content"][0]["source"];
        assert_eq!(source["data"], "");
        assert_eq!(source["omitted_bytes"], 11_000);
        assert_eq!(source["media_type"], "image/png");
        // Claude Code's own copy of what it read, too.
        let file = &v["tool_use_result"]["file"];
        assert_eq!(file["base64"], "");
        assert_eq!(file["omitted_bytes"], 11_000);
        assert_eq!(file["originalSize"], 8250);
        // Other fields named so stay: a tool's input, an MCP tool's result.
        let mut input = json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": "mcp__files__upload", "input": {"name": "a.png", "base64": data}}
        ]}});
        assert!(!shape_for_storage(&mut input));
        assert_eq!(input["message"]["content"][0]["input"]["base64"], data);
        let mut result = json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "ok"}]},
            "tool_use_result": {"base64": data, "file": {"name": "a.png"}}});
        assert!(!shape_for_storage(&mut result));
        assert_eq!(result["tool_use_result"]["base64"], data);
        // Other long strings stay (within the usual limits).
        assert_eq!(
            v["message"]["content"][0]["content"][1]["text"]
                .as_str()
                .map(str::len),
            Some(2000)
        );
    }

    #[test]
    fn user_texts() {
        assert_eq!(
            user_text(&json!({"message": {"content": "hi"}})).as_deref(),
            Some("hi")
        );
        assert_eq!(
            user_text(&json!({"message": {"content": [{"type": "text", "text": "a"}, {"type": "tool_result"}, {"type": "text", "text": "b"}]}})).as_deref(),
            Some("a\nb")
        );
    }
}
