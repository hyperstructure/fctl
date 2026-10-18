//! Remote Control: following a session in the Claude apps.

/// The `claude://` link that opens a Remote Control session in Claude's
/// desktop or mobile app, from the session's claude.ai/code URL. `None` for
/// any other URL.
pub fn claude_app_url(session_url: &str) -> Option<String> {
    let id = session_url.strip_prefix("https://claude.ai/code/")?;
    let known = ["session_", "cse_"]
        .iter()
        .any(|p| id.strip_prefix(p).is_some_and(|rest| !rest.is_empty()));
    let valid = id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    (known && valid).then(|| format!("claude://code/{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_claude_ai_code_sessions_to_the_claude_apps() {
        assert_eq!(
            claude_app_url("https://claude.ai/code/session_01AbC-d_9").as_deref(),
            Some("claude://code/session_01AbC-d_9")
        );
        assert_eq!(
            claude_app_url("https://claude.ai/code/cse_01AbC").as_deref(),
            Some("claude://code/cse_01AbC")
        );
    }

    #[test]
    fn leaves_other_urls_to_the_browser() {
        for url in [
            "https://claude.ai/code",
            "https://claude.ai/code/session_",
            "https://claude.ai/code/new?q=hello",
            "https://claude.ai/code/session_01AbC?org=1",
            "https://claude.ai/code/session_01AbC/extra",
            "https://claude.ai/code/local_01AbC",
            "https://claude.ai/chat/session_01AbC",
            "https://claude-ai.staging.ant.dev/code/session_01AbC",
            "http://claude.ai/code/session_01AbC",
            "https://claude.ai.example.com/code/session_01AbC",
            "claude://code/session_01AbC",
        ] {
            assert_eq!(claude_app_url(url), None, "{url}");
        }
    }
}
