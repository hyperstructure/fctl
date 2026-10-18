//! Notifications: which changes in a session notify the people who follow
//! it, who they reach, what they say, and when they go out. Several that
//! arrive close together go out as one digest.

use crate::perm::{self, SessionAccess};
use fctl_proto::api::{NotificationSettings, PushMessage};
use fctl_proto::session::{InstanceRole, ProjectRole, SessionRole, SessionStatus};
use fctl_proto::{Millis, SessionId};
use serde::{Deserialize, Serialize};

/// The first notification of a burst waits this long for the rest of it.
pub const GATHER_MS: i64 = 10_000;
/// At most one notification a minute: what arrives within a minute of the
/// last one goes out together when the minute is up.
pub const SPACING_MS: i64 = 60_000;
/// The agent's last words in a notification, at most.
pub const EXCERPT_CHARS: usize = 140;
/// Sessions a digest names; it counts the rest.
const DIGEST_LINES: usize = 4;
/// Notifications a digest closes, at most (the latest sessions'), so that
/// it fits a push message.
const DIGEST_REPLACES: usize = 20;

/// What happened to a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The agent finished its turn and waits for a reply.
    Waiting,
    /// It paused on a Claude usage limit and continues when it resets.
    Paused,
    /// It failed.
    Failed,
}

impl Kind {
    /// The change from `before` to `after` that notifies, if it does: a
    /// finished turn, a pause and a failure. Idle without a turn (a resumed
    /// session that has nothing to do yet) does not.
    pub fn of(before: SessionStatus, after: SessionStatus) -> Option<Self> {
        use SessionStatus::*;
        match (before, after) {
            (Running, Idle) => Some(Self::Waiting),
            (b, Paused) if b != Paused => Some(Self::Paused),
            (b, Failed) if b != Failed => Some(Self::Failed),
            _ => None,
        }
    }

    /// Whether `settings` ask for it.
    pub fn wanted(self, settings: &NotificationSettings) -> bool {
        match self {
            Self::Waiting => settings.waiting,
            Self::Paused => settings.paused,
            Self::Failed => settings.failed,
        }
    }

    /// Whether a session in `status` is still what this says: one that got a
    /// reply meanwhile no longer waits for it.
    pub fn still(self, status: SessionStatus) -> bool {
        status
            == match self {
                Self::Waiting => SessionStatus::Idle,
                Self::Paused => SessionStatus::Paused,
                Self::Failed => SessionStatus::Failed,
            }
    }
}

/// A change in a session that its followers hear about.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub session_id: SessionId,
    /// `my-app#12`.
    pub reference: String,
    pub title: String,
    pub kind: Kind,
    /// The agent's last words (waiting) or what went wrong (failed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub at: Millis,
}

/// Someone who follows a session.
#[derive(Clone, Debug)]
pub struct Follower {
    pub user_id: String,
    pub instance_role: InstanceRole,
    pub project_role: Option<ProjectRole>,
    pub share_role: Option<SessionRole>,
    pub is_creator: bool,
    pub settings: NotificationSettings,
}

/// Who hears about `kind`: followers who can still see the session and want
/// to, except `except` (who see it right now, or caused the change).
pub fn recipients<'a>(followers: &'a [Follower], kind: Kind, except: &[String]) -> Vec<&'a str> {
    followers
        .iter()
        .filter(|f| {
            perm::session_role(SessionAccess {
                instance_role: f.instance_role,
                project_role: f.project_role,
                share_role: f.share_role,
                is_creator: f.is_creator,
            })
            .is_some()
        })
        .filter(|f| kind.wanted(&f.settings))
        .filter(|f| !except.contains(&f.user_id))
        .map(|f| f.user_id.as_str())
        .collect()
}

/// When notifications pending since `first` go out, given when the last one
/// went out: after the burst gathered, and a minute after the last one.
pub fn due(first: Millis, last_sent: Option<Millis>) -> Millis {
    let gathered = first + GATHER_MS;
    last_sent.map_or(gathered, |t| gathered.max(t + SPACING_MS))
}

/// The start of a message from the agent, on one line, for a notification:
/// Markdown emphasis, code marks, heading and list markers go, and it is cut
/// at a word after `max` characters.
pub fn excerpt(text: &str, max: usize) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(|line| {
            let line = line.trim_start();
            let line = line.trim_start_matches('#').trim_start();
            ["- ", "* ", "+ "]
                .iter()
                .find_map(|m| line.strip_prefix(m))
                .unwrap_or(line)
        })
        .collect();
    let plain = lines
        .join(" ")
        .replace("**", "")
        .replace("__", "")
        .replace('`', "");
    let words: Vec<&str> = plain.split_whitespace().collect();
    let one_line = words.join(" ");
    if one_line.is_empty() {
        return None;
    }
    if one_line.chars().count() <= max {
        return Some(one_line);
    }
    let mut out = String::new();
    for word in words {
        let next = out.chars().count() + usize::from(!out.is_empty()) + word.chars().count();
        if next > max.saturating_sub(1) {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() {
        // One long word: cut it.
        out = one_line.chars().take(max.saturating_sub(1)).collect();
    }
    out.push('…');
    Some(out)
}

fn says(kind: Kind) -> &'static str {
    match kind {
        Kind::Waiting => "waits for your reply",
        Kind::Paused => "paused on a usage limit",
        Kind::Failed => "failed",
    }
}

