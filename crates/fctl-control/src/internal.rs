//! The Worker ⇄ Durable Object contract: internal requests and their bodies.
//!
//! Durable Objects are reachable only through stubs, so these requests are
//! trusted; the Worker never forwards client headers to them (it builds new
//! requests), and identity travels in `X-Fctl-*` headers it sets itself.

use crate::error::{AppError, AppResult, internal};
use fctl_proto::api::PlanLimit;
use fctl_proto::event::{Attachment, UserMessage};
use fctl_proto::runner::StopReason;
use fctl_proto::session::{RunnerRef, Session, SessionRole, SessionStatus, UserRef};
use fctl_proto::uplink::Delivery;
use fctl_proto::{
    ApiError, AttachmentId, CredentialId, DeliveryId, LoginJobId, Millis, ProjectId, SessionId,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use worker::{Env, Headers, Method, Request, RequestInit, Response, Stub};

pub const HDR_RUNNER: &str = "X-Fctl-Runner";
/// The SHA-256 of the runner token an uplink opened with, which SessionDO
/// checks again while it stays open.
pub const HDR_RUNNER_TOKEN: &str = "X-Fctl-Runner-Token";
pub const HDR_VIEWER: &str = "X-Fctl-Viewer";

// ───────────────────────────── SessionObject ─────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct InitSession {
    pub session: Session,
    pub prompt: UserMessage,
}

#[derive(Serialize, Deserialize)]
pub struct Deliver {
    pub delivery: Delivery,
    /// Idempotency key from the client; repeats return the first delivery.
    #[serde(default)]
    pub client_key: Option<String>,
    /// Stored for the transcript (the user message or interrupt).
    pub event: fctl_proto::event::EventBody,
    /// An interrupt from the session's autopilot, which `event` says its
    /// owner made, as whom it acts; a message from it says so itself
    /// (`UserMessage::autopilot`). For what its autopilot hears of why
    /// the turn ended (`fctl_domain::autopilot::cut_into`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub by_autopilot: bool,
}

#[derive(Serialize, Deserialize)]
pub struct Delivered {
    pub delivery_id: DeliveryId,
}

/// Someone changed the model or the effort level the session's Claude works
/// at (`UpdateSessionRequest`): the run under way switches, if there is one
/// and its host can, and the transcript says so.
#[derive(Serialize, Deserialize)]
pub struct SessionSettings {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    pub by: UserRef,
}

/// A status change decided by the control plane (queue, stop, failure).
#[derive(Serialize, Deserialize, Default)]
pub struct ControlState {
    pub status: Option<SessionStatus>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub runner: Option<RunnerRef>,
    #[serde(default)]
    pub clear_runner: bool,
    #[serde(default)]
    pub title: Option<String>,
    /// A new description; empty clears it.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
    /// Said in the transcript, after the status.
    #[serde(default)]
    pub notice: Option<fctl_proto::event::Notice>,
    /// On whose Claude subscription the session runs from now on: a resume
    /// runs it on the resumer's. Its Remote Control links are theirs.
    #[serde(default)]
    pub claude_user: Option<UserRef>,
    /// How short of disk the session's runner is now, from FleetObject
    /// (`fctl_domain::disk::heard`). A new runner clears what the last one
    /// said.
    #[serde(default)]
    pub disk: Option<DiskNews>,
    /// Someone stopped the session, or deleted it: its agent's question
    /// goes, as nobody will answer it.
    #[serde(default)]
    pub drop_question: bool,
    /// Nothing the session did: when it last changed (`updated_at`) stays
    /// as it was, as `api::sessions::move_to` leaves it in D1 for a move to
    /// another Claude connection that leaves the session as it is. Its
    /// autopilot's wakes take its sessions that changed for news.
    #[serde(default)]
    pub keep_updated_at: bool,
}

/// The question a session's agent asks people now (`/ask`, from `fctl ask`
/// through `crate::api::calls`): none takes back the one it asked.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ask {
    #[serde(default)]
    pub question: Option<fctl_proto::session::SessionQuestion>,
}

