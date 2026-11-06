//! What `fctl attach` says when the agent's work stops coming through: the
//! session's runner went away, it can't reach the agent in the sandbox, or
//! it paused the sandbox for want of disk. The agent may well be working;
//! its events wait in the sandbox. Like the web app, it says nothing about a
//! runner that is back within seconds (a host restarting). It also says
//! when the runner is short of room on its disk, and when it has room again.

use crate::style;
use fctl_domain::size;
use fctl_proto::runner::{DiskRoom, RunnerDisk};
use fctl_proto::session::Session;
use fctl_proto::uplink::{AgentUnreachable, SandboxPaused};
use std::time::{Duration, Instant};

/// How long a runner may be gone before `fctl attach` says so.
pub const AWAY_GRACE: Duration = Duration::from_secs(10);
/// A runner gone this long when `fctl attach` learns of it is reported at once.
const LONG_GONE_MS: i64 = 60_000;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Said {
    Away,
    Unreachable(String),
    Paused(String),
}

/// The session's runner, as `fctl attach` has told it.
#[derive(Default)]
pub struct Watch {
    runner: Option<String>,
    active: bool,
    /// While the runner isn't connected: when it was last heard from, and
    /// when this learned that.
    away: Option<(i64, Instant)>,
    unreachable: Option<AgentUnreachable>,
    paused: Option<SandboxPaused>,
    said: Option<Said>,
    /// How short of disk the runner was, as last told.
    disk_said: Option<DiskRoom>,
}

impl Watch {
    /// The session changed. One that ends has nothing more to say.
    pub fn session(&mut self, s: &Session) {
        self.runner = s.runner.as_ref().map(|r| r.name.clone());
        self.active = s.status.is_active();
        if !self.active {
            self.said = None;
            self.disk_said = None;
        }
    }

    /// The runner's state, from the snapshot or `runner_connected`. Returns
    /// a line to print, if any.
    pub fn runner(
        &mut self,
        connected: bool,
        last_seen_at: Option<i64>,
        agent: Option<&AgentUnreachable>,
        paused: Option<&SandboxPaused>,
        now: Instant,
        now_ms: i64,
    ) -> Option<String> {
        self.away = match (connected, last_seen_at) {
            (false, Some(t)) => Some((t, self.away.map_or(now, |(_, noticed)| noticed))),
            _ => None,
        };
        self.unreachable = if connected { agent.cloned() } else { None };
        self.paused = if connected { paused.cloned() } else { None };
        self.update(now, now_ms)
    }

    /// How short of disk the runner is, from the snapshot or
    /// `runner_connected`. Returns a line to print when that changed.
    pub fn disk(&mut self, disk: Option<&RunnerDisk>) -> Option<String> {
        let room = disk.map(|d| d.room).filter(|_| self.active);
        if room == self.disk_said {
            return None;
        }
        self.disk_said = room;
        let who = self.who();
        let line = match disk.filter(|_| self.active) {
            Some(d) if d.room == DiskRoom::Out => style::red(&format!(
                "◌ {who} is nearly out of disk: {} free; the agent's writes are about to fail",
                size::memory(d.free_mb)
            )),
            Some(d) => style::amber(&format!(
                "◌ {who} is low on disk: {} free, below the {} its host keeps free",
                size::memory(d.free_mb),
                size::memory(d.min_free_mb)
            )),
            None if self.active => style::dim(&format!("● {who} has room on its disk again")),
            None => return None,
        };
        Some(line)
    }

    /// `runner now`, or `the runner`.
    fn who(&self) -> String {
        self.runner
            .as_deref()
            .map_or_else(|| "the runner".to_owned(), |r| format!("runner {r}"))
    }

    /// When the runner's absence is due to be told, while it is not yet.
    pub fn due(&self) -> Option<Instant> {
        let (_, noticed) = self.away?;
        (self.active && self.said != Some(Said::Away)).then_some(noticed + AWAY_GRACE)
    }

    /// Time passed: returns the line about a runner that stays away.
    pub fn tick(&mut self, now: Instant, now_ms: i64) -> Option<String> {
        self.update(now, now_ms)
    }