/// The notification for `events` (oldest first): one session's own, or a
/// digest of several that closes theirs. `None` for no events.
pub fn compose(events: &[Event]) -> Option<PushMessage> {
    let last = events.last()?;
    let [only] = events else {
        return Some(digest(events, last.at));
    };
    let detail = only
        .detail
        .as_deref()
        .and_then(|d| excerpt(d, EXCERPT_CHARS));
    let body = match (only.kind, detail) {
        (Kind::Waiting, Some(words)) => format!("{} waits for your reply\n{words}", only.reference),
        (Kind::Waiting, None) => format!("{} waits for your reply", only.reference),
        (Kind::Paused, _) => format!(
            "{} paused on a Claude usage limit and continues when it resets",
            only.reference
        ),
        (Kind::Failed, Some(why)) => format!("{} failed: {why}", only.reference),
        (Kind::Failed, None) => format!("{} failed", only.reference),
    };
    Some(PushMessage {
        title: only.title.clone(),
        body,
        tag: only.session_id.0.clone(),
        url: format!("/s/{}", only.session_id),
        replaces: Vec::new(),
        at: only.at,
    })
}

fn digest(events: &[Event], at: Millis) -> PushMessage {
    let n = events.len();
    let kind = events.first().map(|e| e.kind);
    let same = events.iter().all(|e| Some(e.kind) == kind);
    let title = match kind {
        Some(Kind::Waiting) if same => format!("{n} sessions wait for your reply"),
        Some(Kind::Paused) if same => format!("{n} sessions paused on a usage limit"),
        Some(Kind::Failed) if same => format!("{n} sessions failed"),
        _ => format!("{n} sessions need you"),
    };
    let mut lines: Vec<String> = events
        .iter()
        .take(DIGEST_LINES)
        .map(|e| {
            if same {
                format!("{} · {}", e.reference, e.title)
            } else {
                format!("{} {} · {}", e.reference, says(e.kind), e.title)
            }
        })
        .collect();
    if n > DIGEST_LINES {
        lines.push(format!("and {} more", n - DIGEST_LINES));
    }
    PushMessage {
        title,
        body: lines.join("\n"),
        tag: "digest".into(),
        url: "/".into(),
        replaces: events
            .iter()
            .rev()
            .take(DIGEST_REPLACES)
            .map(|e| e.session_id.0.clone())
            .collect(),
        at,
    }
}

