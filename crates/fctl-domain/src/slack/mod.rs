//! fctl's Slack app (docs/architecture.md#slack), without I/O: how Slack
//! signs what it sends, the events, interactions and commands it sends,
//! links to messages, what fctl writes there ([`blocks`], [`text`]), what
//! agents are told and may read ([`task`], [`access`]), and the app's
//! manifest ([`manifest`]).
//!
//! Slack's only boundary is membership: an app reads every conversation it
//! is in, and is in every person's direct messages with it. What an agent
//! reaches there is fctl's to narrow ([`access`]).

pub mod access;
pub mod blocks;
pub mod manifest;
pub mod task;
pub mod text;

use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha256;

/// Slack's Web API.
pub const API_URL: &str = "https://slack.com/api";
/// Where an admin installs fctl's app in a Slack workspace (OAuth v2).
pub const AUTHORIZE_URL: &str = "https://slack.com/oauth/v2/authorize";
/// Where people sign in with Slack (OpenID Connect) to link their account.
pub const OIDC_AUTHORIZE_URL: &str = "https://slack.com/openid/connect/authorize";

/// What fctl's bot asks of a Slack workspace: to hear its mentions and the
/// replies in the threads it is in, to post and react, to know who people
/// are, its slash command and shortcut, its agent view, to look
/// conversations up before it reads them, to join the channels a
/// workspace's admins make readable, and to search them.
pub const BOT_SCOPES: [&str; 17] = [
    "app_mentions:read",
    "assistant:write",
    "channels:history",
    "channels:join",
    "channels:read",
    "chat:write",
    "commands",
    "groups:history",
    "groups:read",
    "im:history",
    "im:read",
    "im:write",
    "reactions:write",
    "search:read.public",
    "team:read",
    "users:read",
    "users:read.email",
];

/// What a person's Sign in with Slack asks for: who they are, and in which
/// Slack workspace.
pub const OIDC_SCOPES: &str = "openid profile email";

/// How long a request waits for its person to link their Slack account, or
/// choose a project, before fctl forgets it.
pub const WAIT_MS: i64 = 60 * 60 * 1000;

/// How far a request's signed timestamp may be from now (Slack's five
/// minutes), so that nobody replays an old one.
pub const FRESH_MS: i64 = 5 * 60 * 1000;

/// The most fctl reads of what Slack sends: an event, an interaction or a
/// command is far smaller.
pub const MAX_BODY_BYTES: usize = 512 * 1024;

/// Whether `signature` (`X-Slack-Signature`: `v0=` and the HMAC-SHA256 of
/// `v0:<timestamp>:<body>`, in hex) is Slack's with the app's signing
/// `secret`, for a `timestamp` (`X-Slack-Request-Timestamp`, in seconds)
/// within five minutes of `now`. The comparison takes as long whatever the
/// signature.
pub fn signed(secret: &str, timestamp: &str, body: &[u8], signature: &str, now: i64) -> bool {
    let Ok(at) = timestamp.trim().parse::<i64>() else {
        return false;
    };
    if now.abs_diff(at.saturating_mul(1000)) > FRESH_MS.unsigned_abs() {
        return false;
    }
    let Some(tag) = signature
        .trim()
        .strip_prefix("v0=")
        .and_then(|h| hex::decode(h).ok())
    else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(b"v0:");
    mac.update(timestamp.trim().as_bytes());
    mac.update(b":");
    mac.update(body);
    mac.verify_slice(&tag).is_ok()
}

/// The signature Slack would send for `body` at `timestamp`: for tests and
/// the dev stack's stand-in.
pub fn sign(secret: &str, timestamp: &str, body: &[u8]) -> String {
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(b"v0:");
    mac.update(timestamp.as_bytes());
    mac.update(b":");
    mac.update(body);
    format!("v0={}", hex::encode(mac.finalize().into_bytes()))
}

// ───────────────────────────── ids and times ─────────────────────────────

/// A thread's id in `slack_threads`: its Slack workspace, its conversation
/// and its first message.
pub fn thread_id(team: &str, channel: &str, thread_ts: &str) -> String {
    format!("{team}:{channel}:{thread_ts}")
}

/// A Slack `ts` (`1700000000.000100`: seconds and microseconds) as
/// milliseconds since the epoch.
pub fn ts_millis(ts: &str) -> Option<i64> {
    let (secs, micros) = ts.split_once('.').unwrap_or((ts, "0"));
    let secs: i64 = secs.parse().ok()?;
    let micros: i64 = format!("{micros:0<6}").get(..6)?.parse().ok()?;
    Some(secs * 1000 + micros / 1000)
}

/// Milliseconds since the epoch as a Slack `ts`, for `oldest` and `latest`.
pub fn millis_ts(ms: i64) -> String {
    format!("{}.{:06}", ms.div_euclid(1000), ms.rem_euclid(1000) * 1000)
}

/// A time in UTC, as tasks show when people wrote: `2026-11-25 14:02
/// UTC`.
pub fn utc(ms: i64) -> String {
    let days = ms.div_euclid(crate::cron::DAY_MS);
    let minutes = ms.rem_euclid(crate::cron::DAY_MS) / crate::cron::MINUTE_MS;
    let (y, m, d) = crate::cron::civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        minutes / 60,
        minutes % 60
    )
}

