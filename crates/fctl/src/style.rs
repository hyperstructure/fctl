//! Terminal output: a small palette, respecting NO_COLOR and pipes.

use std::borrow::Cow;
use std::io::IsTerminal;
use std::sync::OnceLock;

/// `s` as it is safe to print to a terminal: the control characters
/// terminals act on are dropped, save newlines, tabs and color codes
/// (`ESC [ … m`, as fctl writes them). What sessions say (an agent's text, a
/// setup script's output, a pull request's link) could otherwise move the
/// cursor and rewrite what is on screen, or set the clipboard or the title.
pub fn terminal_safe(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' | '\t' => out.push(c),
            '\x1b' => {
                let mut rest = chars.clone();
                if rest.next() == Some('[') {
                    let mut params = String::new();
                    while let Some(&p) = rest.peek().filter(|p| p.is_ascii_digit() || **p == ';') {
                        params.push(p);
                        rest.next();
                    }
                    if rest.next() == Some('m') {
                        out.push_str("\x1b[");
                        out.push_str(&params);
                        out.push('m');
                        chars = rest;
                    }
                }
            }
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

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
pub fn runner_state(r: &fctl_proto::api::Runner) -> &'static str {
    use fctl_proto::api::RunnerStatus;
    match (r.status, &r.drain) {
        (RunnerStatus::Disabled, _) => "disabled",
        (_, Some(_)) if r.active_sessions > 0 => "draining",
        (_, Some(_)) => "drained",
        (RunnerStatus::Online, None) => "online",
        (RunnerStatus::Offline, None) => "offline",
    }
}

/// [`runner_state`], colored.
pub fn runner_status(r: &fctl_proto::api::Runner) -> String {
    match runner_state(r) {
        s @ "disabled" => red(s),
        s @ ("draining" | "drained") => amber(s),
        s @ "online" => accent(s),
        s => dim(s),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_colors_and_line_breaks_reach_the_terminal() {
        assert_eq!(terminal_safe("plain\ttext\n"), "plain\ttext\n");
        assert_eq!(terminal_safe("\x1b[1mbold\x1b[0m"), "\x1b[1mbold\x1b[0m");
        // The clipboard (OSC 52), the title, the screen, the cursor.
        assert_eq!(
            terminal_safe("\x1b]52;c;ZXZpbA==\x07ok"),
            "]52;c;ZXZpbA==ok"
        );
        assert_eq!(terminal_safe("\x1b]0;title\x1b\\"), "]0;title\\");
        assert_eq!(terminal_safe("a\x1b[2Jb\x1b[1A"), "a[2Jb[1A");
        assert_eq!(terminal_safe("done\rfake"), "donefake");
        assert_eq!(terminal_safe("x\u{9b}31my"), "x31my");
        assert_eq!(terminal_safe("bell\x07\x08"), "bell");
    }
}
