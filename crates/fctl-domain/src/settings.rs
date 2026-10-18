//! What a project's settings mean for its sessions.

use fctl_proto::session::ProjectSettings;
use serde_json::Value;
use std::collections::BTreeMap;

/// Claude Code leaves out claude.ai connectors when this is `false` (or `0`,
/// `no`, `off`).
pub const CLAUDE_AI_CONNECTORS_ENV: &str = "ENABLE_CLAUDEAI_MCP_SERVERS";

/// How long a session that finished its turn keeps its sandbox, and its
/// slot on the host, when its project sets no idle timeout.
pub const DEFAULT_IDLE_TIMEOUT_MINUTES: u32 = 10;

/// Seconds an idle session waits for a message before it stops, so that its
/// host can take other work. A message resumes it.
pub fn idle_timeout_secs(settings: &ProjectSettings) -> u64 {
    u64::from(
        settings
            .idle_timeout_minutes
            .unwrap_or(DEFAULT_IDLE_TIMEOUT_MINUTES),
    ) * 60
}

/// The environment a session gets from its project's settings: the
/// project's variables, and `ENABLE_CLAUDEAI_MCP_SERVERS=false` when
/// claude.ai connectors are off, whatever the variables say. Every
/// fctl-agent passes these variables to Claude, including those from before
/// `claude_ai_connectors` existed.
pub fn session_env(settings: &ProjectSettings) -> BTreeMap<String, String> {
    let mut env = settings.env.clone();
    if !settings.claude_ai_connectors {
        env.insert(CLAUDE_AI_CONNECTORS_ENV.into(), "false".into());
    }
    env
}

/// Keeps the stored value of settings that an update leaves out. An update
/// replaces all settings, so a client from before a setting existed (a tab
/// opened before a deploy, an older CLI) would reset it to its default
/// without having shown it: for `claude_ai_connectors`, `credit_people` and
/// `credit_claude`, on; for `publish_screenshots`, off; for
/// `screenshot_days`, 30 days (send `null` to choose the default). `sent`
/// is the update's `settings` object as it arrived.
pub fn keep_unsent(settings: &mut ProjectSettings, stored: &ProjectSettings, sent: &Value) {
    if sent.get("claude_ai_connectors").is_none() {
        settings.claude_ai_connectors = stored.claude_ai_connectors;
    }
    if sent.get("credit_people").is_none() {
        settings.credit_people = stored.credit_people;
    }
    if sent.get("credit_claude").is_none() {
        settings.credit_claude = stored.credit_claude;
    }
    if sent.get("publish_screenshots").is_none() {
        settings.publish_screenshots = stored.publish_screenshots;
    }
    if sent.get("screenshot_days").is_none() {
        settings.screenshot_days = stored.screenshot_days;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(connectors: bool, env: &[(&str, &str)]) -> ProjectSettings {
        ProjectSettings {
            claude_ai_connectors: connectors,
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn connectors_on_leave_the_environment_alone() {
        let s = settings(true, &[("NODE_ENV", "test")]);
        assert_eq!(session_env(&s), s.env);
        // A project may still turn them off with the variable itself.
        let s = settings(true, &[(CLAUDE_AI_CONNECTORS_ENV, "0")]);
        assert_eq!(session_env(&s), s.env);
    }

    #[test]
    fn connectors_off_set_the_variable() {
        let env = session_env(&settings(false, &[("NODE_ENV", "test")]));
        assert_eq!(env.len(), 2);
        assert_eq!(env["NODE_ENV"], "test");
        assert_eq!(env[CLAUDE_AI_CONNECTORS_ENV], "false");
        // The setting wins over a project variable that turns them on.
        let env = session_env(&settings(false, &[(CLAUDE_AI_CONNECTORS_ENV, "true")]));
        assert_eq!(env[CLAUDE_AI_CONNECTORS_ENV], "false");
    }

    #[test]
    fn updates_without_the_field_keep_the_stored_choice() {
        let stored = settings(false, &[]);
        let mut s = settings(true, &[("A", "1")]);
        keep_unsent(&mut s, &stored, &json!({ "env": { "A": "1" } }));
        assert!(!s.claude_ai_connectors);
        assert_eq!(s.env["A"], "1");

        let mut s = settings(true, &[]);
        keep_unsent(&mut s, &stored, &json!({ "claude_ai_connectors": true }));
        assert!(s.claude_ai_connectors);

        let mut s = settings(false, &[]);
        keep_unsent(
            &mut s,
            &settings(true, &[]),
            &json!({ "claude_ai_connectors": false }),
        );
        assert!(!s.claude_ai_connectors);
    }

    #[test]
    fn updates_without_the_credit_settings_keep_them_off() {
        let stored = ProjectSettings {
            credit_people: false,
            credit_claude: false,
            ..Default::default()
        };
        // An older client sends settings without the fields: their defaults, on.
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "auto_pull_request": true }));
        assert!(!s.credit_people && !s.credit_claude);

        // Each one sent counts on its own.
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "credit_claude": true }));
        assert!(!s.credit_people && s.credit_claude);
    }

    #[test]
    fn updates_without_the_screenshot_settings_keep_them() {
        let stored = ProjectSettings {
            publish_screenshots: true,
            screenshot_days: Some(2),
            ..Default::default()
        };
        // An older client would turn publishing off and keep links for 30 days.
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "credit_claude": true }));
        assert!(s.publish_screenshots);
        assert_eq!(s.screenshot_days, Some(2));

        // Sent, they count, and `null` chooses the default.
        let mut s = ProjectSettings::default();
        keep_unsent(
            &mut s,
            &stored,
            &json!({ "publish_screenshots": false, "screenshot_days": null }),
        );
        assert!(!s.publish_screenshots);
        assert_eq!(s.screenshot_days, None);
    }

    #[test]
    fn idle_sessions_stop_after_ten_minutes_unless_the_project_says_otherwise() {
        assert_eq!(idle_timeout_secs(&ProjectSettings::default()), 600);
        let s = ProjectSettings {
            idle_timeout_minutes: Some(120),
            ..Default::default()
        };
        assert_eq!(idle_timeout_secs(&s), 7200);
    }
}