/// Whether `s` looks like one of Slack's ids (`T…`, `C…`, `U…`): what goes
/// into URLs and statements as it is.
pub fn is_id(s: &str) -> bool {
    (2..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Whether Slack's error `code` for a post in a thread says the thread
/// takes no posts from fctl any more: the channel is gone or archived, fctl
/// isn't in it, the thread is gone, or an admin keeps fctl from posting
/// there (a read-only channel, a locked thread). What fctl meant to post
/// there then goes to the person who asked; other errors don't say that.
pub fn thread_refuses(code: &str) -> bool {
    matches!(
        code,
        "channel_not_found" | "not_in_channel" | "is_archived" | "thread_not_found"
    ) || code.starts_with("restricted_action")
}

/// Whether `s` looks like a Slack `ts`.
pub fn is_ts(s: &str) -> bool {
    let Some((a, b)) = s.split_once('.') else {
        return false;
    };
    (1..=12).contains(&a.len())
        && (1..=8).contains(&b.len())
        && a.bytes().all(|c| c.is_ascii_digit())
        && b.bytes().all(|c| c.is_ascii_digit())
}

// ───────────────────────────── links ─────────────────────────────

/// A link to a message in Slack, as people copy it: its conversation, its
/// `ts`, and its thread's when it is a reply
/// (`https://acme.slack.com/archives/C123/p1700000000000100?thread_ts=1699999999.000050&cid=C123`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The Slack workspace's subdomain (`acme`), where the link names one.
    pub domain: Option<String>,
    pub channel: String,
    pub ts: String,
    pub thread_ts: Option<String>,
}

impl Link {
    /// The thread it is in: its own, or its thread's.
    pub fn thread(&self) -> &str {
        self.thread_ts.as_deref().unwrap_or(&self.ts)
    }
}

/// Reads a link to a Slack message.
pub fn link(url: &str) -> Option<Link> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    let host = parsed.host_str()?;
    if !(host == "slack.com" || host.ends_with(".slack.com")) {
        return None;
    }
    let domain = host
        .strip_suffix(".slack.com")
        .filter(|d| !d.is_empty() && *d != "app")
        .map(str::to_owned);
    let mut segs = parsed.path_segments()?;
    if segs.next()? != "archives" {
        return None;
    }
    let channel = segs.next()?.to_owned();
    let p = segs.next()?;
    let digits = p.strip_prefix('p')?;
    if !is_id(&channel) || digits.len() < 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (secs, micros) = digits.split_at(digits.len() - 6);
    let ts = format!("{secs}.{micros}");
    let thread_ts = parsed
        .query_pairs()
        .find(|(k, _)| k == "thread_ts")
        .map(|(_, v)| v.into_owned())
        .filter(|t| is_ts(t) && *t != ts);
    Some(Link {
        domain,
        channel,
        ts,
        thread_ts,
    })
}

/// The link to a message (`ts`) in conversation `channel` of the Slack
/// workspace at `<domain>.slack.com`, in its thread (`thread_ts`) when it
/// is a reply; without a domain, Slack's own address, which opens it for
/// whoever is signed in.
pub fn permalink(domain: Option<&str>, channel: &str, ts: &str, thread_ts: Option<&str>) -> String {
    let host = match domain.filter(|d| !d.is_empty()) {
        Some(d) => format!("https://{d}.slack.com"),
        None => "https://slack.com/app_redirect?channel=".to_owned(),
    };
    if domain.is_none_or(str::is_empty) {
        return format!("{host}{channel}");
    }
    let p: String = ts.chars().filter(char::is_ascii_digit).collect();
    match thread_ts.filter(|t| *t != ts) {
        Some(t) => format!("{host}/archives/{channel}/p{p}?thread_ts={t}&cid={channel}"),
        None => format!("{host}/archives/{channel}/p{p}"),
    }
}

// ───────────────────────────── events ─────────────────────────────

/// What Slack sends to the events address, or in a Socket Mode envelope of
/// type `events_api`.
#[derive(Clone, Debug, PartialEq)]
pub enum Envelope {
    /// Slack checks the address as it is set: the answer is the challenge.
    UrlVerification {
        challenge: String,
    },
    Event(Box<EventCallback>),
    Other,
}

/// An event, with the Slack workspace it happened in.
#[derive(Clone, Debug, PartialEq)]
pub struct EventCallback {
    pub team_id: String,
    pub api_app_id: Option<String>,
    pub event_id: Option<String>,
    /// An Enterprise Grid organization's install, which fctl doesn't take.
    pub enterprise_install: bool,
    /// The event happened in a channel shared with another organization
    /// (Slack Connect), which fctl doesn't take.
    pub shared: bool,
    pub event: Event,
}

/// The events fctl's app hears.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Someone mentioned fctl's app.
    AppMention(MessageEvent),
    /// A message in a conversation the app is in.
    Message(MessageEvent),
    /// The app was uninstalled from the Slack workspace.
    AppUninstalled,
    /// Tokens were revoked: the bot's, when its user's id is among `bots`.
    TokensRevoked {
        bots: Vec<String>,
    },
    /// Someone opened the app's Home tab (`tab` home) or its messages.
    AppHomeOpened {
        user: String,
        tab: String,
    },
    /// Someone opened a new thread with the app in its agent view.
    AssistantThreadStarted {
        user: String,
        channel: String,
        thread_ts: String,
    },
    /// Someone pressed stop on the app's agent session in their direct
    /// messages.
    AgentSessionStopped {
        user: Option<String>,
        channel: String,
        thread_ts: String,
    },
    Other,
}