/// What the session's page shows of its runner's disk from now on.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiskNews {
    /// Nothing while it has room.
    #[serde(default)]
    pub disk: Option<fctl_proto::runner::RunnerDisk>,
}

/// fctl's review agent posted findings on a pull request the session's
/// agent opened (`/review-findings`, from `crate::reviews`): its agent gets
/// them in a message from fctl, unless it got those of `rounds` reviews in
/// a row while nobody messaged it, when the transcript says so instead.
#[derive(Serialize, Deserialize)]
pub struct ReviewFindings {
    /// The review's session: a review's findings reach a session once.
    pub review: SessionId,
    /// The message for the agent.
    pub text: String,
    /// What the transcript says when fctl keeps them from the agent.
    pub held: String,
    /// What the transcript shows of the message.
    #[serde(default)]
    pub shown: Option<fctl_proto::event::PassedOnReview>,
    /// The reviews in a row whose findings the agent gets while nobody
    /// messages it, as the review's project's review agents pass them on
    /// (`fctl_domain::reviews::ROUNDS`).
    #[serde(default = "default_rounds")]
    pub rounds: u32,
}

/// [`ReviewFindings::rounds`] and [`PassOn::rounds`] from a Worker before
/// them.
fn default_rounds() -> u32 {
    fctl_domain::reviews::DEFAULT_ROUNDS
}

/// Someone passes on the findings of a review that fctl held back from the
/// session's agent, or that went to no session (`/pass-on`, from
/// `api::sessions::pass_on`): its agent gets them in the message from fctl
/// it would have got, and the reviews in a row count from this one, as
/// after a person's message.
#[derive(Serialize, Deserialize)]
pub struct PassOn {
    /// The review's session.
    pub review: SessionId,
    /// Who passes them on, whom the transcript names.
    pub by: UserRef,
    /// The reviews in a row whose findings the agent gets from now on,
    /// these included, as the review's project's review agents pass them
    /// on, which the transcript says.
    #[serde(default = "default_rounds")]
    pub rounds: u32,
    /// The findings, where they went to no session
    /// (`crate::reviews::pass_on_unsent`); `None`: those the session held
    /// back.
    #[serde(default)]
    pub unsent: Option<fctl_domain::reviews::Unsent>,
}

/// What came of [`ReviewFindings`].
#[derive(Serialize, Deserialize)]
pub struct FindingsTold {
    /// The message, once the session has it.
    #[serde(default)]
    pub delivery_id: Option<DeliveryId>,
    /// Why the agent doesn't get them.
    #[serde(default)]
    pub not_sent: Option<String>,
    /// The session keeps them back from its agent for a person to pass on,
    /// after the findings of [`ReviewFindings::rounds`] reviews in a row.
    #[serde(default)]
    pub held: bool,
}

/// What GitHub says of pull requests a session lists (`/pull-requests`,
/// from `crate::pull_requests`).
#[derive(Serialize, Deserialize)]
pub struct PullRequestsSeen {
    pub prs: Vec<fctl_proto::session::PullRequestRef>,
    /// What happened to those its agent opened, for the agent to hear of
    /// where its project's follow-ups say so (`crate::follow_ups`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub follow_ups: Vec<fctl_domain::follow_ups::Item>,
    /// Those its agent opened that GitHub didn't know yet whether they
    /// conflict with their base branch, where its project follows up on
    /// conflicts: the session asks again soon (`crate::pull_requests::ask_again`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asks: Vec<fctl_domain::follow_ups::Ask>,
}

/// What happened to pull requests the session's agent opened, to pass on
/// to it (`/follow-up`, from `crate::follow_ups`): the session gathers
/// them for `fctl_domain::follow_ups::GATHER_MS`, then sends its agent a
/// message from fctl, which resumes it if it stopped.
#[derive(Serialize, Deserialize)]
pub struct FollowUps {
    pub items: Vec<fctl_domain::follow_ups::Item>,
}

