//! How SessionDO keeps D1's row of a session in step with its summary. The
//! object has the session as it is; D1's row is the copy that lists,
//! searches and the API read, and what they decide by. A write of the
//! summary that fails (D1 can fail for a moment) is tried again, sooner at
//! first and then every few minutes for an hour, until one lands or a
//! later change writes it anyway; and an older write that lands after a
//! newer one is followed by another of the summary as it is then.

/// The shortest wait before a failed write is tried again.
pub const RETRY_FIRST_MS: i64 = 5_000;
/// The longest wait between tries.
pub const RETRY_MAX_MS: i64 = 5 * 60 * 1000;
/// How long after writes started failing the object stops trying again by
/// itself: then the next change to the summary, or the next time its alarm
/// goes off for something else, writes it.
pub const GIVE_UP_MS: i64 = 60 * 60 * 1000;

/// How long after a failed write the object tries again, given since when
/// its writes have failed in a row (`since`): as long again as they have
/// failed, from [`RETRY_FIRST_MS`] to [`RETRY_MAX_MS`], so that tries thin
/// out the longer D1 stays down, however often the session changed
/// meanwhile; `None` once they have failed for [`GIVE_UP_MS`].
pub fn retry_in(since: i64, now: i64) -> Option<i64> {
    let failing = now.saturating_sub(since).max(0);
    (failing < GIVE_UP_MS).then(|| failing.clamp(RETRY_FIRST_MS, RETRY_MAX_MS))
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
    fn tries_again_as_long_after_as_writes_have_failed_for_an_hour() {
        let since = 1_000_000;
        assert_eq!(retry_in(since, since), Some(RETRY_FIRST_MS));
        assert_eq!(retry_in(since, since + 2_000), Some(RETRY_FIRST_MS));
        assert_eq!(retry_in(since, since + 40_000), Some(40_000));
        assert_eq!(retry_in(since, since + 20 * 60_000), Some(RETRY_MAX_MS));
        assert_eq!(retry_in(since, since + GIVE_UP_MS - 1), Some(RETRY_MAX_MS));
        assert_eq!(retry_in(since, since + GIVE_UP_MS), None);
        // A clock that went back waits the least.
        assert_eq!(retry_in(since, since - 10_000), Some(RETRY_FIRST_MS));
    }

    #[test]
    fn a_busy_session_keeps_trying_for_the_hour() {
        // It changes every second while D1 is down: each failed write asks
        // again, and none of them uses the tries up.
        let since = 0;
        let mut now = since;
        let mut tries = 0;
        while let Some(wait) = retry_in(since, now) {
            assert!((RETRY_FIRST_MS..=RETRY_MAX_MS).contains(&wait));
            now += if now < 60_000 { 1_000 } else { wait };
            tries += 1;
        }
        assert!(now >= GIVE_UP_MS, "{now}");
        assert!(tries > 60, "{tries}");
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
