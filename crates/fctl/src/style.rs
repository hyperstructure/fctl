//! Terminal output: a small palette, respecting NO_COLOR and pipes.

use std::io::IsTerminal;
use std::sync::OnceLock;

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal())
}

/// Whether output goes to a terminal that takes colors.
pub fn colors() -> bool {
    enabled()
}

fn paint(code: &str, s: &str) -> String {
    if enabled() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_owned()
    }
}

pub fn accent(s: &str) -> String {
    paint("38;2;45;212;191", s)
}
pub fn blue(s: &str) -> String {
    paint("38;2;125;170;255", s)
}
/// `n` and the noun for it: "1 runner", "2 runners".
pub fn count(n: impl Into<u64>, one: &str, many: &str) -> String {
    let n = n.into();
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn amber(s: &str) -> String {
    paint("38;2;245;181;68", s)
}
pub fn red(s: &str) -> String {
    paint("38;2;248;113;113", s)
}
pub fn dim(s: &str) -> String {
    paint("2", s)
}
pub fn bold(s: &str) -> String {
    paint("1", s)
}

/// A colored dot for a session status.
pub fn status(s: fctl_proto::session::SessionStatus) -> String {
    use fctl_proto::session::SessionStatus::*;
    let label = s.as_str();
    match s {
        Running => accent(&format!("● {label}")),
        Idle => blue(&format!("○ {label}")),
        Queued | Provisioning | Preparing | Stopping => amber(&format!("◌ {label}")),
        Paused => amber(&format!("◍ {label}")),
        Stopped => dim(&format!("○ {label}")),
        Failed => red(&format!("✕ {label}")),
    }
}

/// A runner's state: online, offline, disabled, draining or drained.
pub fn runner_status(r: &fctl_proto::api::Runner) -> String {
    use fctl_proto::api::RunnerStatus;
    match (r.status, &r.drain) {
        (RunnerStatus::Disabled, _) => red("disabled"),
        (_, Some(_)) if r.active_sessions > 0 => amber("draining"),
        (_, Some(_)) => amber("drained"),
        (RunnerStatus::Online, None) => accent("online"),
        (RunnerStatus::Offline, None) => dim("offline"),
    }
}

/// "3m ago", "2h ago", "5d ago".
pub fn ago(ms: i64) -> String {
    ago_from(ms, now_ms())
}

/// [`ago`], as of `now`.
pub fn ago_from(ms: i64, now: i64) -> String {
    let s = ((now - ms) / 1000).max(0);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

/// "in 45m", "in 2h 10m", "in 3d 4h".
pub fn until(ms: i64) -> String {
    let m = ((ms - now_ms()) / 60_000).max(1);
    match m {
        0..=59 => format!("in {m}m"),
        60..=1439 => format!("in {}h {}m", m / 60, m % 60),
        _ => format!("in {}d {}h", m / 1440, m % 1440 / 60),
    }
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn print_json<T: serde::Serialize>(v: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}
