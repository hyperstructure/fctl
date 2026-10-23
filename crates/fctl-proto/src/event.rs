//! The session event model: everything that happens in a session, in order.
//!
//! Events are stored by SessionDO with a canonical `seq` (1, 2, 3, …) and
//! streamed to viewers. Claude's own stream-json messages are carried verbatim
//! in [`EventBody::Claude`] so no detail is lost and new message types work
//! without a protocol change; streaming deltas (`stream_event`) are ephemeral
//! and travel separately (see `viewer::ViewerServer::Stream`).

use crate::session::{
    Memory, PullRequestRef, RemoteControlState, RepoAccess, RepoRef, Retrospective, SessionStatus,
    UserRef,
};
use crate::{AttachmentId, DeliveryId, Millis};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// A stored event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionEvent {
    pub seq: u64,
    pub ts: Millis,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub body: EventBody,
}

/// What happened. Serialized as `{"kind": "...", "data": {...}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum EventBody {
    /// A message from Claude Code's stream-json output (`assistant`, `user`
    /// tool results, `system`, `result`, `rate_limit_event`, …), verbatim.
    Claude(
        #[cfg_attr(
            feature = "ts",
            ts(type = "import('../lib/claude-types').ClaudeMessage")
        )]
        serde_json::Value,
    ),
    /// A person (or automation) wrote to the agent.
    UserMessage(UserMessage),
    /// The session changed state.
    Status(StatusChange),
    /// Output from provisioning, the setup script or the supervisor.
    Log(LogLine),
    /// The repositories' state: while Claude works, when it changed (at most
    /// every ten seconds), after each turn, and when the session stops.
    Git(GitState),
    /// Remote Control was enabled or became unavailable.
    RemoteControl(RemoteControlState),
    /// Something people should notice: a pause for a usage limit, a blocked
    /// overage, an expired credential, a lost runner.
    Notice(Notice),
    /// Someone interrupted the agent's turn.
    Interrupt(Interrupt),
}

impl EventBody {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Claude(_) => "claude",
            Self::UserMessage(_) => "user_message",
            Self::Status(_) => "status",
            Self::Log(_) => "log",
            Self::Git(_) => "git",
            Self::RemoteControl(_) => "remote_control",
            Self::Notice(_) => "notice",
            Self::Interrupt(_) => "interrupt",
        }
    }

    /// The `type` of a Claude message (`assistant`, `result`, …).
    pub fn claude_type(&self) -> Option<&str> {
        match self {
            Self::Claude(v) => v.get("type").and_then(|t| t.as_str()),
            _ => None,
        }
    }
}

/// Where a message to the agent came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum MessageSource {
    Web,
    Cli,
    Api,
    /// Typed in the Claude apps through Remote Control.
    RemoteControl,
    /// Written by fctl itself (for example "continue" after a usage-limit pause).
    System,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UserMessage {
    pub id: DeliveryId,
    /// Absent for system messages.
    pub author: Option<UserRef>,
    /// Empty when the message is only attachments.
    pub text: String,
    pub source: MessageSource,
    /// Files sent with the message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<Attachment>>", optional))]
    pub attachments: Vec<Attachment>,
    /// For a message from fctl that brought the agent back to a pull
    /// request its session opened: what happened there, which the message
    /// tells the agent. Clients from before it show the message's text.
    pub follow_up: Option<FollowUp>,
}

/// What happened on GitHub to a pull request a session's agent opened,
/// which fctl passed on to the agent (`fctl_domain::follow_ups`), for the
/// apps to show as GitHub has it: the message's text says the same to the
/// agent, with how to act on it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct FollowUp {
    pub repo: RepoRef,
    pub number: u64,
    /// The pull request on GitHub.
    pub url: String,
    pub title: Option<String>,
    /// Its state as fctl passed this on: `open`, `draft`, `queued`,
    /// `merged` or `closed`.
    pub state: Option<String>,
    /// The branch it goes into.
    pub base: Option<String>,
    /// What happened, in the order GitHub said it.
    pub events: Vec<FollowUpEvent>,
}

