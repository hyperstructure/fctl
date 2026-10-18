//! Whether a link to fctl-agent is alive, and when to attach again.
//!
//! fctl-agent says hello the moment the host attaches, and answers every
//! ping, so a link that stays quiet is dead: a `docker exec` stuck in an
//! overloaded runtime, or one that never reached the sandbox. The host drops
//! it, which kills the exec, and attaches again, waiting longer after each
//! attempt that fails. A runtime that hangs may come back, so the host keeps
//! trying; one that answers that the agent can't be reached gets two minutes.
//!
//! The people watching the session hear about it ([`Reach`]): once the host
//! has had no working link for a few seconds, the session page says that the
//! agent can't be reached, and since when.

use std::time::{Duration, Instant};

/// How long fctl-agent has to say hello after the host attaches.
pub const HELLO_WITHIN: Duration = Duration::from_secs(30);
/// How often the host pings an attached agent.
pub const PING_EVERY: Duration = Duration::from_secs(15);
/// A link that carries nothing for this long is dead.
pub const SILENT_FOR: Duration = Duration::from_secs(60);

/// The waits between attempts: they double from the first to the longest.
pub const FIRST_WAIT: Duration = Duration::from_secs(1);
pub const LONGEST_WAIT: Duration = Duration::from_secs(30);
/// Attaching that fails outright for this long gives up on the sandbox.
pub const GIVE_UP_AFTER: Duration = Duration::from_secs(120);
/// How long the host may have no working link before the session's viewers
/// hear that the agent can't be reached. Attaching again after a link closes
/// takes a second or two.
pub const REPORT_AFTER: Duration = Duration::from_secs(10);

/// One attached link, as the host hears it.
pub struct Health {
    opened: Instant,
    heard: Option<Instant>,
    pinged: Instant,
}

/// What to do about a link now.
#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    Fine,
    Ping,
    /// Drop it, for this reason.
    Dead(&'static str),
}

impl Health {
    pub fn new(now: Instant) -> Self {
        Self {
            opened: now,
            heard: None,
            pinged: now,
        }
    }

    /// A frame came from the agent.
    pub fn heard(&mut self, now: Instant) {
        self.heard = Some(now);
    }

    /// Whether the agent has spoken on this link.
    pub fn answered(&self) -> bool {
        self.heard.is_some()
    }

    /// When the agent last spoke on this link, or when it opened.
    pub fn last_heard(&self) -> Instant {
        self.heard.unwrap_or(self.opened)
    }

    pub fn check(&mut self, now: Instant) -> Check {
        match self.heard {
            None if now.duration_since(self.opened) >= HELLO_WITHIN => {
                return Check::Dead("fctl-agent did not say hello within 30 seconds");
            }
            Some(at) if now.duration_since(at) >= SILENT_FOR => {
                return Check::Dead("nothing came from fctl-agent for a minute");
            }
            _ => {}
        }
        if now.duration_since(self.pinged) >= PING_EVERY {
            self.pinged = now;
            return Check::Ping;
        }
        Check::Fine
    }
}

/// When to attach again after a link failed.
pub struct Retry {
    wait: Duration,
    /// Since when attaching has failed outright.
    failing_since: Option<Instant>,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            wait: FIRST_WAIT,
            failing_since: None,
        }
    }
}

impl Retry {
    /// The agent said hello: the next failure starts over.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// A link went quiet, or the agent never answered on it. The runtime
    /// may only be slow, so this never gives up.
    pub fn stalled(&mut self) -> Duration {
        self.next_wait()
    }

    /// Attaching failed outright: the exec could not start, or ended before
    /// the agent said hello. `None` once that has gone on for
    /// [`GIVE_UP_AFTER`].
    pub fn failed(&mut self, now: Instant) -> Option<Duration> {
        let since = *self.failing_since.get_or_insert(now);
        (now.duration_since(since) < GIVE_UP_AFTER).then(|| self.next_wait())
    }

    fn next_wait(&mut self) -> Duration {
        let wait = self.wait;
        self.wait = (self.wait * 2).min(LONGEST_WAIT);
        wait
    }
}

/// Whether the host reaches the agent, as the session's viewers hear it:
/// that it can't, once no link has worked for [`REPORT_AFTER`] (again when
/// what goes wrong changes), and that it can once a link says hello again.
#[derive(Default)]
pub struct Reach {
    /// Since when no link has worked, and what went wrong last.
    down: Option<(Instant, String)>,
    /// What went wrong, as last reported, while the viewers think the agent
    /// can't be reached.
    reported: Option<String>,
}

/// What to tell the session's viewers.
#[derive(Debug, PartialEq, Eq)]
pub enum Reached {
    /// The agent can't be reached since `since`, for `reason`.
    Not { since: Instant, reason: String },
    /// It can again.
    Again,
}

impl Reach {
    /// A link said hello. `Again` when the viewers heard that it couldn't.
    pub fn up(&mut self) -> Option<Reached> {
        self.down = None;
        self.reported.take().map(|_| Reached::Again)
    }

    /// No link works, since `since`, because of `reason`. While links keep
    /// failing, the first failure's time stands and the reason is the latest.
    pub fn down(&mut self, since: Instant, reason: impl Into<String>) {
        let reason = reason.into();
        match &mut self.down {
            Some((_, why)) => *why = reason,
            None => self.down = Some((since, reason)),
        }
    }

