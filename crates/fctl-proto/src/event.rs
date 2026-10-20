//! The session event model: everything that happens in a session, in order.
//!
//! Events are stored by SessionDO with a canonical `seq` (1, 2, 3, …) and
//! streamed to viewers. Claude's own stream-json messages are carried verbatim
//! in [`EventBody::Claude`] so no detail is lost and new message types work
//! without a protocol change; streaming deltas (`stream_event`) are ephemeral
//! and travel separately (see `viewer::ViewerServer::Stream`).

use crate::session::{PullRequestRef, RemoteControlState, RepoAccess, SessionStatus, UserRef};
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
    /// The repository changed (after each turn).
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

/// One changed file, as `git diff --numstat` reports it against the base.
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
    /// Changed files relative to the base, including uncommitted work.
    pub files: Vec<FileChange>,
    pub uncommitted: u32,
    pub last_commit: Option<CommitInfo>,
    /// Whether the branch's latest commit is on the remote.
    pub pushed: bool,
    /// The pull request of the branch, when one exists.
    pub pull_request: Option<PullRequestRef>,
    /// The project's other repositories the agent cloned. Absent from agents
    /// that work in the primary repository only.
    pub repos: Option<Vec<RepoGitState>>,
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
    /// `screenshot_published`.
    pub code: String,
    pub message: String,
    /// When the condition clears (usage-limit resets).
    pub until: Option<Millis>,
    /// The screenshot a `screenshot_published` notice is about. Clients
    /// from before it show the message, which holds the link.
    pub published: Option<PublishedFile>,
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
                title: None,
                state: None,
                repo: None,
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
        };
        let json = serde_json::to_value(EventBody::Notice(shot.clone()))?;
        assert_eq!(json["kind"], "notice");
        assert_eq!(json["data"]["published"]["width"], 1440);
        assert!(json["data"].get("until").is_none());
        assert_eq!(
            serde_json::from_value::<EventBody>(json)?,
            EventBody::Notice(shot)
        );
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
