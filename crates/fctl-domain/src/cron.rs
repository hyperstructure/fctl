//! Cron expressions as Vixie cron reads them, for schedules that run again
//! and again (`crate::schedules`): five fields, the minute (0-59), the hour
//! (0-23), the day of the month (1-31), the month (1-12 or `jan`-`dec`) and
//! the day of the week (0-7 or `sun`-`sat`, where 0 and 7 are Sunday). Each
//! field is `*`, a value, a range (`1-5`), a step (`*/15`, `9-17/2`, `5/15`)
//! or a list of them (`1,15`); `@hourly`, `@daily`, `@weekly`, `@monthly`
//! and `@yearly` stand for the usual five.
//!
//! As in Vixie cron, a day matches when its day of the month or its day of
//! the week does, where both fields are restricted (neither starts with
//! `*`), and when both do otherwise.
//!
//! The times are wall-clock times in a time zone ([`Zone`]), which callers
//! supply: the control plane asks JavaScript's `Intl`, so that the zone
//! rules are the platform's and this crate needs no time zone database. A
//! time the clock skips, as it moves to summer time, runs as far past the
//! change as it was meant to be (02:30 runs at 03:30); one the clock goes
//! through twice, as it moves back, runs once, the first time. JavaScript's
//! `Temporal` reads wall-clock times the same way, and so does the web app.

pub const MINUTE_MS: i64 = 60_000;
pub const HOUR_MS: i64 = 60 * MINUTE_MS;
pub const DAY_MS: i64 = 24 * HOUR_MS;

/// How far ahead the next run is looked for: an expression that names no
/// time within it (`0 0 30 2 *`) never runs.
const HORIZON_DAYS: i64 = 5 * 366;
/// How far the clocks of a zone move at a change, at most, as far as
/// finding the next run goes.
const MAX_SHIFT_MS: i64 = 3 * HOUR_MS;

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const WEEKDAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// A time zone, as far as schedules need one.
pub trait Zone {
    /// How far ahead of UTC its clocks are at `utc_ms` (milliseconds since
    /// the epoch), in milliseconds: negative where they are behind.
    fn offset_ms(&self, utc_ms: i64) -> i64;
}

/// UTC, whose clocks never move.
#[derive(Clone, Copy, Debug, Default)]
pub struct Utc;

impl Zone for Utc {
    fn offset_ms(&self, _utc_ms: i64) -> i64 {
        0
    }
}

/// The wall-clock time in `zone` at `utc_ms`, as milliseconds since a
/// local epoch (1970-01-01 00:00 on its clocks).
pub fn local(zone: &dyn Zone, utc_ms: i64) -> i64 {
    utc_ms + zone.offset_ms(utc_ms)
}

/// The instant a wall-clock time in `zone` names (`local_ms`, as from
/// [`local`]): the first, when the clock goes through it twice; for a time
/// the clock skips, the instant as far past the change as the time is past
/// its start.
pub fn instant(zone: &dyn Zone, local_ms: i64) -> i64 {
    // The offsets a day either side: a zone's clocks change at most once
    // in between.
    let before = zone.offset_ms(local_ms - DAY_MS);
    let after = zone.offset_ms(local_ms + DAY_MS);
    let mut first: Option<i64> = None;
    for offset in [before, after] {
        let utc = local_ms - offset;
        if zone.offset_ms(utc) == offset {
            first = Some(first.map_or(utc, |f: i64| f.min(utc)));
        }
    }
    first.unwrap_or(local_ms - before)
}

/// A cron expression, parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cron {
    /// Bit `n` for each minute (0-59), hour (0-23), day of the month
    /// (1-31), month (1-12) and day of the week (0-6, Sunday 0) it names.
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    /// The day of the month, and the day of the week, start with `*`.
    any_day: bool,
    any_weekday: bool,
    /// The expression as given, its fields one space apart.
    text: String,
}

