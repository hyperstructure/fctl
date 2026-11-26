//! What an agent may read and post in Slack (docs/architecture.md#slack).
//!
//! Slack lets fctl's app read every conversation it is in, so fctl narrows
//! it, with what Slack says of the conversation (`conversations.info`)
//! before it reads anything:
//!
//! - a session's own thread, in any conversation it started in;
//! - public channels the workspace's admins made readable, where the
//!   session's project reads them;
//! - never more of a private channel or someone's direct messages than the
//!   session's own thread, never anyone else's direct messages with the
//!   app, and never channels shared with other organizations (Slack
//!   Connect), which fctl doesn't take.

use fctl_proto::ApiError;
use fctl_proto::slack::SlackPlace;
use serde_json::Value;

/// A conversation, as `conversations.info` says it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conversation {
    pub id: String,
    /// Its name without `#`; none for direct messages.
    pub name: Option<String>,
    pub place: SlackPlace,
    /// Shared with another organization (Slack Connect), or across an
    /// Enterprise Grid's workspaces.
    pub shared: bool,
    pub archived: bool,
    /// fctl's app is in it.
    pub member: bool,
    /// For direct messages: whose they are with the app.
    pub user: Option<String>,
}

/// Reads `conversations.info`'s channel. Group direct messages, and what
/// Slack adds later, read as [`SlackPlace::Other`], which fctl reads nothing
/// of beyond a session's own thread.
pub fn conversation(v: &Value) -> Option<Conversation> {
    let c = v.get("channel").unwrap_or(v);
    let b = |k: &str| c.get(k).and_then(Value::as_bool).unwrap_or(false);
    let id = c.get("id").and_then(Value::as_str)?.to_owned();
    let place = if b("is_im") {
        SlackPlace::Dm
    } else if b("is_mpim") {
        SlackPlace::Other
    } else if b("is_private") || b("is_group") {
        SlackPlace::Private
    } else if b("is_channel") || id.starts_with('C') {
        SlackPlace::Channel
    } else {
        SlackPlace::Other
    };
    Some(Conversation {
        name: c
            .get("name")
            .and_then(Value::as_str)
            .filter(|_| place != SlackPlace::Dm)
            .map(str::to_owned),
        place,
        shared: b("is_ext_shared")
            || b("is_shared")
            || b("is_org_shared")
            || b("is_pending_ext_shared"),
        archived: b("is_archived"),
        member: b("is_member") || place == SlackPlace::Dm,
        user: c.get("user").and_then(Value::as_str).map(str::to_owned),
        id,
    })
}

/// What the session reaches, as its workspace and project chose.
#[derive(Clone, Debug, Default)]
pub struct Reach<'a> {
    /// The session's own thread: its conversation and first message.
    pub thread: Option<(&'a str, &'a str)>,
    /// The channels its workspace's admins made readable, by id, where its
    /// project reads them; none where it doesn't.
    pub readable: &'a [String],
}

/// What it asks to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reading<'a> {
    /// A thread (by its first message), or a message in it.
    Thread(&'a str),
    /// A channel's history, or a search in it.
    Channel,
}

/// Whether the session may read `what` in `conversation`: `Err` says why
/// not, in words for the agent.
pub fn may_read(reach: &Reach<'_>, c: &Conversation, what: Reading<'_>) -> Result<(), ApiError> {
    if c.shared {
        return Err(ApiError::forbidden(
            "fctl doesn't read channels shared with other organizations (Slack Connect).",
        ));
    }
    let own = match (reach.thread, what) {
        (Some((channel, ts)), Reading::Thread(t)) => channel == c.id && ts == t,
        _ => false,
    };
    if own {
        return Ok(());
    }
    match c.place {
        SlackPlace::Channel if reach.readable.contains(&c.id) => {
            if c.archived {
                Err(ApiError::forbidden("That channel is archived."))
            } else {
                Ok(())
            }
        }
        SlackPlace::Channel => Err(ApiError::forbidden(format!(
            "{} isn't readable for this session's agents: a workspace's admins choose the channels agents read, and a project whether its agents read them.",
            c.name
                .as_deref()
                .map_or("That channel".to_owned(), |n| format!("#{n}"))
        ))),
        SlackPlace::Private => Err(ApiError::forbidden(
            "That's a private channel: agents read only the thread their session started from there.",
        )),
        SlackPlace::Dm => Err(ApiError::forbidden(
            "Those are someone's direct messages with fctl: agents read only the thread their session started from there.",
        )),
        SlackPlace::Other => Err(ApiError::forbidden(
            "fctl doesn't read that conversation: agents read their own thread and the readable channels.",
        )),
    }
}

/// What an agent does in Slack, for its limits (`slack_calls`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Read,
    Search,
    Post,
}

impl CallKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Search => "search",
            Self::Post => "post",
        }
    }
}

/// How many of each a session's agent makes in an hour: enough to work,
/// too few for one that loops to fill a thread or use up the app's limits
/// at Slack, which every session of the Slack workspace shares.
pub fn limit_of(kind: CallKind) -> u32 {
    match kind {
        CallKind::Read => 120,
        CallKind::Search => 30,
        CallKind::Post => 20,
    }
}

