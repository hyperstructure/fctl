//! Claude subscription tokens: what a token from `claude setup-token` looks
//! like, wherever it is kept (in the control plane, or on its owner's
//! runners), and moving sessions from one connection to another.

use fctl_proto::session::SessionStatus;

/// What is wrong with `token` as a token from `claude setup-token`, or
/// `None` when it looks like one. `token` is trimmed already.
///
/// Long-lived tokens look like `sk-ant-oat01-…`; an API key, which bills
/// per token, looks like `sk-ant-api…`.
pub fn setup_token_problem(token: &str) -> Option<&'static str> {
    if token.starts_with("sk-ant-api") {
        return Some(
            "That is an API key, which bills per token. fctl only runs on subscriptions: run `claude setup-token` and paste its output.",
        );
    }
    let malformed = !token.starts_with("sk-ant-oat")
        || token.len() < 40
        || token.len() > 500
        || token.contains(char::is_whitespace)
        || token.chars().any(char::is_control);
    malformed.then_some("Paste the token `claude setup-token` printed (it starts with sk-ant-oat)")
}

/// `sk-ant-oat01-…Xy3Q`: enough of a token to tell it from another, in the
/// terminal of the machine that keeps it.
pub fn masked(token: &str) -> String {
    let tail: String = token
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let head = token
        .match_indices('-')
        .nth(2)
        .map_or("", |(i, _)| &token[..=i]);
    format!("{head}…{tail}")
}

/// What moving a session to another Claude connection does to its run
/// ([`moving`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// Its run waits (on its subscription's limit, or for a message): it
    /// stops, and the session starts again on the new connection, where
    /// Claude continues the same conversation.
    Restart,
    /// No run works for it, or the last one stops: it starts again on the
    /// new connection, if whoever moves it wants it to go on.
    Start,
    /// It waits to start: it starts on the new connection.
    Queued,
}

/// How a session in `status` moves to another Claude connection, or why
/// it doesn't now.
pub fn moving(status: SessionStatus) -> Result<Move, &'static str> {
    match status {
        SessionStatus::Paused | SessionStatus::Idle => Ok(Move::Restart),
        SessionStatus::Stopping | SessionStatus::Stopped => Ok(Move::Start),
        SessionStatus::Queued => Ok(Move::Queued),
        SessionStatus::Provisioning | SessionStatus::Preparing | SessionStatus::Running => Err(
            "Its agent is at work on the subscription it runs on: take it over once it pauses on a usage limit, waits for a message or stops.",
        ),
        SessionStatus::Failed => {
            Err("The session failed: resuming it runs it on your Claude subscription.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJ-Xy3Q";

    #[test]
    fn takes_only_tokens_from_claude_setup_token() {
        assert_eq!(setup_token_problem(TOKEN), None);
        assert_eq!(
            setup_token_problem("sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789"),
            Some(
                "That is an API key, which bills per token. fctl only runs on subscriptions: run `claude setup-token` and paste its output."
            )
        );
        for bad in [
            "",
            "sk-ant-oat01-short",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789ABCDEF",
            "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz 0123456789ABCDEFGHIJ",
            "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz\u{7}0123456789ABCDEFGHIJ",
        ] {
            assert_eq!(
                setup_token_problem(bad),
                Some("Paste the token `claude setup-token` printed (it starts with sk-ant-oat)"),
                "{bad}"
            );
        }
        assert!(setup_token_problem(&format!("sk-ant-oat01-{}", "a".repeat(500))).is_some());
    }

    #[test]
    fn masks_all_but_the_kind_and_the_end() {
        assert_eq!(masked(TOKEN), "sk-ant-oat01-…Xy3Q");
        // Something else entirely shows its end alone.
        assert_eq!(masked("abcdef"), "…cdef");
        assert_eq!(masked("ab"), "…ab");
    }
    #[test]
    fn sessions_move_while_they_wait_or_rest() {
        assert_eq!(moving(SessionStatus::Paused), Ok(Move::Restart));
        assert_eq!(moving(SessionStatus::Idle), Ok(Move::Restart));
        assert_eq!(moving(SessionStatus::Stopping), Ok(Move::Start));
        assert_eq!(moving(SessionStatus::Stopped), Ok(Move::Start));
        assert_eq!(moving(SessionStatus::Queued), Ok(Move::Queued));
        for at_work in [
            SessionStatus::Provisioning,
            SessionStatus::Preparing,
            SessionStatus::Running,
        ] {
            assert!(moving(at_work).is_err_and(|why| why.contains("at work")));
        }
        assert!(moving(SessionStatus::Failed).is_err_and(|why| why.contains("resuming")));
    }
}