/// One thing that happened to a pull request ([`FollowUp`]).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FollowUpEvent {
    /// A person who can write to the repository wrote to the agent there.
    Comment(FollowUpComment),
    /// Checks of the pull request's newest commit failed.
    ChecksFailed {
        /// The commit, in full.
        sha: String,
        /// The checks that failed, by name.
        names: Vec<String>,
        /// Its checks then, counted as the pull request shows them.
        passed: u32,
        failed: u32,
        pending: u32,
    },
    /// It was merged.
    Merged {
        /// Who merged it: their GitHub login.
        #[cfg_attr(feature = "ts", ts(optional))]
        by: Option<String>,
        /// The branch it went into.
        #[cfg_attr(feature = "ts", ts(optional))]
        base: Option<String>,
    },
    /// It was closed without being merged.
    Closed {
        #[cfg_attr(feature = "ts", ts(optional))]
        by: Option<String>,
    },
    /// Something a newer control plane passes on, which this version
    /// doesn't know.
    #[serde(other)]
    Other,
}

/// What a person wrote to the agent on a pull request: a comment on it, a
/// review's text, or a comment on its lines (one with `path`), which may
/// have come with a review (`review_id`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct FollowUpComment {
    /// GitHub's id of the comment, or of the review.
    pub id: u64,
    /// Their GitHub login, and avatar.
    pub login: String,
    pub avatar_url: Option<String>,
    /// The comment on GitHub.
    pub url: String,
    /// What they wrote, cut short past a few thousand characters.
    pub body: String,
    /// For a review: `approved`, `changes_requested` or `commented`.
    pub review: Option<String>,
    /// For a comment on lines: the file, and its lines in the new version
    /// (`start_line` to `line`), or the old one (`deleted`).
    pub path: Option<String>,
    pub line: Option<u32>,
    pub start_line: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub deleted: bool,
    /// The review it came with, and the comment it answers in its thread.
    pub review_id: Option<u64>,
    pub in_reply_to: Option<u64>,
    pub at: Option<Millis>,
}

/// A file someone attached to a message. The control plane keeps its bytes
/// (`GET /api/v1/sessions/{id}/attachments/{attachment}`); fctl-agent saves it
/// in the sandbox and tells Claude where.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Attachment {
    pub id: AttachmentId,
    /// The file's name, without directories.
    pub name: String,
    /// For example `image/png`; `application/octet-stream` when unknown.
    pub media_type: String,
    /// Bytes.
    pub size: u64,
    /// Pixel size of PNG, JPEG, GIF and WebP images.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct StatusChange {
    pub status: SessionStatus,
    pub reason: Option<String>,
    pub resume_at: Option<Millis>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LogSource {
    /// The host: pulling images, creating the sandbox.
    Host,
    /// The project's setup script.
    Setup,
    /// fctl-agent: cloning, configuring Claude.
    Agent,
    /// Claude Code's stderr.
    Claude,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LogLine {
    pub source: LogSource,
    pub level: LogLevel,
    pub text: String,
}

/// One changed file, as `git diff --numstat` reports it since the branch
/// left its base.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct FileChange {
    pub path: String,
    /// `added`, `modified`, `deleted`, `renamed` or `untracked`.
    pub status: String,
    /// Absent for binary files.
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CommitInfo {
    pub sha: String,
    pub subject: String,
}

/// Repository state of a session's workspace. The fields describe the
/// primary repository; `repos` the others the agent cloned.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct GitState {
    pub branch: Option<String>,
    pub base: Option<String>,
    pub head: Option<String>,
    /// Commits on the branch that are not on the base.
    pub ahead: u32,
    /// Commits on the base that are not on the branch.
    pub behind: u32,
    /// What the branch changed since it left the base (since their merge
    /// base), uncommitted and untracked work included. Older agents
    /// compared the work with the base's tip.
    pub files: Vec<FileChange>,
    pub uncommitted: u32,
    pub last_commit: Option<CommitInfo>,
    /// Whether the branch's latest commit is on the remote.
    pub pushed: bool,
    /// The pull request of the branch, when one exists.
    pub pull_request: Option<PullRequestRef>,
    /// Once the branch's pull request is merged: the commits on the branch
    /// after the head it merged, which the base doesn't have. fctl-agent
    /// doesn't push a merged branch, so they stay in the sandbox unless the
    /// agent pushes them. Absent before, and from older agents.
    pub unmerged: Option<u32>,
    /// The project's other repositories the agent cloned. Absent from agents
    /// that work in the primary repository only.
    pub repos: Option<Vec<RepoGitState>>,
    /// Every pull request fctl-agent follows for the session, the branches'
    /// above included, in the order it found them: those of the branches
    /// the repositories had checked out, and those `gh pr create` printed,
    /// whichever branch, worktree or clone they came from, with their state
    /// as last looked up. Absent from older agents.
    pub pull_requests: Option<Vec<PullRequestRef>>,
}

