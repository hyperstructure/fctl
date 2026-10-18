//! The agent's events as SessionDO takes them from a host: in `agent_seq`
//! order, each once.
//!
//! fctl-agent numbers its events one after another and keeps them until
//! SessionDO acknowledges them. After a reconnect, newer events can arrive
//! before older ones: the older ones went over a link to the agent, or an
//! uplink, that closed, and come again only with the replay the host asks
//! for after every reconnect. Storing the newer ones first would acknowledge
//! the older ones unseen, and the agent's journal would drop them. So
//! SessionDO waits for the replay, which takes a moment, but for at most
//! [`WAIT_FOR_EARLIER_MS`]: a journal that has lost events must not stop
//! the transcript for good.

/// How long SessionDO waits for events that newer ones arrived before.
pub const WAIT_FOR_EARLIER_MS: i64 = 60_000;

/// What to do with an event from a host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take {
    /// It is stored already.
    Seen,
    /// Store it.
    Store,
    /// Earlier events haven't arrived: wait for them. This one comes again
    /// with them.
    Wait,
}

/// What to do with event `agent_seq` when the last one stored is `acked`,
/// waiting for earlier events since `waiting_since`.
pub fn take(acked: u64, agent_seq: u64, waiting_since: Option<i64>, now: i64) -> Take {
    if agent_seq <= acked {
        Take::Seen
    } else if agent_seq == acked + 1
        || waiting_since.is_some_and(|t| now - t >= WAIT_FOR_EARLIER_MS)
    {
        Take::Store
    } else {
        Take::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_stored_in_order_and_once() {
        assert_eq!(take(7, 8, None, 0), Take::Store);
        assert_eq!(take(7, 7, None, 0), Take::Seen);
        assert_eq!(take(7, 3, None, 0), Take::Seen);
        // A session's first event, and a resumed session's first in a new
        // sandbox (it numbers after what SessionDO has).
        assert_eq!(take(0, 1, None, 0), Take::Store);
        assert_eq!(take(120, 121, None, 0), Take::Store);
    }

    #[test]
    fn newer_events_wait_for_older_ones_that_a_replay_brings() {
        // 8 went over a link that closed; 9 came live over the new one,
        // before the replay of 8, 9 and 10.
        let now = 1_000_000;
        assert_eq!(take(7, 9, None, now), Take::Wait);
        assert_eq!(take(7, 9, Some(now), now + 500), Take::Wait);
        assert_eq!(take(7, 8, Some(now), now + 600), Take::Store);
    }

    #[test]
    fn a_minute_later_the_transcript_goes_on_without_them() {
        let since = 1_000_000;
        assert_eq!(take(7, 9, Some(since), since + 59_999), Take::Wait);
        assert_eq!(
            take(7, 9, Some(since), since + WAIT_FOR_EARLIER_MS),
            Take::Store
        );
    }
}
