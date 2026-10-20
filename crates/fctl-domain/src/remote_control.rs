//! Remote Control: following a session in the Claude apps.

use fctl_proto::session::{RemoteControlState, UserRef};

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

/// Gives `rc`, as an agent reported it, the person it belongs to: `user`,
/// on whose Claude subscription the session runs now. A link the session
/// already had (`known`) keeps its person, so that a run's late report
/// doesn't hand it to someone who resumed the session since.
pub fn attribute(rc: &mut RemoteControlState, known: Option<&RemoteControlState>, user: &UserRef) {
    let had = known
        .filter(|k| k.session_url.is_some() && k.session_url == rc.session_url)
        .and_then(|k| k.user.clone());
    rc.user = Some(had.unwrap_or_else(|| user.clone()));
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

    fn person(id: &str) -> UserRef {
        UserRef {
            id: fctl_proto::UserId(id.into()),
            name: id.into(),
            avatar_url: None,
        }
    }

    fn link(url: &str, user: Option<&str>) -> RemoteControlState {
        RemoteControlState {
            enabled: true,
            session_url: Some(url.into()),
            unavailable_reason: None,
            user: user.map(person),
        }
    }

    #[test]
    fn a_new_link_belongs_to_whom_the_session_runs_on() {
        let mut rc = link("https://claude.ai/code/session_B", None);
        attribute(
            &mut rc,
            Some(&link("https://claude.ai/code/session_A", Some("usr_ada"))),
            &person("usr_bo"),
        );
        assert_eq!(rc.user, Some(person("usr_bo")));

        let mut first = link("https://claude.ai/code/session_A", None);
        attribute(&mut first, None, &person("usr_ada"));
        assert_eq!(first.user, Some(person("usr_ada")));
    }

    #[test]
    fn a_link_keeps_its_person_when_someone_else_resumed_since() {
        let known = link("https://claude.ai/code/session_A", Some("usr_ada"));
        let mut late = link("https://claude.ai/code/session_A", None);
        attribute(&mut late, Some(&known), &person("usr_bo"));
        assert_eq!(late.user, Some(person("usr_ada")));

        // Stored before links had a person: the session's runs decide.
        let mut old = link("https://claude.ai/code/session_A", None);
        attribute(
            &mut old,
            Some(&link("https://claude.ai/code/session_A", None)),
            &person("usr_bo"),
        );
        assert_eq!(old.user, Some(person("usr_bo")));
    }

    #[test]
    fn remote_control_that_is_off_belongs_to_whom_the_session_runs_on() {
        // Its reason (a login to connect) is theirs to act on.
        let mut off = RemoteControlState {
            enabled: false,
            session_url: None,
            unavailable_reason: Some("Remote Control needs a full Claude login.".into()),
            user: None,
        };
        let mut known = off.clone();
        known.user = Some(person("usr_ada"));
        attribute(&mut off, Some(&known), &person("usr_bo"));
        assert_eq!(off.user, Some(person("usr_bo")));
    }
}
