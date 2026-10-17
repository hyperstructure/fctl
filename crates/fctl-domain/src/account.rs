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
