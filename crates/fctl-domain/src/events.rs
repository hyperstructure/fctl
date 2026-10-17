//! Understanding Claude Code's stream-json messages: what to store, what to
//! count, and what must stop a session to keep it on the subscription.

use fctl_proto::session::SessionStats;
use serde_json::Value;

/// Strings longer than this are truncated before an event is stored.
pub const MAX_STORED_STRING_BYTES: usize = 128 * 1024;
/// Stored events larger than this (serialized) are reduced further.
pub const MAX_STORED_EVENT_BYTES: usize = 1_500_000;

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
    /// Other `system` messages (`status`, `api_retry`, `compact_boundary`, …).
    System,
    Other,
}

pub fn classify(v: &Value) -> ClaudeKind {
    match v.get("type").and_then(Value::as_str) {
        Some("system") => match v.get("subtype").and_then(Value::as_str) {
            Some("init") => ClaudeKind::Init,
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
    matches!(kind, ClaudeKind::Stream | ClaudeKind::ToolProgress)
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

/// Updates session counters from a Claude message.
pub fn apply_stats(stats: &mut SessionStats, v: &Value) {
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
        }
        ClaudeKind::Result => {
            stats.turns = stats.turns.saturating_add(1);
            if let Some(ms) = v.get("duration_ms").and_then(Value::as_u64) {
                stats.busy_ms = stats.busy_ms.saturating_add(ms);
            }
            // modelUsage is cumulative for the process: the latest result wins.
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
        }
        _ => {}
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
    let mut changed = truncate_strings(v, MAX_STORED_STRING_BYTES);
    if serialized_len(v) > MAX_STORED_EVENT_BYTES
        && let Some(obj) = v.as_object_mut()
            && obj.remove("tool_use_result").is_some() {
                changed = true;
            }
    let mut limit = MAX_STORED_STRING_BYTES / 2;
    while serialized_len(v) > MAX_STORED_EVENT_BYTES && limit >= 256 {
        changed |= truncate_strings(v, limit);
        limit /= 2;
    }
    changed
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
    fn stats() {
        let mut s = SessionStats::default();
        apply_stats(
            &mut s,
            &json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "x"}, {"type": "tool_use", "id": "1"}, {"type": "tool_use", "id": "2"}]}}),
        );
        apply_stats(
            &mut s,
            &json!({"type": "result", "duration_ms": 1500, "modelUsage": {"a": {"inputTokens": 10, "outputTokens": 5, "cacheReadInputTokens": 100}, "b": {"inputTokens": 1, "outputTokens": 1, "cacheReadInputTokens": 0}}}),
        );
        apply_stats(
            &mut s,
            &json!({"type": "result", "duration_ms": 500, "modelUsage": {"a": {"inputTokens": 20, "outputTokens": 9, "cacheReadInputTokens": 150}}}),
        );
        assert_eq!((s.tool_calls, s.turns, s.busy_ms), (2, 2, 2000));
        assert_eq!(
            (s.input_tokens, s.output_tokens, s.cache_read_tokens),
            (20, 9, 150)
        );
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