/// A message, as Slack's `message` and `app_mention` events give it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct MessageEvent {
    pub user: Option<String>,
    pub text: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub channel: String,
    /// `channel`, `group`, `im`, `mpim` (absent from `app_mention`).
    pub channel_type: Option<String>,
    pub subtype: Option<String>,
    pub bot_id: Option<String>,
    pub app_id: Option<String>,
    pub team: Option<String>,
    pub files: Vec<File>,
}

/// A file a message carries, by its name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct File {
    pub name: Option<String>,
    pub title: Option<String>,
}

impl MessageEvent {
    /// A person wrote it: not a bot, an app, or Slack's own notice of a
    /// change (an edit, a deletion, someone joining).
    pub fn by_person(&self) -> bool {
        self.user.is_some()
            && self.bot_id.is_none()
            && self.app_id.is_none()
            && self
                .subtype
                .as_deref()
                .is_none_or(|s| matches!(s, "file_share" | "thread_broadcast"))
    }

    /// The thread it starts or is in.
    pub fn thread(&self) -> &str {
        self.thread_ts.as_deref().unwrap_or(&self.ts)
    }

    /// It is a reply in a thread, not a thread's first message.
    pub fn is_reply(&self) -> bool {
        self.thread_ts.as_deref().is_some_and(|t| t != self.ts)
    }

    /// It is in someone's direct messages with the app.
    pub fn in_dm(&self) -> bool {
        self.channel_type.as_deref() == Some("im") || self.channel.starts_with('D')
    }

    /// It mentions `user` (`<@U…>`): fctl's app, whose `app_mention` event
    /// says the same.
    pub fn mentions(&self, user: &str) -> bool {
        !user.is_empty() && self.text.contains(&format!("<@{user}>"))
    }

    pub fn file_names(&self) -> Vec<String> {
        self.files
            .iter()
            .filter_map(|f| f.name.clone().or_else(|| f.title.clone()))
            .collect()
    }
}