/// What came of [`FollowUps`].
#[derive(Serialize, Deserialize)]
pub struct FollowUpsTaken {
    /// How many it will pass on: none it passed on before.
    pub taken: usize,
    /// Why it passes none on.
    #[serde(default)]
    pub not_sent: Option<String>,
}

/// FleetObject asks a session on a draining runner whether it stops now: it
/// does when it has waited for a message for a while and none is on its way.
#[derive(Serialize, Deserialize)]
pub struct DrainStop {
    /// The runner, for the reason the session shows.
    pub runner_name: String,
}

/// Closes a runner's uplink to a session (`/close-uplink`): the runner was
/// disabled or its token replaced, or it no longer serves the session's
/// workspace.
#[derive(Serialize, Deserialize)]
pub struct CloseUplink {
    pub runner_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct DrainStopped {
    /// Whether the session is stopping now (its host should stop it).
    pub stopping: bool,
}

/// Who opens a viewer socket.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewerIdentity {
    pub user: UserRef,
    /// What agents call them, as of the socket's last check, which goes
    /// with the messages they send over it
    /// (`DeliveryKind::Message::author_called`).
    #[serde(default)]
    pub called: Option<String>,
    pub role: SessionRole,
    /// The token it was opened with, which SessionObject checks again,
    /// with the role, while the socket stays open.
    #[serde(default)]
    pub token_id: Option<String>,
}

// ───────────────────────────── FleetObject ─────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct Schedule {
    pub session_id: SessionId,
}

#[derive(Serialize, Deserialize)]
pub struct Stop {
    pub session_id: SessionId,
    pub reason: StopReason,
    pub destroy: bool,
}

/// Asks FleetDO for the sessions whose sandboxes a runner keeps: those a
/// resume starts there (`POST /homes`).
#[derive(Serialize, Deserialize)]
pub struct Homes {
    pub runner_id: String,
}

/// The sessions whose sandboxes a runner keeps, by id.
#[derive(Serialize, Deserialize, Default)]
pub struct HomeSessions {
    pub sessions: Vec<String>,
}

/// Asks FleetDO what a workspace's pools need now (`POST /demand`): all of
/// them, or those of the enrollment tokens `pools` names.
#[derive(Serialize, Deserialize)]
pub struct DemandOf {
    pub workspace_id: String,
    #[serde(default)]
    pub pools: Option<Vec<String>>,
}

/// A pool's labels, placement or name changed (`POST /pool-changed`): its
/// runners carry them at once, feeds show them, and sessions that wait
/// for its labels may start.
#[derive(Serialize, Deserialize)]
pub struct PoolChanged {
    pub token_id: String,
}

/// Asks FleetDO where a session's sandbox is, and whether a resume now
/// would leave it (`POST /sandbox`).
#[derive(Serialize, Deserialize)]
pub struct SandboxOf {
    pub session_id: String,
}

/// Where a session's sandbox is, as FleetDO has it.
#[derive(Serialize, Deserialize, Default)]
pub struct SandboxAt {
    /// The runner that keeps it: `None` when FleetDO knows of none (it
    /// was uninstalled or removed, or no longer runs the session).
    #[serde(default)]
    pub runner: Option<SandboxRunner>,
    /// A resume now would start the session on another runner, in a new
    /// sandbox without its conversation: its runner can't take it back
    /// (offline, its driver not ready, or it no longer runs the session)
    /// and another runner online would run it. Unasked while its runner
    /// drains.
    #[serde(default)]
    pub elsewhere: bool,
}

/// The runner that keeps a session's sandbox, as FleetDO has it now.
#[derive(Serialize, Deserialize)]
pub struct SandboxRunner {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub draining: bool,
}