    /// What to report now, if anything.
    pub fn check(&mut self, now: Instant) -> Option<Reached> {
        let (since, reason) = self.down.as_ref()?;
        if now.duration_since(*since) < REPORT_AFTER || self.reported.as_ref() == Some(reason) {
            return None;
        }
        self.reported = Some(reason.clone());
        Some(Reached::Not {
            since: *since,
            reason: reason.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn an_agent_that_answers_is_pinged_and_kept() {
        let t0 = Instant::now();
        let mut h = Health::new(t0);
        h.heard(t0 + secs(1));
        assert_eq!(h.check(t0 + secs(14)), Check::Fine);
        assert_eq!(h.check(t0 + secs(15)), Check::Ping);
        assert_eq!(h.check(t0 + secs(16)), Check::Fine);
        // Its pong.
        h.heard(t0 + secs(16));
        assert_eq!(h.check(t0 + secs(30)), Check::Ping);
        h.heard(t0 + secs(30));
        assert_eq!(h.check(t0 + secs(89)), Check::Ping);
        assert!(h.answered());
    }

    #[test]
    fn a_link_that_never_answers_or_goes_quiet_is_dead() {
        let t0 = Instant::now();
        let mut silent = Health::new(t0);
        assert_eq!(silent.check(t0 + secs(15)), Check::Ping);
        assert_eq!(silent.check(t0 + secs(29)), Check::Fine);
        assert_eq!(
            silent.check(t0 + secs(30)),
            Check::Dead("fctl-agent did not say hello within 30 seconds")
        );
        assert!(!silent.answered());

        let mut quiet = Health::new(t0);
        quiet.heard(t0 + secs(1));
        for t in [15, 30, 45, 60] {
            assert_eq!(quiet.check(t0 + secs(t)), Check::Ping, "{t}");
        }
        assert_eq!(
            quiet.check(t0 + secs(61)),
            Check::Dead("nothing came from fctl-agent for a minute")
        );
    }

    #[test]
    fn stalls_wait_longer_each_time_and_never_give_up() {
        let mut r = Retry::default();
        let waits: Vec<u64> = (0..8).map(|_| r.stalled().as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30, 30]);
        r.reset();
        assert_eq!(r.stalled(), secs(1));
    }

    #[test]
    fn failing_outright_gives_up_after_two_minutes() {
        let t0 = Instant::now();
        let mut r = Retry::default();
        assert_eq!(r.failed(t0), Some(secs(1)));
        assert_eq!(r.failed(t0 + secs(5)), Some(secs(2)));
        assert_eq!(r.failed(t0 + secs(119)), Some(secs(4)));
        assert_eq!(r.failed(t0 + secs(120)), None);
        // A hello in between starts over.
        let mut r = Retry::default();
        assert_eq!(r.failed(t0), Some(secs(1)));
        r.reset();
        assert_eq!(r.failed(t0 + secs(130)), Some(secs(1)));
        assert_eq!(r.failed(t0 + secs(200)), Some(secs(2)));
    }

    #[test]
    fn a_link_back_within_seconds_is_never_reported() {
        let t0 = Instant::now();
        let mut reach = Reach::default();
        // The first attach, and a link that closes and comes back.
        reach.down(t0, "no answer from fctl-agent yet");
        assert_eq!(reach.check(t0 + secs(2)), None);
        assert_eq!(reach.up(), None);
        reach.down(t0 + secs(30), "the link to fctl-agent closed");
        assert_eq!(reach.check(t0 + secs(39)), None);
        assert_eq!(reach.up(), None);
        assert_eq!(reach.check(t0 + secs(60)), None);
    }

    #[test]
    fn a_dead_link_is_reported_from_when_the_agent_was_last_heard() {
        let t0 = Instant::now();
        let mut reach = Reach::default();
        // Silent for a minute: down since it was last heard, so at once.
        reach.down(t0, "nothing came from fctl-agent for a minute");
        let not = |since, reason: &str| {
            Some(Reached::Not {
                since,
                reason: reason.to_owned(),
            })
        };
        assert_eq!(
            reach.check(t0 + secs(60)),
            not(t0, "nothing came from fctl-agent for a minute")
        );
        assert_eq!(reach.check(t0 + secs(61)), None);
        // Attaching again fails: same time, the new reason.
        reach.down(
            t0 + secs(62),
            "`docker exec` did not finish within 60 seconds",
        );
        assert_eq!(
            reach.check(t0 + secs(62)),
            not(t0, "`docker exec` did not finish within 60 seconds")
        );
        assert_eq!(reach.check(t0 + secs(90)), None);
        // A link says hello.
        assert_eq!(reach.up(), Some(Reached::Again));
        assert_eq!(reach.up(), None);
        assert_eq!(reach.check(t0 + secs(200)), None);
    }

    #[test]
    fn a_link_that_never_comes_up_is_reported_after_a_few_seconds() {
        let t0 = Instant::now();
        let mut reach = Reach::default();
        reach.down(t0, "no answer from fctl-agent yet");
        assert_eq!(reach.check(t0 + secs(9)), None);
        assert_eq!(
            reach.check(t0 + REPORT_AFTER),
            Some(Reached::Not {
                since: t0,
                reason: "no answer from fctl-agent yet".into()
            })
        );
    }
}