/// The notification `POST /me/notifications/test` sends.
pub fn test_message(now: Millis) -> PushMessage {
    PushMessage {
        title: "Notifications work".into(),
        body: "This browser shows a notification when a session you follow waits for your reply, pauses on a usage limit or fails.".into(),
        tag: "test".into(),
        url: "/settings".into(),
        replaces: Vec::new(),
        at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SessionStatus::*;

    fn event(n: u32, kind: Kind, detail: Option<&str>) -> Event {
        Event {
            session_id: SessionId(format!("ses_{n}")),
            reference: format!("my-app#{n}"),
            title: format!("Task {n}"),
            kind,
            detail: detail.map(str::to_owned),
            at: i64::from(n) * 1000,
        }
    }

    #[test]
    fn finished_turns_pauses_and_failures_notify() {
        assert_eq!(Kind::of(Running, Idle), Some(Kind::Waiting));
        assert_eq!(Kind::of(Running, Paused), Some(Kind::Paused));
        assert_eq!(Kind::of(Queued, Failed), Some(Kind::Failed));
        assert_eq!(Kind::of(Idle, Failed), Some(Kind::Failed));
        // A resumed session that has nothing to do yet, or one that is
        // already where it was, does not.
        assert_eq!(Kind::of(Preparing, Idle), None);
        assert_eq!(Kind::of(Paused, Paused), None);
        assert_eq!(Kind::of(Failed, Failed), None);
        assert_eq!(Kind::of(Idle, Running), None);
        assert_eq!(Kind::of(Running, Stopped), None);
        assert_eq!(Kind::of(Paused, Running), None);

        assert!(Kind::Waiting.still(Idle));
        assert!(!Kind::Waiting.still(Running));
        assert!(Kind::Failed.still(Failed));
        assert!(!Kind::Paused.still(Queued));
    }

    #[test]
    fn followers_who_can_see_it_and_want_it() {
        let follower = |id: &str, project_role, settings| Follower {
            user_id: id.into(),
            instance_role: InstanceRole::Member,
            project_role,
            share_role: None,
            is_creator: false,
            settings,
        };
        let quiet = NotificationSettings {
            waiting: false,
            ..Default::default()
        };
        let followers = [
            follower("usr_ada", Some(ProjectRole::Editor), Default::default()),
            follower("usr_bob", Some(ProjectRole::Viewer), quiet),
            // Left the project: no access, no notifications.
            follower("usr_cy", None, Default::default()),
            follower("usr_dee", Some(ProjectRole::Owner), Default::default()),
        ];
        let watching = vec!["usr_dee".to_owned()];
        assert_eq!(
            recipients(&followers, Kind::Waiting, &watching),
            ["usr_ada"]
        );
        assert_eq!(
            recipients(&followers, Kind::Failed, &[]),
            ["usr_ada", "usr_bob", "usr_dee"]
        );
    }

    #[test]
    fn bursts_gather_and_go_out_a_minute_apart() {
        assert_eq!(due(100_000, None), 110_000);
        assert_eq!(due(100_000, Some(20_000)), 110_000);
        assert_eq!(due(100_000, Some(80_000)), 140_000);
    }

    #[test]
    fn excerpts_are_one_plain_line() {
        let text = "## Done\n\nI fixed the **race** in `redirect()` and opened #45.\n\n- tests pass\n- docs updated";
        assert_eq!(
            excerpt(text, 200).as_deref(),
            Some("Done I fixed the race in redirect() and opened #45. tests pass docs updated")
        );
        assert_eq!(
            excerpt("one two three four", 12).as_deref(),
            Some("one two…")
        );
        assert_eq!(excerpt("abcdefghijkl", 6).as_deref(), Some("abcde…"));
        assert_eq!(excerpt(" \n ", 10), None);
        let cut = excerpt(&"word ".repeat(100), EXCERPT_CHARS).unwrap_or_default();
        assert!(cut.chars().count() <= EXCERPT_CHARS);
        assert!(cut.ends_with("word…"));
    }

    #[test]
    fn one_session_says_what_happened() {
        let waiting = compose(&[event(12, Kind::Waiting, Some("Shall I add tests?"))]);
        assert_eq!(
            waiting,
            Some(PushMessage {
                title: "Task 12".into(),
                body: "my-app#12 waits for your reply\nShall I add tests?".into(),
                tag: "ses_12".into(),
                url: "/s/ses_12".into(),
                replaces: vec![],
                at: 12_000,
            })
        );
        let body = |e: Event| compose(&[e]).map(|m| m.body).unwrap_or_default();
        assert_eq!(
            body(event(
                3,
                Kind::Failed,
                Some("The runner declined the session")
            )),
            "my-app#3 failed: The runner declined the session"
        );
        assert_eq!(
            body(event(
                4,
                Kind::Paused,
                Some("Waiting for the usage limit to reset")
            )),
            "my-app#4 paused on a Claude usage limit and continues when it resets"
        );
        assert_eq!(compose(&[]), None);
    }

    #[test]
    fn several_become_a_digest() {
        let same: Vec<Event> = (1..=6).map(|n| event(n, Kind::Failed, None)).collect();
        let m = compose(&same).unwrap_or_else(|| test_message(0));
        assert_eq!(m.title, "6 sessions failed");
        assert_eq!(
            m.body,
            "my-app#1 · Task 1\nmy-app#2 · Task 2\nmy-app#3 · Task 3\nmy-app#4 · Task 4\nand 2 more"
        );
        assert_eq!(m.tag, "digest");
        assert_eq!(m.url, "/");
        assert_eq!(m.replaces.len(), 6);
        assert_eq!(m.at, 6000);

        let mixed = [
            event(1, Kind::Waiting, Some("Done")),
            event(2, Kind::Failed, None),
        ];
        let m = compose(&mixed).unwrap_or_else(|| test_message(0));
        assert_eq!(m.title, "2 sessions need you");
        assert_eq!(
            m.body,
            "my-app#1 waits for your reply · Task 1\nmy-app#2 failed · Task 2"
        );
        assert_eq!(m.replaces, ["ses_2", "ses_1"]);
    }

    #[test]
    fn messages_fit_a_push() -> Result<(), serde_json::Error> {
        // The longest titles and project slugs, real ids, and a failure
        // that explains itself at length.
        let title = "Ü".repeat(80);
        let slug = "x".repeat(48);
        let reason = "Something went wrong ".repeat(100);
        let events: Vec<Event> = (1..=500)
            .map(|n| Event {
                session_id: SessionId(format!("ses_01JABCDEFGHJKMNPQRST{n:06}")),
                reference: format!("{slug}#{n}"),
                title: title.clone(),
                ..event(n, Kind::Failed, Some(&reason))
            })
            .collect();
        let single = compose(events.get(..1).unwrap_or_default()).map(|m| m.body);
        assert!(single.is_some_and(|b| b.chars().count() < 250));
        for n in [1, 2, 5, 500] {
            let m = compose(events.get(..n).unwrap_or_default()).unwrap_or_else(|| test_message(0));
            assert!(serde_json::to_vec(&m)?.len() < crate::webpush::MAX_PLAINTEXT / 2);
        }
        Ok(())
    }
}