/// Drains a runner, or stops draining it.
#[derive(Serialize, Deserialize)]
pub struct Drain {
    pub runner_id: String,
    pub runner_name: String,
    pub draining: bool,
    /// The admin who did it; absent when the runner's host did.
    #[serde(default)]
    pub by: Option<UserRef>,
    /// Also stop its sessions, and resume them elsewhere from then on: its
    /// host is being uninstalled.
    #[serde(default)]
    pub stop_sessions: bool,
}

#[derive(Serialize, Deserialize)]
pub struct NotifySession {
    pub session: Session,
}

#[derive(Serialize, Deserialize)]
pub struct Activity {
    pub session_id: SessionId,
    pub project_id: ProjectId,
    /// Absent for sessions from before workspaces.
    #[serde(default)]
    pub workspace_id: Option<String>,
    pub kind: String,
    pub weight: u32,
}

/// Who opens a feed socket, and what it shows them
/// (`crate::workspaces::feed_identity`).
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct FeedIdentity {
    pub user_id: String,
    /// An instance admin: sees every runner.
    pub admin: bool,
    /// The projects they could see when the feed opened, or their
    /// workspaces last changed.
    pub projects: Vec<String>,
    /// Workspaces where they see every project: they are an admin there,
    /// or it has a base role.
    #[serde(default)]
    pub sees_all: Vec<String>,
    /// The workspaces open to them.
    #[serde(default)]
    pub workspaces: Vec<String>,
    /// Of those, the ones they are an admin of: they see the runners that
    /// wait there for approval, and how each runner there enrolled.
    #[serde(default)]
    pub admin_of: Vec<String>,
    /// The token the feed was opened with, which FleetObject checks again,
    /// with what they see, while the socket stays open.
    #[serde(default)]
    pub token_id: Option<String>,
    /// When that was last checked.
    #[serde(default)]
    pub checked_at: Option<i64>,
}

impl FeedIdentity {
    /// Whether it shows the same as `other`.
    pub fn same_view(&self, other: &FeedIdentity) -> bool {
        self.user_id == other.user_id
            && self.admin == other.admin
            && self.projects == other.projects
            && self.sees_all == other.sees_all
            && self.workspaces == other.workspaces
            && self.admin_of == other.admin_of
    }

    /// Whether they see a session of `project_id` in `workspace_id`: they
    /// see its project. Starting it counts only while they may start
    /// sessions there (`fctl_domain::perm::session_role`), when they see
    /// the project anyway.
    pub fn sees(&self, project_id: &str, workspace_id: Option<&str>) -> bool {
        self.projects.iter().any(|p| p == project_id)
            || workspace_id.is_some_and(|w| self.sees_all.iter().any(|x| x == w))
    }
}

/// People whose workspaces changed: their feeds see what they now may
/// (`/workspaces-changed`).
#[derive(Serialize, Deserialize)]
pub struct WorkspacesChanged {
    pub user_ids: Vec<String>,
}

/// A runner's workspaces or owner changed: feeds hear about it, and queued
/// sessions may start on it (`/runner-changed`).
#[derive(Serialize, Deserialize)]
pub struct RunnerChanged {
    pub runner_id: String,
    /// An admin of its workspace approved it: its host hears it takes
    /// sessions now.
    #[serde(default)]
    pub approved: bool,
}

/// A runner was removed (`/runner-removed`): its connections close, the
/// sessions it ran stop, and every trace of it in FleetObject goes.
#[derive(Serialize, Deserialize)]
pub struct RunnerRemoved {
    pub runner_id: String,
    /// What the sessions it ran say, now stopped.
    pub reason: String,
}

/// What `/runner` connects: the runner, and, while it waits for approval,
/// for which workspace and until when.
#[derive(Serialize, Deserialize)]
pub struct RunnerIdent {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<fctl_proto::runner::PendingApproval>,
    /// Its enrollment token removes it once it has been offline this long.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_after_ms: Option<i64>,
    /// The SHA-256 of the runner token it connected with, which its
    /// heartbeats check again: once the runner is disabled or its token
    /// replaced, the connection closes, also when the call that says so
    /// failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_hash: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct LiveRunners {
    pub online: Vec<String>,
    pub last_seen: std::collections::BTreeMap<String, Millis>,
    /// What online runners had free at their last heartbeat.
    #[serde(default)]
    pub free: std::collections::BTreeMap<String, Free>,
    /// The runners that drain.
    #[serde(default)]
    pub drains: std::collections::BTreeMap<String, fctl_proto::api::RunnerDrain>,
}