impl GitState {
    /// Every changed file, in the primary repository and the others.
    pub fn all_files(&self) -> impl Iterator<Item = &FileChange> {
        self.files
            .iter()
            .chain(self.repos.iter().flatten().flat_map(|r| r.files.iter()))
    }
}

/// Another repository's state (see [`GitState::repos`]): the same facts,
/// relative to its default branch.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct RepoGitState {
    pub owner: String,
    pub name: String,
    pub access: RepoAccess,
    /// Where it is cloned in the sandbox.
    pub path: Option<String>,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub head: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileChange>,
    pub uncommitted: u32,
    pub last_commit: Option<CommitInfo>,
    pub pushed: bool,
    pub pull_request: Option<PullRequestRef>,
    pub unmerged: Option<u32>,
}

impl RepoGitState {
    pub fn new(
        repo: &crate::session::RepoRef,
        access: RepoAccess,
        path: Option<String>,
        st: GitState,
    ) -> Self {
        Self {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            access,
            path,
            branch: st.branch,
            base: st.base,
            head: st.head,
            ahead: st.ahead,
            behind: st.behind,
            files: st.files,
            uncommitted: st.uncommitted,
            last_commit: st.last_commit,
            pushed: st.pushed,
            pull_request: st.pull_request,
            unmerged: st.unmerged,
        }
    }