    fn update(&mut self, now: Instant, now_ms: i64) -> Option<String> {
        if !self.active {
            self.said = None;
            return None;
        }
        let who = self.who();
        let line = match (&self.away, &self.paused, &self.unreachable) {
            (Some((seen, noticed)), _, _) => {
                let due = now >= *noticed + AWAY_GRACE || now_ms - seen >= LONG_GONE_MS;
                if !due || self.said == Some(Said::Away) {
                    return None;
                }
                self.said = Some(Said::Away);
                style::amber(&format!(
                    "◌ {who} is away (last heard from {}); the agent's work shows up here when it's back",
                    style::ago_from(*seen, now_ms)
                ))
            }
            (None, Some(p), _) => {
                let said = Some(Said::Paused(p.reason.clone()));
                if self.said == said {
                    return None;
                }
                self.said = said;
                style::amber(&format!(
                    "◌ {who} paused the sandbox ({}): {}; the agent carries on once there is room",
                    style::ago_from(p.since, now_ms),
                    p.reason.trim_end_matches(['.', ' '])
                ))
            }
            (None, None, Some(a)) => {
                let said = Some(Said::Unreachable(a.reason.clone()));
                if self.said == said {
                    return None;
                }
                self.said = said;
                style::amber(&format!(
                    "◌ {who} can't reach the agent in its sandbox (last reached {}): {}; it keeps trying",
                    style::ago_from(a.since, now_ms),
                    a.reason.trim_end_matches(['.', ' '])
                ))
            }
            (None, None, None) => match self.said.take()? {
                Said::Away => style::dim(&format!("● {who} is back")),
                Said::Unreachable(_) => style::dim(&format!("● {who} reaches the agent again")),
                Said::Paused(_) => style::dim(&format!("● {who} resumed the sandbox")),
            },
        };
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::session::SessionStatus;

    fn session(status: SessionStatus) -> Session {
        serde_json::from_value(serde_json::json!({
            "id": "ses_1", "project_id": "prj_1", "project_slug": "app", "number": 1,
            "title": "T", "status": status, "created_by": { "id": "usr_1", "name": "Ada" },
            "created_at": 0, "updated_at": 0, "runner": { "id": "rnr_1", "name": "now" },
            "stats": {}, "archived": false
        }))
        .unwrap_or_else(|e| panic!("session: {e}"))
    }

    fn unreachable(reason: &str) -> AgentUnreachable {
        AgentUnreachable {
            since: 0,
            reason: reason.into(),
        }
    }

    #[test]
    fn a_runner_back_within_seconds_goes_unmentioned() {
        let (t0, ms) = (Instant::now(), 100_000);
        let mut w = Watch::default();
        w.session(&session(SessionStatus::Running));
        assert_eq!(w.runner(false, Some(ms - 1_000), None, None, t0, ms), None);
        assert_eq!(w.due(), Some(t0 + AWAY_GRACE));
        assert_eq!(w.tick(t0 + Duration::from_secs(3), ms + 3_000), None);
        assert_eq!(w.runner(true, None, None, None, t0, ms + 4_000), None);
        assert_eq!(w.due(), None);
    }

    #[test]
    fn a_runner_that_stays_away_is_told_once_and_so_is_its_return() {
        let (t0, ms) = (Instant::now(), 100_000);
        let mut w = Watch::default();
        w.session(&session(SessionStatus::Idle));
        assert_eq!(w.runner(false, Some(ms - 20_000), None, None, t0, ms), None);
        let later = t0 + AWAY_GRACE;
        let line = w.tick(later, ms + 10_000).unwrap_or_default();
        assert!(
            line.contains("runner now is away (last heard from just now)"),
            "{line}"
        );
        assert_eq!(w.due(), None);
        assert_eq!(w.tick(later, ms + 20_000), None);
        // Another message while away keeps quiet.
        assert_eq!(
            w.runner(false, Some(ms - 20_000), None, None, later, ms + 20_000),
            None
        );
        let back = w
            .runner(true, None, None, None, later, ms + 30_000)
            .unwrap_or_default();
        assert!(back.contains("runner now is back"), "{back}");
    }

    #[test]
    fn a_runner_long_gone_is_told_at_once() {
        let (t0, ms) = (Instant::now(), 1_000_000);
        let mut w = Watch::default();
        w.session(&session(SessionStatus::Running));
        let line = w.runner(false, Some(ms - 600_000), None, None, t0, ms);
        assert!(line.is_some_and(|l| l.contains("10m ago")));
    }

    #[test]
    fn an_unreachable_agent_is_told_with_each_new_reason() {
        let (t0, ms) = (Instant::now(), 100_000);
        let mut w = Watch::default();
        w.session(&session(SessionStatus::Running));
        let a = unreachable("nothing came from fctl-agent for a minute");
        let line = w
            .runner(true, None, Some(&a), None, t0, ms)
            .unwrap_or_default();
        assert!(line.ends_with("for a minute; it keeps trying"), "{line}");
        assert_eq!(w.runner(true, None, Some(&a), None, t0, ms), None);
        let b = unreachable("`docker exec` did not finish within 60 seconds.");
        let line = w
            .runner(true, None, Some(&b), None, t0, ms)
            .unwrap_or_default();
        assert!(line.ends_with("60 seconds; it keeps trying"), "{line}");
        let again = w.runner(true, None, None, None, t0, ms).unwrap_or_default();
        assert!(
            again.contains("runner now reaches the agent again"),
            "{again}"
        );
    }

    #[test]
    fn a_paused_sandbox_is_told_and_so_is_its_resuming() {
        let (t0, ms) = (Instant::now(), 100_000);
        let mut w = Watch::default();
        w.session(&session(SessionStatus::Running));
        let p = SandboxPaused {
            since: ms - 60_000,
            reason: "it is nearly out of disk, with 800 MiB free".into(),
        };
        let a = unreachable("nothing came from fctl-agent for a minute");
        let line = w
            .runner(true, None, Some(&a), Some(&p), t0, ms)
            .unwrap_or_default();
        assert!(line.contains("runner now paused the sandbox (1m ago): it is nearly out of disk, with 800 MiB free; the agent carries on once there is room"), "{line}");
        assert_eq!(w.runner(true, None, None, Some(&p), t0, ms), None);
        let back = w.runner(true, None, None, None, t0, ms).unwrap_or_default();
        assert!(back.contains("runner now resumed the sandbox"), "{back}");
    }

    #[test]
    fn a_runner_short_of_disk_is_told_as_it_gets_shorter_and_once_it_has_room() {
        let mut w = Watch::default();
        w.session(&session(SessionStatus::Running));
        assert_eq!(w.disk(None), None);
        let low = RunnerDisk {
            room: DiskRoom::Low,
            free_mb: 12_595,
            min_free_mb: 20 * 1024,
            since: 0,
        };
        let line = w.disk(Some(&low)).unwrap_or_default();
        assert!(
            line.contains(
                "runner now is low on disk: 12.3 GiB free, below the 20 GiB its host keeps free"
            ),
            "{line}"
        );
        // Less free, as low: nothing new.
        let lower = RunnerDisk {
            free_mb: 10_000,
            ..low.clone()
        };
        assert_eq!(w.disk(Some(&lower)), None);
        let out = RunnerDisk {
            room: DiskRoom::Out,
            free_mb: 800,
            ..low
        };
        let line = w.disk(Some(&out)).unwrap_or_default();
        assert!(
            line.contains("runner now is nearly out of disk: 800 MiB free; the agent's writes are about to fail"),
            "{line}"
        );
        let line = w.disk(None).unwrap_or_default();
        assert!(
            line.contains("runner now has room on its disk again"),
            "{line}"
        );
        // A session that ended hears nothing more.
        w.session(&session(SessionStatus::Stopped));
        assert_eq!(w.disk(Some(&out)), None);
    }

    #[test]
    fn a_session_that_ends_has_nothing_more_to_say() {
        let (t0, ms) = (Instant::now(), 100_000);
        let mut w = Watch::default();
        w.session(&session(SessionStatus::Running));
        let a = unreachable("x");
        assert!(w.runner(true, None, Some(&a), None, t0, ms).is_some());
        w.session(&session(SessionStatus::Stopped));
        // Its uplink closes as it ends.
        assert_eq!(w.runner(false, Some(ms), None, None, t0, ms), None);
        assert_eq!(w.due(), None);
        assert_eq!(w.tick(t0 + Duration::from_secs(60), ms + 60_000), None);
    }
}