/// Free memory and disk, in MiB.
#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub struct Free {
    pub memory_mb: Option<u64>,
    pub disk_mb: Option<u64>,
}

#[derive(Serialize, Deserialize)]
pub struct LoginStart {
    pub job_id: LoginJobId,
    pub user: UserRef,
    /// The runners that may run it, in the order to try them: the ones
    /// the person registered, then ones that run their sessions in their
    /// workspaces. Absent: any (older Workers).
    #[serde(default)]
    pub runners: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub struct LoginCode {
    pub job_id: LoginJobId,
    pub code: String,
}

/// A credential's plan usage changed: for its owner's feeds (`/usage`).
#[derive(Serialize, Deserialize)]
pub struct ClaudeUsage {
    pub user_id: String,
    pub credential_id: CredentialId,
    pub usage: Vec<PlanLimit>,
}

// ───────────────────────────── NotifyObject ─────────────────────────────

/// What happened to a session, for one of the people who follow it
/// (`/event`).
#[derive(Serialize, Deserialize)]
pub struct Notify {
    pub user_id: String,
    pub event: fctl_domain::notify::Event,
}

// ───────────────────────────── FilesObject ─────────────────────────────

/// A new upload, sent to `/store` in the `meta` query parameter; the bytes
/// are the request body.
#[derive(Serialize, Deserialize)]
pub struct StoreFile {
    pub id: AttachmentId,
    /// Already sanitized.
    pub name: String,
    /// The client's type or one guessed from the name; the object replaces it
    /// for images it recognizes.
    pub media_type: String,
    pub uploaded_by: String,
}

/// Uploads that a message in `session_id` sends: from now on they belong to
/// that session (again for the same session is fine).
#[derive(Serialize, Deserialize)]
pub struct ClaimFiles {
    pub session_id: SessionId,
    pub user_id: String,
    pub ids: Vec<AttachmentId>,
}

/// Files claimed for a session to hand back as unsent uploads.
#[derive(Serialize, Deserialize)]
pub struct UnclaimFiles {
    pub session_id: SessionId,
    pub ids: Vec<AttachmentId>,
}

#[derive(Serialize, Deserialize)]
pub struct ClaimedFiles {
    /// In the order of the ids.
    pub attachments: Vec<Attachment>,
}

/// A screenshot to publish, sent to `/publish` in the `meta` query
/// parameter; the bytes are the request body.
#[derive(Serialize, Deserialize)]
pub struct PublishFile {
    pub id: String,
    pub session_id: SessionId,
    /// Already sanitized.
    pub name: String,
    /// When the object deletes it.
    pub expires_at: Millis,
}

/// What `/publish` stored: an image, as its bytes say.
#[derive(Serialize, Deserialize)]
pub struct PublishedBytes {
    pub media_type: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
}

/// Deletes a published screenshot before it expires (`/unpublish`).
#[derive(Serialize, Deserialize)]
pub struct Unpublish {
    pub id: String,
}

// ───────────────────────────── SearchObject ─────────────────────────────

/// A session's searchable documents, new or changed (`/put`), from its
/// SessionObject. `about`, when present, are its title and description as
/// they are now, in place of those the index has.
#[derive(Serialize, Deserialize)]
pub struct PutDocs {
    pub session_id: SessionId,
    pub project_id: ProjectId,
    #[serde(default)]
    pub about: Option<Vec<fctl_domain::search::Doc>>,
    #[serde(default)]
    pub docs: Vec<fctl_domain::search::Doc>,
}

/// Forgets a session's documents (`/forget`): its project was deleted.
#[derive(Serialize, Deserialize)]
pub struct ForgetSession {
    pub session_id: SessionId,
}

/// A search in a workspace's index (`/search`), among the sessions of
/// `projects` and the sessions `sessions` (shared with the caller):
/// those the caller sees there.
#[derive(Serialize, Deserialize)]
pub struct SearchIndex {
    /// Checked (`fctl_domain::search::query`).
    pub q: String,
    pub projects: Vec<String>,
    pub sessions: Vec<String>,
    pub limit: u32,
}

/// What a workspace's index found: its sessions in order, with `limit + 1`
/// at most to say there are more.
#[derive(Serialize, Deserialize)]
pub struct IndexHits {
    pub sessions: Vec<IndexHit>,
}

#[derive(Serialize, Deserialize)]
pub struct IndexHit {
    pub session_id: String,
    /// `q` is in its title.
    pub titled: bool,
    /// When the newest document that matched was written.
    pub last: Millis,
    pub count: u32,
    pub matches: Vec<fctl_proto::api::SearchMatch>,
}

/// Backfills a session's documents (`/index`, from the hourly job):
/// whether its index has all its events now.
#[derive(Serialize, Deserialize)]
pub struct Indexed {
    pub done: bool,
}

// ───────────────────────────── HooksObject ─────────────────────────────

/// Something that happened in a workspace, for its webhooks that want it
/// (`/emit`): `fctl_domain::hooks::Event` with its id, where it happened.
#[derive(Serialize, Deserialize)]
pub struct HookEmit {
    pub workspace_id: String,
    pub event_id: String,
    pub kind: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub origin: Option<fctl_proto::hooks::WebhookOrigin>,
    pub gist: String,
    pub data: serde_json::Value,
    pub at: Millis,
}

/// A workspace's webhooks changed in D1 (`/changed`), one of them went
/// (`/forget`, with `webhook_id`), or a project did, with its webhooks
/// (`/forget-project`): its HooksObject reads them again.
#[derive(Serialize, Deserialize)]
pub struct HooksChanged {
    pub workspace_id: String,
    #[serde(default)]
    pub webhook_id: Option<String>,
    /// `/forget-project`: a deleted project, whose events go from the log.
    #[serde(default)]
    pub project_id: Option<String>,
    /// `/forget-project`: the project's own webhooks, which went with it
    /// and are forgotten whole, as `/forget` forgets one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webhooks: Vec<String>,
    /// A webhook that was turned on again: since when it fails starts
    /// afresh, so that the three days it gets before fctl turns it off
    /// again start now.
    #[serde(default)]
    pub fresh: Option<String>,
}

/// How some of a workspace's webhooks' deliveries went lately (`/health`).
#[derive(Serialize, Deserialize)]
pub struct HooksHealth {
    pub workspace_id: String,
    pub webhooks: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct HooksHealthAnswer {
    pub health: std::collections::BTreeMap<String, fctl_proto::hooks::WebhookHealth>,
}

/// A webhook's deliveries, newest first (`/deliveries`): those in
/// `state`, of event `kind`, before the cursor `before`.
#[derive(Serialize, Deserialize)]
pub struct HookDeliveries {
    pub workspace_id: String,
    pub webhook_id: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    pub limit: u32,
}

/// One delivery of a webhook (`/delivery`), or resending it now
/// (`/resend`, by `by`).
#[derive(Serialize, Deserialize)]
pub struct HookDelivery {
    pub workspace_id: String,
    pub webhook_id: String,
    pub delivery_id: String,
    #[serde(default)]
    pub by: Option<UserRef>,
}

/// Checks an address a webhook is being added at or pointed to: posts it a
/// challenge, signed with the webhook's secret, and says whether the
/// answer gave it back (`/challenge`).
#[derive(Serialize, Deserialize)]
pub struct HookChallenge {
    pub workspace_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub webhook_id: String,
    pub url: String,
    /// A new webhook's secret, sealed as its row will keep it; a webhook
    /// that exists signs with its own.
    #[serde(default)]
    pub sealed: Option<String>,
}

/// What `/challenge` found: why the answer doesn't let the webhook post
/// there, or nothing when it gave the challenge back.
#[derive(Serialize, Deserialize)]
pub struct HookChallenged {
    #[serde(default)]
    pub refused: Option<String>,
}

/// A sample of event `kind` to a webhook, now (`/test`).
#[derive(Serialize, Deserialize)]
pub struct HookTest {
    pub workspace_id: String,
    pub webhook_id: String,
    pub kind: String,
    pub by: UserRef,
}

/// A webhook's deliveries that failed since `since` wait for an attempt
/// again (`/resend-failed`).
#[derive(Serialize, Deserialize)]
pub struct HookResendFailed {
    pub workspace_id: String,
    pub webhook_id: String,
    pub since: Millis,
}

// ───────────────────────────── calling ─────────────────────────────

pub fn session_stub(env: &Env, session_id: &str) -> AppResult<Stub> {
    Ok(env
        .durable_object("SESSION")?
        .id_from_name(session_id)?
        .get_stub()?)
}

/// The SearchObject of a workspace: the search index of its sessions.
pub fn search_stub(env: &Env, workspace_id: &str) -> AppResult<Stub> {
    Ok(env
        .durable_object("SEARCH")?
        .id_from_name(workspace_id)?
        .get_stub()?)
}

/// The HooksObject of a workspace: its webhooks' deliveries.
pub fn hooks_stub(env: &Env, workspace_id: &str) -> AppResult<Stub> {
    Ok(env
        .durable_object("HOOKS")?
        .id_from_name(workspace_id)?
        .get_stub()?)
}

/// The FilesObject of a project.
pub fn files_stub(env: &Env, project_id: &str) -> AppResult<Stub> {
    Ok(env
        .durable_object("FILES")?
        .id_from_name(project_id)?
        .get_stub()?)
}

/// Claims uploads for a message in `session_id` and returns them; nothing to
/// do without ids.
pub async fn claim_files(
    env: &Env,
    project_id: &str,
    session_id: &SessionId,
    user_id: &str,
    ids: &[AttachmentId],
) -> AppResult<Vec<Attachment>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let stub = files_stub(env, project_id)?;
    let out: ClaimedFiles = post(
        &stub,
        "/claim",
        &ClaimFiles {
            session_id: session_id.clone(),
            user_id: user_id.to_owned(),
            ids: ids.to_vec(),
        },
    )
    .await?;
    Ok(out.attachments)
}

/// Hands files claimed for `session_id` in `project_id` back as unsent
/// uploads, which the FilesObject deletes a day later.
pub async fn unclaim_files(
    env: &Env,
    project_id: &str,
    session_id: &SessionId,
    ids: &[AttachmentId],
) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let stub = files_stub(env, project_id)?;
    let _: serde_json::Value = post(
        &stub,
        "/unclaim",
        &UnclaimFiles {
            session_id: session_id.clone(),
            ids: ids.to_vec(),
        },
    )
    .await?;
    Ok(())
}

