//! The story's clock. The cast's shots are taken on a Tuesday at 09:10;
//! the seed puts that moment as close to when it runs as it can, at a
//! minute when some time zone's clocks read 09:10, so that the times the
//! app shows in that zone are the story's. Days of the week stay the
//! calendar's: the story's Tuesday is whatever day it is there, and
//! schedules move their days with it.

use anyhow::{Context, Result, bail};
use fctl_domain::cron::{self, Zone};
use std::collections::BTreeSet;

/// Zones whose clocks never change, an hour or less apart, east to west:
/// in one of them, it was 09:10 within the last hour.
const ZONES: [(&str, i64); 34] = [
    ("Pacific/Kiritimati", 14 * 60),
    ("Pacific/Fakaofo", 13 * 60),
    ("Pacific/Tarawa", 12 * 60),
    ("Pacific/Noumea", 11 * 60),
    ("Australia/Brisbane", 10 * 60),
    ("Australia/Darwin", 9 * 60 + 30),
    ("Asia/Tokyo", 9 * 60),
    ("Australia/Eucla", 8 * 60 + 45),
    ("Asia/Singapore", 8 * 60),
    ("Asia/Bangkok", 7 * 60),
    ("Asia/Yangon", 6 * 60 + 30),
    ("Asia/Dhaka", 6 * 60),
    ("Asia/Kathmandu", 5 * 60 + 45),
    ("Asia/Kolkata", 5 * 60 + 30),
    ("Asia/Tashkent", 5 * 60),
    ("Asia/Kabul", 4 * 60 + 30),
    ("Asia/Dubai", 4 * 60),
    ("Asia/Tehran", 3 * 60 + 30),
    ("Europe/Istanbul", 3 * 60),
    ("Africa/Johannesburg", 2 * 60),
    ("Africa/Lagos", 60),
    ("Atlantic/Reykjavik", 0),
    ("Atlantic/Cape_Verde", -60),
    ("America/Noronha", -2 * 60),
    ("America/Sao_Paulo", -3 * 60),
    ("America/La_Paz", -4 * 60),
    ("America/Bogota", -5 * 60),
    ("America/Costa_Rica", -6 * 60),
    ("America/Phoenix", -7 * 60),
    ("Pacific/Pitcairn", -8 * 60),
    ("Pacific/Gambier", -9 * 60),
    ("Pacific/Marquesas", -9 * 60 - 30),
    ("Pacific/Honolulu", -10 * 60),
    ("Pacific/Pago_Pago", -11 * 60),
];

const DAY_MS: i64 = 86_400_000;

/// The moment of the shots, and the zone whose clocks read 09:10 then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Story {
    /// Milliseconds since the epoch.
    pub anchor: i64,
    pub zone: &'static str,
    /// How far ahead of UTC the zone's clocks are, in milliseconds.
    pub offset_ms: i64,
}

impl Zone for Story {
    fn offset_ms(&self, _utc_ms: i64) -> i64 {
        self.offset_ms
    }
}

impl Story {
    /// The latest moment at or before `now` at which one of [`ZONES`]
    /// read 09:10.
    pub fn at(now: i64) -> Self {
        let shot = (9 * 60 + 10) * 60_000;
        let mut best = Self {
            anchor: i64::MIN,
            zone: "UTC",
            offset_ms: 0,
        };
        for (zone, minutes) in ZONES {
            let offset_ms = minutes * 60_000;
            let since = ((now + offset_ms).rem_euclid(DAY_MS) - shot).rem_euclid(DAY_MS);
            if now - since > best.anchor {
                best = Self {
                    anchor: now - since,
                    zone,
                    offset_ms,
                };
            }
        }
        best
    }

    /// How many days after Tuesday the story's day falls in its zone.
    pub fn weekday_shift(&self) -> u32 {
        let day = (self.anchor + self.offset_ms).div_euclid(DAY_MS);
        (cron::weekday(day) + 7 - 2) % 7
    }