/// Reads what Slack sent to the events address.
pub fn envelope(v: &Value) -> Envelope {
    let st = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    match v.get("type").and_then(Value::as_str) {
        Some("url_verification") => Envelope::UrlVerification {
            challenge: st(v, "challenge").unwrap_or_default(),
        },
        Some("event_callback") => {
            let Some(team_id) = st(v, "team_id").or_else(|| {
                v.pointer("/authorizations/0/team_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }) else {
                return Envelope::Other;
            };
            let enterprise_install = v
                .pointer("/authorizations/0/is_enterprise_install")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || v.get("is_enterprise_install").and_then(Value::as_bool) == Some(true);
            let shared = v
                .get("is_ext_shared_channel")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let e = v.get("event").cloned().unwrap_or(Value::Null);
            Envelope::Event(Box::new(EventCallback {
                team_id,
                api_app_id: st(v, "api_app_id"),
                event_id: st(v, "event_id"),
                enterprise_install,
                shared,
                event: event(&e),
            }))
        }
        _ => Envelope::Other,
    }
}

fn event(e: &Value) -> Event {
    let st = |k: &str| e.get(k).and_then(Value::as_str).map(str::to_owned);
    let message = || serde_json::from_value::<MessageEvent>(e.clone()).ok();
    match e.get("type").and_then(Value::as_str) {
        Some("app_mention") => message().map_or(Event::Other, Event::AppMention),
        Some("message") => message().map_or(Event::Other, Event::Message),
        Some("app_uninstalled") => Event::AppUninstalled,
        Some("tokens_revoked") => Event::TokensRevoked {
            bots: e
                .pointer("/tokens/bot")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        },
        Some("app_home_opened") => match st("user") {
            Some(user) => Event::AppHomeOpened {
                user,
                tab: st("tab").unwrap_or_else(|| "home".into()),
            },
            None => Event::Other,
        },
        Some("assistant_thread_started") => {
            let t = e.get("assistant_thread").cloned().unwrap_or(Value::Null);
            let s = |k: &str| t.get(k).and_then(Value::as_str).map(str::to_owned);
            match (s("user_id"), s("channel_id"), s("thread_ts")) {
                (Some(user), Some(channel), Some(thread_ts)) => Event::AssistantThreadStarted {
                    user,
                    channel,
                    thread_ts,
                },
                _ => Event::Other,
            }
        }
        Some("agent_session_stopped") => {
            // Its thread, as the agent session names it, or as the event
            // does.
            let channel = e
                .pointer("/agent_session/channel_id")
                .or_else(|| e.get("channel_id"))
                .or_else(|| e.get("channel"))
                .and_then(Value::as_str);
            let thread_ts = e
                .pointer("/agent_session/thread_ts")
                .or_else(|| e.get("thread_ts"))
                .and_then(Value::as_str);
            match (channel, thread_ts) {
                (Some(c), Some(t)) => Event::AgentSessionStopped {
                    user: st("user").or_else(|| st("user_id")).or_else(|| {
                        e.pointer("/agent_session/user_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    }),
                    channel: c.to_owned(),
                    thread_ts: t.to_owned(),
                },
                _ => Event::Other,
            }
        }
        _ => Event::Other,
    }
}

// ───────────────────────────── interactions ─────────────────────────────

/// Someone in Slack, as interactions and commands name them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Actor {
    pub team: String,
    pub user: String,
    /// Their Slack handle, as the payload gives it.
    pub name: Option<String>,
}

/// A button or a menu someone used, in a message or a view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Action {
    pub action_id: String,
    pub block_id: Option<String>,
    /// A button's value, or the option chosen in a menu.
    pub value: Option<String>,
}

/// What someone did with fctl's app's buttons, shortcut or views: what
/// Slack sends to the interactivity address (a form whose `payload` is
/// JSON), or in a Socket Mode envelope of type `interactive`.
#[derive(Clone, Debug, PartialEq)]
pub enum Interaction {
    /// A button or a menu in a message, an ephemeral message or the Home
    /// tab.
    BlockActions {
        actor: Actor,
        channel: Option<String>,
        /// The message it is in, and its thread.
        message_ts: Option<String>,
        thread_ts: Option<String>,
        ephemeral: bool,
        actions: Vec<Action>,
        response_url: Option<String>,
        trigger_id: Option<String>,
        /// What the view it is in keeps, for a view's buttons.
        view_metadata: Option<String>,
        /// Checkboxes ticked in the message, by their block's id.
        checked: Vec<(String, Vec<String>)>,
    },
    /// Someone sent a form fctl opened.
    ViewSubmission {
        actor: Actor,
        callback_id: String,
        private_metadata: String,
        /// Each input's value, by its block's id.
        values: Vec<(String, String)>,
        /// Checkboxes ticked, by their block's id and their values.
        checked: Vec<(String, Vec<String>)>,
    },
    /// A message shortcut (Send to fctl…) on a message.
    MessageAction {
        actor: Actor,
        callback_id: String,
        channel: String,
        channel_name: Option<String>,
        message: MessageEvent,
        response_url: Option<String>,
        trigger_id: String,
    },
    /// A global shortcut (Start an fctl session).
    Shortcut {
        actor: Actor,
        callback_id: String,
        trigger_id: String,
    },
    Other,
}

fn actor(v: &Value) -> Actor {
    let s = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_owned);
    Actor {
        team: s("/team/id")
            .or_else(|| s("/user/team_id"))
            .unwrap_or_default(),
        user: s("/user/id").unwrap_or_default(),
        name: s("/user/username").or_else(|| s("/user/name")),
    }
}

/// Reads an interaction's payload.
pub fn interaction(v: &Value) -> Interaction {
    let s = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_owned);
    match v.get("type").and_then(Value::as_str) {
        Some("block_actions") => Interaction::BlockActions {
            checked: state(v).1,
            actor: actor(v),
            channel: s("/channel/id").or_else(|| s("/container/channel_id")),
            message_ts: s("/container/message_ts").or_else(|| s("/message/ts")),
            thread_ts: s("/container/thread_ts").or_else(|| s("/message/thread_ts")),
            ephemeral: v
                .pointer("/container/is_ephemeral")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            actions: v
                .get("actions")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(action).collect())
                .unwrap_or_default(),
            response_url: s("/response_url"),
            trigger_id: s("/trigger_id"),
            view_metadata: s("/view/private_metadata"),
        },
        Some("view_submission") => {
            let (values, checked) = state(v);
            Interaction::ViewSubmission {
                actor: actor(v),
                callback_id: s("/view/callback_id").unwrap_or_default(),
                private_metadata: s("/view/private_metadata").unwrap_or_default(),
                values,
                checked,
            }
        }
        Some("message_action") => Interaction::MessageAction {
            actor: actor(v),
            callback_id: s("/callback_id").unwrap_or_default(),
            channel: s("/channel/id").unwrap_or_default(),
            channel_name: s("/channel/name"),
            message: v
                .get("message")
                .cloned()
                .and_then(|m| serde_json::from_value(m).ok())
                .unwrap_or_default(),
            response_url: s("/response_url"),
            trigger_id: s("/trigger_id").unwrap_or_default(),
        },
        Some("shortcut") => Interaction::Shortcut {
            actor: actor(v),
            callback_id: s("/callback_id").unwrap_or_default(),
            trigger_id: s("/trigger_id").unwrap_or_default(),
        },
        _ => Interaction::Other,
    }
}

/// What a view's or a message's inputs hold (`state.values`): each input's
/// value, and each set of checkboxes' ticked values, by their block's id.
#[allow(clippy::type_complexity)]
fn state(v: &Value) -> (Vec<(String, String)>, Vec<(String, Vec<String>)>) {
    let mut values = Vec::new();
    let mut checked = Vec::new();
    let blocks = v
        .pointer("/view/state/values")
        .or_else(|| v.pointer("/state/values"))
        .and_then(Value::as_object);
    for (block, inputs) in blocks.into_iter().flatten() {
        let Some(inputs) = inputs.as_object() else {
            continue;
        };
        for input in inputs.values() {
            if let Some(opts) = input.get("selected_options").and_then(Value::as_array) {
                checked.push((
                    block.clone(),
                    opts.iter()
                        .filter_map(|o| o.get("value").and_then(Value::as_str))
                        .map(str::to_owned)
                        .collect(),
                ));
                continue;
            }
            let value = input
                .get("value")
                .and_then(Value::as_str)
                .or_else(|| {
                    input
                        .pointer("/selected_option/value")
                        .and_then(Value::as_str)
                })
                .or_else(|| input.get("selected_conversation").and_then(Value::as_str))
                .or_else(|| input.get("selected_channel").and_then(Value::as_str));
            if let Some(value) = value {
                values.push((block.clone(), value.to_owned()));
            }
        }
    }
    (values, checked)
}