/// The NotifyObject of a person.
pub fn notify_stub(env: &Env, user_id: &str) -> AppResult<Stub> {
    Ok(env
        .durable_object("NOTIFY")?
        .id_from_name(user_id)?
        .get_stub()?)
}

/// Closes runner `runner_id`'s uplink to session `session_id`, as FleetDO
/// does for a session that stops because its runner or its person no
/// longer runs it: a host that ignores the stop keeps no uplink to it, and
/// opens no other (`spec::check_runs`). Failures are logged, not returned.
pub async fn close_uplink(env: &Env, session_id: &str, runner_id: &str) {
    let closed: AppResult<serde_json::Value> = async {
        let stub = session_stub(env, session_id)?;
        post(
            &stub,
            "/close-uplink",
            &CloseUplink {
                runner_id: runner_id.to_owned(),
            },
        )
        .await
    }
    .await;
    if let Err(e) = closed {
        worker::console_error!("closing the uplink of {session_id}: {}", e.0.message);
    }
}

pub fn fleet_stub(env: &Env) -> AppResult<Stub> {
    Ok(env
        .durable_object("FLEET")?
        .id_from_name("fleet")?
        .get_stub()?)
}

fn build(
    method: Method,
    path: &str,
    body: Option<String>,
    headers: &[(&str, &str)],
) -> AppResult<Request> {
    let h = Headers::new();
    if body.is_some() {
        h.set("Content-Type", "application/json")?;
    }
    for (k, v) in headers {
        h.set(k, v)?;
    }
    let mut init = RequestInit::new();
    init.with_method(method).with_headers(h);
    if let Some(b) = body {
        init.with_body(Some(b.into()));
    }
    Ok(Request::new_with_init(
        &format!("https://internal{path}"),
        &init,
    )?)
}