impl Cron {
    /// Parses an expression; the error says what is wrong with it, for
    /// people.
    pub fn parse(input: &str) -> Result<Cron, String> {
        let text = input.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return Err("Write a cron expression, like 0 9 * * 1-5".into());
        }
        let lower = text.to_ascii_lowercase();
        let expanded = match lower.as_str() {
            "@yearly" | "@annually" => "0 0 1 1 *",
            "@monthly" => "0 0 1 * *",
            "@weekly" => "0 0 * * 0",
            "@daily" | "@midnight" => "0 0 * * *",
            "@hourly" => "0 * * * *",
            t if t.starts_with('@') => {
                return Err(format!(
                    "{text} isn't a shorthand fctl knows: use @hourly, @daily, @weekly, @monthly or @yearly"
                ));
            }
            t => t,
        };
        let fields: Vec<&str> = expanded.split(' ').collect();
        let [minute, hour, day, month, weekday] = fields.as_slice() else {
            return Err(
                "Use five fields: the minute, the hour, the day of the month, the month and the day of the week, like 0 9 * * 1-5".into(),
            );
        };
        let minutes = field(minute, 0, 59, &[], "minute")?;
        let hours = field(hour, 0, 23, &[], "hour")?;
        let days = field(day, 1, 31, &[], "day of the month")?;
        let months = field(month, 1, 12, &MONTHS, "month")?;
        // 7 is Sunday too.
        let weekdays = field(weekday, 0, 7, &WEEKDAYS, "day of the week")?;
        let weekdays = (weekdays | weekdays >> 7) & 0x7f;
        Ok(Cron {
            minutes,
            hours,
            days,
            months,
            weekdays,
            any_day: day.starts_with('*'),
            any_weekday: weekday.starts_with('*'),
            text: if lower.starts_with('@') { lower } else { text },
        })
    }

    /// The expression, its fields one space apart.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether it runs on a day: its day of the month (1-31), month (1-12)
    /// and day of the week (0-6, Sunday 0).
    fn runs_on(&self, day: u32, month: u32, weekday: u32) -> bool {
        if self.months & bit(month) == 0 {
            return false;
        }
        let by_day = self.days & bit(day) != 0;
        let by_weekday = self.weekdays & bit(weekday) != 0;
        if self.any_day || self.any_weekday {
            by_day && by_weekday
        } else {
            by_day || by_weekday
        }
    }

    /// The first time of day it names at or after minute `from` of a day
    /// (0-1439), in minutes.
    fn first_time(&self, from: u32) -> Option<u32> {
        let (from_hour, from_minute) = (from / 60, from % 60);
        for hour in from_hour..24 {
            if self.hours & bit(hour) == 0 {
                continue;
            }
            let start = if hour == from_hour { from_minute } else { 0 };
            let later = self.minutes >> start;
            if later != 0 {
                return Some(hour * 60 + start + later.trailing_zeros());
            }
        }
        None
    }

    /// The first wall-clock time it names at or after `from` (milliseconds
    /// since the local epoch), within five years.
    pub fn next_local(&self, from: i64) -> Option<i64> {
        let start = from.div_euclid(MINUTE_MS) + i64::from(from.rem_euclid(MINUTE_MS) != 0);
        let first = start.div_euclid(24 * 60);
        let mut minute = u32::try_from(start.rem_euclid(24 * 60)).unwrap_or(0);
        for day in first..first + HORIZON_DAYS {
            let (_, month, dom) = civil_from_days(day);
            if self.runs_on(dom, month, weekday(day))
                && let Some(t) = self.first_time(minute)
            {
                return Some(day * DAY_MS + i64::from(t) * MINUTE_MS);
            }
            minute = 0;
        }
        None
    }

    /// The first time after `after` (milliseconds since the epoch) that it
    /// names in `zone`, within five years.
    pub fn next_after(&self, zone: &dyn Zone, after: i64) -> Option<i64> {
        // Wall-clock times and instants keep their order, but near a change
        // of the clocks: a time the clock skips runs later than the times
        // just after the change, and the clock goes through others twice. So
        // the search starts before `after` on the clock, and goes on past
        // the first run it finds, by as far as clocks move.
        let mut from = local(zone, after) - MAX_SHIFT_MS;
        let mut best: Option<(i64, i64)> = None;
        for _ in 0..10_000 {
            let Some(at) = self.next_local(from) else {
                break;
            };
            if best.is_some_and(|(_, found)| at > found + MAX_SHIFT_MS) {
                break;
            }
            let utc = instant(zone, at);
            if utc > after && best.is_none_or(|(b, _)| utc < b) {
                best = Some((utc, at));
            }
            from = at + MINUTE_MS;
        }
        best.map(|(utc, _)| utc)
    }

    /// The shortest time between two of its runs, in minutes, as if it ran
    /// every day.
    pub fn shortest_gap_minutes(&self) -> u32 {
        let times: Vec<u32> = (0..24 * 60)
            .filter(|t| self.hours & bit(t / 60) != 0 && self.minutes & bit(t % 60) != 0)
            .collect();
        let (Some(first), Some(last)) = (times.first(), times.last()) else {
            return 24 * 60;
        };
        let around = 24 * 60 - last + first;
        times
            .windows(2)
            .filter_map(|w| Some(w.get(1)? - w.first()?))
            .fold(around, u32::min)
    }

    /// When it runs, in words, after "Runs": "every weekday at 09:00",
    /// "every hour at :15", "on the 1st of every month at 08:30"; the
    /// expression itself where no shorter phrase says it ("on the cron
    /// schedule 0 9 1-7 * 1").
    pub fn describe(&self) -> String {
        let fallback = || format!("on the cron schedule {}", self.text);
        let every_day = self.any_day && self.any_weekday && self.months == ALL_MONTHS;
        let minutes = set(self.minutes, 0, 59);
        let hours = set(self.hours, 0, 23);
        let [minute] = minutes.as_slice() else {
            return fallback();
        };
        if hours.len() == 24 {
            return if every_day {
                format!("every hour at :{minute:02}")
            } else {
                fallback()
            };
        }
        if hours.len() > 4 {
            return fallback();
        }
        let times = list(
            &hours
                .iter()
                .map(|h| format!("{h:02}:{minute:02}"))
                .collect::<Vec<_>>(),
        );
        if every_day {
            return format!("every day at {times}");
        }
        if self.months != ALL_MONTHS {
            return fallback();
        }
        if self.any_day && !self.any_weekday {
            let weekdays = set(self.weekdays, 0, 6);
            return match weekdays.as_slice() {
                [1, 2, 3, 4, 5] => format!("every weekday at {times}"),
                [0, 6] => format!("every Saturday and Sunday at {times}"),
                days => {
                    // Monday first.
                    let mut from_monday: Vec<u32> = days.iter().map(|d| (d + 6) % 7).collect();
                    from_monday.sort_unstable();
                    let names: Vec<String> = from_monday
                        .iter()
                        .filter_map(|d| WEEKDAY_NAMES.get(((d + 1) % 7) as usize))
                        .map(|n| (*n).to_owned())
                        .collect();
                    if names.is_empty() {
                        return fallback();
                    }
                    format!("every {} at {times}", list(&names))
                }
            };
        }
        if !self.any_day && self.any_weekday {
            let days: Vec<String> = set(self.days, 1, 31).into_iter().map(ordinal).collect();
            if days.len() > 4 {
                return fallback();
            }
            return format!("on the {} of every month at {times}", list(&days));
        }
        fallback()
    }
}

