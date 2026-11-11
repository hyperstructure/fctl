//! How SessionDO keeps D1's row of a session in step with its summary. The
//! object has the session as it is; D1's row is the copy that lists,
//! searches and the API read, and what they decide by. A write of the
//! summary that fails (D1 can fail for a moment) is tried again, sooner at
//! first and then every few minutes, until one lands or a later change
//! writes it anyway; and an older write that lands after a newer one is
//! followed by another of the summary as it is then.

/// The first wait before a failed write is tried again; each failure after
/// it doubles it, up to [`RETRY_MAX_MS`].
pub const RETRY_FIRST_MS: i64 = 5_000;
/// The longest wait between tries.
pub const RETRY_MAX_MS: i64 = 5 * 60 * 1000;
/// The most tries after failures in a row, about an hour of them: then the
/// next change to the summary writes it.
pub const RETRIES: u32 = 18;

/// How long after its `failures`-th failed write in a row the object tries
/// again; `None` once it has tried [`RETRIES`] times.
pub fn retry_in(failures: u32) -> Option<i64> {
    let doublings = failures.checked_sub(1).filter(|n| *n < RETRIES)?;
    Some(
        RETRY_FIRST_MS
            .saturating_mul(1 << doublings.min(16))
            .min(RETRY_MAX_MS),
    )
}

/// The writes of the summary under way and done, by the order they
/// started in. The object takes other requests while a write waits for D1,
/// so two can be under way at once, and D1 may finish the newer first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Writes {
    /// How many have started.
    started: u64,
    /// The newest that has landed.
    landed: u64,
}

/// What a write that landed leaves to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landed {
    /// D1 has the summary as this write started with, and no write under
    /// way would change it: none failed that it doesn't make up for.
    Current,
    /// A newer write started since, which says whether D1 has the
    /// summary: it may still fail, or have failed.
    Newer,
    /// A newer write landed before it, which this one undid: write the
    /// summary again.
    Overtaken,
}

impl Writes {
    /// A write starts: its number, for [`Writes::landed`].
    pub fn start(&mut self) -> u64 {
        self.started += 1;
        self.started
    }

    /// Write `n` landed.
    pub fn landed(&mut self, n: u64) -> Landed {
        let overtaken = self.landed > n;
        self.landed = self.landed.max(n);
        if overtaken {
            Landed::Overtaken
        } else if n < self.started {
            Landed::Newer
        } else {
            Landed::Current
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tries_again_sooner_first_then_every_five_minutes_for_about_an_hour() {
        assert_eq!(retry_in(0), None);
        let waits: Vec<i64> = (1..=RETRIES).map_while(retry_in).collect();
        assert_eq!(
            &waits[..7],
            &[5_000, 10_000, 20_000, 40_000, 80_000, 160_000, 300_000]
        );
        assert_eq!(waits.len(), RETRIES as usize);
        assert!(waits[7..].iter().all(|w| *w == RETRY_MAX_MS));
        let total: i64 = waits.iter().sum();
        assert!((55 * 60_000..75 * 60_000).contains(&total), "{total}");
        assert_eq!(retry_in(RETRIES + 1), None);
        assert_eq!(retry_in(u32::MAX), None);
    }

    #[test]
    fn writes_that_land_in_order_leave_nothing_to_do() {
        let mut w = Writes::default();
        let a = w.start();
        assert_eq!(w.landed(a), Landed::Current);
        let b = w.start();
        let c = w.start();
        assert_eq!(w.landed(b), Landed::Newer);
        assert_eq!(w.landed(c), Landed::Current);
    }

    #[test]
    fn an_older_write_landing_after_a_newer_one_writes_again() {
        // The summary said queued as the first started, failed as the
        // second did; D1 finished the second first.
        let mut w = Writes::default();
        let queued = w.start();
        let failed = w.start();
        assert_eq!(w.landed(failed), Landed::Current);
        assert_eq!(w.landed(queued), Landed::Overtaken);
        // The write that puts it right.
        let again = w.start();
        assert_eq!(w.landed(again), Landed::Current);
    }

    #[test]
    fn an_older_write_landing_doesnt_settle_a_newer_one_under_way() {
        // The newer one may still fail: until it lands, D1 may be behind.
        let mut w = Writes::default();
        let older = w.start();
        let newer = w.start();
        assert_eq!(w.landed(older), Landed::Newer);
        assert_eq!(w.landed(newer), Landed::Current);
    }
}