async fn decode<T: DeserializeOwned>(mut res: Response) -> AppResult<T> {
    let status = res.status_code();
    let text = res.text().await?;
    if status >= 400 {
        // DOs answer errors with an ApiError body.
        return Err(match serde_json::from_str::<ApiError>(&text) {
            Ok(e) => AppError(e),
            Err(_) => internal(format!("durable object answered {status}: {text}")),
        });
    }
    Ok(serde_json::from_str(&text)?)
}

/// Sends a request body through to a Durable Object unread (a
/// `ReadableStream`) and decodes the JSON answer.
pub async fn post_body<T: DeserializeOwned>(
    stub: &Stub,
    path: &str,
    body: wasm_bindgen::JsValue,
) -> AppResult<T> {
    let mut init = RequestInit::new();
    init.with_method(Method::Post).with_body(Some(body));
    let req = Request::new_with_init(&format!("https://internal{path}"), &init)?;
    decode(stub.fetch_with_request(req).await?).await
}

/// GETs from a Durable Object and returns its response as is (file bodies).
pub async fn fetch(stub: &Stub, path: &str) -> AppResult<Response> {
    fetch_with(stub, path, &[]).await
}

/// The same, with request headers (a file's `Range`).
pub async fn fetch_with(stub: &Stub, path: &str, headers: &[(&str, &str)]) -> AppResult<Response> {
    let req = build(Method::Get, path, None, headers)?;
    Ok(stub.fetch_with_request(req).await?)
}