    /// `owner/name`.
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warn,
    Error,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Notice {
    pub level: NoticeLevel,
    /// Stable identifier, for example `usage_limit`, `overage_blocked`,
    /// `api_key_refused`, `credential_expired`, `runner_lost`,
    /// `screenshot_published`, `memory_added`, `memory_stale`,
    /// `retrospective`, `review_posted`, `review_held`, `follow_up_held`.
    pub code: String,
    pub message: String,
    /// When the condition clears (usage-limit resets).
    pub until: Option<Millis>,
    /// The screenshot a `screenshot_published` notice is about. Clients
    /// from before it show the message, which holds the link.
    pub published: Option<PublishedFile>,
    /// The note a `memory_added` or `memory_stale` notice is about, as it
    /// was then. Clients from before it show the message.
    pub memory: Option<Memory>,
    /// The retrospective a `retrospective` notice holds. Clients from
    /// before it show the message.
    pub retrospective: Option<Retrospective>,
    /// What fctl posted for a review agent, in a `review_posted` notice.
    /// Clients from before it show the message.
    pub review: Option<crate::api::PostedReview>,
    /// What happened to a pull request that fctl didn't pass on to the
    /// agent, in a `follow_up_held` notice. Clients from before it show the
    /// message.
    pub follow_up: Option<FollowUp>,
}

/// A screenshot an agent published on the control plane (`fctl publish`):
/// anyone who has the link can open it, until it expires.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PublishedFile {
    /// `https://<instance>/api/v1/published/<token>/<name>`: the token, 256
    /// random bits, is all it takes to open it.
    pub url: String,
    /// The file's name, without directories.
    pub name: String,
    /// `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
    pub media_type: String,
    /// Bytes.
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// When the link stops working and the file is deleted.
    pub expires_at: Millis,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Interrupt {
    pub id: DeliveryId,
    pub by: Option<UserRef>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UserId;

    #[test]
    fn event_wire_format() -> Result<(), serde_json::Error> {
        let ev = SessionEvent {
            seq: 7,
            ts: 1_700_000_000_000,
            body: EventBody::UserMessage(UserMessage {
                id: DeliveryId("dlv_1".into()),
                author: Some(UserRef {
                    id: UserId("usr_1".into()),
                    name: "Ada".into(),
                    avatar_url: None,
                }),
                text: "hi".into(),
                source: MessageSource::Web,
                attachments: vec![],
                follow_up: None,
            }),
        };
        let json = serde_json::to_value(&ev)?;
        assert_eq!(json["seq"], 7);
        assert_eq!(json["kind"], "user_message");
        assert_eq!(json["data"]["text"], "hi");
        assert!(json["data"]["author"].get("avatar_url").is_none());
        let back: SessionEvent = serde_json::from_value(json)?;
        assert_eq!(back, ev);
        Ok(())
    }

    #[test]
    fn git_state_of_one_repository_keeps_its_shape() -> Result<(), serde_json::Error> {
        // What agents that know one repository send, and older clients read.
        let old = serde_json::json!({
            "branch": "fctl/1", "base": "main", "ahead": 1, "behind": 0,
            "files": [{ "path": "a.rs", "status": "added", "additions": 3, "deletions": 0 }],
            "uncommitted": 0, "pushed": true
        });
        let st: GitState = serde_json::from_value(old.clone())?;
        assert_eq!(st.repos, None);
        assert_eq!(serde_json::to_value(&st)?, old);
        let mut st = st;
        st.repos = Some(vec![RepoGitState {
            owner: "acme".into(),
            name: "lib".into(),
            access: RepoAccess::Write,
            files: vec![FileChange {
                path: "b.rs".into(),
                status: "modified".into(),
                additions: Some(1),
                deletions: Some(2),
            }],
            pull_request: Some(PullRequestRef {
                number: 7,
                url: "https://github.com/acme/lib/pull/7".into(),
                ..Default::default()
            }),
            ..Default::default()
        }]);
        assert_eq!(st.all_files().count(), 2);
        let json = serde_json::to_value(&st)?;
        assert_eq!(json["repos"][0]["access"], "write");
        assert_eq!(serde_json::from_value::<GitState>(json)?, st);
        Ok(())
    }

    #[test]
    fn notices_carry_a_published_screenshot_as_an_option() -> Result<(), serde_json::Error> {
        let old: Notice = serde_json::from_value(serde_json::json!({
            "level": "warn", "code": "usage_limit", "message": "Paused", "until": 5
        }))?;
        assert_eq!(old.published, None);
        assert!(serde_json::to_value(&old)?.get("published").is_none());
        let shot = Notice {
            level: NoticeLevel::Info,
            code: "screenshot_published".into(),
            message: "Published after.png".into(),
            until: None,
            published: Some(PublishedFile {
                url: "https://fctl.test/api/v1/published/abc/after.png".into(),
                name: "after.png".into(),
                media_type: "image/png".into(),
                size: 1234,
                width: Some(1440),
                height: Some(900),
                expires_at: 1_800_000_000_000,
            }),
            memory: None,
            retrospective: None,
            review: None,
            follow_up: None,
        };
        let json = serde_json::to_value(EventBody::Notice(shot.clone()))?;
        assert_eq!(json["kind"], "notice");
        assert_eq!(json["data"]["published"]["width"], 1440);
        assert!(json["data"].get("until").is_none());
        assert!(json["data"].get("memory").is_none());
        assert_eq!(
            serde_json::from_value::<EventBody>(json)?,
            EventBody::Notice(shot)
        );
        Ok(())
    }

    #[test]
    fn notices_carry_a_memory_or_a_retrospective() -> Result<(), serde_json::Error> {
        let remembered: Notice = serde_json::from_value(serde_json::json!({
            "level": "info", "code": "memory_added", "message": "Remembered: Run npm ci first",
            "memory": {
                "id": "mem_1", "project_id": "prj_1", "text": "Run npm ci first",
                "created_at": 5
            }
        }))?;
        assert_eq!(
            remembered.memory.map(|m| m.text).as_deref(),
            Some("Run npm ci first")
        );
        assert_eq!(remembered.retrospective, None);
        let retro: Notice = serde_json::from_value(serde_json::json!({
            "level": "info", "code": "retrospective", "message": "Recorded a retrospective",
            "retrospective": {
                "id": "rtr_1", "project_id": "prj_1",
                "session": { "id": "ses_1", "number": 1, "title": "Build it" },
                "created_at": 5, "outcome": "completed", "worked_on": "Built it."
            }
        }))?;
        assert_eq!(
            retro.retrospective.map(|r| r.worked_on).as_deref(),
            Some("Built it.")
        );
        Ok(())
    }

    #[test]
    fn follow_ups_say_what_happened_as_github_has_it() -> Result<(), serde_json::Error> {
        let fu = FollowUp {
            repo: RepoRef {
                owner: "acme".into(),
                name: "web".into(),
            },
            number: 12,
            url: "https://github.com/acme/web/pull/12".into(),
            title: Some("Fix the login redirect".into()),
            state: Some("open".into()),
            base: Some("main".into()),
            events: vec![
                FollowUpEvent::Comment(FollowUpComment {
                    id: 456,
                    login: "ada".into(),
                    avatar_url: None,
                    url: "https://github.com/acme/web/pull/12#discussion_r456".into(),
                    body: "@fctl keep the query string".into(),
                    review: None,
                    path: Some("src/login.rs".into()),
                    line: Some(42),
                    start_line: None,
                    deleted: false,
                    review_id: Some(789),
                    in_reply_to: None,
                    at: Some(5),
                }),
                FollowUpEvent::ChecksFailed {
                    sha: "3f2a1c9".into(),
                    names: vec!["Rust tests".into()],
                    passed: 7,
                    failed: 1,
                    pending: 1,
                },
                FollowUpEvent::Merged {
                    by: Some("ada".into()),
                    base: None,
                },
            ],
        };
        let json = serde_json::to_value(&fu)?;
        assert_eq!(json["events"][0]["kind"], "comment");
        assert_eq!(json["events"][0]["path"], "src/login.rs");
        assert!(json["events"][0].get("deleted").is_none());
        assert!(json["events"][0].get("review").is_none());
        assert_eq!(json["events"][1]["kind"], "checks_failed");
        assert_eq!(json["events"][1]["names"][0], "Rust tests");
        assert_eq!(
            json["events"][2],
            serde_json::json!({ "kind": "merged", "by": "ada" })
        );
        let back: FollowUp = serde_json::from_value(json)?;
        assert_eq!(back, fu);
        // What a newer control plane passes on, which this version reads
        // as something else rather than failing the whole event.
        let newer: FollowUp = serde_json::from_value(serde_json::json!({
            "repo": { "owner": "acme", "name": "web" }, "number": 12,
            "url": "https://github.com/acme/web/pull/12",
            "events": [{ "kind": "conflicts", "base": "main" }, { "kind": "closed" }]
        }))?;
        assert_eq!(
            newer.events,
            [FollowUpEvent::Other, FollowUpEvent::Closed { by: None }]
        );
        // Messages without one, as before.
        let m: UserMessage = serde_json::from_value(serde_json::json!({
            "id": "dlv_1", "text": "hi", "source": "system"
        }))?;
        assert_eq!(m.follow_up, None);
        Ok(())
    }

    #[test]
    fn claude_messages_are_verbatim() -> Result<(), serde_json::Error> {
        let raw = serde_json::json!({"type": "assistant", "message": {"content": []}, "extra": 1});
        let ev: SessionEvent = serde_json::from_value(
            serde_json::json!({"seq": 1, "ts": 2, "kind": "claude", "data": raw}),
        )?;
        assert_eq!(ev.body.claude_type(), Some("assistant"));
        let EventBody::Claude(v) = &ev.body else {
            panic!("expected a claude event");
        };
        assert_eq!(v["extra"], 1);
        Ok(())
    }
}
