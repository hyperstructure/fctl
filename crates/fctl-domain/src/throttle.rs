//! How often a session may ask for its credentials. Each answer mints new
//! GitHub tokens or reads the Claude credential, in the one object that
//! schedules every runner's sessions, and what runs in a sandbox can pose
//! as its agent and ask without end. An agent asks for GitHub tokens at
//! most every five minutes, and Claude asks for a new access token as its
//! old one runs out: far below [`CREDENTIALS_PER_WINDOW`].

use std::collections::VecDeque;

/// How far back a session's requests count.
pub const CREDENTIAL_WINDOW_MS: i64 = 10 * 60 * 1000;
/// The most credential requests a session makes in [`CREDENTIAL_WINDOW_MS`].
pub const CREDENTIALS_PER_WINDOW: usize = 30;

/// Whether a request at `now` is within `most` in `window` ms, given when
/// the earlier ones came (`times`, oldest first), which it updates: those
/// too old go, and this one counts when it is allowed.
pub fn allow(times: &mut VecDeque<i64>, now: i64, window: i64, most: usize) -> bool {
    while times.front().is_some_and(|t| now - *t >= window) {
        times.pop_front();
    }
    if times.len() >= most {
        return false;
    }
    times.push_back(now);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_waits_for_the_window_to_move_on() {
        let mut times = VecDeque::new();
        for i in 0..3 {
            assert!(allow(&mut times, 1_000 + i, 60_000, 3));
        }
        // A fourth within the minute is refused, and doesn't count.
        assert!(!allow(&mut times, 2_000, 60_000, 3));
        assert_eq!(times.len(), 3);
        // Once the first is a minute old, there is room again.
        assert!(allow(&mut times, 61_000, 60_000, 3));
        assert_eq!(times.len(), 3);
    }

    #[test]
    fn an_agent_refreshing_as_it_does_is_never_refused() {
        // GitHub tokens every five minutes for a day, and a Claude token
        // now and then.
        let mut times = VecDeque::new();
        for i in 0..288 {
            let now = i * 5 * 60 * 1000;
            assert!(allow(
                &mut times,
                now,
                CREDENTIAL_WINDOW_MS,
                CREDENTIALS_PER_WINDOW
            ));
            if i % 12 == 0 {
                assert!(allow(
                    &mut times,
                    now + 1,
                    CREDENTIAL_WINDOW_MS,
                    CREDENTIALS_PER_WINDOW
                ));
            }
        }
    }
}
