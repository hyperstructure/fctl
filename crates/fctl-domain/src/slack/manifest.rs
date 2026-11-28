//! The manifest that creates an instance's Slack app, with its addresses
//! filled in (docs/self-hosting.md#connect-slack-optional): Slack's form
//! takes it as JSON or YAML, and a link opens the form with it.

use super::BOT_SCOPES;
use serde_json::{Value, json};

/// The addresses Slack calls, under the instance's.
pub struct Urls {
    pub events: String,
    pub interactivity: String,
    pub commands: String,
    pub installed: String,
    pub linked: String,
}

impl Urls {
    pub fn of(public_url: &str) -> Self {
        let base = public_url.trim_end_matches('/');
        Self {
            events: format!("{base}/api/v1/slack/events"),
            interactivity: format!("{base}/api/v1/slack/interactivity"),
            commands: format!("{base}/api/v1/slack/commands"),
            installed: format!("{base}/api/v1/slack/installed"),
            linked: format!("{base}/api/v1/slack/linked"),
        }
    }
}

/// The events fctl's app hears.
pub const BOT_EVENTS: [&str; 8] = [
    "agent_session_stopped",
    "app_home_opened",
    "app_mention",
    "app_uninstalled",
    "message.channels",
    "message.groups",
    "message.im",
    "tokens_revoked",
];

/// The app's name in Slack, from the instance's (Slack takes at most 35
/// characters).
fn app_name(instance: &str) -> String {
    let name: String = instance.trim().chars().take(35).collect();
    if name.is_empty() { "fctl".into() } else { name }
}

/// The manifest for an instance named `instance` at `public_url`. With
/// `socket_mode`, Slack sends its events over a WebSocket the instance
/// opens, for an instance Slack can't reach; otherwise to its addresses.
pub fn manifest(instance: &str, public_url: &str, socket_mode: bool) -> Value {
    let u = Urls::of(public_url);
    let name = app_name(instance);
    let mut command = json!({
        "command": "/fctl",
        "description": "Start an fctl session, or see yours",
        "usage_hint": "[what to do] | status",
        "should_escape": true,
    });
    let mut events = json!({ "bot_events": BOT_EVENTS });
    let mut interactivity = json!({ "is_enabled": true });
    if !socket_mode {
        command["url"] = json!(u.commands);
        events["request_url"] = json!(u.events);
        interactivity["request_url"] = json!(u.interactivity);
    }
    json!({
        "_metadata": { "major_version": 2, "minor_version": 1 },
        "display_information": {
            "name": name,
            "description": "Start and steer fctl's agents from Slack.",
            "long_description": "Mention fctl in a channel, or write to it in its messages, and an agent starts on the work in a sandbox, as you, on your Claude subscription. Its card in the thread shows how it goes: what it's doing, its plan, its pull requests with their checks. Your replies in the thread steer it, its questions come with a button for each answer, and its replies come back where you asked. Agents read the thread they started from, and only the public channels your workspace's admins make readable.",
            "background_color": "#171717",
        },
        "features": {
            "app_home": {
                "home_tab_enabled": true,
                "messages_tab_enabled": true,
                "messages_tab_read_only_enabled": false,
                "agent_tasks_enabled": false,
            },
            "bot_user": { "display_name": name, "always_online": true },
            "agent_view": {
                "agent_description": "Starts fctl's agents on your projects: say what to do, and an agent works on it in a sandbox, as you, and reports back here.",
                "suggested_prompts": [
                    { "title": "Fix a bug", "message": "Fix the bug where " },
                    { "title": "Explain some code", "message": "How does this work: " },
                    { "title": "What's running?", "message": "What are my sessions doing?" },
                ],
            },
            "slash_commands": [command],
            "shortcuts": [
                {
                    "name": "Send to fctl",
                    "type": "message",
                    "callback_id": super::blocks::callback::SEND_SHORTCUT,
                    "description": "Start an fctl session from this message",
                },
                {
                    "name": "Start an fctl session",
                    "type": "global",
                    "callback_id": super::blocks::callback::START_SHORTCUT,
                    "description": "Start an fctl session on one of your projects",
                },
            ],
        },
        "oauth_config": {
            "redirect_urls": [u.installed, u.linked],
            "scopes": {
                "bot": BOT_SCOPES,
                "user": ["openid", "profile", "email"],
            },
        },
        "settings": {
            "event_subscriptions": events,
            "interactivity": interactivity,
            "org_deploy_enabled": false,
            "socket_mode_enabled": socket_mode,
            "token_rotation_enabled": false,
        },
    })
}

/// The link that opens Slack's form to create an app from a manifest.
pub fn create_url(manifest: &Value) -> String {
    let encoded: String =
        url::form_urlencoded::byte_serialize(manifest.to_string().as_bytes()).collect();
    format!("https://api.slack.com/apps?new_app=1&manifest_json={encoded}")
}

