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

/// A new project starts with claude.ai connectors off, so that people opt
/// in: a create request that leaves `claude_ai_connectors` out (or sends no
/// settings at all) gets `false`, though a `ProjectSettings` read without
/// the field is `true` (projects from before it keep their connectors).
/// Its follow-ups start on, each one the request leaves out, though a
/// `FollowUps` read without one is off (projects from before it, which
/// `crate::pull_requests::moved_onto`'s SQL reads the same way); the
/// project then stores each of them. `sent` is the request's `settings` as
/// it arrived.
pub fn for_new_project(settings: &mut ProjectSettings, sent: &Value) {
    if sent.get("claude_ai_connectors").is_none() {
        settings.claude_ai_connectors = false;
    }
    let follow_ups = sent.get("follow_ups");
    let unsent = |key: &str| follow_ups.and_then(|f| f.get(key)).is_none();
    if unsent("comments") {
        settings.follow_ups.comments = true;
    }
    if unsent("checks") {
        settings.follow_ups.checks = true;
    }
    if unsent("closed") {
        settings.follow_ups.closed = true;
    }
    if unsent("conflicts") {
        settings.follow_ups.conflicts = true;
    }
}

/// Keeps the stored value of settings that an update leaves out. An update
/// replaces all settings, so a client from before a setting existed (a tab
/// opened before a deploy, an older CLI) would reset it to its default
/// without having shown it: for `claude_ai_connectors`, `credit_people`,
/// `credit_claude`, `project_memory` and `retrospectives`, on; for
/// `publish_screenshots` and `package_caches`, off; for `screenshot_days`,
/// 30 days (send `null` to choose the default); for `github_access`, push,
/// which would let agents push again; for `pull_request_descriptions`,
/// balanced; for each of `follow_ups`, off. An
/// `image_build` it leaves out stays unless it chooses an image instead
/// (send `null` for none). `sent` is the update's `settings` object as it
/// arrived.
pub fn keep_unsent(settings: &mut ProjectSettings, stored: &ProjectSettings, sent: &Value) {
    let follow_ups = sent.get("follow_ups");
    let unsent = |key: &str| follow_ups.and_then(|f| f.get(key)).is_none();
    if unsent("comments") {
        settings.follow_ups.comments = stored.follow_ups.comments;
    }
    if unsent("checks") {
        settings.follow_ups.checks = stored.follow_ups.checks;
    }
    if unsent("closed") {
        settings.follow_ups.closed = stored.follow_ups.closed;
    }
    if unsent("conflicts") {
        settings.follow_ups.conflicts = stored.follow_ups.conflicts;
    }
    if sent.get("image_build").is_none() && settings.image.is_none() {
        settings.image_build = stored.image_build.clone();
    }
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
    if sent.get("package_caches").is_none() {
        settings.package_caches = stored.package_caches;
    }
    if sent.get("screenshot_days").is_none() {
        settings.screenshot_days = stored.screenshot_days;
    }
    if sent.get("project_memory").is_none() {
        settings.project_memory = stored.project_memory;
    }
    if sent.get("retrospectives").is_none() {
        settings.retrospectives = stored.retrospectives;
    }
    if sent.get("github_access").is_none() {
        settings.github_access = stored.github_access;
    }
    if sent.get("pull_request_descriptions").is_none() {
        settings.pull_request_descriptions = stored.pull_request_descriptions;
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
    fn new_projects_start_without_connectors_unless_asked() {
        let mut s = ProjectSettings::default();
        for_new_project(&mut s, &Value::Null);
        assert!(!s.claude_ai_connectors);
        let mut s = ProjectSettings::default();
        for_new_project(&mut s, &json!({ "setup_script": "npm ci" }));
        assert!(!s.claude_ai_connectors);
        let mut s = ProjectSettings::default();
        for_new_project(&mut s, &json!({ "claude_ai_connectors": true }));
        assert!(s.claude_ai_connectors);
    }

    #[test]
    fn new_projects_start_with_follow_ups_on_unless_asked() {
        use fctl_proto::session::FollowUps;
        let mut s = ProjectSettings::default();
        for_new_project(&mut s, &Value::Null);
        assert_eq!(s.follow_ups, FollowUps::ALL);
        let mut s = ProjectSettings::default();
        for_new_project(&mut s, &json!({ "setup_script": "npm ci" }));
        assert_eq!(s.follow_ups, FollowUps::ALL);
        // What a request sends stays; the rest are on.
        let sent = json!({ "follow_ups": { "checks": false, "closed": false } });
        let mut s: ProjectSettings = serde_json::from_value(sent.clone()).unwrap_or_default();
        for_new_project(&mut s, &sent);
        assert_eq!(
            s.follow_ups,
            FollowUps {
                comments: true,
                checks: false,
                closed: false,
                conflicts: true,
            }
        );
        let sent = serde_json::to_value(ProjectSettings::default()).unwrap_or_default();
        let mut s = ProjectSettings::default();
        for_new_project(&mut s, &sent);
        assert_eq!(s.follow_ups, FollowUps::default());
        // Stored, each one is there, so a project reads them back as on.
        let stored = serde_json::to_value(&ProjectSettings {
            follow_ups: FollowUps::ALL,
            ..Default::default()
        })
        .unwrap_or_default();
        assert_eq!(stored["follow_ups"]["conflicts"], json!(true));
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
    fn updates_without_github_access_keep_agents_held_to_it() {
        use fctl_proto::session::GithubAccess;
        let stored = ProjectSettings {
            github_access: Some(GithubAccess::Comment),
            ..Default::default()
        };
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "auto_pull_request": true }));
        assert_eq!(s.github_access, Some(GithubAccess::Comment));
        let mut s = ProjectSettings {
            github_access: Some(GithubAccess::Push),
            ..Default::default()
        };
        keep_unsent(&mut s, &stored, &json!({ "github_access": "push" }));
        assert_eq!(s.github_access, Some(GithubAccess::Push));
    }

    #[test]
    fn updates_keep_the_follow_ups_they_leave_out() {
        use fctl_proto::session::FollowUps;
        let stored = ProjectSettings {
            follow_ups: FollowUps {
                comments: true,
                checks: true,
                closed: false,
                conflicts: true,
            },
            ..Default::default()
        };
        // An older client knows none of them.
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "auto_pull_request": true }));
        assert_eq!(s.follow_ups, stored.follow_ups);
        // Each one sent counts on its own; one a newer version adds stays,
        // as conflicts do for a tab from before them.
        let sent = json!({ "follow_ups": { "checks": false, "closed": true } });
        let mut s: ProjectSettings = serde_json::from_value(sent.clone()).unwrap_or_default();
        keep_unsent(&mut s, &stored, &sent);
        assert_eq!(
            s.follow_ups,
            FollowUps {
                comments: true,
                checks: false,
                closed: true,
                conflicts: true,
            }
        );
        let sent = json!({ "follow_ups": { "comments": true, "checks": true, "closed": false, "conflicts": false } });
        let mut s: ProjectSettings = serde_json::from_value(sent.clone()).unwrap_or_default();
        keep_unsent(&mut s, &stored, &sent);
        assert!(!s.follow_ups.conflicts);
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
    fn updates_without_the_build_keep_it_unless_they_choose_an_image() {
        let stored = ProjectSettings {
            image_build: Some(fctl_proto::session::ImageBuild {
                file: "Dockerfile".into(),
                context: None,
            }),
            ..Default::default()
        };
        // An older client, which doesn't know builds.
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "credit_claude": true }));
        assert_eq!(s.image_build, stored.image_build);
        // One that chooses an image replaces the build with it.
        let mut s = ProjectSettings {
            image: Some("ghcr.io/acme/agent:1".into()),
            ..Default::default()
        };
        keep_unsent(&mut s, &stored, &json!({ "image": "ghcr.io/acme/agent:1" }));
        assert_eq!(s.image_build, None);
        // `null` stops building.
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "image_build": null }));
        assert_eq!(s.image_build, None);
        // Settings as this version sends them always say.
        let sent = serde_json::to_value(ProjectSettings::default()).unwrap_or_default();
        assert_eq!(sent.get("image_build"), Some(&Value::Null));
    }

    #[test]
    fn updates_without_memory_and_retrospectives_keep_them_off() {
        let stored = ProjectSettings {
            project_memory: false,
            retrospectives: false,
            ..Default::default()
        };
        // An older client sends settings without the fields: their defaults, on.
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "auto_pull_request": true }));
        assert!(!s.project_memory && !s.retrospectives);
        // Each one sent counts on its own.
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "project_memory": true }));
        assert!(s.project_memory && !s.retrospectives);
    }

    #[test]
    fn updates_without_package_caches_keep_them_on() {
        let stored = ProjectSettings {
            package_caches: true,
            ..Default::default()
        };
        // An older client sends settings without the field: its default, off.
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "auto_pull_request": true }));
        assert!(s.package_caches);
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "package_caches": false }));
        assert!(!s.package_caches);
    }

    #[test]
    fn updates_without_pull_request_descriptions_keep_the_choice() {
        use fctl_proto::session::Verbosity;
        let stored = ProjectSettings {
            pull_request_descriptions: Verbosity::Brief,
            ..Default::default()
        };
        // An older client sends settings without the field: its default, balanced.
        let mut s = ProjectSettings::default();
        keep_unsent(&mut s, &stored, &json!({ "auto_pull_request": true }));
        assert_eq!(s.pull_request_descriptions, Verbosity::Brief);
        let sent = json!({ "pull_request_descriptions": "balanced" });
        let mut s: ProjectSettings = serde_json::from_value(sent.clone()).unwrap_or_default();
        keep_unsent(&mut s, &stored, &sent);
        assert_eq!(s.pull_request_descriptions, Verbosity::Balanced);
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
