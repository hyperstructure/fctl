//! Claude plan usage: what a `rate_limit_event` says about each of a
//! subscription's usage limits (the five-hour window, the week, a model's
//! own weekly limit), so the control plane can keep the latest per
//! credential and people see how much of their plan is left.
//!
//! Claude Code reports the limit closest to stopping requests at the top of
//! `rate_limit_info` (`rateLimitType`, `status`, `utilization`, `resetsAt`).
//! Recent versions also send `unifiedWindows`, the five-hour and weekly
//! windows, and report again whenever one of them moves by a percent; older
//! versions report only when the status changes.

use fctl_proto::Millis;
use fctl_proto::api::{LimitStatus, PlanLimit};
use serde_json::Value;

/// Paid extra usage, which fctl never lets a session use: not a plan limit.
const OVERAGE: &str = "overage";
/// Usage runs a little past a limit at most; beyond this it is not a share.
const MAX_UTILIZATION: f64 = 10.0;
/// Claude reports a handful of limits. The event comes from the sandbox, so
/// one can't add more than this.
pub const MAX_LIMITS: usize = 8;

/// The plan limits a Claude message reports, one per window, in display
/// order and stamped `observed_at`. Empty for anything but a
/// `rate_limit_event`.
pub fn readings(v: &Value, observed_at: Millis) -> Vec<PlanLimit> {
    if v.get("type").and_then(Value::as_str) != Some("rate_limit_event") {
        return Vec::new();
    }
    let Some(info) = v.get("rate_limit_info") else {
        return Vec::new();
    };
    let mut out: Vec<PlanLimit> = Vec::new();
    if let Some(windows) = info.get("unifiedWindows").and_then(Value::as_object) {
        let windows = windows
            .iter()
            .filter(|(name, _)| is_window(name))
            .take(MAX_LIMITS);
        for (name, w) in windows {
            let reading = PlanLimit {
                window: name.clone(),
                utilization: utilization(w.get("utilization")),
                resets_at: w.get("resetsAt").and_then(epoch_ms),
                status: None,
                observed_at,
            };
            if reading.utilization.is_some() || reading.resets_at.is_some() {
                out.push(reading);
            }
        }
    }
    // The limit closest to stopping requests: its status, and its numbers
    // when the windows above leave it out (a model's weekly limit, or an
    // older Claude Code).
    if let Some(name) = info
        .get("rateLimitType")
        .and_then(Value::as_str)
        .filter(|n| is_window(n))
    {
        let status = info
            .get("status")
            .and_then(Value::as_str)
            .and_then(LimitStatus::parse);
        let used = utilization(info.get("utilization"));
        let resets_at = info.get("resetsAt").and_then(epoch_ms);
        match out.iter_mut().find(|l| l.window == name) {
            Some(l) => {
                l.status = status;
                l.utilization = l.utilization.or(used);
                l.resets_at = l.resets_at.or(resets_at);
            }
            None if status.is_some() || used.is_some() || resets_at.is_some() => {
                out.push(PlanLimit {
                    window: name.to_owned(),
                    utilization: used,
                    resets_at,
                    status,
                    observed_at,
                });
            }
            None => {}
        }
    }
    sort(&mut out);
    out
}

/// Plan usage as the control plane stores it: a JSON array of readings
/// (`window`, `utilization`, `resets_at`, `status`, `observed_at`), in
/// display order. Anything unreadable is left out.
pub fn from_json(json: &str) -> Vec<PlanLimit> {
    #[derive(serde::Deserialize)]
    struct Row {
        window: String,
        utilization: Option<f64>,
        resets_at: Option<f64>,
        status: Option<String>,
        observed_at: f64,
    }
    let rows: Vec<Row> = serde_json::from_str(json).unwrap_or_default();
    let mut limits: Vec<PlanLimit> = rows
        .into_iter()
        .map(|r| PlanLimit {
            window: r.window,
            utilization: r.utilization,
            resets_at: r.resets_at.map(|t| t as Millis),
            status: r.status.as_deref().and_then(LimitStatus::parse),
            observed_at: r.observed_at as Millis,
        })
        .collect();
    sort(&mut limits);
    limits
}