/// A manifest as YAML, as Slack's form shows it: block style, every string
/// quoted as JSON quotes it (which YAML reads the same).
pub fn yaml(v: &Value) -> String {
    let mut out = String::new();
    write_yaml(v, 0, &mut out);
    out
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => Value::String(s.clone()).to_string(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

fn write_yaml(v: &Value, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                match val {
                    Value::Object(m) if !m.is_empty() => {
                        out.push_str(&format!("{pad}{k}:\n"));
                        write_yaml(val, indent + 2, out);
                    }
                    Value::Array(a) if !a.is_empty() => {
                        out.push_str(&format!("{pad}{k}:\n"));
                        write_yaml(val, indent + 2, out);
                    }
                    Value::Object(_) => out.push_str(&format!("{pad}{k}: {{}}\n")),
                    Value::Array(_) => out.push_str(&format!("{pad}{k}: []\n")),
                    _ => out.push_str(&format!("{pad}{k}: {}\n", scalar(val))),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::Object(m) if !m.is_empty() => {
                        let mut inner = String::new();
                        write_yaml(item, indent + 2, &mut inner);
                        // The first key goes on the dash's line.
                        let inner = inner.trim_start();
                        out.push_str(&format!("{pad}- {inner}"));
                    }
                    _ => out.push_str(&format!("{pad}- {}\n", scalar(item))),
                }
            }
        }
        other => out.push_str(&format!("{pad}{}\n", scalar(other))),
    }
}

/// Whether Slack can reach an instance at `public_url` to deliver events
/// over HTTP: an `https://` address of a public host. Otherwise only Socket
/// Mode brings them.
pub fn reachable(public_url: &str) -> bool {
    let (reach, _) = crate::hooks::Reach::parse(None, None);
    public_url.starts_with("https://") && crate::hooks::check_url(public_url, &reach).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a test expects to be there: it fails the test where it isn't.
    trait Must<T> {
        fn must(self) -> T;
    }

    impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
        fn must(self) -> T {
            match self {
                Ok(v) => v,
                Err(e) => panic!("{e:?}"),
            }
        }
    }

    impl<T> Must<T> for Option<T> {
        fn must(self) -> T {
            match self {
                Some(v) => v,
                None => panic!("nothing there"),
            }
        }
    }

    #[test]
    fn the_manifest_names_the_instances_addresses() {
        let m = manifest("Acme fctl", "https://fctl.acme.dev/", false);
        assert_eq!(m["display_information"]["name"], "Acme fctl");
        assert_eq!(
            m["settings"]["event_subscriptions"]["request_url"],
            "https://fctl.acme.dev/api/v1/slack/events"
        );
        assert_eq!(
            m["settings"]["interactivity"]["request_url"],
            "https://fctl.acme.dev/api/v1/slack/interactivity"
        );
        assert_eq!(
            m["features"]["slash_commands"][0]["url"],
            "https://fctl.acme.dev/api/v1/slack/commands"
        );
        assert_eq!(
            m["oauth_config"]["redirect_urls"][1],
            "https://fctl.acme.dev/api/v1/slack/linked"
        );
        assert_eq!(m["features"]["app_home"]["agent_tasks_enabled"], false);
        assert_eq!(m["settings"]["org_deploy_enabled"], false);
        let events = m["settings"]["event_subscriptions"]["bot_events"]
            .as_array()
            .must();
        assert!(events.contains(&json!("agent_session_stopped")));
        // Over Socket Mode, no addresses for events.
        let s = manifest("fctl", "http://fctl.internal", true);
        assert!(
            s["settings"]["event_subscriptions"]
                .get("request_url")
                .is_none()
        );
        assert_eq!(s["settings"]["socket_mode_enabled"], true);
        assert!(s["features"]["slash_commands"][0].get("url").is_none());
        // Slack's limits.
        assert!(
            m["features"]["agent_view"]["agent_description"]
                .as_str()
                .must()
                .chars()
                .count()
                <= 300
        );
        let long = m["display_information"]["long_description"]
            .as_str()
            .must()
            .chars()
            .count();
        assert!((175..=4000).contains(&long));
        assert_eq!(app_name(&"x".repeat(50)).chars().count(), 35);
    }

    #[test]
    fn the_manifest_reads_as_yaml_and_opens_slacks_form() {
        let m = manifest("fctl", "https://app.fctl.sh", false);
        let y = yaml(&m);
        assert!(y.contains("display_information:\n  background_color: \"#171717\"\n"));
        assert!(y.contains("\n  name: \"fctl\"\n"));
        assert!(y.contains("  slash_commands:\n    - command: \"/fctl\"\n"));
        assert!(y.contains("      - \"app_mentions:read\"\n"));
        assert!(y.contains("  org_deploy_enabled: false\n"));
        let url = create_url(&m);
        assert!(url.starts_with("https://api.slack.com/apps?new_app=1&manifest_json=%7B"));
        assert!(reachable("https://app.fctl.sh"));
        assert!(!reachable("http://fctl.acme.dev"));
        assert!(!reachable("https://localhost:8787"));
        assert!(!reachable("https://10.0.0.4"));
    }
}