/// What a limit reached says to the agent.
pub fn limited(kind: CallKind) -> String {
    match kind {
        CallKind::Read => format!(
            "This session read Slack {} times in the last hour, its most: try again later.",
            limit_of(kind)
        ),
        CallKind::Search => format!(
            "This session searched Slack {} times in the last hour, its most: try again later.",
            limit_of(kind)
        ),
        CallKind::Post => format!(
            "This session posted in Slack {} times in the last hour, its most: say the rest in your turn's last message, which fctl posts in the thread.",
            limit_of(kind)
        ),
    }
}

/// A channel a request names (`#deploys`, `deploys`, `C0123ABC`, or a
/// link to it) among `channels` (id, name): its id.
pub fn find_channel<'a>(asked: &str, channels: &'a [(String, String)]) -> Option<&'a str> {
    let a = asked.trim();
    let a = super::link(a)
        .map(|l| l.channel)
        .unwrap_or_else(|| a.trim_start_matches('#').to_owned());
    channels
        .iter()
        .find(|(id, name)| id == &a || name.eq_ignore_ascii_case(&a))
        .map(|(id, _)| id.as_str())
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

    fn info(v: Value) -> Conversation {
        conversation(&json!({ "ok": true, "channel": v })).must()
    }

    #[test]
    fn conversations_read_as_their_kind() {
        let public =
            info(json!({ "id": "C1", "name": "deploys", "is_channel": true, "is_member": true }));
        assert_eq!(public.place, SlackPlace::Channel);
        assert_eq!(public.name.as_deref(), Some("deploys"));
        let private =
            info(json!({ "id": "C2", "name": "secret", "is_channel": true, "is_private": true }));
        assert_eq!(private.place, SlackPlace::Private);
        let dm = info(json!({ "id": "D1", "is_im": true, "user": "U1" }));
        assert_eq!((dm.place, dm.user.as_deref()), (SlackPlace::Dm, Some("U1")));
        let group = info(json!({ "id": "G1", "is_mpim": true, "name": "mpdm-a--b" }));
        assert_eq!(group.place, SlackPlace::Other);
        let shared = info(
            json!({ "id": "C3", "name": "partners", "is_channel": true, "is_ext_shared": true }),
        );
        assert!(shared.shared);
    }

    #[test]
    fn agents_read_their_own_thread_and_readable_channels_only() {
        let readable = vec!["C1".to_owned()];
        let reach = Reach {
            thread: Some(("C2", "1.1")),
            readable: &readable,
        };
        let public = info(json!({ "id": "C1", "name": "deploys", "is_channel": true }));
        let other = info(json!({ "id": "C9", "name": "random", "is_channel": true }));
        let private =
            info(json!({ "id": "C2", "name": "secret", "is_channel": true, "is_private": true }));
        let dm = info(json!({ "id": "D1", "is_im": true, "user": "U9" }));
        assert!(may_read(&reach, &public, Reading::Channel).is_ok());
        assert!(may_read(&reach, &public, Reading::Thread("5.5")).is_ok());
        let no = may_read(&reach, &other, Reading::Channel);
        assert!(no.is_err_and(|e| e.message.contains("#random isn't readable")));
        // In a private channel: its own thread, nothing else.
        assert!(may_read(&reach, &private, Reading::Thread("1.1")).is_ok());
        assert!(may_read(&reach, &private, Reading::Thread("2.2")).is_err());
        assert!(may_read(&reach, &private, Reading::Channel).is_err());
        // Someone's direct messages with the app: never.
        assert!(may_read(&reach, &dm, Reading::Thread("1.1")).is_err());
        let own_dm = Reach {
            thread: Some(("D1", "1.1")),
            readable: &[],
        };
        assert!(may_read(&own_dm, &dm, Reading::Thread("1.1")).is_ok());
        assert!(may_read(&own_dm, &dm, Reading::Channel).is_err());
        // Shared channels: never, even when made readable.
        let shared_readable = vec!["C3".to_owned()];
        let reach = Reach {
            thread: Some(("C3", "1.1")),
            readable: &shared_readable,
        };
        let shared = info(
            json!({ "id": "C3", "name": "partners", "is_channel": true, "is_ext_shared": true }),
        );
        assert!(may_read(&reach, &shared, Reading::Thread("1.1")).is_err());
    }

    #[test]
    fn channels_are_found_by_name_id_or_link() {
        let channels = vec![
            ("C1".to_owned(), "deploys".to_owned()),
            ("C2".to_owned(), "incidents".to_owned()),
        ];
        assert_eq!(find_channel("#Deploys", &channels), Some("C1"));
        assert_eq!(find_channel("C2", &channels), Some("C2"));
        assert_eq!(
            find_channel(
                "https://acme.slack.com/archives/C2/p1700000000000100",
                &channels
            ),
            Some("C2")
        );
        assert_eq!(find_channel("random", &channels), None);
    }
}
