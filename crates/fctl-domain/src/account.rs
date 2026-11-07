//! Deleting an account and exporting its data: the pure parts.
//!
//! A deleted account that created sessions or projects others still use is
//! kept as a placeholder, so their rows stay consistent, but it loses its
//! name, email, picture and every way to sign in.

/// The name a deleted account keeps on what it created.
pub const DELETED_NAME: &str = "Deleted user";

/// The email address a deleted account keeps: unique, and never deliverable
/// (`.invalid` is reserved by RFC 2606).
pub fn tombstone_email(user_id: &str) -> String {
    format!("deleted-{}@deleted.invalid", user_id.to_ascii_lowercase())
}

/// Whether `email` is a deleted account's ([`tombstone_email`]).
pub fn is_tombstone(email: &str) -> bool {
    email.starts_with("deleted-") && email.ends_with("@deleted.invalid")
}

/// Whether `typed` confirms deleting the account with this email address.
pub fn confirms_deletion(typed: &str, email: &str) -> bool {
    let email = email.trim();
    !email.is_empty() && typed.trim().eq_ignore_ascii_case(email)
}

/// Milliseconds since the epoch as RFC 3339 in UTC, e.g.
/// `2026-10-17T21:32:23.000Z`.
pub fn iso8601(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

/// Year, month and day of a day number (days since 1970-01-01), after
/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// The export's file name: `fctl-export-2026-10-17.json`.
pub fn export_filename(now: i64) -> String {
    let day = iso8601(now);
    format!("fctl-export-{}.json", day.get(..10).unwrap_or("export"))
}

/// The longest admins can make people wait between two exports: a year.
pub const MAX_EXPORT_INTERVAL_HOURS: u32 = 8760;

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;

/// When someone who last exported their data at `last` can export it
/// again, with `interval_hours` between two exports: `None` when they can
/// now.
pub fn next_export_at(last: Option<i64>, interval_hours: u32, now: i64) -> Option<i64> {
    let next = last?.saturating_add(i64::from(interval_hours) * HOUR_MS);
    (next > now).then_some(next)
}

/// How often people can export their data: `once a week`, `once every 3
/// days`, `at any time`.
pub fn export_interval_words(hours: u32) -> String {
    match hours {
        0 => "at any time".into(),
        1 => "once an hour".into(),
        24 => "once a day".into(),
        168 => "once a week".into(),
        MAX_EXPORT_INTERVAL_HOURS => "once a year".into(),
        h if h % 168 == 0 => format!("once every {} weeks", h / 168),
        h if h % 24 == 0 => format!("once every {} days", h / 24),
        h => format!("once every {h} hours"),
    }
}

/// How long until `at`, rounded up, so that trying again then works: `in
/// 3 days`, `in 5 hours`, `in 12 minutes`, `in a minute`.
pub fn until(at: i64, now: i64) -> String {
    let left = at.saturating_sub(now);
    let up = |unit: i64| (left + unit - 1) / unit;
    if left <= MINUTE_MS {
        "in a minute".into()
    } else if left < 2 * HOUR_MS {
        format!("in {} minutes", up(MINUTE_MS))
    } else if left < 2 * DAY_MS {
        format!("in {} hours", up(HOUR_MS))
    } else {
        format!("in {} days", up(DAY_MS))
    }
}

/// Why an export must wait until `next_at`.
pub fn export_wait(interval_hours: u32, next_at: i64, now: i64) -> String {
    format!(
        "You can export your data {}. Try again {}.",
        export_interval_words(interval_hours),
        until(next_at, now)
    )
}

/// The hours between two exports as admins type them: `1w`, `7d`, `24h`,
/// or `0` (`off`, `none`) to let people export at any time.
pub fn parse_export_interval(input: &str) -> Result<u32, String> {
    let s = input.trim().to_ascii_lowercase();
    if matches!(s.as_str(), "0" | "off" | "none") {
        return Ok(0);
    }
    let bad =
        || format!("{input:?} isn't a wait between exports: use 1d, 1w, 30d, or off for none");
    match crate::enrollment::parse_hours(&s) {
        Ok(Some(h)) if h <= MAX_EXPORT_INTERVAL_HOURS => Ok(h),
        Ok(Some(_)) => Err(format!(
            "{input:?} is longer than people may be made to wait: a year at most"
        )),
        _ => Err(bad()),
    }
}

/// A serialized JSON object without its closing brace, so that the export
/// can append a field it streams (`{"a":1}` → `{"a":1`).
pub fn open_object(json: &str) -> Option<&str> {
    json.trim_end()
        .strip_suffix('}')
        .filter(|s| s.starts_with('{'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tombstones_are_unique_and_undeliverable() {
        assert_eq!(
            tombstone_email("usr_01ABC"),
            "deleted-usr_01abc@deleted.invalid"
        );
        assert_ne!(tombstone_email("usr_1"), tombstone_email("usr_2"));
        assert!(is_tombstone(&tombstone_email("usr_01ABC")));
        assert!(!is_tombstone("ada@example.com"));
    }

    #[test]
    fn confirmation_is_the_email_address() {
        assert!(confirms_deletion("ada@example.com", "ada@example.com"));
        assert!(confirms_deletion("  Ada@Example.com ", "ada@example.com"));
        assert!(!confirms_deletion("ada@example.org", "ada@example.com"));
        assert!(!confirms_deletion("", ""));
        assert!(!confirms_deletion("delete", "ada@example.com"));
    }

    #[test]
    fn exports_wait_their_interval() {
        let day = 86_400_000;
        let now = 1_792_272_742_000;
        // Never exported, or no wait: now.
        assert_eq!(next_export_at(None, 168, now), None);
        assert_eq!(next_export_at(Some(now - 1000), 0, now), None);
        // A week after the last one, to the millisecond.
        assert_eq!(
            next_export_at(Some(now - 2 * day), 168, now),
            Some(now + 5 * day)
        );
        assert_eq!(next_export_at(Some(now - 7 * day), 168, now), None);
        assert_eq!(
            next_export_at(Some(now - 7 * day + 1), 168, now),
            Some(now + 1)
        );
        // An admin who shortens the wait lets people in sooner.
        assert_eq!(next_export_at(Some(now - 2 * day), 24, now), None);
    }

    #[test]
    fn waits_read_as_people_say_them() {
        assert_eq!(export_interval_words(0), "at any time");
        assert_eq!(export_interval_words(1), "once an hour");
        assert_eq!(export_interval_words(12), "once every 12 hours");
        assert_eq!(export_interval_words(24), "once a day");
        assert_eq!(export_interval_words(72), "once every 3 days");
        assert_eq!(export_interval_words(168), "once a week");
        assert_eq!(export_interval_words(336), "once every 2 weeks");
        assert_eq!(export_interval_words(720), "once every 30 days");
        assert_eq!(export_interval_words(8760), "once a year");
        let now = 1_792_272_742_000;
        let (min, hour, day) = (60_000, 3_600_000, 86_400_000);
        assert_eq!(until(now + 1, now), "in a minute");
        assert_eq!(until(now + min, now), "in a minute");
        assert_eq!(until(now + min + 1, now), "in 2 minutes");
        assert_eq!(until(now + 2 * hour - 1, now), "in 120 minutes");
        assert_eq!(until(now + 2 * hour, now), "in 2 hours");
        assert_eq!(until(now + 47 * hour + 1, now), "in 48 hours");
        assert_eq!(until(now + 2 * day, now), "in 2 days");
        assert_eq!(until(now + 6 * day + 1, now), "in 7 days");
        assert_eq!(
            export_wait(168, now + 5 * day, now),
            "You can export your data once a week. Try again in 5 days."
        );
    }

    #[test]
    fn admins_type_the_wait_as_a_duration() {
        assert_eq!(parse_export_interval("1w"), Ok(168));
        assert_eq!(parse_export_interval("7d"), Ok(168));
        assert_eq!(parse_export_interval(" 24h "), Ok(24));
        assert_eq!(parse_export_interval("90m"), Ok(2));
        assert_eq!(parse_export_interval("1y"), Ok(8760));
        assert_eq!(parse_export_interval("0"), Ok(0));
        assert_eq!(parse_export_interval("Off"), Ok(0));
        assert_eq!(parse_export_interval("none"), Ok(0));
        assert!(parse_export_interval("2y").is_err_and(|e| e.contains("a year at most")));
        // A number alone could mean anything, and never means nothing here.
        assert!(parse_export_interval("7").is_err());
        assert!(parse_export_interval("never").is_err());
        assert!(parse_export_interval("soon").is_err());
    }

    #[test]
    fn formats_times() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso8601(1_709_210_096_789), "2024-02-29T12:34:56.789Z");
        assert_eq!(iso8601(1_792_272_742_000), "2026-10-17T21:32:22.000Z");
        assert_eq!(iso8601(-1), "1969-12-31T23:59:59.999Z");
        assert_eq!(
            export_filename(1_709_210_096_789),
            "fctl-export-2024-02-29.json"
        );
    }

    #[test]
    fn opens_objects_for_appending() {
        let open = open_object("{\"a\":1}\n").unwrap_or_default();
        assert_eq!(open, "{\"a\":1");
        let doc = format!("{open},\"b\":[2]}}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&doc).ok(),
            Some(serde_json::json!({ "a": 1, "b": [2] }))
        );
        assert_eq!(open_object("[1]"), None);
        assert_eq!(open_object("1}"), None);
    }
}
