//! What a project's settings mean for its sessions.

use fctl_proto::session::ProjectSettings;
use serde_json::Value;
use std::collections::BTreeMap;

/// Claude Code leaves out claude.ai connectors when this is `false` (or `0`,
/// `no`, `off`).
pub const CLAUDE_AI_CONNECTORS_ENV: &str = "ENABLE_CLAUDEAI_MCP_SERVERS";

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
/// without having shown it: for `claude_ai_connectors` and `commit_credit`,
/// on. `sent` is the update's `settings` object as it arrived.
pub fn keep_unsent(settings: &mut ProjectSettings, stored: &ProjectSettings, sent: &Value) {
    if sent.get("claude_ai_connectors").is_none() {
        settings.claude_ai_connectors = stored.claude_ai_connectors;
    }
    if sent.get("commit_credit").is_none() {
        settings.commit_credit = stored.commit_credit;
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
    fn updates_without_commit_credit_keep_it_off() {
        let stored = ProjectSettings {
            commit_credit: false,
            ..Default::default()
        };
        // An older client sends settings without the field: its default, on.
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "auto_pull_request": true }));
        assert!(!s.commit_credit);

        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "commit_credit": true }));
        assert!(s.commit_credit);
    }
}
