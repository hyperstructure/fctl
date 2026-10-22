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

/// Whether an agent's Remote Control link can be kept: an `https` page of
/// claude.ai (or claude.com). The apps open it as the session's link in
/// the Claude apps (`fctl open --claude` with `open` or `xdg-open`), so a
/// link to another site, or a path, is dropped.
pub fn is_session_url(url: &str) -> bool {
    is_page_of(url, &["claude.ai", "claude.com"])
}

/// Whether a Claude sign-in's link (`claude auth login`'s authorization
/// page) can be shown and opened: an `https` OAuth page of claude.ai,
/// claude.com or anthropic.com. The sign-in runs on a runner of the
/// person's workspaces, which may be another member's, and the link comes
/// from its host; one to another site could ask for their Claude account
/// on a look-alike page.
pub fn is_login_url(url: &str) -> bool {
    is_page_of(url, &["claude.ai", "claude.com", "anthropic.com"])
        && url
            .split(['?', '#'])
            .next()
            .is_some_and(|page| page.contains("/oauth/authorize"))
}

/// Whether `url` is an `https` page of one of `domains` or their
/// subdomains, with nothing in it that a browser or `open` could read as
/// another site, or as more arguments.
fn is_page_of(url: &str, domains: &[&str]) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let theirs = domains
        .iter()
        .any(|d| host == *d || host.strip_suffix(d).is_some_and(|sub| sub.ends_with('.')));
    theirs
        && url.len() <= 2048
        && !url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\' || c == '@')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_claude_links() {
        for ok in [
            "https://claude.ai/code/session_01AbC-d_9",
            "https://claude.ai/code/cse_01AbC",
            "https://claude.ai/code/session_01AbC?org=1",
            "https://code.claude.com/session_01AbC",
        ] {
            assert!(is_session_url(ok), "{ok}");
        }
        for bad in [
            "http://claude.ai/code/session_01AbC",
            "https://claude.ai.evil.example/code/session_01AbC",
            "https://evilclaude.ai/code/session_01AbC",
            "https://claude.ai@evil.example/code",
            "https://evil.example\\@claude.ai/code",
            "https://claude.ai:443@evil.example/",
            "/Applications/Calculator.app",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://claude.ai/code/session_1 --flag",
        ] {
            assert!(!is_session_url(bad), "{bad}");
        }
    }

    #[test]
    fn keeps_only_claudes_sign_in_pages() {
        for ok in [
            "https://claude.ai/oauth/authorize?code=true&client_id=9d1c&redirect_uri=https%3A%2F%2Fconsole.anthropic.com%2Foauth%2Fcode%2Fcallback&state=x",
            "https://claude.com/cai/oauth/authorize?code=true&state=x",
            "https://platform.claude.com/oauth/authorize?state=x",
            "https://console.anthropic.com/oauth/authorize?state=x",
        ] {
            assert!(is_login_url(ok), "{ok}");
        }
        for bad in [
            "https://claude-ai.example/oauth/authorize?state=x",
            "https://claude.ai.evil.example/oauth/authorize?state=x",
            "https://evil.example/oauth/authorize?next=https://claude.ai",
            "https://claude.ai@evil.example/oauth/authorize?state=x",
            "http://claude.ai/oauth/authorize?state=x",
            "https://claude.ai/code/session_01AbC",
            "https://claude.ai/login?next=/oauth/authorize",
            "javascript:alert(1)//oauth/authorize",
            "file:///oauth/authorize",
        ] {
            assert!(!is_login_url(bad), "{bad}");
        }
    }

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