fn action(a: &Value) -> Action {
    let s = |p: &str| a.pointer(p).and_then(Value::as_str).map(str::to_owned);
    Action {
        action_id: s("/action_id").unwrap_or_default(),
        block_id: s("/block_id"),
        value: s("/value")
            .or_else(|| s("/selected_option/value"))
            .or_else(|| {
                a.get("selected_options")
                    .and_then(Value::as_array)
                    .map(|o| {
                        o.iter()
                            .filter_map(|o| o.get("value").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join(",")
                    })
            }),
    }
}

// ───────────────────────────── commands ─────────────────────────────

/// `/fctl`, as Slack sends it: a form, or the payload of a Socket Mode
/// envelope of type `slash_commands`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    pub actor: Actor,
    pub command: String,
    pub text: String,
    pub channel: String,
    pub channel_name: Option<String>,
    pub response_url: Option<String>,
    pub trigger_id: Option<String>,
    pub enterprise_install: bool,
}

/// Reads a slash command from its fields, by name.
pub fn command(get: impl Fn(&str) -> Option<String>) -> Command {
    Command {
        actor: Actor {
            team: get("team_id").unwrap_or_default(),
            user: get("user_id").unwrap_or_default(),
            name: get("user_name"),
        },
        command: get("command").unwrap_or_default(),
        text: get("text").unwrap_or_default(),
        channel: get("channel_id").unwrap_or_default(),
        channel_name: get("channel_name").filter(|n| n != "directmessage" && n != "privategroup"),
        response_url: get("response_url"),
        trigger_id: get("trigger_id"),
        enterprise_install: get("is_enterprise_install").as_deref() == Some("true"),
    }
}

/// What `/fctl` asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    /// A session, with this task.
    Start(String),
    /// Your sessions, and what waits for you.
    Status,
    /// How `/fctl` works.
    Help,
}

/// Reads `/fctl`'s text.
pub fn asked(text: &str) -> Asked {
    let t = text.trim();
    match t.to_ascii_lowercase().as_str() {
        "" | "help" | "-h" | "--help" | "?" => Asked::Help,
        "status" | "list" | "ls" | "sessions" => Asked::Status,
        _ => Asked::Start(t.to_owned()),
    }
}

// ───────────────────────────── installing and signing in ─────────────────────────────

/// What `oauth.v2.access` answers when an admin installs fctl's app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub team: String,
    pub team_name: String,
    pub bot_token: String,
    pub bot_user: String,
    pub app_id: Option<String>,
    pub scopes: Vec<String>,
    /// Who installed it, in Slack.
    pub installer: Option<String>,
    /// An Enterprise Grid organization's install, which fctl doesn't take.
    pub enterprise: bool,
}

/// Reads `oauth.v2.access`'s answer: `Err` with Slack's error, or why it
/// can't be used.
pub fn installed(v: &Value) -> Result<Installed, String> {
    if v.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(v
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Slack refused the installation")
            .to_owned());
    }
    let s = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_owned);
    let enterprise = v
        .get("is_enterprise_install")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(Installed {
        team: match s("/team/id") {
            Some(t) => t,
            None if enterprise => String::new(),
            None => return Err("Slack's answer names no Slack workspace".into()),
        },
        team_name: s("/team/name").unwrap_or_else(|| "Slack".into()),
        bot_token: s("/access_token").ok_or("Slack's answer has no bot token")?,
        bot_user: s("/bot_user_id").ok_or("Slack's answer has no bot user")?,
        app_id: s("/app_id"),
        scopes: s("/scope")
            .map(|sc| sc.split(',').map(str::to_owned).collect())
            .unwrap_or_default(),
        installer: s("/authed_user/id"),
        enterprise,
    })
}

/// Who a Sign in with Slack signed in, from its ID token's claims (which
/// came straight from Slack's token endpoint, over TLS, for fctl's own
/// client: fctl reads them as they are).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedIn {
    pub team: String,
    pub team_name: Option<String>,
    pub team_domain: Option<String>,
    pub user: String,
    pub name: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub picture: Option<String>,
    pub nonce: Option<String>,
}

/// Reads an ID token's claims (its middle part), without checking its
/// signature: only for a token fctl's client got from Slack itself.
pub fn signed_in(id_token: &str) -> Option<SignedIn> {
    use base64::Engine;
    let claims = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(claims.trim_end_matches('='))
        .ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Some(SignedIn {
        team: s("https://slack.com/team_id")?,
        team_name: s("https://slack.com/team_name"),
        team_domain: s("https://slack.com/team_domain"),
        user: s("https://slack.com/user_id").or_else(|| s("sub"))?,
        name: s("name")
            .filter(|n| !n.trim().is_empty())
            .or_else(|| s("given_name"))
            .unwrap_or_else(|| "Someone".into()),
        email: s("email"),
        email_verified: v
            .get("email_verified")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        picture: s("picture"),
        nonce: s("nonce"),
    })
}

/// The address that installs fctl's app in a Slack workspace.
pub fn authorize_url(
    base: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    team: Option<&str>,
) -> String {
    let mut u = url::form_urlencoded::Serializer::new(String::new());
    u.append_pair("client_id", client_id)
        .append_pair("scope", &BOT_SCOPES.join(","))
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", state);
    if let Some(t) = team.filter(|t| is_id(t)) {
        u.append_pair("team", t);
    }
    format!("{base}?{}", u.finish())
}

