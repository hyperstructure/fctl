//! What the fleet does with a run that a runner reports but that the fleet
//! hasn't assigned to it: the runner was away, or the session was deleted,
//! stopped or resumed elsewhere meanwhile.

use fctl_proto::runner::StopReason;
use fctl_proto::session::SessionStatus;

/// The session as the control plane has it.
#[derive(Clone, Copy, Debug)]
pub struct Known<'a> {
    /// The runner of its current or last run. The fleet forgets it when it
    /// ends a run itself: a runner that stayed away, or was uninstalled.
    pub runner_id: Option<&'a str>,
    pub status: SessionStatus,
    /// Someone put it away.
    pub archived: bool,
}

/// What becomes of the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unassigned {
    /// It is still the session's run on this runner.
    Keep,
    /// The fleet stopped the session because this runner stayed away, and
    /// no other run has started since: it carries on here, and its agent's
    /// work from meanwhile comes through.
    CarryOn,
    /// The run stops, for this reason.
    Stop(StopReason),
}

/// `known` is the session (`None`: deleted), and `stopped_away` whether the
/// fleet stopped it because `runner_id` stayed away.
pub fn unassigned(runner_id: &str, known: Option<Known<'_>>, stopped_away: bool) -> Unassigned {
    let Some(known) = known else {
        return Unassigned::Stop(StopReason::Deleted);
    };
    match known.runner_id {
        // It moved to another runner.
        Some(id) if id != runner_id => Unassigned::Stop(StopReason::Shutdown),
        // Stopped, resumed but waiting for a runner, or kept up to date by
        // the run's uplink, which stayed open. Not a resume that failed, nor
        // a session someone put away.
        _ if stopped_away && known.status != SessionStatus::Failed && !known.archived => {
            Unassigned::CarryOn
        }
        Some(_) if !known.status.is_terminal() => Unassigned::Keep,
        // Someone stopped it, or it ended.
        _ => Unassigned::Stop(StopReason::User),
    }
}

/// The run the fleet has a session on: its runner, and the run's id when
/// the fleet started it.
#[derive(Clone, Copy, Debug)]
pub struct Current<'a> {
    pub runner_id: &'a str,
    pub run_id: Option<&'a str>,
}

/// Whether a runner's report that a run of the session ended is about an
/// earlier run than `current`: one on another runner, such as the run a
/// runner that was away comes back with after the session moved, or an
/// earlier one on the same runner. Such a report leaves the session alone.
pub fn ended_earlier(runner_id: &str, run_id: Option<&str>, current: Option<Current<'_>>) -> bool {
    current.is_some_and(|c| {
        c.runner_id != runner_id
            || matches!((run_id, c.run_id), (Some(ended), Some(cur)) if ended != cur)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_end_of_the_current_run_counts() {
        let current = Some(Current {
            runner_id: "r1",
            run_id: Some("run_2"),
        });
        assert!(!ended_earlier("r1", Some("run_2"), current));
        // Hosts that recovered a session after a restart may not know its run.
        assert!(!ended_earlier("r1", None, current));
        // Nothing runs it any more: the end is news.
        assert!(!ended_earlier("r1", Some("run_1"), None));
    }

    #[test]
    fn the_end_of_an_earlier_run_does_not() {
        let current = Some(Current {
            runner_id: "r2",
            run_id: Some("run_2"),
        });
        assert!(ended_earlier("r1", Some("run_1"), current));
        assert!(ended_earlier("r1", None, current));
        assert!(ended_earlier("r2", Some("run_1"), current));
    }

    fn known(runner_id: Option<&str>, status: SessionStatus) -> Option<Known<'_>> {
        Some(Known {
            runner_id,
            status,
            archived: false,
        })
    }

    #[test]
    fn a_session_stopped_while_its_runner_was_away_carries_on() {
        let stopped = known(None, SessionStatus::Stopped);
        assert_eq!(unassigned("r1", stopped, true), Unassigned::CarryOn);
    }

    #[test]
    fn so_does_one_resumed_meanwhile_that_no_runner_took() {
        let queued = known(None, SessionStatus::Queued);
        assert_eq!(unassigned("r1", queued, true), Unassigned::CarryOn);
    }

    #[test]
    fn so_does_one_whose_uplink_stayed_open() {
        let running = known(None, SessionStatus::Running);
        assert_eq!(unassigned("r1", running, true), Unassigned::CarryOn);
        let here = known(Some("r1"), SessionStatus::Stopped);
        assert_eq!(unassigned("r1", here, true), Unassigned::CarryOn);
    }

    #[test]
    fn one_that_moved_to_another_runner_stops() {
        let elsewhere = known(Some("r2"), SessionStatus::Running);
        assert_eq!(
            unassigned("r1", elsewhere, true),
            Unassigned::Stop(StopReason::Shutdown)
        );
        let ended_there = known(Some("r2"), SessionStatus::Stopped);
        assert_eq!(
            unassigned("r1", ended_there, false),
            Unassigned::Stop(StopReason::Shutdown)
        );
    }

    #[test]
    fn one_that_someone_stopped_stays_stopped() {
        let stopped = known(None, SessionStatus::Stopped);
        assert_eq!(
            unassigned("r1", stopped, false),
            Unassigned::Stop(StopReason::User)
        );
        let resumed = known(None, SessionStatus::Queued);
        assert_eq!(
            unassigned("r1", resumed, false),
            Unassigned::Stop(StopReason::User)
        );
    }

    #[test]
    fn one_whose_resume_failed_stops() {
        let failed = known(None, SessionStatus::Failed);
        assert_eq!(
            unassigned("r1", failed, true),
            Unassigned::Stop(StopReason::User)
        );
    }

    #[test]
    fn one_put_away_stays_stopped() {
        let archived = Some(Known {
            runner_id: None,
            status: SessionStatus::Stopped,
            archived: true,
        });
        assert_eq!(
            unassigned("r1", archived, true),
            Unassigned::Stop(StopReason::User)
        );
    }

    #[test]
    fn a_deleted_session_stops() {
        assert_eq!(
            unassigned("r1", None, true),
            Unassigned::Stop(StopReason::Deleted)
        );
    }

    #[test]
    fn a_session_still_on_this_runner_is_kept() {
        let running = known(Some("r1"), SessionStatus::Running);
        assert_eq!(unassigned("r1", running, false), Unassigned::Keep);
        let ended = known(Some("r1"), SessionStatus::Stopped);
        assert_eq!(
            unassigned("r1", ended, false),
            Unassigned::Stop(StopReason::User)
        );
    }
}