/// POSTs JSON to a Durable Object and decodes the JSON answer.
pub async fn post<B: Serialize, T: DeserializeOwned>(
    stub: &Stub,
    path: &str,
    body: &B,
) -> AppResult<T> {
    let req = build(Method::Post, path, Some(serde_json::to_string(body)?), &[])?;
    decode(stub.fetch_with_request(req).await?).await
}

pub async fn get<T: DeserializeOwned>(stub: &Stub, path: &str) -> AppResult<T> {
    let req = build(Method::Get, path, None, &[])?;
    decode(stub.fetch_with_request(req).await?).await
}

/// Forwards a WebSocket upgrade to a Durable Object with identity headers.
pub async fn upgrade(stub: &Stub, path: &str, headers: &[(&str, &str)]) -> AppResult<Response> {
    let mut all: Vec<(&str, &str)> = vec![("Upgrade", "websocket")];
    all.extend_from_slice(headers);
    let req = build(Method::Get, path, None, &all)?;
    Ok(stub.fetch_with_request(req).await?)
}

/// Inside a DO: an ApiError response.
pub fn do_error(e: &ApiError) -> worker::Result<Response> {
    Ok(crate::http::error_response(e))
}

/// Inside a DO: a JSON response.
pub fn do_json<T: Serialize>(body: &T) -> worker::Result<Response> {
    let text = serde_json::to_string(body).map_err(|e| worker::Error::RustError(e.to_string()))?;
    let h = Headers::new();
    h.set("Content-Type", "application/json")?;
    Ok(Response::ok(text)?.with_headers(h))
}