/// The address where someone signs in with Slack to link their account,
/// in the Slack workspace `team` where fctl knows it.
pub fn oidc_url(
    base: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    nonce: &str,
    team: Option<&str>,
) -> String {
    let mut u = url::form_urlencoded::Serializer::new(String::new());
    u.append_pair("response_type", "code")
        .append_pair("scope", OIDC_SCOPES)
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", state)
        .append_pair("nonce", nonce);
    if let Some(t) = team.filter(|t| is_id(t)) {
        u.append_pair("team", t);
    }
    format!("{base}?{}", u.finish())
}

/// What Slack's Web API answered: `Err` with its error code
/// (`channel_not_found`, `not_in_channel`, `ratelimited`…).
pub fn answer(v: &Value) -> Result<&Value, String> {
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(v)
    } else {
        Err(v
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown_error")
            .to_owned())
    }
}

/// A Slack user, as `users.info` gives them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub name: String,
    pub handle: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub bot: bool,
    pub deleted: bool,
    /// A guest of the Slack workspace (single- or multi-channel), which
    /// Slack keeps from agent apps.
    pub guest: bool,
    /// Their time zone, by its IANA name.
    pub tz: Option<String>,
}

/// Reads `users.info`'s user.
pub fn user(v: &Value) -> Option<User> {
    let u = v.get("user").unwrap_or(v);
    let s = |p: &str| {
        u.pointer(p)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let b = |p: &str| u.pointer(p).and_then(Value::as_bool).unwrap_or(false);
    let display = s("/profile/display_name");
    let real = s("/profile/real_name")
        .or_else(|| s("/real_name"))
        .or_else(|| display.clone())
        .or_else(|| s("/name"))?;
    Some(User {
        id: s("/id")?,
        handle: display.filter(|d| *d != real),
        name: real,
        email: s("/profile/email"),
        avatar_url: s("/profile/image_72").or_else(|| s("/profile/image_48")),
        bot: b("/is_bot") || s("/id").as_deref() == Some("USLACKBOT"),
        deleted: b("/deleted"),
        guest: b("/is_restricted") || b("/is_ultra_restricted"),
        tz: s("/tz"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    const SECRET: &str = "8f742231b10e8888abcd99yyyzzz85a5";

    #[test]
    fn a_thread_refuses_posts_only_where_slack_says_so() {
        for code in [
            "channel_not_found",
            "not_in_channel",
            "is_archived",
            "thread_not_found",
            "restricted_action",
            "restricted_action_read_only_channel",
            "restricted_action_thread_locked",
        ] {
            assert!(thread_refuses(code), "{code}");
        }
        for code in [
            "msg_too_long",
            "no_text",
            "token_revoked",
            "missing_scope",
            "invalid_blocks",
        ] {
            assert!(!thread_refuses(code), "{code}");
        }
    }

    #[test]
    fn slacks_signature_counts_within_five_minutes() {
        // Slack's own example (docs.slack.dev, verifying requests).
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J&team_domain=testteamnow&channel_id=G8PSS9T3V&channel_name=foobar&user_id=U2CERLKJA&user_name=roadrunner&command=%2Fwebhook-collect&text=&response_url=https%3A%2F%2Fhooks.slack.com%2Fcommands%2FT1DC2JH3J%2F397700885554%2F96rGlfmibIGlgcZRskXaIFfN&trigger_id=398738663015.47445629121.803a0bc887a14d10d2c447fce8b6703c";
        let ts = "1531420618";
        let sig = "v0=a2114d57b48eac39b9ad189dd8316235a7b4a8d21a10bd27519666489c69b503";
        let now = 1_531_420_618_000;
        assert!(signed(SECRET, ts, body, sig, now));
        assert_eq!(sign(SECRET, ts, body), sig);
        assert!(signed(SECRET, ts, body, sig, now + 4 * 60 * 1000));
        assert!(!signed(SECRET, ts, body, sig, now + 6 * 60 * 1000));
        assert!(!signed("another", ts, body, sig, now));
        assert!(!signed(SECRET, ts, b"token=changed", sig, now));
        assert!(!signed(SECRET, ts, body, "a2114d57", now));
        assert!(!signed(SECRET, "soon", body, sig, now));
    }

    #[test]
    fn ts_reads_as_milliseconds_and_back() {
        assert_eq!(ts_millis("1700000000.000100"), Some(1_700_000_000_000));
        assert_eq!(ts_millis("1700000000.123456"), Some(1_700_000_000_123));
        assert_eq!(ts_millis("1700000000"), Some(1_700_000_000_000));
        assert_eq!(ts_millis("later"), None);
        assert_eq!(millis_ts(1_700_000_000_123), "1700000000.123000");
        assert_eq!(utc(1_700_000_000_000), "2023-11-14 22:13 UTC");
        assert!(is_ts("1700000000.000100"));
        assert!(!is_ts("1700000000"));
        assert!(is_id("C0123ABC") && !is_id("c0123") && !is_id("C"));
    }

    #[test]
    fn links_read_their_message_and_thread() {
        let l = link("https://acme.slack.com/archives/C0123ABC/p1700000000000100").must();
        assert_eq!(l.domain.as_deref(), Some("acme"));
        assert_eq!(l.channel, "C0123ABC");
        assert_eq!(l.ts, "1700000000.000100");
        assert_eq!(l.thread_ts, None);
        assert_eq!(l.thread(), "1700000000.000100");
        let r = link("https://acme.slack.com/archives/C0123ABC/p1700000500000200?thread_ts=1700000000.000100&cid=C0123ABC").must();
        assert_eq!(r.thread(), "1700000000.000100");
        assert_eq!(
            permalink(Some("acme"), "C0123ABC", &r.ts, Some(r.thread())),
            "https://acme.slack.com/archives/C0123ABC/p1700000500000200?thread_ts=1700000000.000100&cid=C0123ABC"
        );
        assert_eq!(
            permalink(Some("acme"), "C0123ABC", "1700000000.000100", None),
            "https://acme.slack.com/archives/C0123ABC/p1700000000000100"
        );
        assert!(link("https://evil.example/archives/C0123ABC/p1700000000000100").is_none());
        assert!(link("https://acme.slack.com/messages/C0123ABC").is_none());
        assert!(link("https://acme.slack.com/archives/c01/p1700000000000100").is_none());
    }

    #[test]
    fn events_read_as_slack_sends_them() {
        let mention = json!({
            "type": "event_callback", "team_id": "T1", "api_app_id": "A1", "event_id": "Ev1",
            "authorizations": [{ "team_id": "T1", "user_id": "UBOT", "is_bot": true, "is_enterprise_install": false }],
            "event": { "type": "app_mention", "user": "U1", "text": "<@UBOT> fix the flaky test",
                "ts": "1700000000.000100", "channel": "C1", "event_ts": "1700000000.000100" }
        });
        let Envelope::Event(e) = envelope(&mention) else {
            panic!("not an event")
        };
        assert_eq!(e.team_id, "T1");
        assert!(!e.enterprise_install && !e.shared);
        let Event::AppMention(m) = &e.event else {
            panic!("not a mention")
        };
        assert!(m.by_person() && m.mentions("UBOT") && !m.is_reply());
        assert_eq!(m.thread(), "1700000000.000100");

        let reply = json!({ "type": "event_callback", "team_id": "T1", "event": {
            "type": "message", "user": "U2", "text": "and the docs", "ts": "1700000100.000200",
            "thread_ts": "1700000000.000100", "channel": "C1", "channel_type": "channel" } });
        let Envelope::Event(e) = envelope(&reply) else {
            panic!()
        };
        let Event::Message(m) = &e.event else {
            panic!()
        };
        assert!(m.is_reply() && !m.in_dm() && m.by_person());

        let bot = json!({ "type": "event_callback", "team_id": "T1", "event": {
            "type": "message", "subtype": "bot_message", "bot_id": "B1", "text": "hi", "ts": "1.2", "channel": "C1" } });
        let Envelope::Event(e) = envelope(&bot) else {
            panic!()
        };
        let Event::Message(m) = &e.event else {
            panic!()
        };
        assert!(!m.by_person());
        let edited = json!({ "type": "message", "subtype": "message_changed", "user": "U1", "ts": "1.2", "channel": "C1" });
        assert!(
            !serde_json::from_value::<MessageEvent>(edited)
                .must()
                .by_person()
        );

        assert_eq!(
            envelope(&json!({ "type": "url_verification", "challenge": "abc" })),
            Envelope::UrlVerification {
                challenge: "abc".into()
            }
        );
        let revoked = json!({ "type": "event_callback", "team_id": "T1", "event": {
            "type": "tokens_revoked", "tokens": { "oauth": ["U1"], "bot": ["UBOT"] } } });
        let Envelope::Event(e) = envelope(&revoked) else {
            panic!()
        };
        assert_eq!(
            e.event,
            Event::TokensRevoked {
                bots: vec!["UBOT".into()]
            }
        );
        let shared = json!({ "type": "event_callback", "team_id": "T1", "is_ext_shared_channel": true,
            "event": { "type": "app_mention", "user": "U9", "text": "<@UBOT> hi", "ts": "1.2", "channel": "C9" } });
        let Envelope::Event(e) = envelope(&shared) else {
            panic!()
        };
        assert!(e.shared);
        let started = json!({ "type": "event_callback", "team_id": "T1", "event": {
            "type": "assistant_thread_started",
            "assistant_thread": { "user_id": "U1", "channel_id": "D1", "thread_ts": "1729999327.187299",
                "context": { "channel_id": "C1", "team_id": "T1" } } } });
        let Envelope::Event(e) = envelope(&started) else {
            panic!()
        };
        assert_eq!(
            e.event,
            Event::AssistantThreadStarted {
                user: "U1".into(),
                channel: "D1".into(),
                thread_ts: "1729999327.187299".into()
            }
        );
    }

    #[test]
    fn interactions_read_their_actions_and_values() {
        let click = json!({
            "type": "block_actions", "user": { "id": "U1", "username": "ada", "team_id": "T1" },
            "team": { "id": "T1", "domain": "acme" },
            "container": { "type": "message", "message_ts": "1.5", "channel_id": "C1", "thread_ts": "1.1", "is_ephemeral": false },
            "channel": { "id": "C1", "name": "web-team" },
            "actions": [{ "action_id": "fctl_answer_1", "block_id": "q", "value": "1", "type": "button" }],
            "response_url": "https://hooks.slack.com/actions/x", "trigger_id": "t"
        });
        let Interaction::BlockActions {
            actor,
            channel,
            thread_ts,
            actions,
            ..
        } = interaction(&click)
        else {
            panic!()
        };
        assert_eq!(actor.user, "U1");
        assert_eq!(actor.team, "T1");
        assert_eq!(channel.as_deref(), Some("C1"));
        assert_eq!(thread_ts.as_deref(), Some("1.1"));
        assert_eq!(actions[0].value.as_deref(), Some("1"));

        let submit = json!({
            "type": "view_submission", "user": { "id": "U1", "team_id": "T1" }, "team": { "id": "T1" },
            "view": { "callback_id": "fctl_send", "private_metadata": "{\"c\":\"C1\"}",
                "state": { "values": {
                    "project": { "p": { "type": "static_select", "selected_option": { "value": "web" } } },
                    "task": { "t": { "type": "plain_text_input", "value": "Fix it" } },
                    "updates": { "u": { "type": "checkboxes", "selected_options": [{ "value": "updates" }] } }
                } } }
        });
        let Interaction::ViewSubmission {
            callback_id,
            values,
            checked,
            ..
        } = interaction(&submit)
        else {
            panic!()
        };
        assert_eq!(callback_id, "fctl_send");
        assert!(values.contains(&("project".into(), "web".into())));
        assert!(values.contains(&("task".into(), "Fix it".into())));
        assert_eq!(checked, vec![("updates".into(), vec!["updates".into()])]);
    }

    #[test]
    fn slash_commands_read_their_fields() {
        let fields = [
            ("team_id", "T1"),
            ("user_id", "U1"),
            ("command", "/fctl"),
            ("text", "  status "),
            ("channel_id", "C1"),
            ("channel_name", "web-team"),
        ];
        let c = command(|k| {
            fields
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        });
        assert_eq!(c.actor.user, "U1");
        assert_eq!(c.channel_name.as_deref(), Some("web-team"));
        assert_eq!(asked(&c.text), Asked::Status);
        assert_eq!(asked(""), Asked::Help);
        assert_eq!(
            asked("fix the login redirect"),
            Asked::Start("fix the login redirect".into())
        );
    }

    #[test]
    fn installs_and_sign_ins_read_what_slack_answers() {
        let ok = json!({ "ok": true, "access_token": "xoxb-1", "token_type": "bot", "scope": "chat:write,commands",
            "bot_user_id": "UBOT", "app_id": "A1", "team": { "id": "T1", "name": "Acme" }, "enterprise": null,
            "is_enterprise_install": false, "authed_user": { "id": "U1" } });
        let i = installed(&ok).must();
        assert_eq!((i.team.as_str(), i.bot_user.as_str()), ("T1", "UBOT"));
        assert_eq!(i.scopes, vec!["chat:write", "commands"]);
        assert!(!i.enterprise);
        assert_eq!(
            installed(&json!({ "ok": false, "error": "invalid_code" })),
            Err("invalid_code".into())
        );
        let grid = json!({ "ok": true, "access_token": "xoxb-1", "bot_user_id": "UBOT",
            "enterprise": { "id": "E1" }, "is_enterprise_install": true, "team": null });
        assert!(installed(&grid).must().enterprise);

        use base64::Engine;
        let claims = json!({ "iss": "https://slack.com", "sub": "U1", "https://slack.com/user_id": "U1",
            "https://slack.com/team_id": "T1", "https://slack.com/team_name": "Acme",
            "https://slack.com/team_domain": "acme", "email": "ada@acme.dev", "email_verified": true,
            "name": "Ada Lovelace", "picture": "https://avatars.slack-edge.com/a.png", "nonce": "n1" });
        let token = format!(
            "e30.{}.sig",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let who = signed_in(&token).must();
        assert_eq!((who.team.as_str(), who.user.as_str()), ("T1", "U1"));
        assert_eq!(who.team_domain.as_deref(), Some("acme"));
        assert!(who.email_verified);
        assert_eq!(who.nonce.as_deref(), Some("n1"));
        assert!(signed_in("not-a-token").is_none());
    }

    #[test]
    fn users_read_their_names_and_whether_they_are_guests() {
        let u = user(&json!({ "ok": true, "user": { "id": "U1", "name": "ada", "real_name": "Ada Lovelace",
            "tz": "Europe/London", "is_restricted": true,
            "profile": { "display_name": "ada", "real_name": "Ada Lovelace", "email": "ada@acme.dev", "image_72": "https://a/72.png" } } }))
        .must();
        assert_eq!(u.name, "Ada Lovelace");
        assert_eq!(u.handle.as_deref(), Some("ada"));
        assert!(u.guest && !u.bot);
        assert_eq!(u.tz.as_deref(), Some("Europe/London"));
    }

    #[test]
    fn authorize_urls_carry_the_scopes_and_the_team() {
        let u = authorize_url(
            AUTHORIZE_URL,
            "123.456",
            "https://fctl.acme.dev/api/v1/slack/installed",
            "st",
            Some("T1"),
        );
        assert!(u.starts_with(
            "https://slack.com/oauth/v2/authorize?client_id=123.456&scope=app_mentions%3Aread%2C"
        ));
        assert!(u.contains("&team=T1"));
        let o = oidc_url(
            OIDC_AUTHORIZE_URL,
            "123.456",
            "https://fctl.acme.dev/api/v1/slack/linked",
            "st",
            "n",
            None,
        );
        assert!(o.contains("scope=openid+profile+email") && o.contains("nonce=n"));
        assert!(!o.contains("team="));
    }
}