const ALL_MONTHS: u64 = 0b1_1111_1111_1110;

fn bit(n: u32) -> u64 {
    1u64.checked_shl(n).unwrap_or(0)
}

/// The values whose bits are set in `bits`, from `lo` to `hi`.
fn set(bits: u64, lo: u32, hi: u32) -> Vec<u32> {
    (lo..=hi).filter(|n| bits & bit(*n) != 0).collect()
}

/// "a", "a and b", "a, b and c".
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// "1st", "2nd", "3rd", "4th", "11th", "21st".
pub fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// One field's values as bits: `lo` to `hi`, with `names` for `lo`
/// onwards; `what` names the field in errors.
fn field(text: &str, lo: u32, hi: u32, names: &[&str], what: &str) -> Result<u64, String> {
    let value = |v: &str| -> Result<u32, String> {
        if let Some(i) = names.iter().position(|n| *n == v) {
            return u32::try_from(i).map(|i| i + lo).map_err(|e| e.to_string());
        }
        match v.parse::<u32>() {
            Ok(n) if (lo..=hi).contains(&n) => Ok(n),
            Ok(n) => Err(format!("The {what} must be from {lo} to {hi}, not {n}")),
            Err(_) => Err(format!("{v} isn't a {what}")),
        }
    };
    let mut bits = 0u64;
    for part in text.split(',') {
        if part.is_empty() {
            return Err(format!("The {what} has an empty item in its list"));
        }
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => match step.parse::<u32>() {
                Ok(s) if s > 0 && s <= hi => (range, Some(s)),
                _ => return Err(format!("{step} isn't a step for the {what}")),
            },
            None => (part, None),
        };
        let (first, last) = if range == "*" {
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            let (a, b) = (value(a)?, value(b)?);
            if a > b {
                return Err(format!(
                    "The {what} range {range} runs backwards: write it from the first to the last"
                ));
            }
            (a, b)
        } else {
            let a = value(range)?;
            (a, if step.is_some() { hi } else { a })
        };
        let mut v = first;
        while v <= last {
            bits |= bit(v);
            v += step.unwrap_or(1);
        }
    }
    Ok(bits)
}

