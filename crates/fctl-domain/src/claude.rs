//! Claude subscription tokens: what a token from `claude setup-token` looks
//! like, wherever it is kept (in the control plane, or on its owner's
//! runners).

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
}
