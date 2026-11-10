//! How much of a conversation Claude keeps in its context: the size near
//! which a project's sessions compact it ([`ProjectSettings::compact_window`]),
//! as people choose and write it.
//!
//! Claude Code compacts the conversation, summarizing it, once its context
//! nears its auto-compact window: 33K tokens before it in Claude Code
//! 2.1.322, which reads the window from `CLAUDE_CODE_AUTO_COMPACT_WINDOW`
//! (or the `autoCompactWindow` setting), takes sizes below 100K as 100K and
//! caps them at the model's own window. By default the window is the
//! model's whole one (1M tokens for Opus 5.5, compacting at 967K). Each step
//! of a session reads its whole context again, cached or not, so a smaller
//! window spends less of a Claude plan on long sessions.

use fctl_proto::session::ProjectSettings;

/// The variable Claude Code reads its auto-compact window from. It takes
/// precedence over Claude's settings, a repository's among them.
pub const COMPACT_WINDOW_ENV: &str = "CLAUDE_CODE_AUTO_COMPACT_WINDOW";

/// The smallest window people choose: Claude Code reads smaller ones as
/// this.
pub const MIN_TOKENS: u32 = 100_000;
/// The largest: Claude caps the window at the model's own anyway.
pub const MAX_TOKENS: u32 = 1_000_000;

/// The sizes the web app offers, besides Claude's default and one of
/// people's own.
pub const CHOICES: [u32; 3] = [250_000, 400_000, 600_000];

/// Whether `tokens` is a window people may choose.
pub fn valid(tokens: u32) -> bool {
    (MIN_TOKENS..=MAX_TOKENS).contains(&tokens)
}

/// What [`valid`] allows, for an error.
pub fn bounds() -> String {
    format!("Use {} to {} tokens", short(MIN_TOKENS), short(MAX_TOKENS))
}

/// A number of tokens as people write it: `400000`, `400k`, `400K`,
/// `1m`, `1.5M`, `400 000` or `400,000`.
pub fn parse(input: &str) -> Option<u64> {
    let s: String = input
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | ',' | '_'))
        .collect();
    let (number, unit) = match s.char_indices().last()? {
        (i, 'k' | 'K') => (&s[..i], 1_000u64),
        (i, 'm' | 'M') => (&s[..i], 1_000_000u64),
        _ => (s.as_str(), 1),
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let digits = |d: &str| d.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || !digits(fraction) || (unit == 1 && !fraction.is_empty()) {
        return None;
    }
    let whole: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    // Digits past what the unit holds don't make a token.
    let mut scale = unit;
    let mut part = 0u64;
    for c in fraction.chars() {
        scale /= 10;
        if scale == 0 {
            break;
        }
        part += u64::from(c.to_digit(10)?) * scale;
    }
    whole.checked_mul(unit)?.checked_add(part)
}

/// `250K`, `1M`, `1.5M`, `967K`, `25.5K`: tokens in thousands, or
/// millions from a million up, to a tenth, as the web app writes them.
pub fn short(tokens: u32) -> String {
    let t = u64::from(tokens);
    let tenths = |unit: u64| {
        let n = (t * 10 + unit / 2) / unit;
        match n % 10 {
            0 => format!("{}", n / 10),
            d => format!("{}.{d}", n / 10),
        }
    };
    match t {
        // 999,950 and up round to a million.
        999_950.. => format!("{}M", tenths(1_000_000)),
        1_000.. => format!("{}K", tenths(1_000)),
        _ => t.to_string(),
    }
}

/// The variables a project's compact window gives its sessions: Claude
/// Code's, when the project chose one. Where it didn't, a project
/// variable of the same name stays as people set it.
pub fn env(settings: &ProjectSettings) -> Option<(&'static str, String)> {
    settings
        .compact_window
        .map(|t| (COMPACT_WINDOW_ENV, t.to_string()))
}

/// Where a project's sessions compact, in words.
pub fn describe(settings: &ProjectSettings) -> String {
    match settings.compact_window {
        Some(t) => format!(
            "Claude compacts the conversation as its context nears {} tokens",
            short(t)
        ),
        None => "Claude compacts the conversation near the model's whole window (Claude's default)"
            .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sizes_as_people_write_them() {
        for (input, tokens) in [
            ("400000", 400_000),
            ("400k", 400_000),
            ("400K", 400_000),
            (" 250k ", 250_000),
            ("1m", 1_000_000),
            ("1M", 1_000_000),
            ("1.5M", 1_500_000),
            ("0.25m", 250_000),
            (".5m", 500_000),
            ("400 000", 400_000),
            ("400,000", 400_000),
            ("400_000", 400_000),
            ("2.5k", 2_500),
            ("1.23456m", 1_234_560),
        ] {
            assert_eq!(parse(input), Some(tokens), "{input}");
        }
        for input in [
            "", "k", "m", ".", "400.5", "abc", "4o0k", "-400k", "400kk", "1e6",
        ] {
            assert_eq!(parse(input), None, "{input}");
        }
        assert_eq!(parse("99999999999999999999m"), None);
    }

    #[test]
    fn writes_sizes_short() {
        assert_eq!(short(250_000), "250K");
        assert_eq!(short(967_000), "967K");
        assert_eq!(short(1_000_000), "1M");
        assert_eq!(short(1_500_000), "1.5M");
        assert_eq!(short(123_456), "123.5K");
        assert_eq!(short(25_500), "25.5K");
        assert_eq!(short(999_999), "1M");
        assert_eq!(short(1_234_560), "1.2M");
        assert_eq!(short(950), "950");
        assert_eq!(bounds(), "Use 100K to 1M tokens");
    }

    #[test]
    fn allows_what_claude_takes() {
        assert!(valid(MIN_TOKENS) && valid(MAX_TOKENS) && valid(400_000));
        assert!(!valid(99_999) && !valid(1_000_001) && !valid(0));
        assert!(CHOICES.iter().all(|&c| valid(c)));
    }

    #[test]
    fn gives_claude_the_window_a_project_chose() {
        let mut s = ProjectSettings::default();
        assert_eq!(env(&s), None);
        assert!(describe(&s).contains("Claude's default"));
        s.compact_window = Some(400_000);
        assert_eq!(env(&s), Some((COMPACT_WINDOW_ENV, "400000".to_owned())));
        assert!(describe(&s).ends_with("nears 400K tokens"));
    }
}