/// The day of the week of a day number (days since 1970-01-01, a
/// Thursday): 0 is Sunday.
pub fn weekday(days: i64) -> u32 {
    u32::try_from((days + 4).rem_euclid(7)).unwrap_or(0)
}

/// Year, month (1-12) and day (1-31) of a day number (days since
/// 1970-01-01), after Howard Hinnant's `civil_from_days`.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

/// Days since 1970-01-01 of a date, after Howard Hinnant's
/// `days_from_civil`.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let (m, d) = (i64::from(m), i64::from(d));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Central European time: UTC+1, and UTC+2 from the last Sunday of
    /// March to the last Sunday of October, changing at 01:00 UTC.
    pub struct Stockholm;

    impl Zone for Stockholm {
        fn offset_ms(&self, utc_ms: i64) -> i64 {
            let (year, _, _) = civil_from_days(utc_ms.div_euclid(DAY_MS));
            let last_sunday = |month: u32| {
                let end = days_from_civil(year, month, 31);
                end - i64::from(weekday(end))
            };
            let starts = last_sunday(3) * DAY_MS + HOUR_MS;
            let ends = last_sunday(10) * DAY_MS + HOUR_MS;
            if (starts..ends).contains(&utc_ms) {
                2 * HOUR_MS
            } else {
                HOUR_MS
            }
        }
    }

    /// Fixed offsets, for zones without summer time.
    pub struct Fixed(pub i64);

    impl Zone for Fixed {
        fn offset_ms(&self, _utc_ms: i64) -> i64 {
            self.0
        }
    }

    /// Milliseconds since the epoch of a UTC date and time.
    pub fn utc(y: i64, mo: u32, d: u32, h: i64, mi: i64) -> i64 {
        days_from_civil(y, mo, d) * DAY_MS + h * HOUR_MS + mi * MINUTE_MS
    }

    fn cron(s: &str) -> Cron {
        match Cron::parse(s) {
            Ok(c) => c,
            Err(e) => panic!("{s}: {e}"),
        }
    }

    /// The next `n` runs after `from`, in `zone`.
    fn runs(c: &Cron, zone: &dyn Zone, from: i64, n: usize) -> Vec<i64> {
        let mut out = Vec::new();
        let mut at = from;
        for _ in 0..n {
            let Some(next) = c.next_after(zone, at) else {
                break;
            };
            out.push(next);
            at = next;
        }
        out
    }

    #[test]
    fn parses_fields_ranges_steps_lists_and_names() {
        let c = cron("*/15 9-17 * * mon-fri");
        assert_eq!(set(c.minutes, 0, 59), vec![0, 15, 30, 45]);
        assert_eq!(set(c.hours, 0, 23), (9..=17).collect::<Vec<_>>());
        assert_eq!(set(c.weekdays, 0, 6), vec![1, 2, 3, 4, 5]);
        assert!(c.any_day && !c.any_weekday);
        let c = cron("5/20 0 1,15 JAN,jul 7");
        assert_eq!(set(c.minutes, 0, 59), vec![5, 25, 45]);
        assert_eq!(set(c.days, 1, 31), vec![1, 15]);
        assert_eq!(set(c.months, 1, 12), vec![1, 7]);
        // 7 and 0 are both Sunday.
        assert_eq!(set(c.weekdays, 0, 6), vec![0]);
        assert_eq!(cron("0 0 * * 0").weekdays, cron("0 0 * * 7").weekdays);
        assert_eq!(cron("  0   9 * *  1 ").as_str(), "0 9 * * 1");
        assert_eq!(cron("@Daily").as_str(), "@daily");
        assert_eq!(cron("@daily").hours, cron("0 0 * * *").hours);
        assert_eq!(cron("@weekly").weekdays, bit(0));
    }

    #[test]
    fn refuses_what_it_cant_read() {
        for (bad, says) in [
            ("", "Write a cron expression"),
            ("0 9 * *", "five fields"),
            ("0 9 * * * *", "five fields"),
            ("60 * * * *", "minute must be from 0 to 59, not 60"),
            ("0 24 * * *", "hour must be from 0 to 23"),
            ("0 0 0 * *", "day of the month must be from 1 to 31, not 0"),
            ("0 0 * 13 *", "month must be from 1 to 12"),
            ("0 0 * * 8", "day of the week must be from 0 to 7"),
            ("0 0 * * fri-mon", "runs backwards"),
            ("*/0 * * * *", "isn't a step"),
            ("0,,5 * * * *", "empty item"),
            ("x * * * *", "x isn't a minute"),
            ("@reboot", "isn't a shorthand"),
        ] {
            let Err(err) = Cron::parse(bad) else {
                panic!("{bad} parsed");
            };
            assert!(err.contains(says), "{bad}: {err}");
        }
    }

    #[test]
    fn finds_the_next_runs_in_utc() {
        // Friday 23 October 2026, 10:41 UTC.
        let now = utc(2026, 10, 23, 10, 41);
        let c = cron("0 9 * * 1-5");
        assert_eq!(
            runs(&c, &Utc, now, 3),
            vec![
                utc(2026, 10, 26, 9, 0),
                utc(2026, 10, 27, 9, 0),
                utc(2026, 10, 28, 9, 0)
            ]
        );
        // Strictly after: a run due now is not the next.
        assert_eq!(
            c.next_after(&Utc, utc(2026, 10, 26, 9, 0)),
            Some(utc(2026, 10, 27, 9, 0))
        );
        assert_eq!(
            cron("15 * * * *").next_after(&Utc, now),
            Some(utc(2026, 10, 23, 11, 15))
        );
        assert_eq!(
            cron("@monthly").next_after(&Utc, now),
            Some(utc(2026, 11, 1, 0, 0))
        );
        // Leap days come every four years; 30 February never does.
        assert_eq!(
            cron("0 12 29 2 *").next_after(&Utc, now),
            Some(utc(2028, 2, 29, 12, 0))
        );
        assert_eq!(cron("0 0 30 2 *").next_after(&Utc, now), None);
        assert_eq!(
            cron("0 0 31 * *").next_after(&Utc, utc(2026, 11, 1, 0, 0)),
            Some(utc(2026, 12, 31, 0, 0))
        );
    }

    #[test]
    fn a_restricted_day_of_the_month_or_week_is_enough() {
        // The 1st, or any Monday.
        let c = cron("0 8 1 * mon");
        assert_eq!(
            runs(&c, &Utc, utc(2026, 10, 23, 0, 0), 3),
            vec![
                utc(2026, 10, 26, 8, 0),
                utc(2026, 11, 1, 8, 0),
                utc(2026, 11, 2, 8, 0)
            ]
        );
        // A day of the month that starts with a star must match as well:
        // Mondays on odd days, so not 26 October or 2 November.
        let c = cron("0 8 */2 * mon");
        assert_eq!(
            c.next_after(&Utc, utc(2026, 10, 23, 0, 0)),
            Some(utc(2026, 11, 9, 8, 0))
        );
    }

    #[test]
    fn follows_the_zone_and_its_summer_time() {
        // 09:00 in Stockholm is 07:00 UTC in summer time, 08:00 after it.
        let c = cron("0 9 * * *");
        assert_eq!(
            runs(&c, &Stockholm, utc(2026, 10, 23, 12, 0), 4),
            vec![
                utc(2026, 10, 24, 7, 0),
                // Summer time ends at 01:00 UTC on Sunday 25 October.
                utc(2026, 10, 25, 8, 0),
                utc(2026, 10, 26, 8, 0),
                utc(2026, 10, 27, 8, 0)
            ]
        );
        assert_eq!(
            cron("30 17 * * *").next_after(&Fixed(-4 * HOUR_MS), utc(2026, 10, 23, 12, 0)),
            Some(utc(2026, 10, 23, 21, 30))
        );
    }

    #[test]
    fn a_skipped_time_runs_as_far_past_the_change() {
        // Summer time starts on Sunday 29 March 2026: 02:00 becomes 03:00.
        let c = cron("30 2 * * *");
        assert_eq!(
            runs(&c, &Stockholm, utc(2026, 3, 28, 12, 0), 2),
            vec![
                // 03:30 summer time.
                utc(2026, 3, 29, 1, 30),
                utc(2026, 3, 30, 0, 30)
            ]
        );
        // Every 30 minutes: 02:00 and 02:30 come out at 03:00 and 03:30,
        // which run once each.
        let c = cron("*/30 * * * *");
        assert_eq!(
            runs(&c, &Stockholm, utc(2026, 3, 29, 0, 15), 4),
            vec![
                utc(2026, 3, 29, 0, 30),
                utc(2026, 3, 29, 1, 0),
                utc(2026, 3, 29, 1, 30),
                utc(2026, 3, 29, 2, 0)
            ]
        );
    }

    #[test]
    fn a_time_the_clock_goes_through_twice_runs_once() {
        // Summer time ends on Sunday 25 October 2026: 03:00 becomes 02:00.
        let c = cron("30 2 * * *");
        assert_eq!(
            runs(&c, &Stockholm, utc(2026, 10, 24, 12, 0), 2),
            vec![utc(2026, 10, 25, 0, 30), utc(2026, 10, 26, 1, 30)]
        );
        // Hourly, the hour that comes twice runs once.
        let c = cron("0 * * * *");
        assert_eq!(
            runs(&c, &Stockholm, utc(2026, 10, 24, 23, 30), 4),
            vec![
                utc(2026, 10, 25, 0, 0),
                // 02:00 summer time, then 03:00 winter time: the second
                // 02:00 doesn't run.
                utc(2026, 10, 25, 2, 0),
                utc(2026, 10, 25, 3, 0),
                utc(2026, 10, 25, 4, 0)
            ]
        );
    }

    #[test]
    fn instants_of_wall_clock_times() {
        let local = |y, mo, d, h, mi| utc(y, mo, d, h, mi);
        assert_eq!(
            instant(&Stockholm, local(2026, 7, 1, 9, 0)),
            utc(2026, 7, 1, 7, 0)
        );
        assert_eq!(
            instant(&Stockholm, local(2026, 3, 29, 2, 30)),
            utc(2026, 3, 29, 1, 30)
        );
        assert_eq!(
            instant(&Stockholm, local(2026, 10, 25, 2, 30)),
            utc(2026, 10, 25, 0, 30)
        );
        assert_eq!(
            super::local(&Stockholm, utc(2026, 1, 1, 0, 0)),
            local(2026, 1, 1, 1, 0)
        );
    }

    #[test]
    fn the_shortest_gap_between_runs() {
        assert_eq!(cron("*/15 * * * *").shortest_gap_minutes(), 15);
        assert_eq!(cron("0,10 9 * * *").shortest_gap_minutes(), 10);
        // From 23:55 to 00:05 the next day.
        assert_eq!(cron("5,55 0,23 * * *").shortest_gap_minutes(), 10);
        assert_eq!(cron("0 9 * * *").shortest_gap_minutes(), 24 * 60);
        assert_eq!(cron("0 * * * *").shortest_gap_minutes(), 60);
        assert_eq!(cron("* * * * *").shortest_gap_minutes(), 1);
    }

    #[test]
    fn says_when_it_runs() {
        for (expr, says) in [
            ("0 9 * * *", "every day at 09:00"),
            ("@daily", "every day at 00:00"),
            ("0 9,17 * * *", "every day at 09:00 and 17:00"),
            ("15 * * * *", "every hour at :15"),
            ("0 9 * * 1-5", "every weekday at 09:00"),
            ("0 9 * * mon-fri", "every weekday at 09:00"),
            ("30 8 * * 1", "every Monday at 08:30"),
            ("0 16 * * 1,4", "every Monday and Thursday at 16:00"),
            (
                "0 16 * * 5,3,1",
                "every Monday, Wednesday and Friday at 16:00",
            ),
            ("0 10 * * 0", "every Sunday at 10:00"),
            ("0 10 * * 0,6", "every Saturday and Sunday at 10:00"),
            (
                "0 10 * * 6,0,1",
                "every Monday, Saturday and Sunday at 10:00",
            ),
            ("0 8 1 * *", "on the 1st of every month at 08:00"),
            (
                "0 8 1,15 * *",
                "on the 1st and 15th of every month at 08:00",
            ),
            ("0 8 22 * *", "on the 22nd of every month at 08:00"),
            (
                "*/15 9-17 * * 1-5",
                "on the cron schedule */15 9-17 * * 1-5",
            ),
            ("0 9 1-7 * 1", "on the cron schedule 0 9 1-7 * 1"),
            ("0 9 * 1 *", "on the cron schedule 0 9 * 1 *"),
            ("15 * * * 1", "on the cron schedule 15 * * * 1"),
        ] {
            assert_eq!(cron(expr).describe(), says, "{expr}");
        }
    }

    #[test]
    fn ordinals() {
        let all: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 31]
            .into_iter()
            .map(ordinal)
            .collect();
        assert_eq!(
            all,
            [
                "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "23rd", "31st"
            ]
        );
    }

    #[test]
    fn civil_dates_round_trip() {
        for days in [-1, 0, 59, 365, 10_957, 20_384, 20_749] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(20_749), (2026, 10, 23));
        assert_eq!(weekday(20_749), 5);
    }
}