/// Adds newer readings to `limits`: each replaces the reading of its window.
pub fn merge(limits: &mut Vec<PlanLimit>, newer: Vec<PlanLimit>) {
    for reading in newer {
        match limits.iter_mut().find(|l| l.window == reading.window) {
            Some(l) => *l = reading,
            None => limits.push(reading),
        }
    }
    sort(limits);
}

/// Display order: the five-hour window, the week, then each model's weekly
/// limit by name.
pub fn sort(limits: &mut [PlanLimit]) {
    let rank = |w: &str| match w {
        "five_hour" => 0,
        "seven_day" => 1,
        _ => 2,
    };
    limits.sort_by(|a, b| {
        rank(&a.window)
            .cmp(&rank(&b.window))
            .then_with(|| a.window.cmp(&b.window))
    });
}

/// What a limit is called: "5-hour limit", "Weekly limit", "Weekly Opus
/// limit".
pub fn label(window: &str) -> String {
    let words = |s: &str| -> String {
        let s = s.replace('_', " ");
        let mut c = s.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default()
    };
    match window {
        "five_hour" => "5-hour limit".into(),
        "seven_day" => "Weekly limit".into(),
        // Claude Code calls this one the Fable limit.
        "seven_day_overage_included" => "Weekly Fable limit".into(),
        w => match w.strip_prefix("seven_day_") {
            Some(model) => format!("Weekly {} limit", words(model)),
            None => format!("{} limit", words(w)),
        },
    }
}

/// Where a limit stands at `now`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Plenty left, or nothing known about the share.
    Fine,
    /// Most of it is used: Claude warned, or 80 % is gone.
    Close,
    /// Used up: sessions pause until it resets.
    Reached,
    /// The window started over since the reading, so the share is stale.
    Reset,
}

/// A share this high counts as close to the limit.
pub const CLOSE: f64 = 0.8;

pub fn standing(limit: &PlanLimit, now: Millis) -> Standing {
    let used = limit.utilization.unwrap_or(0.0);
    if limit.resets_at.is_some_and(|t| t <= now) {
        Standing::Reset
    } else if limit.status == Some(LimitStatus::Rejected) || used >= 1.0 {
        Standing::Reached
    } else if limit.status == Some(LimitStatus::AllowedWarning) || used >= CLOSE {
        Standing::Close
    } else {
        Standing::Fine
    }
}