    /// `expr` (a five-field cron) on the days of the week the story's
    /// Tuesday moved to: `0 2 * * 2` is `0 2 * * 0` when it's a Sunday.
    pub fn cron(&self, expr: &str) -> Result<String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        let [minute, hour, dom, month, dow] = fields[..] else {
            bail!("{expr:?} isn't a cron of five fields");
        };
        let shift = self.weekday_shift();
        if dow == "*" || shift == 0 {
            return Ok(expr.to_owned());
        }
        let mut days = BTreeSet::new();
        for item in dow.split(',') {
            let (a, b) = item.split_once('-').unwrap_or((item, item));
            let parse = |d: &str| {
                d.parse::<u32>()
                    .ok()
                    .filter(|d| *d <= 7)
                    .with_context(|| format!("{expr:?}: days of the week are numbers"))
            };
            for d in parse(a)?..=parse(b)? {
                days.insert((d % 7 + shift) % 7);
            }
        }
        let days: Vec<String> = days.iter().map(u32::to_string).collect();
        Ok(format!("{minute} {hour} {dom} {month} {}", days.join(",")))
    }

    /// When `expr` (as [`Story::cron`] moves it) runs next after the
    /// moment of the shots.
    pub fn next_run(&self, expr: &str) -> Result<Option<i64>> {
        let c = cron::Cron::parse(&self.cron(expr)?).map_err(anyhow::Error::msg)?;
        Ok(c.next_after(self, self.anchor))
    }

    /// A moment as fctl writes it in its zone: `Tuesday 27 October 2026 at
    /// 13:15`.
    pub fn stamp(&self, at: i64) -> String {
        fctl_domain::schedules::stamp(self, at)
    }
}

#[cfg(test)]
mod tests {
    use super::{DAY_MS, Story, ZONES};

    /// 2026-10-25, a Sunday, at 13:47 UTC.
    const NOW: i64 = 1_792_936_020_000;

    #[test]
    fn the_shots_are_at_0910_somewhere_within_the_hour() {
        let s = Story::at(NOW);
        assert!(s.anchor <= NOW && NOW - s.anchor < 3_600_000, "{s:?}");
        let local = (s.anchor + s.offset_ms).rem_euclid(DAY_MS);
        assert_eq!(local, (9 * 60 + 10) * 60_000);
        // 13:47 UTC: 09:10 was 37 minutes ago at -04:00, 7 at -04:30 (none),
        // so -04:00, in La Paz.
        assert_eq!(s.zone, "America/La_Paz");
        // Every minute of a day finds one within the hour.
        for m in 0..24 * 60 {
            let now = NOW + m * 60_000;
            assert!(now - Story::at(now).anchor < 3_600_000);
        }
        let mut offsets: Vec<i64> = ZONES.iter().map(|(_, o)| *o).collect();
        offsets.dedup();
        assert_eq!(offsets.len(), ZONES.len(), "two zones share an offset");
    }

    #[test]
    fn schedules_keep_to_the_story_s_days() -> anyhow::Result<()> {
        // A Sunday in La Paz: the story's Tuesday is a Sunday, five days on.
        let s = Story::at(NOW);
        assert_eq!(s.weekday_shift(), 5);
        assert_eq!(s.cron("0 2 * * 2")?, "0 2 * * 0");
        assert_eq!(s.cron("0 3 * * 6")?, "0 3 * * 4");
        assert_eq!(s.cron("0 1 * * *")?, "0 1 * * *");
        assert_eq!(s.cron("0 9 * * 1-2")?, "0 9 * * 0,6");
        assert!(s.cron("0 9 * * MON").is_err());
        // The weekly one ran this morning, so it runs next in a week.
        let next = s.next_run("0 2 * * 2")?.unwrap_or_default();
        assert_eq!(next - s.anchor, 7 * DAY_MS - (7 * 60 + 10) * 60_000);
        assert_eq!(
            s.next_run("0 1 * * *")?.map(|n| n - s.anchor),
            Some((15 * 60 + 50) * 60_000)
        );
        assert_eq!(
            s.stamp(s.anchor + (4 * 60 + 5) * 60_000),
            "Sunday 25 October 2026 at 13:15"
        );
        Ok(())
    }
}