/// A window name as Claude writes them (`five_hour`, `seven_day_opus`),
/// other than paid extra usage.
fn is_window(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name != OVERAGE
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn utilization(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
        .filter(|u| u.is_finite() && (0.0..=MAX_UTILIZATION).contains(u))
}

/// Claude Code reports times in seconds since the epoch; milliseconds are
/// accepted too.
fn epoch_ms(v: &Value) -> Option<Millis> {
    let t = v.as_f64().filter(|t| t.is_finite() && *t > 0.0)?;
    Some(if t < 1e11 {
        (t * 1000.0) as Millis
    } else {
        t as Millis
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const AT: Millis = 1_760_000_123_456;

    fn limit(
        window: &str,
        utilization: Option<f64>,
        resets_at: Option<Millis>,
        status: Option<LimitStatus>,
    ) -> PlanLimit {
        PlanLimit {
            window: window.into(),
            utilization,
            resets_at,
            status,
            observed_at: AT,
        }
    }

    #[test]
    fn windows_from_recent_claude_code() {
        // As Claude Code 2.1.302 writes it after a response.
        let v = json!({"type": "rate_limit_event", "rate_limit_info": {
            "status": "allowed", "isUsingOverage": false,
            "unifiedWindows": {
                "seven_day": {"utilization": 0.18, "resetsAt": 1_760_400_000},
                "five_hour": {"utilization": 0.42, "resetsAt": 1_760_010_000}
            }
        }, "uuid": "u", "session_id": "s"});
        assert_eq!(
            readings(&v, AT),
            vec![
                limit("five_hour", Some(0.42), Some(1_760_010_000_000), None),
                limit("seven_day", Some(0.18), Some(1_760_400_000_000), None),
            ]
        );
    }

    #[test]
    fn the_limit_closest_to_stopping_requests_has_a_status() {
        let v = json!({"type": "rate_limit_event", "rate_limit_info": {
            "status": "allowed_warning", "rateLimitType": "five_hour",
            "utilization": 0.9, "resetsAt": 1_760_010_000, "surpassedThreshold": 0.9,
            "unifiedWindows": {
                "five_hour": {"utilization": 0.91, "resetsAt": 1_760_010_000},
                "seven_day": {"utilization": 0.5, "resetsAt": 1_760_400_000}
            }
        }});
        assert_eq!(
            readings(&v, AT),
            vec![
                limit(
                    "five_hour",
                    Some(0.91),
                    Some(1_760_010_000_000),
                    Some(LimitStatus::AllowedWarning)
                ),
                limit("seven_day", Some(0.5), Some(1_760_400_000_000), None),
            ]
        );
    }

    #[test]
    fn older_claude_code_reports_one_limit() {
        let v = json!({"type": "rate_limit_event", "rate_limit_info": {
            "status": "allowed_warning", "rateLimitType": "seven_day_opus",
            "utilization": 0.8, "resetsAt": 1_760_400_000_000_i64
        }});
        assert_eq!(
            readings(&v, AT),
            vec![limit(
                "seven_day_opus",
                Some(0.8),
                Some(1_760_400_000_000),
                Some(LimitStatus::AllowedWarning)
            )]
        );
        // A reached limit may come without a share.
        let v = json!({"type": "rate_limit_event", "rate_limit_info": {
            "status": "rejected", "rateLimitType": "five_hour", "resetsAt": 1_760_010_000
        }});
        assert_eq!(
            readings(&v, AT),
            vec![limit(
                "five_hour",
                None,
                Some(1_760_010_000_000),
                Some(LimitStatus::Rejected)
            )]
        );
    }

    #[test]
    fn paid_extra_usage_and_bare_statuses_are_not_limits() {
        let overage = json!({"type": "rate_limit_event", "rate_limit_info": {
            "status": "allowed_warning", "rateLimitType": "overage", "utilization": 0.3,
            "isUsingOverage": true, "unifiedWindows": {"overage": {"utilization": 0.3, "resetsAt": 1}}
        }});
        assert_eq!(readings(&overage, AT), vec![]);
        let allowed = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed"}});
        assert_eq!(readings(&allowed, AT), vec![]);
        let named_only = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "unknown", "rateLimitType": "five_hour"}});
        assert_eq!(readings(&named_only, AT), vec![]);
        let other = json!({"type": "assistant", "rate_limit_info": {"status": "rejected", "rateLimitType": "five_hour"}});
        assert_eq!(readings(&other, AT), vec![]);
    }

    #[test]
    fn odd_values_are_left_out() {
        let v = json!({"type": "rate_limit_event", "rate_limit_info": {
            "status": "rejected", "rateLimitType": "Five Hour",
            "unifiedWindows": {
                "five_hour": {"utilization": -0.1, "resetsAt": "soon"},
                "seven_day": {"utilization": 1.04, "resetsAt": -5},
                "seven_day_sonnet": {"utilization": 250, "resetsAt": 1_760_400_000},
                "x".repeat(41): {"utilization": 0.1, "resetsAt": 1_760_400_000},
                "../etc": {"utilization": 0.1}
            }
        }});
        assert_eq!(
            readings(&v, AT),
            vec![
                limit("seven_day", Some(1.04), None, None),
                limit("seven_day_sonnet", None, Some(1_760_400_000_000), None),
            ]
        );
    }

    #[test]
    fn one_event_adds_a_handful_of_limits_at_most() {
        let windows: serde_json::Map<String, Value> = (0..20)
            .map(|i| {
                (
                    format!("seven_day_m{i:02}"),
                    json!({"utilization": 0.1, "resetsAt": 1_760_400_000}),
                )
            })
            .collect();
        let v = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "unifiedWindows": windows}});
        assert_eq!(readings(&v, AT).len(), MAX_LIMITS);
    }

    #[test]
    fn stored_usage() {
        // As D1 returns it: json_group_array of json_object, NULLs included.
        let json = r#"[
            {"window":"seven_day","utilization":0.18,"resets_at":1760400000000,"status":null,"observed_at":1760000123456},
            {"window":"five_hour","utilization":null,"resets_at":1760010000000,"status":"rejected","observed_at":1760000123456},
            {"window":"seven_day_opus","utilization":0.5,"resets_at":null,"status":"bogus","observed_at":1760000123456}
        ]"#;
        assert_eq!(
            from_json(json),
            vec![
                limit(
                    "five_hour",
                    None,
                    Some(1_760_010_000_000),
                    Some(LimitStatus::Rejected)
                ),
                limit("seven_day", Some(0.18), Some(1_760_400_000_000), None),
                limit("seven_day_opus", Some(0.5), None, None),
            ]
        );
        assert_eq!(from_json("[]"), vec![]);
        assert_eq!(from_json("not json"), vec![]);
    }

    #[test]
    fn labels() {
        assert_eq!(label("five_hour"), "5-hour limit");
        assert_eq!(label("seven_day"), "Weekly limit");
        assert_eq!(label("seven_day_opus"), "Weekly Opus limit");
        assert_eq!(label("seven_day_sonnet"), "Weekly Sonnet limit");
        assert_eq!(label("seven_day_overage_included"), "Weekly Fable limit");
        assert_eq!(label("seven_day_new_model"), "Weekly New model limit");
        assert_eq!(label("thirty_day"), "Thirty day limit");
    }

    #[test]
    fn standings() {
        let now = 1_000;
        let at = |utilization, resets_at, status| PlanLimit {
            window: "five_hour".into(),
            utilization,
            resets_at,
            status,
            observed_at: 0,
        };
        assert_eq!(
            standing(&at(Some(0.42), Some(2_000), None), now),
            Standing::Fine
        );
        assert_eq!(standing(&at(None, None, None), now), Standing::Fine);
        assert_eq!(
            standing(&at(Some(0.85), Some(2_000), None), now),
            Standing::Close
        );
        let warned = at(Some(0.5), None, Some(LimitStatus::AllowedWarning));
        assert_eq!(standing(&warned, now), Standing::Close);
        assert_eq!(
            standing(&at(Some(1.02), Some(2_000), None), now),
            Standing::Reached
        );
        let rejected = at(None, Some(2_000), Some(LimitStatus::Rejected));
        assert_eq!(standing(&rejected, now), Standing::Reached);
        // Once the window starts over, the reading says nothing about it.
        let over = at(Some(1.0), Some(1_000), Some(LimitStatus::Rejected));
        assert_eq!(standing(&over, now), Standing::Reset);
    }

    #[test]
    fn newer_readings_replace_their_window() {
        let mut limits = vec![
            limit("seven_day", Some(0.1), Some(2), None),
            limit("five_hour", Some(0.2), Some(1), None),
        ];
        merge(
            &mut limits,
            vec![
                limit(
                    "seven_day_opus",
                    Some(0.9),
                    Some(3),
                    Some(LimitStatus::AllowedWarning),
                ),
                limit("five_hour", Some(0.3), Some(1), None),
            ],
        );
        assert_eq!(
            limits,
            vec![
                limit("five_hour", Some(0.3), Some(1), None),
                limit("seven_day", Some(0.1), Some(2), None),
                limit(
                    "seven_day_opus",
                    Some(0.9),
                    Some(3),
                    Some(LimitStatus::AllowedWarning)
                ),
            ]
        );
    }
}
