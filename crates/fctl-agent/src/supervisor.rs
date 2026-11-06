//! The supervisor: one actor that owns the session.
//!
//! Inputs: frames from the host, output from Claude, results of background
//! work (cloning, git state), and a clock. Outputs: journaled events, state
//! and ephemeral streams to the host, and stdin for Claude.

use crate::attachments;
use crate::caches;
use crate::claude::{self, AskError, Claude, ClaudeOut, LaunchSpec};
use crate::followed::Pulls;
use crate::journal::Journal;
use crate::link::{self, LinkEvent};
use crate::local;
use crate::orphan;
use crate::prompt;
use crate::refresh::Refresh;
use crate::tasks::{self, Task, Tasks};
use crate::user;
use crate::workspace::{self, Moment, Paths};
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fctl_domain::events::{self as ev, BillingGuard, ClaudeKind};
use fctl_domain::models::ModelName;
use fctl_domain::{files, published, validate};
use fctl_proto::agent::{
    AgentConfig, AgentHello, AgentToHost, HostToAgent, SESSION_SOCKET_ENV, SessionReply,
    SessionRequest,
};
use fctl_proto::api::{AgentAnswer, AgentCall};
use fctl_proto::event::{
    Attachment, EventBody, FileDiff, GitState, LogLevel, LogLine, LogSource, Notice, NoticeLevel,
    PublishedFile,
};
use fctl_proto::runner::{
    ClaudeCredential, ClaudeCredentialKind, CredentialKind, GitIdentity, GithubCredential,
    StopReason,
};
use fctl_proto::session::{GithubAccess, ProjectSettings, RemoteControlState, SessionStatus};
use fctl_proto::uplink::{
    AgentEvent, ContextWindow, Delivery, DeliveryKind, FEATURE_ATTACHMENTS, FEATURE_SETTINGS,
    SessionState,
};
use fctl_proto::{ApiError, DeliveryId, PROTOCOL_VERSION, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long `fctl publish` waits for the host to upload a screenshot.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(120);
/// How long `fctl memory` and `fctl retro` wait for the control plane.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a refresh of the repositories' state may take, pushes and pull
/// request lookups included, before the next one goes ahead without it.
const SYNC_TIMEOUT: Duration = Duration::from_secs(10 * 60);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// What survives a supervisor restart (no secrets).
#[derive(Default, Serialize, Deserialize)]
struct Persisted {
    claude_started: bool,
    delivered: VecDeque<String>,
    remote_control_url: Option<String>,
    /// The title and description the agent gave the session (`fctl edit`).
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// Background tasks that were still running when the session stopped:
    /// the next message tells Claude they didn't finish.
    #[serde(default)]
    unfinished: Vec<Task>,
    /// The pull requests fctl-agent follows for the session.
    #[serde(default)]
    pulls: Pulls,
    /// A review agent posted its review (`fctl review submit`).
    #[serde(default)]
    review_posted: bool,
    /// fctl-agent reminded a review agent that ended without posting.
    #[serde(default)]
    review_nudged: bool,
    /// The Claude this supervisor started, while it may run: one that takes
    /// the session over after this one died stops it first.
    #[serde(default)]
    claude: Option<orphan::Process>,
    /// Claude was on a turn (the session ran) when the state was saved.
    #[serde(default)]
    working: bool,
    /// When fctl-agent last had Claude carry on with a turn that was cut
    /// off ([`Supervisor::take_over`]).
    #[serde(default)]
    carried_on_at: Option<i64>,
    /// A turn of Claude's that a supervisor before this one didn't see to
    /// its end: the transcript says so once Claude runs again, also when
    /// this supervisor dies before it does.
    #[serde(default)]
    cut_off: Option<CutOff>,
}

/// A turn of Claude's that a supervisor before this one didn't see to its
/// end ([`Supervisor::take_over`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CutOff {
    /// Claude ran on without it, unseen, until this one stopped it.
    Unseen,
    /// Claude stopped with it: the sandbox, or fctl-agent, stopped.
    Stopped,
}

/// How long after Claude carried on with a turn that was cut off a second
/// cut-off turn waits for a message instead: a sandbox that keeps losing
/// fctl-agent doesn't keep Claude going.
const CARRY_ON_AGAIN_AFTER: Duration = Duration::from_secs(10 * 60);

/// How long a review agent that ended without posting waits before
/// fctl-agent reminds it ([`Supervisor::remind_reviewer`]).
const REVIEW_REMINDER_AFTER: Duration = Duration::from_secs(5);

/// Why a review agent's session stopped once its review was posted.
const REVIEW_POSTED: &str = "Finished: its review is posted";

pub(crate) enum Work {
    Prepared(Result<(), String>),
    /// A refresh of the repositories' state at a moment ended: what they
    /// hold, the pull requests it follows and the changed files' diffs
    /// (when the host takes them), unless it failed.
    Git(Option<(GitState, Pulls, Option<Vec<FileDiff>>)>, Moment),
    Log(LogLevel, String, LogSource),
    /// The effort level Claude works at ([`claude::applied_effort`]).
    Effort(String),
    /// The context window Claude works in ([`claude::context_window`]).
    Context(ContextWindow),
    /// Claude's answers to a change of model or effort level
    /// ([`DeliveryKind::Settings`]): what was asked for, and the answer.
    Settings {
        delivery: DeliveryId,
        model: Option<(String, std::result::Result<Value, AskError>)>,
        effort: Option<(String, std::result::Result<Value, AskError>)>,
        /// The model and effort level the spec had before: what Claude
        /// starts on again if it refuses the new ones.
        before: (Option<String>, Option<String>),
    },
}

/// How many refused changes of model or effort level fctl-agent remembers,
/// to say so again when the host sends one again.
const REFUSALS_KEPT: usize = 50;

/// How long Claude has to switch models: it may ask the API about the
/// model first.
const SETTINGS_TIMEOUT: Duration = Duration::from_secs(90);

/// Why we asked the host for a credential.
enum CredFor {
    /// Answer Claude's `oauth_token_refresh` with this request id.
    ClaudeRefresh(String),
    GithubRefresh,
}

pub struct Supervisor {
    state_dir: PathBuf,
    /// Where the GitHub tokens and git's credential helper go, which the
    /// session's user reads: `state_dir`, or the user's `~/.fctl/state`
    /// where fctl-agent runs as root.
    user_state: PathBuf,
    /// Where programs in the sandbox reach us (`fctl edit`), told to Claude.
    session_socket: Option<PathBuf>,
    journal: Journal,
    persisted: Persisted,
    host: Option<(u64, mpsc::UnboundedSender<String>)>,
    config: Option<AgentConfig>,
    paths: Option<Paths>,
    state: SessionState,
    claude: Option<Claude>,
    claude_tx: mpsc::UnboundedSender<ClaudeOut>,
    work_tx: mpsc::UnboundedSender<Work>,
    /// Messages that arrived before Claude was ready.
    queue: VecDeque<Delivery>,
    turn_active: bool,
    /// The turn under way is one Claude started on its own, which its
    /// `idle` ends as well as its `result`.
    claude_turn: bool,
    /// Claude said last that it works (`session_state_changed`: `running`
    /// or `requires_action`), on a turn or for background work it hands
    /// Claude when it ends: the idle clock waits for its `idle`.
    claude_running: bool,
    /// Claude's background tasks that still run.
    tasks: Tasks,
    /// Why the session stops, and when it stops anyway.
    stopping: Option<(String, Instant)>,
    /// Since when the session has waited with nothing running: the idle
    /// timeout's clock.
    idle_since: Option<Instant>,
    /// Since the last turn ended, while the session waits: background work
    /// keeps it up for [`tasks::limit`] from then.
    waiting_since: Option<Instant>,
    resume_at: Option<i64>,
    github_expires_at: i64,
    pending_creds: HashMap<String, CredFor>,
    next_request: u64,
    outbox: Vec<AgentEvent>,
    restarts: VecDeque<Instant>,
    stderr_budget: (Instant, u32),
    claude_credential: Option<ClaudeCredential>,
    exit_after: Option<Instant>,
    /// The repositories' state reported last.
    last_git: Option<GitState>,
    /// The changed files' diffs sent last, which a host that attaches gets
    /// again.
    last_diffs: Option<Vec<FileDiff>>,
    /// When to refresh the repositories' state while Claude works.
    refresh: Refresh,
    /// A refresh of the repositories' state runs: one at a time.
    syncing: bool,
    /// A turn ended: the refresh that pushes and looks up pull requests
    /// follows the one that runs.
    turn_ended: bool,
    /// Files that came before `Configure` (the task's): saved once the
    /// sandbox's home is known.
    pending_files: Vec<(Attachment, String)>,
    /// Said once that the sandbox's fctl cannot rename the session.
    told_no_fctl: bool,
    /// The sandbox's Claude Code, once checked.
    claude_version: Option<claude::Version>,
    /// `fctl publish` calls waiting for the host's answer, by request id.
    publishing: HashMap<String, (oneshot::Sender<SessionReply>, Instant)>,
    /// Publishes the caller stopped waiting for: if the host publishes one
    /// after all, the transcript still says so.
    late_publishes: VecDeque<String>,
    /// `fctl memory` and `fctl retro` calls waiting for the control plane's
    /// answer, by request id.
    calling: HashMap<String, (oneshot::Sender<SessionReply>, Instant)>,
    /// Calls the caller stopped waiting for: if the control plane answers
    /// one after all, the transcript still says what was kept.
    late_calls: VecDeque<String>,
    /// Said once that the sandbox's fctl cannot keep the project's memory
    /// or record retrospectives.
    told_old_fctl: bool,
    /// Logged once: the sandbox's fctl has no `fctl ticket`.
    told_old_fctl_tickets: bool,
    /// The project's package caches, once readied for the session.
    cache: Option<caches::Ready>,
    /// Changes of model or effort level that Claude hasn't answered yet:
    /// one that comes again meanwhile is the same, whose answer follows.
    settings_waiting: HashSet<String>,
    /// Changes Claude refused, newest last, with why: one that comes again
    /// (the host lost the answer with its link) gets the same answer.
    settings_refused: VecDeque<(String, String)>,
    /// The name the session knows Claude's model by: the alias it was
    /// started on or switched to, say, rather than the ID Claude gives it.
    model_name: ModelName,
}

impl Supervisor {
    pub fn new(
        state_dir: PathBuf,
        session_socket: Option<PathBuf>,
    ) -> Result<(
        Self,
        mpsc::UnboundedReceiver<ClaudeOut>,
        mpsc::UnboundedReceiver<Work>,
    )> {
        let journal = Journal::open(&state_dir)?;
        let persisted: Persisted =
            user::as_root(|| std::fs::read_to_string(state_dir.join("state.json")))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
        let (claude_tx, claude_rx) = mpsc::unbounded_channel();
        let (work_tx, work_rx) = mpsc::unbounded_channel();
        Ok((
            Self {
                user_state: state_dir.clone(),
                state_dir,
                session_socket,
                journal,
                host: None,
                config: None,
                paths: None,
                state: SessionState {
                    status: SessionStatus::Provisioning,
                    reason: None,
                    resume_at: None,
                    branch: None,
                    model: None,
                    effort: None,
                    claude_session_id: None,
                    remote_control: None,
                    stats: None,
                    title: persisted.title.clone(),
                    description: persisted.description.clone(),
                    context: None,
                },
                persisted,
                claude: None,
                claude_tx,
                work_tx,
                queue: VecDeque::new(),
                turn_active: false,
                claude_turn: false,
                claude_running: false,
                tasks: Tasks::default(),
                stopping: None,
                idle_since: None,
                waiting_since: None,
                resume_at: None,
                github_expires_at: i64::MAX,
                pending_creds: HashMap::new(),
                next_request: 0,
                outbox: Vec::new(),
                restarts: VecDeque::new(),
                stderr_budget: (Instant::now(), 0),
                claude_credential: None,
                exit_after: None,
                last_git: None,
                last_diffs: None,
                refresh: Refresh::default(),
                syncing: false,
                turn_ended: false,
                pending_files: Vec::new(),
                told_no_fctl: false,
                claude_version: None,
                publishing: HashMap::new(),
                late_publishes: VecDeque::new(),
                calling: HashMap::new(),
                late_calls: VecDeque::new(),
                told_old_fctl: false,
                told_old_fctl_tickets: false,
                cache: None,
                settings_waiting: HashSet::new(),
                settings_refused: VecDeque::new(),
                model_name: ModelName::default(),
            },
            claude_rx,
            work_rx,
        ))
    }

    pub async fn run(
        mut self,
        mut link_rx: mpsc::UnboundedReceiver<LinkEvent>,
        mut claude_rx: mpsc::UnboundedReceiver<ClaudeOut>,
        mut work_rx: mpsc::UnboundedReceiver<Work>,
        mut local_rx: mpsc::UnboundedReceiver<local::Call>,
    ) -> Result<()> {
        let mut flush = tokio::time::interval(Duration::from_millis(40));
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                Some(e) = link_rx.recv() => self.on_link(e).await,
                Some(out) = claude_rx.recv() => self.on_claude(out).await,
                Some(w) = work_rx.recv() => self.on_work(w).await,
                Some(call) = local_rx.recv() => self.on_session_call(call),
                _ = flush.tick() => self.flush(),
                _ = tick.tick() => {
                    self.on_tick().await;
                    if self.exit_after.is_some_and(|t| Instant::now() >= t) {
                        self.flush();
                        return Ok(());
                    }
                }
            }
        }
    }

    // ───────────────────────────── host link ─────────────────────────────

    fn send(&self, msg: &AgentToHost) {
        if let (Some((_, tx)), Some(text)) = (&self.host, link::encode(msg)) {
            let _ = tx.send(text);
        }
    }

    /// Hands the host the changed files' diffs when they differ from those
    /// it got last.
    fn send_diffs(&mut self, files: Vec<FileDiff>) {
        let same = |a: &[FileDiff], b: &[FileDiff]| {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.key() == y.key())
        };
        if self
            .last_diffs
            .as_deref()
            .is_some_and(|last| same(last, &files))
        {
            return;
        }
        self.send(&AgentToHost::Diff {
            files: files.clone(),
        });
        self.last_diffs = Some(files);
    }

    fn flush(&mut self) {
        if self.outbox.is_empty() || self.host.is_none() {
            return;
        }
        // Batches of at most 100 events.
        while !self.outbox.is_empty() {
            let n = self.outbox.len().min(100);
            let batch: Vec<AgentEvent> = self.outbox.drain(..n).collect();
            self.send(&AgentToHost::Events { events: batch });
        }
    }

    fn hello(&self) -> AgentToHost {
        AgentToHost::Hello(AgentHello {
            protocol: PROTOCOL_VERSION,
            version: VERSION.to_owned(),
            configured: self.config.is_some(),
            status: self.state.status,
            agent_seq: self.journal.last_seq(),
            claude_version: self.claude_version.map(|v| v.to_string()),
            features: vec![FEATURE_ATTACHMENTS.to_owned(), FEATURE_SETTINGS.to_owned()],
        })
    }

    async fn on_link(&mut self, e: LinkEvent) {
        match e {
            LinkEvent::Connected(id, tx) => {
                let (seq, acked) = (self.journal.last_seq(), self.journal.acked());
                match self.host.as_ref() {
                    Some((old, _)) => tracing::info!(
                        "host link {id} replaces link {old}; journal at agent_seq {seq}, {acked} acknowledged"
                    ),
                    None => tracing::info!(
                        "host link {id} attached; journal at agent_seq {seq}, {acked} acknowledged"
                    ),
                }
                self.host = Some((id, tx));
                let hello = self.hello();
                self.send(&hello);
            }
            LinkEvent::Disconnected(id) => {
                if self.host.as_ref().is_some_and(|(cur, _)| *cur == id) {
                    tracing::info!("host link {id} detached");
                    self.host = None;
                }
            }
            LinkEvent::Frame(id, frame) => {
                if self.host.as_ref().is_some_and(|(cur, _)| *cur == id) {
                    self.on_frame(frame).await;
                }
            }
        }
    }

    async fn on_frame(&mut self, frame: HostToAgent) {
        match frame {
            HostToAgent::Ping => self.send(&AgentToHost::Pong),
            HostToAgent::Configure { config } => {
                if self.config.is_none() {
                    self.configure(*config);
                } else {
                    self.log(
                        LogSource::Agent,
                        LogLevel::Debug,
                        "Already configured; ignoring a repeated Configure",
                    );
                }
            }
            HostToAgent::Replay { after_agent_seq } => {
                self.outbox.clear();
                match self.journal.read_after(after_agent_seq) {
                    Ok(events) => {
                        if let Some((id, _)) = &self.host {
                            tracing::info!(
                                "replaying {} events after agent_seq {after_agent_seq} to host link {id}",
                                events.len()
                            );
                        }
                        for chunk in events.chunks(100) {
                            self.send(&AgentToHost::Events {
                                events: chunk.to_vec(),
                            });
                        }
                    }
                    Err(e) => tracing::error!("journal replay failed: {e:#}"),
                }
                self.send(&AgentToHost::State {
                    state: self.state.clone(),
                });
                // The host passes on to the control plane those it
                // hasn't sent over its uplink.
                if let Some(files) = &self.last_diffs {
                    self.send(&AgentToHost::Diff {
                        files: files.clone(),
                    });
                }
            }
            HostToAgent::Ack { agent_seq } => {
                if let Err(e) = self.journal.ack(agent_seq) {
                    tracing::warn!("journal ack failed: {e:#}");
                }
            }
            HostToAgent::Deliver { delivery } => self.on_delivery(delivery).await,
            HostToAgent::File { attachment, data } => self.on_file(attachment, data),
            HostToAgent::Credentials {
                request_id,
                claude,
                github,
                github_read,
                error,
            } => {
                self.on_credentials(request_id, claude, github, github_read, error);
            }
            HostToAgent::Stop { reason, grace_ms } => {
                self.begin_stop(stop_text(reason).to_owned(), grace_ms)
                    .await
            }
            HostToAgent::Published {
                request_id,
                file,
                error,
            } => self.on_published(&request_id, file, error),
            HostToAgent::Answer {
                request_id,
                answer,
                error,
            } => self.on_answer(&request_id, answer, error),
        }
    }

    // ───────────────────────────── journaling and state ─────────────────────────────

    fn emit(&mut self, body: EventBody) {
        match self.journal.append(now_ms(), body) {
            Ok(ev) => self.outbox.push(ev),
            Err(e) => tracing::error!("journal write failed: {e:#}"),
        }
    }

    fn log(&mut self, source: LogSource, level: LogLevel, text: &str) {
        self.emit(EventBody::Log(LogLine {
            source,
            level,
            text: text.to_owned(),
        }));
    }

    fn notice(&mut self, level: NoticeLevel, code: &str, message: &str, until: Option<i64>) {
        self.emit(EventBody::Notice(Notice {
            ticket: None,
            level,
            code: code.into(),
            message: message.into(),
            until,
            published: None,
            memory: None,
            retrospective: None,
            review: None,
            summarized: None,
            follow_up: None,
        }));
    }

    fn set_status(&mut self, status: SessionStatus, reason: Option<String>) {
        if self.state.status == status && self.state.reason == reason {
            return;
        }
        let waited = self.state.status == SessionStatus::Idle;
        self.state.status = status;
        self.state.reason = reason;
        // Whether Claude was on a turn, for a supervisor that takes over.
        let working = status == SessionStatus::Running;
        if self.persisted.working != working {
            self.persisted.working = working;
            self.persist();
        }
        self.state.resume_at = if status == SessionStatus::Paused {
            self.resume_at
        } else {
            None
        };
        let waits = status == SessionStatus::Idle;
        // The idle clock starts once background work has ended too, and
        // Claude is done.
        self.idle_since = self.nothing_runs().then(Instant::now);
        self.waiting_since = match (waits, waited) {
            (true, true) => self.waiting_since,
            (true, false) => Some(Instant::now()),
            (false, _) => None,
        };
        self.flush();
        self.send(&AgentToHost::State {
            state: self.state.clone(),
        });
    }

    // ───────────────────────────── attachments ─────────────────────────────

    fn on_file(&mut self, attachment: Attachment, data: String) {
        match self.paths.as_ref().map(|p| p.attachments.clone()) {
            Some(dir) => self.save_file(&dir, &attachment, &data),
            None => self.pending_files.push((attachment, data)),
        }
    }

    fn save_file(&mut self, dir: &std::path::Path, attachment: &Attachment, data: &str) {
        if let Err(e) = attachments::save(dir, attachment, data) {
            self.log(
                LogSource::Agent,
                LogLevel::Warn,
                &format!("Could not save {}: {e:#}", attachment.name),
            );
        }
    }

    fn persist(&self) {
        if let Ok(text) = serde_json::to_string(&self.persisted) {
            let path = self.state_dir.join("state.json");
            let _ = user::as_root(|| std::fs::write(path, text));
        }
    }

    // ───────────────────────────── the session socket ─────────────────────────────

    /// Answers a program in the sandbox: now, or once the host has
    /// uploaded the screenshot it publishes.
    fn on_session_call(&mut self, call: local::Call) {
        let reply = match call.request {
            SessionRequest::Edit {
                session_id,
                title,
                description,
            } => self.edit_session(session_id, title, description),
            SessionRequest::Publish {
                session_id,
                name,
                data,
            } => match self.publish(session_id, name, data) {
                Ok(request_id) => {
                    self.publishing
                        .insert(request_id, (call.reply, Instant::now()));
                    return;
                }
                Err(error) => Err(error),
            },
            SessionRequest::Call {
                session_id,
                call: what,
            } => match self.call(session_id, what) {
                Ok(request_id) => {
                    self.calling
                        .insert(request_id, (call.reply, Instant::now()));
                    return;
                }
                Err(error) => Err(error),
            },
        };
        let _ = call
            .reply
            .send(reply.unwrap_or_else(|error| SessionReply::Error { error }));
    }

    /// The spec, if the session started and `session_id` (when given) is
    /// the one this sandbox runs.
    fn this_session(
        &self,
        session_id: Option<&SessionId>,
    ) -> std::result::Result<fctl_proto::runner::SessionSpec, ApiError> {
        let Some(spec) = self.config.as_ref().map(|c| c.spec.clone()) else {
            return Err(ApiError::new(
                fctl_proto::ErrorCode::Conflict,
                "The session is still starting; try again in a moment",
            ));
        };
        if let Some(id) = session_id.filter(|id| **id != spec.session_id) {
            return Err(ApiError::bad_request(format!(
                "This sandbox runs {}, not {id}",
                spec.session_id
            )));
        }
        Ok(spec)
    }

    /// Hands a screenshot to the host to publish, if the project publishes
    /// screenshots and it is an image GitHub shows; returns the request's id.
    fn publish(
        &mut self,
        session_id: Option<SessionId>,
        name: String,
        data: String,
    ) -> std::result::Result<String, ApiError> {
        let spec = self.this_session(session_id.as_ref())?;
        if !spec.project.settings.publish_screenshots {
            return Err(ApiError::forbidden(
                "This project doesn't publish screenshots: its maintainers can turn it on in the project's settings, once the instance's admins allow it. Describe what the screenshots show instead.",
            ));
        }
        let bytes = STANDARD
            .decode(data.as_bytes())
            .map_err(|e| ApiError::bad_request(format!("The screenshot isn't base64: {e}")))?;
        published::check(&bytes)?;
        if self.host.is_none() {
            return Err(ApiError::new(
                fctl_proto::ErrorCode::Unavailable,
                "fctl-agent can't reach its host right now; try again in a minute",
            ));
        }
        self.next_request += 1;
        let request_id = format!("pub_{}", self.next_request);
        self.send(&AgentToHost::Publish {
            request_id: request_id.clone(),
            name: files::sanitize_name(&name),
            data,
        });
        Ok(request_id)
    }

    /// The host's answer to a publish: the caller gets it, and the
    /// transcript shows what was published.
    fn on_published(
        &mut self,
        request_id: &str,
        file: Option<PublishedFile>,
        error: Option<ApiError>,
    ) {
        let Some((tx, _)) = self.publishing.remove(request_id) else {
            // Too late for the caller, not for the transcript.
            if let Some(i) = self.late_publishes.iter().position(|id| id == request_id) {
                self.late_publishes.remove(i);
                if let Some(file) = file {
                    self.emit(EventBody::Notice(published_notice(&file, now_ms())));
                    self.flush();
                }
            }
            return;
        };
        let reply = match (file, error) {
            (Some(file), _) => {
                self.emit(EventBody::Notice(published_notice(&file, now_ms())));
                self.flush();
                SessionReply::Published { file }
            }
            (None, error) => SessionReply::Error {
                error: error.unwrap_or_else(|| {
                    ApiError::new(
                        fctl_proto::ErrorCode::Internal,
                        "The host published nothing",
                    )
                }),
            },
        };
        let _ = tx.send(reply);
    }

    /// Answers the publishes the host left unanswered for too long.
    fn expire_publishes(&mut self) {
        let late: Vec<String> = self
            .publishing
            .iter()
            .filter(|(_, (_, since))| since.elapsed() > PUBLISH_TIMEOUT)
            .map(|(id, _)| id.clone())
            .collect();
        for id in late {
            if let Some((tx, _)) = self.publishing.remove(&id) {
                let _ = tx.send(SessionReply::Error {
                    error: ApiError::new(
                        fctl_proto::ErrorCode::Unavailable,
                        "The host didn't publish the screenshot in time; try again",
                    ),
                });
                self.late_publishes.push_back(id);
                if self.late_publishes.len() > 100 {
                    self.late_publishes.pop_front();
                }
            }
        }
    }

    /// Hands what a program in the sandbox asks about the project (`fctl
    /// memory`, `fctl retro`) to the host, if the project turned it on;
    /// returns the request's id.
    fn call(
        &mut self,
        session_id: Option<SessionId>,
        call: AgentCall,
    ) -> std::result::Result<String, ApiError> {
        let spec = self.this_session(session_id.as_ref())?;
        match call {
            AgentCall::Retrospective { .. }
            | AgentCall::Retrospectives
            | AgentCall::UpdateRetrospective { .. }
                if !spec.retrospectives =>
            {
                return Err(ApiError::forbidden(
                    "This project doesn't ask for retrospectives: its maintainers can turn them on in the project's settings.",
                ));
            }
            AgentCall::Memories { .. }
            | AgentCall::Remember { .. }
            | AgentCall::MarkStale { .. }
                if spec.memory.is_none() =>
            {
                return Err(ApiError::forbidden(
                    "This project keeps no memory: its maintainers can turn it on in the project's settings.",
                ));
            }
            AgentCall::ResolveThread { .. }
                if spec.github_access.is_some_and(|g| !g.tokens_write()) =>
            {
                return Err(ApiError::forbidden(
                    "This session's agent doesn't comment on pull requests, so it can't resolve review threads.",
                ));
            }
            AgentCall::ReviewThreads
            | AgentCall::SubmitReview { .. }
            | AgentCall::SummarizePullRequest { .. }
                if spec.review.is_none() =>
            {
                return Err(ApiError::forbidden(
                    "This session doesn't review a pull request: `fctl review` is for review agents' sessions.",
                ));
            }
            AgentCall::Ticket { .. }
                if spec
                    .tickets
                    .as_ref()
                    .is_none_or(|t| t.via == fctl_proto::tickets::TicketsVia::Nothing) =>
            {
                return Err(ApiError::forbidden(
                    "This project's agents reach no ticket tracker: its maintainers choose one in the project's Tickets tab.",
                ));
            }
            _ => {}
        }
        if self.host.is_none() {
            return Err(ApiError::new(
                fctl_proto::ErrorCode::Unavailable,
                "fctl-agent can't reach its host right now; try again in a minute",
            ));
        }
        self.next_request += 1;
        let request_id = format!("call_{}", self.next_request);
        self.send(&AgentToHost::Call {
            request_id: request_id.clone(),
            call,
        });
        Ok(request_id)
    }

    /// The control plane's answer to a call: the caller gets it, and the
    /// transcript shows what the agent kept.
    fn on_answer(
        &mut self,
        request_id: &str,
        answer: Option<AgentAnswer>,
        error: Option<ApiError>,
    ) {
        let Some((tx, _)) = self.calling.remove(request_id) else {
            // Too late for the caller, not for the transcript.
            if let Some(i) = self.late_calls.iter().position(|id| id == request_id) {
                self.late_calls.remove(i);
                if let Some(answer) = &answer {
                    self.notice_kept(answer);
                }
            }
            return;
        };
        let reply = match (answer, error) {
            (Some(answer), _) => {
                self.notice_kept(&answer);
                SessionReply::Answer { answer }
            }
            (None, error) => SessionReply::Error {
                error: error.unwrap_or_else(|| {
                    ApiError::new(fctl_proto::ErrorCode::Internal, "The host answered nothing")
                }),
            },
        };
        let _ = tx.send(reply);
    }

    /// Adds to the transcript what an answer says the agent kept: a note it
    /// remembered (and those it replaced), a note it marked stale, a
    /// retrospective it recorded or updated.
    fn notice_kept(&mut self, answer: &AgentAnswer) {
        if matches!(answer, AgentAnswer::ReviewPosted { .. }) && !self.persisted.review_posted {
            self.persisted.review_posted = true;
            self.persist();
        }
        let notices = kept_notices(answer);
        if notices.is_empty() {
            return;
        }
        for n in notices {
            self.emit(EventBody::Notice(n));
        }
        self.flush();
    }

    /// Answers the calls the host left unanswered for too long.
    fn expire_calls(&mut self) {
        let late: Vec<String> = self
            .calling
            .iter()
            .filter(|(_, (_, since))| since.elapsed() > CALL_TIMEOUT)
            .map(|(id, _)| id.clone())
            .collect();
        for id in late {
            if let Some((tx, _)) = self.calling.remove(&id) {
                let _ = tx.send(SessionReply::Error {
                    error: ApiError::new(
                        fctl_proto::ErrorCode::Unavailable,
                        "The control plane didn't answer in time; try again",
                    ),
                });
                self.late_calls.push_back(id);
                if self.late_calls.len() > 100 {
                    self.late_calls.pop_front();
                }
            }
        }
    }

    /// The agent names or describes its session: the title and description
    /// travel with the session state, which the host passes on to the
    /// control plane (and sends again after every reconnect).
    fn edit_session(
        &mut self,
        session_id: Option<SessionId>,
        title: Option<String>,
        description: Option<String>,
    ) -> std::result::Result<SessionReply, ApiError> {
        let spec = self.this_session(session_id.as_ref())?;
        if spec.review.is_some() {
            return Err(ApiError::forbidden(
                "fctl names a review session after the pull request it reviews: leave its title as it is.",
            ));
        }
        if title.is_none() && description.is_none() {
            return Err(ApiError::bad_request("Give a title, a description or both"));
        }
        let title = title.as_deref().map(validate::session_title).transpose()?;
        let description = description
            .as_deref()
            .map(validate::session_description)
            .transpose()?;
        if let Some(t) = title {
            self.state.title = Some(t);
        }
        // An empty description clears the one the session has.
        if let Some(d) = description {
            self.state.description = Some(d.unwrap_or_default());
        }
        self.persisted.title = self.state.title.clone();
        self.persisted.description = self.state.description.clone();
        self.persist();
        self.flush();
        self.send(&AgentToHost::State {
            state: self.state.clone(),
        });
        Ok(SessionReply::Edited {
            id: spec.session_id.clone(),
            project_slug: spec.project.slug.clone(),
            number: spec.number,
            title: self.state.title.clone().unwrap_or(spec.title),
            description: self.state.description.clone().filter(|d| !d.is_empty()),
        })
    }

    fn mark_delivered(&mut self, id: &DeliveryId) {
        self.persisted.delivered.push_back(id.0.clone());
        while self.persisted.delivered.len() > 2000 {
            self.persisted.delivered.pop_front();
        }
        self.persist();
    }

    fn was_delivered(&self, id: &DeliveryId) -> bool {
        self.persisted.delivered.iter().any(|d| d == &id.0)
    }

    // ───────────────────────────── preparation ─────────────────────────────

    fn configure(&mut self, config: AgentConfig) {
        if let Some(seq) = config.first_agent_seq
            && let Err(e) = self.journal.start_after(seq)
        {
            tracing::error!("journal: {e:#}");
        }
        let home = config
            .home
            .clone()
            .map(PathBuf::from)
            .or_else(user::home)
            .unwrap_or_else(|| PathBuf::from("/root"));
        // `~/workspace` (container sandboxes) is relative to HOME.
        let workdir = match config.workdir.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(&config.workdir),
        };
        let mut config = config;
        config.workdir = workdir.display().to_string();
        let paths = Paths::new(&workdir, &home, &self.user_state);
        self.state.branch = Some(config.spec.branch.clone());
        self.state.claude_session_id = Some(config.spec.claude_session_id.clone());
        let creds = &config.spec.credentials;
        if let Some(at) = expiry(creds.github.as_ref(), creds.github_read.as_ref()) {
            self.github_expires_at = at;
        }
        self.claude_credential = Some(config.spec.credentials.claude.clone());
        self.paths = Some(paths.clone());
        for (att, data) in std::mem::take(&mut self.pending_files) {
            self.save_file(&paths.attachments, &att, &data);
        }
        let spec = config.spec.clone();
        // Those the session lists: in a new sandbox, fctl-agent knows no
        // others.
        if self
            .persisted
            .pulls
            .follow(spec.pull_requests.clone(), &spec)
        {
            self.persist();
        }
        let sandbox_env = prompt::sandbox_env(config.sandbox);
        self.cache = None;
        if let Some(cache) = &config.cache {
            match caches::prepare(cache, &home, &|key| std::env::var_os(key)) {
                Ok(ready) => {
                    self.log(
                        LogSource::Agent,
                        LogLevel::Info,
                        &format!(
                            "Package managers keep their downloads in {}, which the project's sessions on this runner share",
                            ready.dir.display()
                        ),
                    );
                    self.cache = Some(ready);
                }
                Err(why) => self.log(LogSource::Agent, LogLevel::Info, &why),
            }
        }
        // The project's environment comes after it, and wins.
        let cache_env = self
            .cache
            .as_ref()
            .map(|c| c.env.clone())
            .unwrap_or_default();
        self.config = Some(config);
        self.set_status(
            SessionStatus::Preparing,
            Some("Preparing the workspace".into()),
        );
        let repos = spec.project.repos.as_deref().unwrap_or_default();
        let github = spec.github_access.unwrap_or_default();
        if !repos.is_empty() {
            self.log(
                LogSource::Agent,
                LogLevel::Info,
                &prompt::repos_line(repos, &paths.repos.display().to_string(), github),
            );
        }
        let tracker = spec
            .tickets
            .as_ref()
            .is_some_and(|t| t.via != fctl_proto::tickets::TicketsVia::Nothing);
        if let Some(line) = prompt::github_line(github, tracker)
            && (spec.project.repo.is_some() || !repos.is_empty())
        {
            self.log(LogSource::Agent, LogLevel::Info, line);
        }
        for r in repos {
            if let Some(why) = &r.unavailable {
                self.notice(NoticeLevel::Warn, "repo_unavailable", why, None);
            }
        }
        let work = self.work_tx.clone();
        tokio::spawn(async move {
            let (log_tx, mut log_rx) = mpsc::unbounded_channel::<(LogLevel, String)>();
            let fwd_work = work.clone();
            let forward = tokio::spawn(async move {
                while let Some((level, text)) = log_rx.recv().await {
                    let _ = fwd_work.send(Work::Log(level, text, LogSource::Agent));
                }
            });
            let result = async {
                workspace::configure_git(&paths, &spec.git_author).await?;
                workspace::write_routes(&paths, &spec)?;
                workspace::write_github_tokens(
                    &paths,
                    spec.credentials.github.as_ref(),
                    spec.credentials.github_read.as_ref(),
                )?;
                workspace::prepare(&paths, &spec, &log_tx).await?;
                if let Some(script) = spec
                    .project
                    .settings
                    .setup_script
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                {
                    let env: Vec<(String, String)> = cache_env
                        .into_iter()
                        .chain(
                            spec.project
                                .settings
                                .env
                                .iter()
                                .chain(spec.secrets.iter())
                                .map(|(k, v)| (k.clone(), v.clone())),
                        )
                        .chain(sandbox_env)
                        .collect();
                    let (setup_tx, mut setup_rx) = mpsc::unbounded_channel::<(LogLevel, String)>();
                    let setup_work = work.clone();
                    let setup_fwd = tokio::spawn(async move {
                        while let Some((level, text)) = setup_rx.recv().await {
                            let _ = setup_work.send(Work::Log(level, text, LogSource::Setup));
                        }
                    });
                    let ok = workspace::setup_script(&paths, script, &env, &setup_tx).await?;
                    drop(setup_tx);
                    let _ = setup_fwd.await;
                    if !ok {
                        let _ = log_tx.send((
                            LogLevel::Warn,
                            "The setup script failed; continuing so the agent can fix it".into(),
                        ));
                    }
                }
                anyhow::Ok(())
            }
            .await;
            drop(log_tx);
            let _ = forward.await;
            let _ = work.send(Work::Prepared(result.map_err(|e| format!("{e:#}"))));
        });
    }

    async fn on_work(&mut self, w: Work) {
        match w {
            Work::Log(level, text, source) => self.log(source, level, &text),
            Work::Prepared(Ok(())) => {
                if let Err(e) = self.start_claude().await {
                    let msg = format!("Claude could not start: {e:#}");
                    self.notice(NoticeLevel::Error, "claude_start_failed", &msg, None);
                    self.set_status(SessionStatus::Failed, Some(msg));
                    self.finish().await;
                }
            }
            Work::Prepared(Err(e)) => {
                self.notice(NoticeLevel::Error, "prepare_failed", &e, None);
                self.set_status(SessionStatus::Failed, Some(e));
                self.finish().await;
            }
            Work::Git(synced, moment) => {
                self.syncing = false;
                // Once the session has stopped, the state it stopped with
                // is its last.
                if let Some((state, pulls, diffs)) = synced.filter(|_| self.exit_after.is_none()) {
                    if pulls != self.persisted.pulls {
                        self.persisted.pulls = pulls;
                        self.persist();
                    }
                    // Before the state that has their digests, so that the
                    // control plane has them when its viewers ask.
                    if let Some(files) = diffs {
                        self.send_diffs(files);
                    }
                    // While Claude works, only a change is news, and a
                    // repository in the middle of a rebase has none yet.
                    let working = matches!(moment, Moment::Working { .. });
                    let news =
                        self.last_git.as_ref() != Some(&state) && !workspace::detached(&state);
                    if !working || news {
                        if let Some(pr) = &state.pull_request {
                            tracing::info!("pull request #{} {}", pr.number, pr.url);
                        }
                        self.last_git = Some(state.clone());
                        self.emit(EventBody::Git(state));
                    }
                }
                self.sync();
            }
            Work::Effort(effort) => {
                if self.state.effort.as_ref() != Some(&effort) {
                    self.state.effort = Some(effort);
                    self.flush();
                    self.send(&AgentToHost::State {
                        state: self.state.clone(),
                    });
                }
            }
            Work::Context(window) => {
                if self.state.context != Some(window) {
                    self.state.context = Some(window);
                    self.flush();
                    self.send(&AgentToHost::State {
                        state: self.state.clone(),
                    });
                }
            }
            Work::Settings {
                delivery,
                model,
                effort,
                before,
            } => self.settings_applied(delivery, model, effort, before),
        }
    }

    /// Claude answered a change of model or effort level: the session says
    /// what it works on now, which also takes back what the control plane
    /// showed of a change Claude refused, and the transcript says why.
    fn settings_applied(
        &mut self,
        delivery: DeliveryId,
        model: Option<(String, std::result::Result<Value, AskError>)>,
        effort: Option<(String, std::result::Result<Value, AskError>)>,
        before: (Option<String>, Option<String>),
    ) {
        self.settings_waiting.remove(&delivery.0);
        let mut refused = Vec::new();
        if let Some((asked, answer)) = model {
            match answer {
                // By the name it was asked for (an alias, say), which
                // Claude answers with the model's ID.
                Ok(v) => {
                    let id = v.get("model").and_then(Value::as_str).unwrap_or(&asked);
                    self.model_name.switched(&asked, id);
                    self.state.model = Some(asked);
                }
                Err(AskError::Refused(e)) => {
                    refused.push(format!("Claude didn't switch to {asked}: {e}"));
                    // It starts on the one it had, if it starts again.
                    if let Some(c) = self.config.as_mut() {
                        c.spec.model = before.0.clone();
                    }
                }
                // It started again on the new one, as its `system/init` says.
                Err(AskError::Exited) => {}
                // It may switch yet, and starts on the new one if it starts
                // again: by the name asked for either way.
                Err(AskError::TimedOut) => {
                    self.model_name.switching(&asked);
                    refused.push(format!(
                    "Claude didn't say in time whether it switched to {asked}; it works on it if it starts again in this run."
                    ));
                }
            }
        }
        if let Some((asked, answer)) = effort {
            match answer {
                // Until Claude says what it works at (`get_settings`), which
                // its limits may make less.
                Ok(_) => self.state.effort = Some(asked),
                Err(AskError::Refused(e)) => {
                    refused.push(format!(
                        "Claude didn't change its effort level to {asked}: {e}"
                    ));
                    if let Some(c) = self.config.as_mut() {
                        c.spec.effort = before.1.clone();
                    }
                }
                Err(AskError::Exited) => {}
                Err(AskError::TimedOut) => refused.push(format!(
                    "Claude didn't say in time whether it works at {asked} effort; it does if it starts again in this run."
                )),
            }
        }
        if let Some(c) = self.claude.as_mut() {
            read_effort(c, &self.work_tx);
            read_context(c, &self.work_tx);
        }
        self.flush();
        // Taken only now: until Claude answered, the same change coming
        // again waits for this answer.
        self.mark_delivered(&delivery);
        let error = (!refused.is_empty()).then(|| refused.join(" "));
        if let Some(e) = &error {
            self.settings_refused
                .push_back((delivery.0.clone(), e.clone()));
            while self.settings_refused.len() > REFUSALS_KEPT {
                self.settings_refused.pop_front();
            }
        }
        // The delivery first: SessionDO shows the change until it is done,
        // and then what the state says.
        self.send(&AgentToHost::Delivered {
            delivery_id: delivery,
            ok: error.is_none(),
            error,
        });
        self.send(&AgentToHost::State {
            state: self.state.clone(),
        });
    }

    // ───────────────────────────── Claude ─────────────────────────────

    fn write_claude_config(
        &self,
        home: &std::path::Path,
        workdir: &str,
        project: &ProjectSettings,
        co_authors: &[GitIdentity],
    ) -> Result<()> {
        let claude_dir = home.join(".claude");
        std::fs::create_dir_all(&claude_dir)?;
        let path = home.join(".claude.json");
        let mut cfg: Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({}));
        cfg["hasCompletedOnboarding"] = json!(true);
        cfg["bypassPermissionsModeAccepted"] = json!(true);
        cfg["autoUpdates"] = json!(false);
        cfg["projects"][workdir]["hasTrustDialogAccepted"] = json!(true);
        cfg["projects"][workdir]["hasCompletedProjectOnboarding"] = json!(true);
        if let Some(raw) = self
            .claude_credential
            .as_ref()
            .and_then(|c| c.account.as_ref())
            .and_then(|a| a.raw.clone())
        {
            cfg["oauthAccount"] = raw;
        }
        std::fs::write(&path, serde_json::to_string_pretty(&cfg)?)?;
        // The image's settings stay, with fctl's over them.
        let settings_path = claude_dir.join("settings.json");
        let image = match std::fs::read_to_string(&settings_path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                let kept = claude_dir.join("settings.json.invalid");
                tracing::warn!(
                    "{} is not JSON ({e}); keeping it as {}",
                    settings_path.display(),
                    kept.display()
                );
                let _ = std::fs::rename(&settings_path, &kept);
                Value::Null
            }),
            Err(_) => Value::Null,
        };
        let settings =
            claude::merge_user_settings(&image, &claude::user_settings(project, co_authors));
        std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)?;
        Ok(())
    }

    async fn start_claude(&mut self) -> Result<()> {
        let (Some(config), Some(paths)) = (self.config.clone(), self.paths.clone()) else {
            anyhow::bail!("not configured");
        };
        let spec = &config.spec;
        let workdir = config.workdir.clone();
        let bin = config.claude_bin.clone().unwrap_or_else(|| "claude".into());
        if self.claude_version.is_none() {
            match claude::check_version(&bin).await {
                Ok(Some(version)) => self.claude_version = Some(version),
                Ok(None) => tracing::warn!("{bin} --version said no version; starting it anyway"),
                Err(why) => anyhow::bail!(why),
            }
        }
        self.write_claude_config(
            &paths.home,
            &workdir,
            &spec.project.settings,
            &spec.co_authored_by,
        )?;
        // Whether the agent keeps the project's memory: then Claude Code's
        // own, which would last only as long as the sandbox, is off.
        let mut keeps_memory = false;
        if spec.review.is_some() {
            // A review agent posts what it found through fctl, which named
            // its session: without `fctl review` its work would be lost.
            if self.session_socket.is_none() || !fctl_can(&paths, "review submit").await {
                anyhow::bail!(
                    "this sandbox's fctl can't post reviews (`fctl review`), so the review agent couldn't post what it finds. The default image, ghcr.io/hyperstructure/sandbox, can; an image of your own needs a recent fctl on the PATH."
                );
            }
            if let Err(e) = claude::write_instructions(&paths.home, prompt::REVIEW_AGENTS_MD) {
                tracing::warn!("writing ~/AGENTS.md failed: {e:#}");
            }
        } else if self.session_socket.is_some() && fctl_can(&paths, "edit").await {
            // Publishing needs the project's consent and an fctl that can.
            let screenshots =
                spec.project.settings.publish_screenshots && fctl_can(&paths, "publish").await;
            let days = screenshots.then(|| published::days(&spec.project.settings));
            // So do the memory and retrospectives.
            let memory = match &spec.memory {
                Some(m) if fctl_can(&paths, "memory").await => Some(m),
                _ => None,
            };
            let retrospectives = spec.retrospectives && fctl_can(&paths, "retro").await;
            // Agents keep one for each piece of work where their fctl
            // updates them.
            let update_retrospectives = retrospectives && fctl_can(&paths, "retro update").await;
            let lacking = (spec.memory.is_some() && memory.is_none())
                || (spec.retrospectives && !retrospectives);
            if lacking && !self.told_old_fctl {
                self.told_old_fctl = true;
                self.log(
                    LogSource::Agent,
                    LogLevel::Info,
                    "This sandbox's fctl has no `fctl memory` or `fctl retro`, so the agent keeps no project memory and records no retrospectives. The default image, ghcr.io/hyperstructure/sandbox, has them; an image of your own needs a recent fctl on the PATH.",
                );
            }
            keeps_memory = memory.is_some();
            // An agent that comments resolves review threads through fctl.
            let review_threads = spec.github_access == Some(GithubAccess::Comment)
                && fctl_can(&paths, "resolve").await;
            let extras = prompt::Extras {
                screenshot_days: days,
                memory,
                retrospectives,
                update_retrospectives,
                review_threads,
                now: now_ms(),
            };
            if let Err(e) = claude::write_instructions(&paths.home, &prompt::agents_md(&extras)) {
                tracing::warn!("writing ~/AGENTS.md failed: {e:#}");
            }
        } else if !self.told_no_fctl {
            self.told_no_fctl = true;
            self.log(
                LogSource::Agent,
                LogLevel::Info,
                "This sandbox has no fctl that can rename the session, so it keeps the title it started with. The default image, ghcr.io/hyperstructure/sandbox, has one; an image of your own needs fctl on the PATH.",
            );
        }
        let setting_sources = match claude::project_settings_are_unsafe(&paths.workdir) {
            Some(why) => {
                let msg = format!(
                    "Ignoring the repository's Claude settings because {why}: fctl only runs Claude on subscriptions."
                );
                self.notice(NoticeLevel::Warn, "project_settings_ignored", &msg, None);
                "user"
            }
            None => "user,project,local",
        };
        let cred = self
            .claude_credential
            .clone()
            .unwrap_or_else(|| spec.credentials.claude.clone());
        let project_env = fctl_domain::settings::session_env(&spec.project.settings);
        let mut env = std::collections::BTreeMap::new();
        let cache_env = self
            .cache
            .iter()
            .flat_map(|c| c.env.iter().map(|(k, v)| (k, v)));
        for (k, v) in cache_env
            .chain(project_env.iter())
            .chain(spec.secrets.iter())
            .chain(config.env.iter())
        {
            env.insert(k.clone(), v.clone());
        }
        env.insert("CLAUDE_CODE_OAUTH_TOKEN".into(), cred.access_token.clone());
        env.insert(
            "CLAUDE_CODE_OAUTH_SCOPES".into(),
            if cred.scopes.is_empty() {
                "user:inference".into()
            } else {
                cred.scopes.join(" ")
            },
        );
        if let Some(t) = &cred.subscription_type {
            env.insert("CLAUDE_CODE_SUBSCRIPTION_TYPE".into(), t.clone());
        }
        env.insert("IS_SANDBOX".into(), "1".into());
        env.insert("DISABLE_AUTOUPDATER".into(), "1".into());
        // Turns Claude starts on its own (for background work) count too.
        env.insert(tasks::SESSION_STATE_ENV.into(), "1".into());
        env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        env.insert("FCTL_SESSION_ID".into(), spec.session_id.0.clone());
        env.insert("FCTL_SESSION_URL".into(), spec.web_url.clone());
        if keeps_memory {
            env.insert(claude::DISABLE_AUTO_MEMORY_ENV.into(), "1".into());
        }
        if let Some(socket) = &self.session_socket {
            env.insert(SESSION_SOCKET_ENV.into(), socket.display().to_string());
        }
        env.extend(prompt::sandbox_env(config.sandbox));
        // Resuming needs Claude's conversation. A sandbox made for a resumed
        // session (the stopped one expired) has none: start a new one there.
        let wants_resume = spec.resume || self.persisted.claude_started;
        let resume = wants_resume
            && (self.persisted.claude_started
                || claude::conversation_exists(&paths.home, &spec.claude_session_id));
        let fresh_resume = wants_resume && !resume;
        // `fctl ticket` reaches the tracker, where the sandbox's fctl has it.
        let tickets_cli = match spec.tickets.as_ref() {
            Some(t) if t.via == fctl_proto::tickets::TicketsVia::Fctl => {
                let can = self.session_socket.is_some() && fctl_can(&paths, "ticket").await;
                if !can && !self.told_old_fctl_tickets {
                    self.told_old_fctl_tickets = true;
                    self.log(
                        LogSource::Agent,
                        LogLevel::Info,
                        "This sandbox's fctl has no `fctl ticket`, so the agent can't reach the project's ticket tracker. The default image, ghcr.io/hyperstructure/sandbox, has it; an image of your own needs a recent fctl on the PATH.",
                    );
                }
                can
            }
            _ => false,
        };
        let system = match &spec.review {
            Some(review) => {
                let previous_here = match review.previous_sha.as_deref() {
                    Some(sha) => workspace::has_commit(&paths.workdir, &paths.home, sha).await,
                    None => false,
                };
                prompt::review_system_prompt(
                    spec,
                    review,
                    &workdir,
                    &workspace::review_branch(spec, review),
                    previous_here,
                    &paths,
                    config.sandbox,
                )
            }
            None => prompt::system_prompt(
                spec,
                &workdir,
                &paths,
                config.sandbox,
                self.cache.as_ref(),
                tickets_cli,
            ),
        };
        let launch = LaunchSpec {
            bin: &bin,
            cwd: &paths.workdir,
            home: &paths.home,
            claude_session_id: &spec.claude_session_id,
            resume,
            model: spec
                .model
                .as_deref()
                .or(spec.project.settings.model.as_deref()),
            effort: spec
                .effort
                .as_deref()
                .or(spec.project.settings.effort.as_deref()),
            append_system_prompt: &system,
            setting_sources,
            extra_args: &config.claude_args,
            env,
        };
        self.model_name.started(launch.model);
        let mut c = Claude::spawn(launch, self.claude_tx.clone())?;
        self.persisted.claude = c.pid().and_then(orphan::Process::of);
        self.persist();
        self.log(
            LogSource::Agent,
            LogLevel::Info,
            &format!("Started Claude Code (pid {})", c.pid().unwrap_or(0)),
        );
        let init = c
            .request(json!({ "subtype": "initialize" }), Duration::from_secs(90))
            .await;
        if let Err(e) = init {
            c.kill().await;
            anyhow::bail!("Claude did not initialize: {e}");
        }
        read_effort(&mut c, &self.work_tx);
        read_context(&mut c, &self.work_tx);
        self.persisted.claude_started = true;
        self.persist();
        // Remote Control: follow the session in claude.ai/code and the apps.
        if spec.project.settings.remote_control {
            let rc = if cred.kind == ClaudeCredentialKind::Oauth
                && cred.scopes.iter().any(|s| s == "user:profile")
            {
                let name = format!("{} · {}", spec.project.name, spec.title);
                match c
                    .request(
                        json!({ "subtype": "remote_control", "enabled": true, "name": name }),
                        Duration::from_secs(45),
                    )
                    .await
                {
                    Ok(v) => RemoteControlState {
                        enabled: true,
                        session_url: v
                            .get("session_url")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        unavailable_reason: None,
                        user: None,
                    },
                    Err(e) => RemoteControlState {
                        enabled: false,
                        session_url: None,
                        unavailable_reason: Some(e),
                        user: None,
                    },
                }
            } else {
                RemoteControlState {
                    enabled: false,
                    session_url: None,
                    unavailable_reason: Some(
                        "Remote Control needs a full Claude login. Connect one under Settings → Claude.".into(),
                    ),
                    user: None,
                }
            };
            self.persisted.remote_control_url = rc.session_url.clone();
            self.persist();
            self.state.remote_control = Some(rc.clone());
            self.emit(EventBody::RemoteControl(rc));
        }
        self.claude = Some(c);
        // The first turn, then anything that queued up meanwhile.
        if fresh_resume {
            self.log(
                LogSource::Agent,
                LogLevel::Info,
                "The earlier conversation went with the old sandbox; Claude starts a new one on the branch",
            );
            // The task's files came along again, into the new sandbox.
            let files = self.prepare_files(&spec.task_attachments);
            let task = spec
                .task
                .as_deref()
                .map(|t| prompt::with_files(t, &files.note));
            let other_repos = spec.project.repos.as_ref().is_some_and(|r| !r.is_empty());
            let text = prompt::fresh_resume(&spec.branch, task.as_deref(), other_repos);
            self.send_content(&text, files.images, None).await;
        } else if !resume && let Some(p) = spec.prompt.clone() {
            self.send_turn(&p, None, None, &spec.task_attachments).await;
        }
        if let Some(cut_off) = self.persisted.cut_off.take() {
            self.carry_on(cut_off).await;
        }
        if self.turn_active {
            self.set_status(SessionStatus::Running, None);
        } else {
            self.wait().await;
        }
        while let Some(d) = self.queue.pop_front() {
            self.on_delivery(d).await;
        }
        Ok(())
    }

    fn prepare_files(&self, files: &[Attachment]) -> attachments::Prepared {
        attachments::prepare(self.paths.as_ref().map(|p| p.attachments.as_path()), files)
    }

    /// A message for Claude, with its files.
    async fn send_turn(
        &mut self,
        text: &str,
        author: Option<prompt::Author<'_>>,
        delivery: Option<&DeliveryId>,
        files: &[Attachment],
    ) {
        let starter = self.config.as_ref().map(|c| &c.spec.author);
        let files = self.prepare_files(files);
        let text = prompt::user_turn(&prompt::with_files(text, &files.note), author, starter);
        self.send_content(&text, files.images, delivery).await;
    }

    async fn send_content(
        &mut self,
        text: &str,
        images: Vec<Value>,
        delivery: Option<&DeliveryId>,
    ) {
        if self.claude.is_none() {
            return;
        }
        // The first message after a stop that cut background work short.
        let text = if self.persisted.unfinished.is_empty() {
            text.to_owned()
        } else {
            let note = prompt::unfinished_tasks(&std::mem::take(&mut self.persisted.unfinished));
            self.persist();
            format!("{note}\n\n{text}")
        };
        if let Some(c) = &self.claude {
            let mut content = vec![json!({ "type": "text", "text": text })];
            content.extend(images);
            c.send_user(content, &fctl_domain::ids::uuid_v4());
            self.turn_active = true;
            // A turn of Claude's own now has this message to answer too.
            self.claude_turn = false;
            self.idle_since = None;
            if let Some(id) = delivery {
                self.mark_delivered(id);
                self.send(&AgentToHost::Delivered {
                    delivery_id: id.clone(),
                    ok: true,
                    error: None,
                });
            }
            if self.state.status != SessionStatus::Running {
                self.set_status(SessionStatus::Running, None);
            }
        }
    }

    async fn on_delivery(&mut self, d: Delivery) {
        if self.settings_waiting.contains(&d.id.0) {
            // Its answer follows.
            return;
        }
        if self.was_delivered(&d.id) {
            let refused = self
                .settings_refused
                .iter()
                .find(|(id, _)| id == &d.id.0)
                .map(|(_, why)| why.clone());
            self.send(&AgentToHost::Delivered {
                delivery_id: d.id,
                ok: refused.is_none(),
                error: refused,
            });
            return;
        }
        match &d.kind {
            DeliveryKind::Message {
                text,
                author,
                author_called,
                attachments,
            } => {
                if self.stopping.is_some() {
                    // Leave it unconfirmed: the control plane keeps it for the next run.
                    return;
                }
                if self.claude.is_none() {
                    self.queue.push_back(d.clone());
                    return;
                }
                // A message during a usage-limit pause waits for the reset.
                if self.state.status == SessionStatus::Paused {
                    self.queue.push_back(d.clone());
                    return;
                }
                let author = author.as_ref().map(|user| prompt::Author {
                    user,
                    called: author_called.as_deref(),
                });
                self.send_turn(text, author, Some(&d.id), attachments).await;
            }
            DeliveryKind::Settings { model, effort, .. } => {
                // Before the session's spec came: once Claude has started.
                let Some(spec) = self.config.as_ref().map(|c| &c.spec) else {
                    self.queue.push_back(d.clone());
                    return;
                };
                let before = (spec.model.clone(), spec.effort.clone());
                // What Claude starts on if it starts again in this run.
                if let Some(config) = self.config.as_mut() {
                    if let Some(m) = model {
                        config.spec.model = Some(m.clone());
                    }
                    if let Some(e) = effort {
                        config.spec.effort = Some(e.clone());
                    }
                }
                let Some(c) = self.claude.as_mut() else {
                    // It starts on them.
                    self.mark_delivered(&d.id);
                    self.send(&AgentToHost::Delivered {
                        delivery_id: d.id,
                        ok: true,
                        error: None,
                    });
                    return;
                };
                // At once, and in either order: the effort level is a
                // setting of its own, which Claude applies to whichever
                // model it works on.
                let switched = model.clone().map(|m| {
                    let answer = c.ask(
                        json!({ "subtype": "set_model", "model": m }),
                        SETTINGS_TIMEOUT,
                    );
                    (m, answer)
                });
                let changed = effort.clone().map(|e| {
                    let answer = c.ask(
                        json!({ "subtype": "apply_flag_settings", "settings": { "effortLevel": e } }),
                        SETTINGS_TIMEOUT,
                    );
                    (e, answer)
                });
                self.settings_waiting.insert(d.id.0.clone());
                let work = self.work_tx.clone();
                tokio::spawn(async move {
                    let model = match switched {
                        Some((m, answer)) => Some((m, answer.await)),
                        None => None,
                    };
                    let effort = match changed {
                        Some((e, answer)) => Some((e, answer.await)),
                        None => None,
                    };
                    let _ = work.send(Work::Settings {
                        delivery: d.id,
                        model,
                        effort,
                        before,
                    });
                });
            }
            DeliveryKind::Interrupt { .. } => {
                if let Some(c) = self.claude.as_mut()
                    && self.turn_active
                {
                    c.request_nowait(json!({ "subtype": "interrupt" }));
                }
                self.queue
                    .retain(|q| !matches!(q.kind, DeliveryKind::Message { .. }));
                self.mark_delivered(&d.id);
                self.send(&AgentToHost::Delivered {
                    delivery_id: d.id,
                    ok: true,
                    error: None,
                });
            }
        }
    }

    async fn on_claude(&mut self, out: ClaudeOut) {
        match out {
            ClaudeOut::Message(v) => self.on_claude_message(v).await,
            ClaudeOut::ControlRequest {
                request_id,
                request,
            } => self.on_claude_request(request_id, request),
            ClaudeOut::Stderr(line) => {
                // At most 60 stderr lines a minute reach the transcript.
                let (since, n) = self.stderr_budget;
                if since.elapsed() > Duration::from_secs(60) {
                    self.stderr_budget = (Instant::now(), 0);
                }
                if self.stderr_budget.1 < 60 {
                    self.stderr_budget.1 += 1;
                    self.log(LogSource::Claude, LogLevel::Warn, &line);
                } else if n == 60 {
                    self.stderr_budget.1 += 1;
                    self.log(
                        LogSource::Claude,
                        LogLevel::Warn,
                        "(more stderr output suppressed)",
                    );
                }
            }
            ClaudeOut::Exited(code) => self.on_claude_exit(code).await,
        }
    }

    async fn on_claude_message(&mut self, v: Value) {
        // Claude's word on its state: acted on, not kept.
        if let Some(state) = tasks::session_state(&v) {
            match state {
                "idle" => self.claude_went_idle().await,
                "running" | "requires_action" => {
                    self.claude_running = true;
                    if !self.turn_active {
                        self.claude_started_turn();
                    }
                }
                _ => {}
            }
            return;
        }
        // It doesn't report every turn it starts on its own: their
        // messages tell.
        if !self.turn_active && tasks::in_turn(&v) {
            self.claude_started_turn();
        }
        let kind = ev::classify(&v);
        if ev::is_ephemeral(kind) {
            self.send(&AgentToHost::Stream { event: v });
            return;
        }
        if let Some(guard) = ev::billing_guard(&v) {
            self.emit(EventBody::Claude(v.clone()));
            self.on_billing_guard(guard).await;
            return;
        }
        if kind == ClaudeKind::Init
            && let Some(m) = v.get("model").and_then(Value::as_str)
        {
            // The control plane takes a model from `system/init` only for
            // a session that names none: the state says which this is.
            let shown = self.model_name.init(m);
            if self.state.model.as_ref() != Some(&shown) {
                self.state.model = Some(shown);
                self.send(&AgentToHost::State {
                    state: self.state.clone(),
                });
            }
        }
        let is_result = kind == ClaudeKind::Result;
        let more_queued = v
            .get("queued_turn_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0;
        let tasks_changed = self.tasks.observe(&v);
        self.refresh.observe(&v, Instant::now());
        self.emit(EventBody::Claude(v));
        if is_result {
            // `/effort high` in a message changes the effort level, and so
            // can the Claude apps over Remote Control. `/model` changes the
            // context window too.
            if let Some(c) = self.claude.as_mut() {
                read_effort(c, &self.work_tx);
                read_context(c, &self.work_tx);
            }
            self.turn_active = more_queued;
            self.claude_turn = false;
            if !more_queued
                && self.stopping.is_none()
                && self.state.status == SessionStatus::Running
            {
                self.wait().await;
            }
            self.after_turn();
        } else if tasks_changed
            && !self.turn_active
            && self.stopping.is_none()
            && self.state.status == SessionStatus::Idle
        {
            self.wait().await;
        }
    }

    /// The session waits: for Claude's background tasks while any run,
    /// then for a message. A review agent's waits for nothing once its
    /// review is posted: nobody messages it and it posts once, so it stops,
    /// background work and all, and frees its runner's slot.
    async fn wait(&mut self) {
        if self.persisted.review_posted {
            self.begin_stop(REVIEW_POSTED.to_owned(), 30_000).await;
            return;
        }
        let reviews = self
            .config
            .as_ref()
            .is_some_and(|c| c.spec.review.is_some());
        let reason = if reviews && self.tasks.is_empty() {
            // It waits for nobody: fctl-agent reminds it to post, once,
            // and a minute after its last turn it stops.
            "The review session stops in a minute".to_owned()
        } else {
            tasks::waiting_reason(self.tasks.len())
        };
        self.set_status(SessionStatus::Idle, Some(reason));
        if self.idle_since.is_none() && self.nothing_runs() {
            self.idle_since = Some(Instant::now());
        }
    }

    /// Whether the session waits for a message with nothing running: no
    /// background task, and Claude said last that it is done, if it says.
    fn nothing_runs(&self) -> bool {
        self.state.status == SessionStatus::Idle && self.tasks.is_empty() && !self.claude_running
    }

    /// Claude started a turn on its own, to take what its background work
    /// reported: it counts as any other, and stops the idle clock.
    fn claude_started_turn(&mut self) {
        self.turn_active = true;
        self.claude_turn = true;
        self.idle_since = None;
        if self.stopping.is_some() || self.state.status == SessionStatus::Paused {
            // A session that stops takes no more turns, and a paused one
            // waits for its usage limit to reset.
            if let Some(c) = self.claude.as_mut() {
                c.request_nowait(json!({ "subtype": "interrupt" }));
            }
        } else if self.state.status == SessionStatus::Idle {
            self.set_status(SessionStatus::Running, None);
        }
    }

    /// Claude says it is done: no turn is under way, and none comes for
    /// background work that has ended. A `result` ends each turn, but one
    /// Claude started on its own can end without. A session that waits
    /// starts its idle clock, unless background tasks still run.
    async fn claude_went_idle(&mut self) {
        self.claude_running = false;
        let ended = self.turn_active && self.claude_turn;
        if ended {
            self.turn_active = false;
            self.claude_turn = false;
        }
        let waits = match self.state.status {
            SessionStatus::Idle => true,
            SessionStatus::Running => ended,
            _ => false,
        };
        if waits && self.stopping.is_none() {
            self.wait().await;
        }
    }

    /// After each turn: git state, pushes and pull requests, in every
    /// repository, once the refresh that runs, if one does, has ended.
    fn after_turn(&mut self) {
        self.turn_ended = true;
        self.sync();
    }

    /// Starts the next refresh of the repositories' state, unless one runs:
    /// the one after a turn, or one while Claude works once it is due
    /// ([`Refresh`]).
    fn sync(&mut self) {
        let now = Instant::now();
        if self.syncing || self.exit_after.is_some() || !(self.turn_ended || self.refresh.due(now))
        {
            return;
        }
        let (Some(config), Some(paths)) = (self.config.clone(), self.paths.clone()) else {
            return;
        };
        let opened = self.refresh.start(now);
        let moment = if std::mem::take(&mut self.turn_ended) {
            Moment::TurnEnded
        } else {
            Moment::Working {
                opened_pr: opened.ran,
            }
        };
        if self
            .persisted
            .pulls
            .follow(opened.pull_requests, &config.spec)
        {
            self.persist();
        }
        self.syncing = true;
        let last = self.last_git.clone();
        let mut pulls = self.persisted.pulls.clone();
        let work = self.work_tx.clone();
        tokio::spawn(async move {
            let (push, diffs) = (config.push_on_commit, config.diffs);
            let spec = &config.spec;
            let last = last.as_ref();
            let sync = workspace::sync(&paths, spec, push, moment, last, &mut pulls, diffs);
            let state = match tokio::time::timeout(SYNC_TIMEOUT, sync).await {
                Ok(Ok(synced)) => {
                    for (level, text) in synced.logs {
                        let _ = work.send(Work::Log(level, text, LogSource::Agent));
                    }
                    Some((synced.state, pulls, synced.diffs))
                }
                Ok(Err(e)) => {
                    tracing::debug!("reading the repositories' state failed: {e:#}");
                    None
                }
                Err(_) => {
                    tracing::warn!("reading the repositories' state took too long; gave up");
                    None
                }
            };
            let _ = work.send(Work::Git(state, moment));
        });
    }

    async fn on_billing_guard(&mut self, guard: BillingGuard) {
        match guard {
            BillingGuard::ApiKey { source } => {
                let msg = format!(
                    "Claude reported an API key credential ({source}). fctl only runs Claude on subscriptions, so the session was stopped before it could bill an API account."
                );
                self.notice(NoticeLevel::Error, "api_key_refused", &msg, None);
                if let Some(mut c) = self.claude.take() {
                    c.kill().await;
                }
                self.set_status(SessionStatus::Failed, Some(msg));
                self.finish().await;
            }
            BillingGuard::Overage => {
                if let Some(c) = self.claude.as_mut() {
                    c.request_nowait(json!({ "subtype": "interrupt" }));
                }
                let until = now_ms() + 60 * 60 * 1000;
                self.resume_at = Some(until);
                self.notice(
                    NoticeLevel::Warn,
                    "overage_blocked",
                    "Claude started using paid extra usage, so fctl interrupted it. The session continues when the subscription limit resets.",
                    Some(until),
                );
                self.set_status(
                    SessionStatus::Paused,
                    Some("Paused to avoid paid extra usage".into()),
                );
            }
            BillingGuard::LimitReached { resets_at, limit } => {
                let until = resets_at.unwrap_or(now_ms() + 60 * 60 * 1000) + 30_000;
                self.resume_at = Some(until);
                let which = limit.unwrap_or_else(|| "usage".into()).replace('_', "-");
                self.notice(
                    NoticeLevel::Info,
                    "usage_limit",
                    &format!("The Claude {which} limit is reached. fctl continues the session when it resets."),
                    Some(until),
                );
                self.set_status(
                    SessionStatus::Paused,
                    Some("Waiting for the usage limit to reset".into()),
                );
            }
        }
    }

    fn on_claude_request(&mut self, request_id: String, request: Value) {
        let subtype = request
            .get("subtype")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match subtype {
            "oauth_token_refresh" => {
                let rid = self.request_id();
                self.pending_creds
                    .insert(rid.clone(), CredFor::ClaudeRefresh(request_id));
                self.send(&AgentToHost::CredentialRequest {
                    request_id: rid,
                    kind: CredentialKind::Claude,
                });
            }
            "can_use_tool" => {
                let input = request.get("input").cloned().unwrap_or_else(|| json!({}));
                if let Some(c) = &self.claude {
                    c.respond(
                        &request_id,
                        Ok(json!({ "behavior": "allow", "updatedInput": input })),
                    );
                }
            }
            other => {
                if let Some(c) = &self.claude {
                    c.respond(
                        &request_id,
                        Err(format!("fctl-agent does not handle {other}")),
                    );
                }
            }
        }
    }

    fn request_id(&mut self) -> String {
        self.next_request += 1;
        format!("cred_{}", self.next_request)
    }

    fn on_credentials(
        &mut self,
        request_id: Option<String>,
        claude: Option<ClaudeCredential>,
        github: Option<GithubCredential>,
        github_read: Option<GithubCredential>,
        error: Option<String>,
    ) {
        if let (Some(at), Some(paths)) =
            (expiry(github.as_ref(), github_read.as_ref()), &self.paths)
        {
            self.github_expires_at = at;
            if let Err(e) =
                workspace::write_github_tokens(paths, github.as_ref(), github_read.as_ref())
            {
                tracing::warn!("writing the GitHub tokens failed: {e:#}");
            }
        }
        if let Some(c) = &claude {
            self.claude_credential = Some(c.clone());
        }
        let Some(rid) = request_id else { return };
        match self.pending_creds.remove(&rid) {
            Some(CredFor::ClaudeRefresh(claude_req)) => {
                let answer = match (claude, error) {
                    (Some(c), _) => Ok(json!({ "accessToken": c.access_token })),
                    (None, Some(e)) => {
                        self.notice(
                            NoticeLevel::Error,
                            "credential_expired",
                            &format!("Claude's credential could not be refreshed: {e}"),
                            None,
                        );
                        Ok(json!({ "accessToken": null, "reason": "host_declined" }))
                    }
                    (None, None) => Ok(json!({ "accessToken": null })),
                };
                if let Some(c) = &self.claude {
                    c.respond(&claude_req, answer);
                }
            }
            Some(CredFor::GithubRefresh) => {
                if let Some(e) = error {
                    self.log(
                        LogSource::Agent,
                        LogLevel::Warn,
                        &format!("GitHub token refresh failed: {e}"),
                    );
                }
            }
            None => {}
        }
    }

    /// Takes the session over from a supervisor that died: stops the
    /// Claude it left running, which works on unseen and would share the
    /// conversation with the one this supervisor starts, and remembers a
    /// turn it cut off, which the transcript then tells of ([`Self::carry_on`]).
    pub async fn take_over(&mut self) {
        let left = self.persisted.claude.take();
        let ran_on = match left {
            Some(claude) => claude.stop(Duration::from_secs(5)).await,
            None => false,
        };
        if ran_on {
            tracing::warn!(
                "stopped Claude (pid {}), or what it started, which the supervisor before this one left running",
                left.map_or(0, |c| c.pid)
            );
        }
        // Only a turn under way was cut off: a session that waited has
        // nothing to carry on, whatever Claude left running (a dev server,
        // a watcher) or Claude itself, which ends once its input closes.
        let now = match (self.persisted.working, ran_on) {
            (false, _) => None,
            (true, true) => Some(CutOff::Unseen),
            (true, false) => Some(CutOff::Stopped),
        };
        // One that a supervisor before took over, and died before Claude
        // ran again, still counts.
        let cut_off = match (self.persisted.cut_off, now) {
            (Some(CutOff::Unseen), _) | (_, Some(CutOff::Unseen)) => Some(CutOff::Unseen),
            (before, now) => before.or(now),
        };
        if left.is_some() || self.persisted.working || cut_off != self.persisted.cut_off {
            self.persisted.working = false;
            self.persisted.cut_off = cut_off;
            self.persist();
        }
    }

    /// Says in the transcript that a turn of Claude's was cut off, and has
    /// Claude carry on with it, unless it did so for another one shortly
    /// before: then Claude waits for a message.
    async fn carry_on(&mut self, cut_off: CutOff) {
        // Said once: it was taken from the saved state.
        self.persist();
        let now = now_ms();
        let again = self
            .persisted
            .carried_on_at
            .is_some_and(|at| now.saturating_sub(at) < CARRY_ON_AGAIN_AFTER.as_millis() as i64);
        let what = match cut_off {
            CutOff::Unseen => {
                "fctl-agent restarted in the sandbox while Claude worked, so Claude's last steps aren't shown here, and fctl stopped that Claude to start it again on the conversation."
            }
            CutOff::Stopped => {
                "Claude's turn was cut off: fctl-agent stopped in the sandbox while Claude worked."
            }
        };
        let then = if again {
            "It was cut off shortly before too, so Claude waits for a message."
        } else {
            "Claude carries on with the turn, from where it left off."
        };
        self.notice(
            NoticeLevel::Warn,
            "turn_cut_off",
            &format!("{what} {then}"),
            None,
        );
        if again {
            return;
        }
        self.persisted.carried_on_at = Some(now);
        self.persist();
        let text = prompt::cut_off_turn(cut_off == CutOff::Unseen);
        self.send_turn(&text, None, None, &[]).await;
    }

    async fn on_claude_exit(&mut self, _code: Option<i32>) {
        let code = match self.claude.take() {
            Some(mut c) => tokio::time::timeout(Duration::from_secs(5), c.wait())
                .await
                .ok()
                .flatten(),
            None => return,
        };
        self.persisted.claude = None;
        self.persist();
        self.turn_active = false;
        self.claude_turn = false;
        // A restarted Claude Code doesn't follow the old one's background
        // tasks, and hasn't said what it does.
        self.tasks.take();
        self.claude_running = false;
        if self.stopping.is_some() || self.state.status.is_terminal() {
            return;
        }
        let now = Instant::now();
        self.restarts
            .retain(|t| now.duration_since(*t) < Duration::from_secs(600));
        if self.restarts.len() >= 3 {
            let msg = format!(
                "Claude exited (code {}) three times in ten minutes",
                code.map_or("?".into(), |c| c.to_string())
            );
            self.notice(NoticeLevel::Error, "claude_crashed", &msg, None);
            self.set_status(SessionStatus::Failed, Some(msg));
            self.finish().await;
            return;
        }
        self.restarts.push_back(now);
        self.notice(
            NoticeLevel::Warn,
            "claude_restarted",
            &format!(
                "Claude exited unexpectedly (code {}); resuming the conversation",
                code.map_or("?".into(), |c| c.to_string())
            ),
            None,
        );
        if let Err(e) = self.start_claude().await {
            let msg = format!("Claude could not restart: {e:#}");
            self.set_status(SessionStatus::Failed, Some(msg));
            self.finish().await;
        }
    }

    // ───────────────────────────── timers and stopping ─────────────────────────────

    /// A review agent that ended its turn without posting its review, with
    /// nothing running, is reminded once: its review would be lost.
    async fn remind_reviewer(&mut self) {
        let reviews = self
            .config
            .as_ref()
            .is_some_and(|c| c.spec.review.is_some());
        let waited = self
            .idle_since
            .is_some_and(|t| t.elapsed() >= REVIEW_REMINDER_AFTER);
        if !reviews
            || self.persisted.review_posted
            || self.persisted.review_nudged
            || !waited
            || !self.nothing_runs()
            || self.stopping.is_some()
            || self.claude.is_none()
        {
            return;
        }
        self.persisted.review_nudged = true;
        self.persist();
        self.notice(
            NoticeLevel::Info,
            "review_reminder",
            "The review agent ended its turn without posting its review, so fctl reminded it to.",
            None,
        );
        self.send_turn(prompt::REVIEW_REMINDER, None, None, &[])
            .await;
    }

    /// A review agent's session whose review was posted while it waited
    /// (the answer came late) or paused (a usage limit) stops then, as the
    /// end of the agent's turn stops it otherwise ([`Supervisor::wait`]).
    async fn stop_reviewed(&mut self) {
        let waits = matches!(
            self.state.status,
            SessionStatus::Idle | SessionStatus::Paused
        );
        if self.persisted.review_posted && waits && self.stopping.is_none() {
            self.begin_stop(REVIEW_POSTED.to_owned(), 30_000).await;
        }
    }

    async fn on_tick(&mut self) {
        let now = now_ms();
        self.expire_publishes();
        self.expire_calls();
        self.sync();
        self.remind_reviewer().await;
        self.stop_reviewed().await;
        // Usage limit reset: continue.
        if self.state.status == SessionStatus::Paused && self.resume_at.is_some_and(|t| now >= t) {
            self.resume_at = None;
            self.set_status(
                SessionStatus::Running,
                Some("Resuming after the usage limit reset".into()),
            );
            self.send_turn(
                "The usage limit has reset. Continue where you left off.",
                None,
                None,
                &[],
            )
            .await;
            while let Some(d) = self.queue.pop_front() {
                self.on_delivery(d).await;
            }
        }
        // GitHub tokens last an hour; refresh both ten minutes early.
        if self.github_expires_at - now < 10 * 60 * 1000 && self.host.is_some() {
            self.github_expires_at = now + 5 * 60 * 1000; // retry in five minutes if this fails
            let rid = self.request_id();
            self.pending_creds
                .insert(rid.clone(), CredFor::GithubRefresh);
            self.send(&AgentToHost::CredentialRequest {
                request_id: rid,
                kind: CredentialKind::Github,
            });
        }
        // The idle timeout, and the most background work may add to it:
        // the tasks Claude Code reports, or whatever it says it still runs
        // for once its turns have ended.
        if let Some(idle) = self.config.as_ref().map(|c| tasks::idle_timeout(&c.spec))
            && self.stopping.is_none()
        {
            let limit = tasks::limit(idle);
            if self.idle_since.is_some_and(|t| t.elapsed() > idle) {
                // A review agent's session ends when its review is done:
                // one that posted it stopped already.
                let reason = match &self.config {
                    Some(c) if c.spec.review.is_some() => "Finished without posting its review",
                    _ => stop_text(StopReason::Idle),
                };
                self.begin_stop(reason.to_owned(), 30_000).await;
            } else if (!self.tasks.is_empty() || self.claude_running)
                && self.waiting_since.is_some_and(|t| t.elapsed() > limit)
            {
                self.begin_stop(tasks::limit_reason(limit), 30_000).await;
            }
        }
        // A stop whose grace period ran out.
        if let Some((_, deadline)) = self.stopping
            && Instant::now() >= deadline
            && self.exit_after.is_none()
        {
            self.finish().await;
        }
    }

    /// Stops the session for the reason its status gives (`why`): once the
    /// turn under way ends, interrupted, or after `grace_ms` anyway.
    async fn begin_stop(&mut self, why: String, grace_ms: u64) {
        if self.stopping.is_some() {
            return;
        }
        let deadline = Instant::now() + Duration::from_millis(grace_ms.clamp(1_000, 120_000));
        self.stopping = Some((why.clone(), deadline));
        self.set_status(SessionStatus::Stopping, Some(why));
        if self.turn_active {
            if let Some(c) = self.claude.as_mut() {
                c.request_nowait(json!({ "subtype": "interrupt" }));
            }
        } else {
            self.finish().await;
        }
    }

    /// Stops Claude, records the final repository state, and exits soon.
    async fn finish(&mut self) {
        if self.exit_after.is_some() {
            return;
        }
        // Claude's background work stops with it: the transcript says what,
        // and so does the next message, for Claude.
        let unfinished = self.tasks.take();
        if !unfinished.is_empty() {
            let notice = tasks::stopped_notice(&unfinished);
            self.notice(NoticeLevel::Warn, "background_tasks_stopped", &notice, None);
            for t in unfinished {
                if !self.persisted.unfinished.iter().any(|u| u.id == t.id) {
                    self.persisted.unfinished.push(t);
                }
            }
            self.persist();
        }
        if let Some(mut c) = self.claude.take() {
            c.kill().await;
        }
        // A session stopped or failed doesn't carry on with a turn cut off
        // before.
        let cut_off = self.persisted.cut_off.take();
        if self.persisted.claude.take().is_some() || cut_off.is_some() {
            self.persist();
        }
        if let (Some(config), Some(paths)) = (self.config.clone(), self.paths.clone()) {
            let (push, last) = (config.push_on_commit, self.last_git.as_ref());
            // The pull requests found before still stand: only a pushed
            // branch's that it follows none of is looked up, which a turn
            // cut short may not have reported.
            let mut pulls = self.persisted.pulls.clone();
            let stopping = Moment::Stopping;
            let diffs = config.diffs;
            if let Ok(synced) = workspace::sync(
                &paths,
                &config.spec,
                push,
                stopping,
                last,
                &mut pulls,
                diffs,
            )
            .await
            {
                if pulls != self.persisted.pulls {
                    self.persisted.pulls = pulls;
                    self.persist();
                }
                for (level, text) in synced.logs {
                    self.log(LogSource::Agent, level, &text);
                }
                if let Some(files) = synced.diffs {
                    self.send_diffs(files);
                }
                self.emit(EventBody::Git(synced.state));
            }
        }
        let (status, reason) = match &self.stopping {
            Some((why, _)) => (SessionStatus::Stopped, Some(why.clone())),
            None if self.state.status == SessionStatus::Failed => {
                (SessionStatus::Failed, self.state.reason.clone())
            }
            None => (SessionStatus::Stopped, Some("Stopped".into())),
        };
        self.set_status(status, reason.clone());
        self.flush();
        self.send(&AgentToHost::Exited { status, reason });
        // Give the host a moment to read everything.
        self.exit_after = Some(Instant::now() + Duration::from_secs(3));
    }
}

/// When the first of a session's GitHub tokens expires.
fn expiry(write: Option<&GithubCredential>, read: Option<&GithubCredential>) -> Option<i64> {
    write.iter().chain(read.iter()).map(|c| c.expires_at).min()
}

/// Whether the sandbox's `fctl` has a command (`edit`, `publish`, `retro
/// update`): custom images may have none, and older images an older one.
async fn fctl_can(paths: &Paths, command: &str) -> bool {
    let args: Vec<&str> = command.split_whitespace().chain(["--help"]).collect();
    let help = workspace::run(&paths.home, &paths.home, "fctl", &args);
    matches!(
        tokio::time::timeout(Duration::from_secs(10), help).await,
        Ok(Ok(out)) if out.ok
    )
}

/// What the transcript says about a published screenshot: the link is in
/// the message for clients from before `published`.
fn published_notice(file: &PublishedFile, now: i64) -> Notice {
    let days = ((file.expires_at - now) as f64 / 86_400_000.0)
        .round()
        .max(1.0) as i64;
    let lasts = if days == 1 {
        "a day".to_owned()
    } else {
        format!("{days} days")
    };
    Notice {
        ticket: None,
        level: NoticeLevel::Info,
        code: "screenshot_published".into(),
        message: format!(
            "Published {} for {lasts}, for anyone with the link: {}",
            file.name, file.url
        ),
        until: None,
        published: Some(file.clone()),
        memory: None,
        retrospective: None,
        review: None,
        summarized: None,
        follow_up: None,
    }
}

/// What the transcript says about what the agent kept: the note it
/// remembered (and those it took the place of), or marked stale (a
/// duplicate too), or the retrospective it recorded, or updated
/// (`retrospective_updated`, with the retrospective as it now is). The
/// message stands alone for clients from before the notice's `memory` and
/// `retrospective`.
fn kept_notices(answer: &AgentAnswer) -> Vec<Notice> {
    let notice = |code: &str, message: String| Notice {
        ticket: None,
        level: NoticeLevel::Info,
        code: code.into(),
        message,
        until: None,
        published: None,
        memory: None,
        retrospective: None,
        review: None,
        summarized: None,
        follow_up: None,
    };
    match answer {
        AgentAnswer::Memories { .. } | AgentAnswer::Retrospectives { .. } => vec![],
        AgentAnswer::Memory {
            memory,
            replaced,
            also_replaced,
        } => {
            let mut notices = Vec::new();
            if memory.stale.is_some() {
                notices.push(Notice {
                    memory: Some(memory.clone()),
                    ..notice("memory_stale", fctl_domain::memory::stale_message(memory))
                });
            } else {
                notices.push(Notice {
                    memory: Some(memory.clone()),
                    ..notice("memory_added", fctl_domain::memory::added_message(memory))
                });
                for old in replaced.iter().chain(also_replaced) {
                    notices.push(Notice {
                        memory: Some(old.clone()),
                        ..notice("memory_stale", fctl_domain::memory::stale_message(old))
                    });
                }
            }
            notices
        }
        AgentAnswer::Retrospective { retrospective } => vec![Notice {
            retrospective: Some(retrospective.clone()),
            ..notice(
                if retrospective.updated_at.is_some() {
                    "retrospective_updated"
                } else {
                    "retrospective"
                },
                fctl_domain::retrospectives::message(retrospective),
            )
        }],
        // The tool call shows it.
        AgentAnswer::Thread { .. } | AgentAnswer::ReviewThreads { .. } => vec![],
        AgentAnswer::ReviewPosted { review } if !review.again => vec![Notice {
            review: Some(review.clone()),
            ..notice("review_posted", posted_message(review))
        }],
        AgentAnswer::ReviewPosted { .. } => vec![],
        AgentAnswer::PullRequestSummarized { summarized } => vec![Notice {
            summarized: Some(summarized.clone()),
            ..notice("pull_request_summarized", summarized_message(summarized))
        }],
        // The control plane adds what fctl did in the tracker itself.
        AgentAnswer::Ticket { .. } => vec![],
    }
}

/// What the transcript says fctl posted for a review agent.
fn posted_message(r: &fctl_proto::api::PostedReview) -> String {
    let findings = r.comments + r.in_text;
    let found = match findings {
        0 => "nothing to point out".to_owned(),
        1 => "1 issue".to_owned(),
        n => format!("{n} issues"),
    };
    let mut s = format!("fctl posted the review: {found}");
    if let Some(url) = &r.url {
        s.push_str(&format!(" ({url})"));
    }
    s.push('.');
    match r.resolved.len() {
        0 => {}
        1 => s.push_str(" It marked 1 earlier finding fixed."),
        n => s.push_str(&format!(" It marked {n} earlier findings fixed.")),
    }
    match r.still_open {
        0 => {}
        1 => s.push_str(" 1 earlier finding is still open."),
        n => s.push_str(&format!(" {n} earlier findings are still open.")),
    }
    if r.sent_to.is_some() {
        s.push_str(
            " fctl sent the findings to the agent of the session that opened the pull request.",
        );
    }
    s
}

/// What the transcript says of the summary fctl put in a reviewed pull
/// request's description.
fn summarized_message(s: &fctl_proto::api::SummarizedPullRequest) -> String {
    format!(
        "fctl put the review agent's summary at the end of the description of {}#{} ({}){}: {} risk.",
        s.repo.full_name(),
        s.number,
        s.url,
        if s.replaced {
            ", in place of an earlier review's"
        } else {
            ""
        },
        s.summary.risk.as_str()
    )
}

/// Asks Claude which effort level it works at; the answer comes back as
/// [`Work::Effort`]. Claude Code before `get_settings` says nothing.
fn read_effort(claude: &mut Claude, work: &mpsc::UnboundedSender<Work>) {
    let answer = claude.ask(
        json!({ "subtype": "get_settings" }),
        Duration::from_secs(30),
    );
    let work = work.clone();
    tokio::spawn(async move {
        match answer.await {
            Ok(settings) => {
                if let Some(effort) = claude::applied_effort(&settings) {
                    let _ = work.send(Work::Effort(effort));
                }
            }
            Err(e) => tracing::debug!("get_settings: {e}"),
        }
    });
}

/// Asks Claude which context window it works in; the answer comes back as
/// [`Work::Context`]. A `summary` comes from the last response's usage and
/// Claude's own estimates, without asking the API to count each part of the
/// context. Claude Code before `get_context_usage` says nothing.
fn read_context(claude: &mut Claude, work: &mpsc::UnboundedSender<Work>) {
    let answer = claude.ask(
        json!({ "subtype": "get_context_usage", "detail": "summary" }),
        Duration::from_secs(30),
    );
    let work = work.clone();
    tokio::spawn(async move {
        match answer.await {
            Ok(usage) => {
                if let Some(window) = claude::context_window(&usage) {
                    let _ = work.send(Work::Context(window));
                }
            }
            Err(e) => tracing::debug!("get_context_usage: {e}"),
        }
    });
}

fn stop_text(r: StopReason) -> &'static str {
    match r {
        StopReason::User => "Stopped",
        StopReason::Idle => "Stopped after being idle",
        StopReason::Deleted => "Stopped because the session was deleted",
        StopReason::Shutdown => "Stopped because the host is shutting down",
    }
}

/// Used by `main` to run the supervisor and its sockets.
pub async fn serve(socket: PathBuf, state_dir: PathBuf) -> Result<()> {
    // The lock is held until the supervisor exits.
    let (listener, _lock) = match link::claim(&socket).await? {
        link::Claim::Serving(listener, lock) => (listener, lock),
        link::Claim::Taken => {
            tracing::info!(
                "fctl-agent is already serving {}; this one exits",
                socket.display()
            );
            return Ok(());
        }
    };
    let session_socket = local::socket_path(&socket);
    let user_state = state_dir.clone();
    serving(listener, socket, state_dir, session_socket, user_state).await
}

/// [`serve`], as root, with the host's socket already bound
/// ([`crate::protected`]).
#[cfg(target_os = "linux")]
pub async fn serve_protected(started: crate::protected::Started) -> Result<()> {
    let crate::protected::Started {
        listener,
        lock: _lock,
        socket,
        state,
        session_socket,
        user_state,
    } = started;
    let listener = tokio::net::UnixListener::from_std(listener)?;
    serving(listener, socket, state, session_socket, user_state).await
}

async fn serving(
    listener: tokio::net::UnixListener,
    socket: PathBuf,
    state_dir: PathBuf,
    session_socket: PathBuf,
    user_state: PathBuf,
) -> Result<()> {
    let (link_tx, link_rx) = mpsc::unbounded_channel();
    tokio::spawn(link::accept_loop(listener, link_tx));
    // Without the session socket the agent cannot rename the session, but
    // the session still runs.
    let (local_tx, local_rx) = mpsc::unbounded_channel();
    let session_socket = match link::bind(&session_socket).await {
        Ok(listener) => {
            tokio::spawn(local::accept_loop(listener, local_tx));
            Some(session_socket)
        }
        Err(e) => {
            tracing::warn!("no session socket: {e:#}");
            None
        }
    };
    let (mut sup, claude_rx, work_rx) = Supervisor::new(state_dir, session_socket.clone())?;
    sup.user_state = user_state;
    sup.take_over().await;
    tracing::info!("fctl-agent {VERSION} serving {}", socket.display());
    let r = sup.run(link_rx, claude_rx, work_rx, local_rx).await;
    let _ = user::as_root(|| std::fs::remove_file(&socket));
    if let Some(s) = session_socket {
        let _ = std::fs::remove_file(s);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the supervisor answers right away (`None`: it waits for the host).
    fn ask(sup: &mut Supervisor, request: SessionRequest) -> Option<SessionReply> {
        let (tx, mut rx) = oneshot::channel();
        sup.on_session_call(local::Call { request, reply: tx });
        rx.try_recv().ok()
    }

    fn edit(title: Option<&str>, description: Option<&str>) -> SessionRequest {
        SessionRequest::Edit {
            session_id: None,
            title: title.map(str::to_owned),
            description: description.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn the_agent_names_and_describes_its_session() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let state_dir = dir.path().join("state");
        let (mut sup, _claude, _work) = Supervisor::new(state_dir.clone(), None)?;
        let early = ask(&mut sup, edit(Some("Too early"), None));
        assert!(matches!(early, Some(SessionReply::Error { .. })));

        sup.config = Some(AgentConfig {
            spec: crate::workspace::tests::spec()?,
            workdir: dir.path().join("workspace").display().to_string(),
            home: None,
            claude_bin: None,
            claude_args: vec![],
            env: Default::default(),
            push_on_commit: true,
            first_agent_seq: None,
            sandbox: None,
            cache: None,
            diffs: false,
        });
        let reply = ask(
            &mut sup,
            edit(
                Some("  Fix the   login test "),
                Some("Retries the redirect.\nAdds a test."),
            ),
        );
        let Some(SessionReply::Edited {
            id,
            number,
            title,
            description,
            ..
        }) = reply
        else {
            panic!("expected Edited, got {reply:?}");
        };
        assert_eq!((id.as_str(), number), ("ses_test", 1));
        assert_eq!(title, "Fix the login test");
        assert_eq!(
            description.as_deref(),
            Some("Retries the redirect. Adds a test.")
        );
        assert_eq!(sup.state.title.as_deref(), Some("Fix the login test"));

        // An empty description clears it; the title stays.
        let cleared = ask(&mut sup, edit(None, Some("")));
        assert!(matches!(
            cleared,
            Some(SessionReply::Edited { ref title, description: None, .. }) if title == "Fix the login test"
        ));
        assert_eq!(sup.state.description.as_deref(), Some(""));

        // A restarted supervisor keeps reporting them.
        let (again, _claude, _work) = Supervisor::new(state_dir, None)?;
        assert_eq!(again.state.title.as_deref(), Some("Fix the login test"));

        let invalid = ask(&mut sup, edit(Some(" "), None));
        assert!(
            matches!(invalid, Some(SessionReply::Error { ref error }) if error.field.as_deref() == Some("title"))
        );
        let elsewhere = ask(
            &mut sup,
            SessionRequest::Edit {
                session_id: Some(SessionId("ses_other".into())),
                title: Some("Not mine".into()),
                description: None,
            },
        );
        assert!(matches!(elsewhere, Some(SessionReply::Error { .. })));
        assert_eq!(sup.state.title.as_deref(), Some("Fix the login test"));
        Ok(())
    }

    fn config(dir: &std::path::Path, publish: bool) -> anyhow::Result<AgentConfig> {
        let mut spec = crate::workspace::tests::spec()?;
        spec.project.settings.publish_screenshots = publish;
        Ok(AgentConfig {
            spec,
            workdir: dir.join("workspace").display().to_string(),
            home: None,
            claude_bin: None,
            claude_args: vec![],
            env: Default::default(),
            push_on_commit: true,
            first_agent_seq: None,
            sandbox: None,
            cache: None,
            diffs: false,
        })
    }

    fn png() -> String {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend_from_slice(&1440u32.to_be_bytes());
        b.extend_from_slice(&900u32.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        STANDARD.encode(b)
    }

    fn publish(data: String) -> SessionRequest {
        SessionRequest::Publish {
            session_id: Some(SessionId("ses_test".into())),
            name: "shots/after.png".into(),
            data,
        }
    }

    #[tokio::test]
    async fn publishes_screenshots_through_the_host() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        sup.config = Some(config(dir.path(), true)?);
        let (host_tx, mut host_rx) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));

        let (tx, mut rx) = oneshot::channel();
        sup.on_session_call(local::Call {
            request: publish(png()),
            reply: tx,
        });
        // The host uploads it; the caller waits.
        let AgentToHost::Publish {
            request_id, name, ..
        } = serde_json::from_str(&host_rx.try_recv()?)?
        else {
            panic!("expected a publish frame");
        };
        assert_eq!(name, "after.png");
        assert!(rx.try_recv().is_err());

        let file = PublishedFile {
            url: "https://fctl.test/api/v1/published/abc/after.png".into(),
            name: "after.png".into(),
            media_type: "image/png".into(),
            size: 33,
            width: Some(1440),
            height: Some(900),
            expires_at: now_ms() + 30 * 86_400_000,
        };
        sup.on_frame(HostToAgent::Published {
            request_id: request_id.clone(),
            file: Some(file.clone()),
            error: None,
        })
        .await;
        assert!(matches!(rx.try_recv(), Ok(SessionReply::Published { file: ref f }) if *f == file));
        // The transcript says so, with the link.
        let AgentToHost::Events { events } = serde_json::from_str(&host_rx.try_recv()?)? else {
            panic!("expected events");
        };
        let EventBody::Notice(n) = &events[0].body else {
            panic!("expected a notice");
        };
        assert_eq!(n.code, "screenshot_published");
        assert_eq!(n.published.as_ref(), Some(&file));
        assert_eq!(
            n.message,
            "Published after.png for 30 days, for anyone with the link: https://fctl.test/api/v1/published/abc/after.png"
        );
        // A late or repeated answer goes nowhere.
        sup.on_published(&request_id, Some(file), None);
        assert!(host_rx.try_recv().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn diffs_go_to_the_host_before_the_state_and_again_to_a_new_one() -> anyhow::Result<()> {
        use crate::followed::Pulls;
        use crate::workspace::Moment;
        let dir = tempfile::tempdir()?;
        let (mut sup, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        let mut config = config(dir.path(), false)?;
        config.diffs = true;
        sup.config = Some(config);
        let (host_tx, mut host_rx) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        let mut diffs = vec![fctl_domain::diffs::untracked(
            "notes.md",
            b"# Notes\n",
            true,
        )];
        fctl_domain::diffs::fit(&mut diffs);
        let state = GitState {
            files: vec![fctl_proto::event::FileChange {
                path: "notes.md".into(),
                status: "untracked".into(),
                additions: Some(1),
                deletions: Some(0),
                digest: Some(diffs[0].digest.clone()),
            }],
            ..GitState::default()
        };
        let synced = |d: &Vec<FileDiff>| {
            let found = (state.clone(), Pulls::default(), Some(d.clone()));
            Work::Git(Some(found), Moment::TurnEnded)
        };
        sup.on_work(synced(&diffs)).await;
        sup.flush();
        let frames: Vec<AgentToHost> = std::iter::from_fn(|| host_rx.try_recv().ok())
            .map(|f| serde_json::from_str(&f))
            .collect::<Result<_, _>>()?;
        assert!(
            matches!(&frames[..], [AgentToHost::Diff { files }, AgentToHost::Events { events }]
                if *files == diffs && matches!(events[0].body, EventBody::Git(_))),
            "{frames:?}"
        );
        // The same again: nothing new for the host.
        sup.on_work(synced(&diffs)).await;
        sup.flush();
        let frames: Vec<String> = std::iter::from_fn(|| host_rx.try_recv().ok()).collect();
        assert!(
            frames.iter().all(|f| !f.contains(r#""type":"diff""#)),
            "{frames:?}"
        );
        // A host that attaches gets them again, after the state.
        sup.on_frame(HostToAgent::Replay {
            after_agent_seq: u64::MAX,
        })
        .await;
        let frames: Vec<AgentToHost> = std::iter::from_fn(|| host_rx.try_recv().ok())
            .map(|f| serde_json::from_str(&f))
            .collect::<Result<_, _>>()?;
        assert!(
            matches!(&frames[..], [AgentToHost::State { .. }, AgentToHost::Diff { files }] if *files == diffs),
            "{frames:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn publishes_only_where_the_project_does_and_only_images() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        let (host_tx, mut host_rx) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        sup.config = Some(config(dir.path(), false)?);
        let off = ask(&mut sup, publish(png()));
        assert!(
            matches!(off, Some(SessionReply::Error { ref error }) if error.message.contains("doesn't publish screenshots"))
        );
        sup.config = Some(config(dir.path(), true)?);
        let svg = ask(&mut sup, publish(STANDARD.encode("<svg/>")));
        assert!(
            matches!(svg, Some(SessionReply::Error { ref error }) if error.field.as_deref() == Some("file"))
        );
        let garbled = ask(&mut sup, publish("not base64!".into()));
        assert!(matches!(garbled, Some(SessionReply::Error { .. })));
        assert!(host_rx.try_recv().is_err());

        // Without a host there is nobody to upload it.
        sup.host = None;
        let alone = ask(&mut sup, publish(png()));
        assert!(
            matches!(alone, Some(SessionReply::Error { ref error }) if error.code == fctl_proto::ErrorCode::Unavailable)
        );
        Ok(())
    }

    #[tokio::test]
    async fn gives_up_on_a_host_that_does_not_answer() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        sup.config = Some(config(dir.path(), true)?);
        let (host_tx, mut host_rx) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        let (tx, mut rx) = oneshot::channel();
        sup.on_session_call(local::Call {
            request: publish(png()),
            reply: tx,
        });
        sup.expire_publishes();
        assert!(rx.try_recv().is_err());
        for (_, since) in sup.publishing.values_mut() {
            *since = Instant::now() - PUBLISH_TIMEOUT - Duration::from_secs(1);
        }
        sup.expire_publishes();
        assert!(
            matches!(rx.try_recv(), Ok(SessionReply::Error { ref error }) if error.message.contains("in time"))
        );
        assert!(sup.publishing.is_empty());
        // The host publishes it after all: the transcript says so, once.
        let file = PublishedFile {
            url: "https://fctl.test/api/v1/published/abc/after.png".into(),
            name: "after.png".into(),
            media_type: "image/png".into(),
            size: 33,
            width: None,
            height: None,
            expires_at: now_ms() + 86_400_000,
        };
        let AgentToHost::Publish { request_id, .. } = serde_json::from_str(&host_rx.try_recv()?)?
        else {
            panic!("expected a publish frame");
        };
        sup.on_published(&request_id, Some(file.clone()), None);
        let AgentToHost::Events { events } = serde_json::from_str(&host_rx.try_recv()?)? else {
            panic!("expected events");
        };
        assert!(
            matches!(&events[0].body, EventBody::Notice(n) if n.message.starts_with("Published after.png for a day"))
        );
        sup.on_published(&request_id, Some(file), None);
        assert!(host_rx.try_recv().is_err());
        Ok(())
    }

    /// A supervisor of a project that keeps a memory and asks for
    /// retrospectives (`memory`, `retros`), with a host link.
    fn keeping(
        dir: &std::path::Path,
        memory: bool,
        retros: bool,
    ) -> anyhow::Result<(Supervisor, mpsc::UnboundedReceiver<String>)> {
        let (mut sup, _claude, _work) = Supervisor::new(dir.join("state"), None)?;
        let mut config = config(dir, false)?;
        config.spec.memory = memory.then(fctl_proto::runner::MemorySpec::default);
        config.spec.retrospectives = retros;
        sup.config = Some(config);
        let (host_tx, host_rx) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        Ok((sup, host_rx))
    }

    fn calling(call: AgentCall) -> SessionRequest {
        SessionRequest::Call {
            session_id: Some(SessionId("ses_test".into())),
            call,
        }
    }

    fn note(id: &str, text: &str, stale: Option<&str>) -> fctl_proto::session::Memory {
        fctl_proto::session::Memory {
            id: fctl_proto::MemoryId(id.into()),
            project_id: fctl_proto::ProjectId("prj_test".into()),
            text: text.into(),
            session: None,
            created_by: None,
            created_at: 1,
            edited_at: None,
            edited_by: None,
            stale: stale.map(|reason| fctl_proto::session::Staleness {
                at: 2,
                reason: Some(reason.into()),
                session: None,
                by: None,
                replaced_by: None,
                duplicate_of: None,
            }),
        }
    }

    /// The notices the agent sent the host.
    fn notices(host_rx: &mut mpsc::UnboundedReceiver<String>) -> anyhow::Result<Vec<Notice>> {
        let AgentToHost::Events { events } = serde_json::from_str(&host_rx.try_recv()?)? else {
            panic!("expected events");
        };
        Ok(events
            .into_iter()
            .filter_map(|e| match e.body {
                EventBody::Notice(n) => Some(n),
                _ => None,
            })
            .collect())
    }

    #[tokio::test]
    async fn keeps_the_projects_memory_through_the_host() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host_rx) = keeping(dir.path(), true, false)?;
        let (tx, mut rx) = oneshot::channel();
        sup.on_session_call(local::Call {
            request: calling(AgentCall::Remember {
                text: "Run npm ci in web/ first".into(),
                replaces: Some(fctl_proto::MemoryId("mem_old".into())),
                also_replaces: vec![fctl_proto::MemoryId("mem_older".into())],
            }),
            reply: tx,
        });
        // The host asks the control plane; the caller waits.
        let AgentToHost::Call { request_id, call } = serde_json::from_str(&host_rx.try_recv()?)?
        else {
            panic!("expected a call frame");
        };
        assert!(
            matches!(call, AgentCall::Remember { ref text, ref also_replaces, .. } if text == "Run npm ci in web/ first" && also_replaces.len() == 1)
        );
        assert!(rx.try_recv().is_err());
        let answer = AgentAnswer::Memory {
            memory: note("mem_new", "Run npm ci in web/ first", None),
            replaced: Some(note(
                "mem_old",
                "Run npm install",
                Some("Replaced by mem_new"),
            )),
            also_replaced: vec![note("mem_older", "Run yarn", Some("Replaced by mem_new"))],
        };
        sup.on_frame(HostToAgent::Answer {
            request_id: request_id.clone(),
            answer: Some(answer.clone()),
            error: None,
        })
        .await;
        assert!(
            matches!(rx.try_recv(), Ok(SessionReply::Answer { answer: ref a }) if *a == answer)
        );
        // The transcript says what was remembered, and what it replaced.
        let n = notices(&mut host_rx)?;
        assert_eq!(n.len(), 3);
        assert_eq!(n[0].code, "memory_added");
        assert_eq!(n[0].message, "Remembered: Run npm ci in web/ first");
        assert_eq!(n[0].memory.as_ref().map(|m| m.id.as_str()), Some("mem_new"));
        assert_eq!(n[1].code, "memory_stale");
        assert_eq!(
            n[1].message,
            "Marked a memory stale: Run npm install (Replaced by mem_new)"
        );
        assert_eq!(n[2].code, "memory_stale");
        assert_eq!(
            n[2].memory.as_ref().map(|m| m.id.as_str()),
            Some("mem_older")
        );
        // A late or repeated answer goes nowhere.
        sup.on_answer(&request_id, Some(answer), None);
        assert!(host_rx.try_recv().is_err());

        // Reading the memory says nothing in the transcript.
        let (tx, mut rx) = oneshot::channel();
        sup.on_session_call(local::Call {
            request: calling(AgentCall::Memories { stale: true }),
            reply: tx,
        });
        let AgentToHost::Call { request_id, .. } = serde_json::from_str(&host_rx.try_recv()?)?
        else {
            panic!("expected a call frame");
        };
        sup.on_answer(
            &request_id,
            Some(AgentAnswer::Memories { memories: vec![] }),
            None,
        );
        assert!(matches!(rx.try_recv(), Ok(SessionReply::Answer { .. })));
        assert!(host_rx.try_recv().is_err());

        // What the control plane refuses, the caller hears.
        let (tx, mut rx) = oneshot::channel();
        sup.on_session_call(local::Call {
            request: calling(AgentCall::MarkStale {
                id: fctl_proto::MemoryId("mem_gone".into()),
                reason: "Moved".into(),
                duplicate_of: None,
            }),
            reply: tx,
        });
        let AgentToHost::Call { request_id, .. } = serde_json::from_str(&host_rx.try_recv()?)?
        else {
            panic!("expected a call frame");
        };
        sup.on_answer(&request_id, None, Some(ApiError::not_found("Memory")));
        assert!(
            matches!(rx.try_recv(), Ok(SessionReply::Error { ref error }) if error.code == fctl_proto::ErrorCode::NotFound)
        );

        // A duplicate goes stale, and the transcript says which note it
        // repeats.
        let (tx, mut rx) = oneshot::channel();
        sup.on_session_call(local::Call {
            request: calling(AgentCall::MarkStale {
                id: fctl_proto::MemoryId("mem_copy".into()),
                reason: "Duplicate of mem_keep".into(),
                duplicate_of: Some(fctl_proto::MemoryId("mem_keep".into())),
            }),
            reply: tx,
        });
        let AgentToHost::Call { request_id, call } = serde_json::from_str(&host_rx.try_recv()?)?
        else {
            panic!("expected a call frame");
        };
        assert!(
            matches!(call, AgentCall::MarkStale { duplicate_of: Some(ref of), .. } if of.as_str() == "mem_keep")
        );
        let mut copy = note(
            "mem_copy",
            "Run npm ci first",
            Some("Duplicate of mem_keep"),
        );
        if let Some(stale) = copy.stale.as_mut() {
            stale.duplicate_of = Some(fctl_proto::MemoryId("mem_keep".into()));
        }
        sup.on_answer(
            &request_id,
            Some(AgentAnswer::Memory {
                memory: copy,
                replaced: None,
                also_replaced: vec![],
            }),
            None,
        );
        assert!(matches!(rx.try_recv(), Ok(SessionReply::Answer { .. })));
        let n = notices(&mut host_rx)?;
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].code, "memory_stale");
        assert_eq!(
            n[0].message,
            "Marked a memory a duplicate of mem_keep: Run npm ci first"
        );
        Ok(())
    }

    #[tokio::test]
    async fn records_retrospectives_through_the_host() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host_rx) = keeping(dir.path(), false, true)?;
        let (tx, mut rx) = oneshot::channel();
        sup.on_session_call(local::Call {
            request: calling(AgentCall::Retrospective {
                outcome: fctl_proto::session::RetrospectiveOutcome::Partial,
                worked_on: "Retried failed uploads.".into(),
                approach: None,
                challenges: None,
                insights: None,
            }),
            reply: tx,
        });
        let AgentToHost::Call { request_id, .. } = serde_json::from_str(&host_rx.try_recv()?)?
        else {
            panic!("expected a call frame");
        };
        let retrospective = fctl_proto::session::Retrospective {
            id: fctl_proto::RetrospectiveId("rtr_1".into()),
            project_id: fctl_proto::ProjectId("prj_test".into()),
            session: fctl_proto::session::SessionRef {
                id: SessionId("ses_test".into()),
                number: 1,
                title: "Retry uploads".into(),
            },
            started_by: None,
            created_at: 5,
            updated_at: None,
            outcome: fctl_proto::session::RetrospectiveOutcome::Partial,
            worked_on: "Retried failed uploads.".into(),
            approach: None,
            challenges: None,
            insights: None,
        };
        sup.on_answer(
            &request_id,
            Some(AgentAnswer::Retrospective {
                retrospective: retrospective.clone(),
            }),
            None,
        );
        assert!(matches!(rx.try_recv(), Ok(SessionReply::Answer { .. })));
        let n = notices(&mut host_rx)?;
        assert_eq!(n[0].code, "retrospective");
        assert_eq!(
            n[0].message,
            "Recorded a retrospective (partly done): Retried failed uploads."
        );
        assert_eq!(n[0].retrospective.as_ref(), Some(&retrospective));

        // A later turn finishes the work and updates it: the transcript
        // shows it as it now is.
        let (tx, mut rx) = oneshot::channel();
        sup.on_session_call(local::Call {
            request: calling(AgentCall::UpdateRetrospective {
                id: retrospective.id.clone(),
                outcome: Some(fctl_proto::session::RetrospectiveOutcome::Completed),
                worked_on: None,
                approach: None,
                challenges: None,
                insights: None,
            }),
            reply: tx,
        });
        let AgentToHost::Call { request_id, call } = serde_json::from_str(&host_rx.try_recv()?)?
        else {
            panic!("expected a call frame");
        };
        assert!(matches!(call, AgentCall::UpdateRetrospective { .. }));
        let updated = fctl_proto::session::Retrospective {
            updated_at: Some(9),
            outcome: fctl_proto::session::RetrospectiveOutcome::Completed,
            ..retrospective
        };
        sup.on_answer(
            &request_id,
            Some(AgentAnswer::Retrospective {
                retrospective: updated.clone(),
            }),
            None,
        );
        assert!(matches!(rx.try_recv(), Ok(SessionReply::Answer { .. })));
        let n = notices(&mut host_rx)?;
        assert_eq!(n[0].code, "retrospective_updated");
        assert_eq!(
            n[0].message,
            "Updated a retrospective (completed): Retried failed uploads."
        );
        assert_eq!(n[0].retrospective.as_ref(), Some(&updated));

        // Listing them keeps nothing new, so the transcript stays as it is.
        let (tx, mut rx) = oneshot::channel();
        sup.on_session_call(local::Call {
            request: calling(AgentCall::Retrospectives),
            reply: tx,
        });
        let AgentToHost::Call { request_id, .. } = serde_json::from_str(&host_rx.try_recv()?)?
        else {
            panic!("expected a call frame");
        };
        sup.on_answer(
            &request_id,
            Some(AgentAnswer::Retrospectives {
                retrospectives: vec![updated],
            }),
            None,
        );
        assert!(matches!(rx.try_recv(), Ok(SessionReply::Answer { .. })));
        assert!(host_rx.try_recv().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn calls_only_for_what_the_project_turned_on() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host_rx) = keeping(dir.path(), false, false)?;
        let list = ask(&mut sup, calling(AgentCall::Memories { stale: false }));
        assert!(
            matches!(list, Some(SessionReply::Error { ref error }) if error.message.contains("keeps no memory"))
        );
        let retro = ask(
            &mut sup,
            calling(AgentCall::Retrospective {
                outcome: fctl_proto::session::RetrospectiveOutcome::Completed,
                worked_on: "Work".into(),
                approach: None,
                challenges: None,
                insights: None,
            }),
        );
        assert!(
            matches!(retro, Some(SessionReply::Error { ref error }) if error.message.contains("doesn't ask for retrospectives"))
        );
        for call in [
            AgentCall::Retrospectives,
            AgentCall::UpdateRetrospective {
                id: fctl_proto::RetrospectiveId("rtr_1".into()),
                outcome: Some(fctl_proto::session::RetrospectiveOutcome::Completed),
                worked_on: None,
                approach: None,
                challenges: None,
                insights: None,
            },
        ] {
            let refused = ask(&mut sup, calling(call));
            assert!(
                matches!(refused, Some(SessionReply::Error { ref error }) if error.message.contains("doesn't ask for retrospectives"))
            );
        }
        assert!(host_rx.try_recv().is_err());
        // Another session's sandbox can't call for this one.
        let (mut sup, mut host_rx) = keeping(dir.path(), true, true)?;
        let elsewhere = ask(
            &mut sup,
            SessionRequest::Call {
                session_id: Some(SessionId("ses_other".into())),
                call: AgentCall::Memories { stale: false },
            },
        );
        assert!(matches!(elsewhere, Some(SessionReply::Error { .. })));
        assert!(host_rx.try_recv().is_err());
        // Without a host there is nobody to ask.
        sup.host = None;
        let alone = ask(&mut sup, calling(AgentCall::Memories { stale: false }));
        assert!(
            matches!(alone, Some(SessionReply::Error { ref error }) if error.code == fctl_proto::ErrorCode::Unavailable)
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_late_answer_still_reaches_the_transcript() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host_rx) = keeping(dir.path(), true, false)?;
        let (tx, mut rx) = oneshot::channel();
        sup.on_session_call(local::Call {
            request: calling(AgentCall::Remember {
                text: "Deploys happen on Fridays".into(),
                replaces: None,
                also_replaces: vec![],
            }),
            reply: tx,
        });
        let AgentToHost::Call { request_id, .. } = serde_json::from_str(&host_rx.try_recv()?)?
        else {
            panic!("expected a call frame");
        };
        sup.expire_calls();
        assert!(rx.try_recv().is_err());
        for (_, since) in sup.calling.values_mut() {
            *since = Instant::now() - CALL_TIMEOUT - Duration::from_secs(1);
        }
        sup.expire_calls();
        assert!(
            matches!(rx.try_recv(), Ok(SessionReply::Error { ref error }) if error.message.contains("in time"))
        );
        let answer = AgentAnswer::Memory {
            memory: note("mem_1", "Deploys happen on Fridays", None),
            replaced: None,
            also_replaced: vec![],
        };
        sup.on_answer(&request_id, Some(answer.clone()), None);
        let n = notices(&mut host_rx)?;
        assert_eq!(n[0].message, "Remembered: Deploys happen on Fridays");
        sup.on_answer(&request_id, Some(answer), None);
        assert!(host_rx.try_recv().is_err());
        Ok(())
    }

    /// A configured supervisor in the middle of a turn, with a host link.
    fn in_a_turn(
        dir: &std::path::Path,
    ) -> anyhow::Result<(Supervisor, mpsc::UnboundedReceiver<String>)> {
        let (mut sup, _claude, _work) = Supervisor::new(dir.join("state"), None)?;
        sup.config = Some(config(dir, false)?);
        let (host_tx, host_rx) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        sup.turn_active = true;
        sup.set_status(SessionStatus::Running, None);
        Ok((sup, host_rx))
    }

    /// A Claude Code that keeps what it is sent, in `stdin.jsonl` in `dir`.
    fn stub_claude(dir: &std::path::Path, sup: &Supervisor) -> anyhow::Result<Claude> {
        script_claude(dir, sup, "#!/bin/sh\nexec cat > \"$HOME/stdin.jsonl\"\n")
    }

    /// A Claude Code that runs `script`, in `dir`.
    fn script_claude(
        dir: &std::path::Path,
        sup: &Supervisor,
        script: &str,
    ) -> anyhow::Result<Claude> {
        let bin = dir.join("claude");
        crate::claude::tests::write_script(&bin, script)?;
        Claude::spawn(
            LaunchSpec {
                bin: &bin.display().to_string(),
                cwd: dir,
                home: dir,
                claude_session_id: "s",
                resume: false,
                model: None,
                effort: None,
                append_system_prompt: "",
                setting_sources: "user",
                extra_args: &[],
                env: Default::default(),
            },
            sup.claude_tx.clone(),
        )
    }

    /// What the stub Claude Code has been sent, once it has `n` lines.
    async fn sent_to_claude(dir: &std::path::Path, n: usize) -> anyhow::Result<Vec<Value>> {
        for _ in 0..250 {
            let text = std::fs::read_to_string(dir.join("stdin.jsonl")).unwrap_or_default();
            let lines: Vec<Value> = text
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            if lines.len() >= n {
                return Ok(lines);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        anyhow::bail!("Claude got fewer than {n} lines")
    }

    fn task_started(id: &str, description: &str, kind: &str) -> Value {
        json!({"type": "system", "subtype": "task_started", "task_id": id, "tool_use_id": "toolu_1",
               "description": description, "task_type": kind, "is_backgrounded": true,
               "session_id": "s", "uuid": "u"})
    }

    fn task_ended(id: &str) -> Value {
        json!({"type": "system", "subtype": "task_notification", "task_id": id, "tool_use_id": "toolu_1",
               "status": "completed", "output_file": "", "summary": "Done", "session_id": "s", "uuid": "u"})
    }

    fn turn_state(state: &str) -> Value {
        json!({"type": "system", "subtype": "session_state_changed", "state": state, "session_id": "s", "uuid": "u"})
    }

    fn result() -> Value {
        json!({"type": "result", "subtype": "success", "is_error": false, "result": "Done",
               "queued_turn_count": 0, "session_id": "s", "uuid": "u"})
    }

    fn waiting(sup: &Supervisor) -> (SessionStatus, Option<&str>) {
        (sup.state.status, sup.state.reason.as_deref())
    }

    /// The session states the host has been sent since last asked.
    fn states_sent(host: &mut mpsc::UnboundedReceiver<String>) -> Vec<SessionState> {
        let mut out = vec![];
        while let Ok(text) = host.try_recv() {
            if let Ok(AgentToHost::State { state }) = serde_json::from_str(&text) {
                out.push(state);
            }
        }
        out
    }

    /// The events the host has been sent since last asked.
    fn events_sent(host: &mut mpsc::UnboundedReceiver<String>) -> Vec<EventBody> {
        let mut out = vec![];
        while let Ok(text) = host.try_recv() {
            if let Ok(AgentToHost::Events { events }) = serde_json::from_str(&text) {
                out.extend(events.into_iter().map(|e| e.body));
            }
        }
        out
    }

    fn bash(id: &str, command: &str) -> Value {
        json!({"type": "assistant", "parent_tool_use_id": null, "session_id": "s", "uuid": "u",
               "message": {"id": format!("msg_{id}"), "role": "assistant",
                           "content": [{"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command}}]}})
    }

    fn tool_result(id: &str) -> Value {
        json!({"type": "user", "parent_tool_use_id": null, "session_id": "s", "uuid": "u",
               "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": "", "is_error": false}]}})
    }

    /// `system/init`, which starts each turn.
    fn init() -> Value {
        json!({"type": "system", "subtype": "init", "apiKeySource": "none", "model": "claude-opus-5-5",
               "session_id": "s", "uuid": "u"})
    }

    fn says(text: &str) -> Value {
        json!({"type": "assistant", "parent_tool_use_id": null, "session_id": "s", "uuid": "u",
               "message": {"id": "msg_says", "role": "assistant", "content": [{"type": "text", "text": text}]}})
    }

    /// A subagent's message, which names the `Agent` call that started it.
    fn subagent_says(call: &str, text: &str) -> Value {
        json!({"type": "assistant", "parent_tool_use_id": call, "session_id": "s", "uuid": "u",
               "message": {"id": "msg_sub", "role": "assistant", "content": [{"type": "text", "text": text}]}})
    }

    /// An `Agent` call that starts a subagent in the background.
    fn agent(id: &str, description: &str) -> Value {
        json!({"type": "assistant", "parent_tool_use_id": null, "session_id": "s", "uuid": "u",
               "message": {"id": format!("msg_{id}"), "role": "assistant",
                           "content": [{"type": "tool_use", "id": id, "name": "Agent",
                                        "input": {"description": description, "prompt": description, "run_in_background": true}}]}})
    }

    /// Hands the supervisor the end of the refresh of the repositories'
    /// state that runs, and sends the host what it emitted.
    async fn synced(sup: &mut Supervisor, work: &mut mpsc::UnboundedReceiver<Work>) {
        assert!(sup.syncing, "no refresh runs");
        while let Ok(Some(w)) = tokio::time::timeout(Duration::from_secs(10), work.recv()).await {
            let done = matches!(w, Work::Git(..));
            sup.on_work(w).await;
            if done {
                break;
            }
        }
        sup.flush();
    }

    /// What the events are: the kind, or `result` for Claude's.
    fn kinds(events: &[EventBody]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e.claude_type() {
                Some("result") => "result",
                _ => e.kind(),
            })
            .collect()
    }

    #[tokio::test]
    async fn commits_show_while_claude_works() -> anyhow::Result<()> {
        use crate::refresh::{EVERY, QUIET};
        let dir = tempfile::tempdir()?;
        let (mut sup, _claude, mut work) = Supervisor::new(dir.path().join("state"), None)?;
        sup.config = Some(config(dir.path(), false)?);
        let paths = Paths::new(
            &dir.path().join("workspace"),
            &dir.path().join("home"),
            &dir.path().join("state"),
        );
        std::fs::create_dir_all(&paths.workdir)?;
        std::fs::create_dir_all(&paths.home)?;
        let (wd, home) = (paths.workdir.clone(), paths.home.clone());
        let init = ["init", "-q", "-b", "fctl/1-test"];
        assert!(workspace::run(&wd, &home, "git", &init).await?.ok);
        sup.paths = Some(paths);
        let (host_tx, mut host) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        sup.turn_active = true;
        sup.set_status(SessionStatus::Running, None);

        // The agent commits in the middle of its turn.
        sup.on_claude_message(bash("t1", "git commit -m 'Add notes'"))
            .await;
        std::fs::write(wd.join("notes.md"), "# Notes\n")?;
        let add = workspace::run(&wd, &home, "git", &["add", "notes.md"]).await?;
        let commit = [
            "-c",
            "user.name=Ada",
            "-c",
            "user.email=ada@example.com",
            "commit",
            "-q",
            "-m",
            "Add notes",
        ];
        let commit = workspace::run(&wd, &home, "git", &commit).await?;
        assert!(add.ok && commit.ok, "{}", commit.stderr);
        sup.on_claude_message(tool_result("t1")).await;
        // The refresh waits for the burst of tool calls to end…
        sup.on_tick().await;
        assert!(!sup.syncing);
        sup.refresh.backdate(QUIET);
        sup.on_tick().await;
        synced(&mut sup, &mut work).await;
        // …then the state shows the commit before the turn ends, and
        // nothing was pushed: not even tried, without a remote.
        let events = events_sent(&mut host);
        assert_eq!(kinds(&events), ["claude", "claude", "git"]);
        let Some(EventBody::Git(state)) = events.last() else {
            panic!("expected the repository's state");
        };
        assert_eq!(
            (state.ahead, state.files.len(), state.pushed),
            (1, 1, false)
        );
        let subject = state.last_commit.as_ref().map(|c| c.subject.as_str());
        assert_eq!(subject, Some("Add notes"));
        assert_eq!(sup.state.status, SessionStatus::Running);

        // A rebase under way, its HEAD detached, waits for a later refresh.
        let detach = ["checkout", "-q", "--detach"];
        assert!(workspace::run(&wd, &home, "git", &detach).await?.ok);
        sup.on_claude_message(bash("t2", "git rebase -i main"))
            .await;
        sup.on_claude_message(tool_result("t2")).await;
        sup.refresh.backdate(EVERY);
        sup.on_tick().await;
        synced(&mut sup, &mut work).await;
        assert_eq!(kinds(&events_sent(&mut host)), ["claude", "claude"]);
        let attach = ["checkout", "-q", "fctl/1-test"];
        assert!(workspace::run(&wd, &home, "git", &attach).await?.ok);

        // A refresh that finds nothing new says nothing, and the turn that
        // ends meanwhile has its state reported once it is done.
        sup.on_claude_message(bash("t3", "cargo test")).await;
        sup.on_claude_message(tool_result("t3")).await;
        sup.refresh.backdate(EVERY);
        sup.on_tick().await;
        assert!(sup.syncing);
        sup.on_claude_message(result()).await;
        assert!(sup.turn_ended);
        synced(&mut sup, &mut work).await;
        assert_eq!(
            kinds(&events_sent(&mut host)),
            ["claude", "claude", "result"]
        );
        // After the turn fctl-agent pushes, which fails without a remote.
        synced(&mut sup, &mut work).await;
        let events = events_sent(&mut host);
        assert_eq!(kinds(&events), ["log", "git"]);
        assert!(matches!(&events[0], EventBody::Log(l) if l.text.starts_with("git push failed")));
        assert!(!sup.syncing && !sup.turn_ended);
        Ok(())
    }

    #[tokio::test]
    async fn pull_requests_gh_pr_create_printed_are_followed() -> anyhow::Result<()> {
        use crate::refresh::QUIET;
        let dir = tempfile::tempdir()?;
        let state_dir = dir.path().join("state");
        let (mut sup, _claude, mut work) = Supervisor::new(state_dir.clone(), None)?;
        sup.config = Some(config(dir.path(), false)?);
        let mut paths = Paths::new(
            &dir.path().join("workspace"),
            &dir.path().join("home"),
            &state_dir,
        );
        // No gh to ask: the pull request is listed as its URL names it.
        paths.gh = dir.path().join("no-gh").display().to_string();
        std::fs::create_dir_all(&paths.workdir)?;
        std::fs::create_dir_all(&paths.home)?;
        let init = ["init", "-q", "-b", "fctl/1-test"];
        assert!(
            workspace::run(&paths.workdir, &paths.home, "git", &init)
                .await?
                .ok
        );
        sup.paths = Some(paths);
        let (host_tx, mut host) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        sup.turn_active = true;
        sup.set_status(SessionStatus::Running, None);

        // The agent opens one from a branch in a worktree.
        let create = "cd ~/second && gh pr create --head fctl/1-second --fill";
        sup.on_claude_message(bash("t1", create)).await;
        let mut printed = tool_result("t1");
        printed["message"]["content"][0]["content"] =
            json!("https://github.com/acme/widgets/pull/2\n");
        sup.on_claude_message(printed).await;
        sup.refresh.backdate(QUIET);
        sup.on_tick().await;
        synced(&mut sup, &mut work).await;
        let events = events_sent(&mut host);
        let Some(EventBody::Git(state)) = events.last() else {
            panic!("expected the repositories' state");
        };
        let listed: Vec<&str> = state
            .pull_requests
            .iter()
            .flatten()
            .map(|p| p.url.as_str())
            .collect();
        assert_eq!(listed, ["https://github.com/acme/widgets/pull/2"]);
        // A restarted supervisor follows it still.
        let (again, _claude, _work) = Supervisor::new(state_dir, None)?;
        assert_eq!(again.persisted.pulls, sup.persisted.pulls);
        assert_eq!(again.persisted.pulls.listed().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn the_session_says_which_effort_claude_works_at() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _claude, mut work) = Supervisor::new(dir.path().join("state"), None)?;
        sup.config = Some(config(dir.path(), false)?);
        let (host_tx, mut host) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        sup.turn_active = true;
        sup.set_status(SessionStatus::Running, None);
        // A Claude Code that answers `get_settings` as 2.1.304 does.
        let script = r##"#!/bin/sh
while IFS= read -r line; do
  case "$line" in *'"get_settings"'*)
    id=$(printf '%s' "$line" | sed 's/.*"request_id":"\([^"]*\)".*/\1/')
    printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s","response":{"effective":{},"sources":[],"applied":{"model":"claude-opus-5-5","effort":"xhigh","advisor":null}}}}\n' "$id" ;;
  esac
done
"##;
        sup.claude = Some(script_claude(dir.path(), &sup, script)?);

        // Claude says at the end of each turn.
        sup.on_claude_message(result()).await;
        let said = tokio::time::timeout(Duration::from_secs(10), work.recv()).await?;
        let Some(Work::Effort(effort)) = said else {
            panic!("expected Claude's effort level");
        };
        assert_eq!(effort, "xhigh");
        states_sent(&mut host);
        sup.on_work(Work::Effort(effort)).await;
        let states = states_sent(&mut host);
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].effort.as_deref(), Some("xhigh"));
        assert_eq!(states[0].status, SessionStatus::Idle);
        // The same level again changes nothing.
        sup.on_work(Work::Effort("xhigh".into())).await;
        assert!(states_sent(&mut host).is_empty());
        // A model that takes none.
        sup.on_work(Work::Effort(String::new())).await;
        assert_eq!(states_sent(&mut host)[0].effort.as_deref(), Some(""));
        Ok(())
    }

    /// What the host has been told of deliveries since last asked.
    fn delivered_sent(
        host: &mut mpsc::UnboundedReceiver<String>,
    ) -> Vec<(DeliveryId, bool, Option<String>)> {
        let mut out = vec![];
        while let Ok(text) = host.try_recv() {
            if let Ok(AgentToHost::Delivered {
                delivery_id,
                ok,
                error,
            }) = serde_json::from_str(&text)
            {
                out.push((delivery_id, ok, error));
            }
        }
        out
    }

    fn settings(id: &str, model: Option<&str>, effort: Option<&str>) -> Delivery {
        Delivery {
            id: DeliveryId(id.into()),
            kind: DeliveryKind::Settings {
                model: model.map(str::to_owned),
                effort: effort.map(str::to_owned),
                by: None,
            },
        }
    }

    #[tokio::test]
    async fn people_change_the_model_and_effort_claude_works_at() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _claude, mut work) = Supervisor::new(dir.path().join("state"), None)?;
        sup.config = Some(config(dir.path(), false)?);
        let (host_tx, mut host) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        sup.state.model = Some("claude-opus-5-5".into());
        sup.state.effort = Some("xhigh".into());
        // A Claude Code that switches to Sonnet as 2.1.318 does, and takes
        // any effort level, but knows no Haiku.
        let script = r##"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$HOME/stdin.jsonl"
  id=$(printf '%s' "$line" | sed 's/.*"request_id":"\([^"]*\)".*/\1/')
  case "$line" in
    *'"model":"haiku"'*)
      printf '{"type":"control_response","response":{"subtype":"error","request_id":"%s","error":"not_found"}}\n' "$id" ;;
    *'"set_model"'*)
      printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s","response":{"model":"claude-sonnet-5-5"}}}\n' "$id" ;;
    *'"effortLevel":"max"'*)
      printf '{"type":"control_response","response":{"subtype":"error","request_id":"%s","error":"capped at xhigh"}}\n' "$id" ;;
    *'"apply_flag_settings"'*)
      printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s"}}\n' "$id" ;;
  esac
done
"##;
        sup.claude = Some(script_claude(dir.path(), &sup, script)?);

        sup.on_delivery(settings("dlv_1", Some("sonnet"), Some("medium")))
            .await;
        // Sent again before Claude answered (a new link): the answer follows.
        sup.on_delivery(settings("dlv_1", Some("sonnet"), Some("medium")))
            .await;
        assert!(delivered_sent(&mut host).is_empty());
        let sent = sent_to_claude(dir.path(), 2).await?;
        assert_eq!(sent[0]["request"]["subtype"], "set_model");
        assert_eq!(sent[0]["request"]["model"], "sonnet");
        assert_eq!(sent[1]["request"]["subtype"], "apply_flag_settings");
        assert_eq!(sent[1]["request"]["settings"]["effortLevel"], "medium");
        let answered = tokio::time::timeout(Duration::from_secs(10), work.recv()).await?;
        let Some(answered @ Work::Settings { .. }) = answered else {
            panic!("expected Claude's answers");
        };
        sup.on_work(answered).await;
        // The session says what Claude works on now, by the name asked for.
        let mut frames = vec![];
        while let Ok(text) = host.try_recv() {
            frames.push(text);
        }
        let state = frames
            .iter()
            .find_map(|t| match serde_json::from_str(t) {
                Ok(AgentToHost::State { state }) => Some(state),
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("no state sent"))?;
        assert_eq!(state.model.as_deref(), Some("sonnet"));
        assert_eq!(state.effort.as_deref(), Some("medium"));
        assert!(frames.iter().any(|t| matches!(
            serde_json::from_str(t),
            Ok(AgentToHost::Delivered {
                ok: true,
                error: None,
                ..
            })
        )));
        // A Claude that starts again in this run starts on them.
        let spec = &sup
            .config
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("not configured"))?
            .spec;
        assert_eq!(spec.model.as_deref(), Some("sonnet"));
        assert_eq!(spec.effort.as_deref(), Some("medium"));
        // The next turn's `system/init` names Sonnet by its ID: the session
        // keeps the name asked for, and says nothing new.
        let mut sonnet = init();
        sonnet["model"] = json!("claude-sonnet-5-5");
        sup.on_claude_message(sonnet).await;
        assert_eq!(sup.state.model.as_deref(), Some("sonnet"));
        assert!(
            !std::iter::from_fn(|| host.try_recv().ok())
                .any(|t| matches!(serde_json::from_str(&t), Ok(AgentToHost::State { .. })))
        );
        // `/model` in a message moved it to another model: by its ID.
        sup.on_claude_message(init()).await;
        assert_eq!(sup.state.model.as_deref(), Some("claude-opus-5-5"));
        let mut sonnet = init();
        sonnet["model"] = json!("claude-sonnet-5-5");
        sup.on_claude_message(sonnet).await;
        assert_eq!(sup.state.model.as_deref(), Some("sonnet"));
        while host.try_recv().is_ok() {}
        // Delivered again over a new link: taken once.
        sup.on_delivery(settings("dlv_1", Some("sonnet"), Some("medium")))
            .await;
        assert_eq!(delivered_sent(&mut host).len(), 1);
        assert_eq!(sent_to_claude(dir.path(), 2).await?.len(), 2);

        // A model Claude refuses: it keeps working on the one it had, and
        // the transcript says why.
        sup.on_delivery(settings("dlv_2", Some("haiku"), None))
            .await;
        let answered = tokio::time::timeout(Duration::from_secs(10), work.recv()).await?;
        let Some(answered @ Work::Settings { .. }) = answered else {
            panic!("expected Claude's answer");
        };
        sup.on_work(answered).await;
        let mut frames = vec![];
        while let Ok(text) = host.try_recv() {
            frames.push(text);
        }
        assert!(frames.iter().any(|t| matches!(
            serde_json::from_str(t),
            Ok(AgentToHost::State { state }) if state.model.as_deref() == Some("sonnet")
        )));
        let refused = frames.iter().find_map(|t| match serde_json::from_str(t) {
            Ok(AgentToHost::Delivered {
                ok: false, error, ..
            }) => error,
            _ => None,
        });
        assert_eq!(
            refused.as_deref(),
            Some("Claude didn't switch to haiku: not_found")
        );
        // A Claude that starts again starts on the model it had.
        assert_eq!(
            sup.config.as_ref().and_then(|c| c.spec.model.as_deref()),
            Some("sonnet")
        );
        // Sent again after a link that lost the answer: refused again.
        sup.on_delivery(settings("dlv_2", Some("haiku"), None))
            .await;
        let again = delivered_sent(&mut host);
        assert_eq!(again.len(), 1);
        assert!(!again[0].1);
        assert_eq!(
            again[0].2.as_deref(),
            Some("Claude didn't switch to haiku: not_found")
        );

        // An effort level it refuses: a Claude that starts again starts on
        // the one it had.
        sup.on_delivery(settings("dlv_3", None, Some("max"))).await;
        let answered = tokio::time::timeout(Duration::from_secs(10), work.recv()).await?;
        let Some(answered @ Work::Settings { .. }) = answered else {
            panic!("expected Claude's answer");
        };
        sup.on_work(answered).await;
        assert_eq!(
            sup.config.as_ref().and_then(|c| c.spec.effort.as_deref()),
            Some("medium")
        );
        assert!(
            delivered_sent(&mut host)
                .iter()
                .any(|(id, ok, _)| id.0 == "dlv_3" && !ok)
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_session_started_on_an_alias_keeps_it() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host) = in_a_turn(dir.path())?;
        while host.try_recv().is_ok() {}
        // `claude --model opus`, whose `system/init` names Opus by its ID.
        sup.model_name.started(Some("opus"));
        sup.on_claude_message(init()).await;
        let models: Vec<Option<String>> = states_sent(&mut host)
            .into_iter()
            .map(|s| s.model)
            .collect();
        assert_eq!(models, [Some("opus".to_owned())]);
        // Started on Claude's default: by the ID Claude gives it.
        sup.model_name.started(None);
        sup.on_claude_message(init()).await;
        assert_eq!(sup.state.model.as_deref(), Some("claude-opus-5-5"));
        Ok(())
    }

    #[tokio::test]
    async fn only_claude_refusing_a_change_puts_the_old_one_back() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        let (host_tx, mut host) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        sup.config = Some(config(dir.path(), false)?);
        let spec = |sup: &Supervisor| {
            sup.config
                .as_ref()
                .map(|c| (c.spec.model.clone(), c.spec.effort.clone()))
        };
        let before = (Some("opus".to_owned()), Some("xhigh".to_owned()));
        let asked = |sup: &mut Supervisor| {
            if let Some(c) = sup.config.as_mut() {
                c.spec.model = Some("sonnet".into());
                c.spec.effort = Some("medium".into());
            }
        };
        // Claude exited while it was asked: it started again on the new ones.
        asked(&mut sup);
        sup.settings_applied(
            DeliveryId("dlv_1".into()),
            Some(("sonnet".into(), Err(AskError::Exited))),
            Some(("medium".into(), Err(AskError::Exited))),
            before.clone(),
        );
        assert_eq!(
            spec(&sup),
            Some((Some("sonnet".into()), Some("medium".into())))
        );
        assert!(delivered_sent(&mut host)[0].1);
        // No answer in time: it may switch yet, so they stay, and the
        // transcript says it didn't say.
        sup.settings_applied(
            DeliveryId("dlv_2".into()),
            Some(("sonnet".into(), Err(AskError::TimedOut))),
            None,
            before.clone(),
        );
        assert_eq!(spec(&sup).and_then(|s| s.0), Some("sonnet".into()));
        let told = delivered_sent(&mut host);
        assert!(!told[0].1);
        assert!(told[0].2.as_deref().is_some_and(|e| e.contains("in time")));
        // Refused: Claude starts on the old ones again.
        sup.settings_applied(
            DeliveryId("dlv_3".into()),
            Some(("sonnet".into(), Err(AskError::Refused("not_found".into())))),
            Some(("medium".into(), Err(AskError::Refused("no".into())))),
            before.clone(),
        );
        assert_eq!(spec(&sup), Some(before));
        Ok(())
    }

    #[tokio::test]
    async fn claude_starts_on_settings_changed_before_it_did() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        let (host_tx, mut host) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        // Before the session's spec: it waits for Claude.
        sup.on_delivery(settings("dlv_1", None, Some("low"))).await;
        assert_eq!(sup.queue.len(), 1);
        assert!(delivered_sent(&mut host).is_empty());
        // Once configured, while the workspace is prepared: Claude starts
        // on it.
        sup.queue.clear();
        sup.config = Some(config(dir.path(), false)?);
        sup.on_delivery(settings("dlv_1", None, Some("low"))).await;
        assert_eq!(
            sup.config.as_ref().and_then(|c| c.spec.effort.as_deref()),
            Some("low")
        );
        let delivered = delivered_sent(&mut host);
        assert_eq!(delivered.len(), 1);
        assert!(delivered[0].1);
        Ok(())
    }

    #[tokio::test]
    async fn the_session_says_which_context_window_claude_works_in() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _claude, mut work) = Supervisor::new(dir.path().join("state"), None)?;
        sup.config = Some(config(dir.path(), false)?);
        let (host_tx, mut host) = mpsc::unbounded_channel::<String>();
        sup.host = Some((1, host_tx));
        sup.turn_active = true;
        sup.set_status(SessionStatus::Running, None);
        // A Claude Code that answers a summary of its context as 2.1.308
        // does (categories left out), and nothing else.
        let script = r##"#!/bin/sh
while IFS= read -r line; do
  case "$line" in *'"detail":"summary"'*'"get_context_usage"'*)
    id=$(printf '%s' "$line" | sed 's/.*"request_id":"\([^"]*\)".*/\1/')
    printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s","response":{"categories":[],"totalTokens":16480,"maxTokens":1000000,"rawMaxTokens":1000000,"percentage":2,"model":"claude-opus-5-5","autoCompactThreshold":967000,"isAutoCompactEnabled":true,"autocompactSource":"model-default","apiUsage":null}}}\n' "$id" ;;
  esac
done
"##;
        sup.claude = Some(script_claude(dir.path(), &sup, script)?);

        // Claude says at the end of each turn.
        sup.on_claude_message(result()).await;
        let window = loop {
            match tokio::time::timeout(Duration::from_secs(10), work.recv()).await? {
                Some(Work::Context(window)) => break window,
                Some(_) => continue,
                None => panic!("expected Claude's context window"),
            }
        };
        let expected = ContextWindow {
            tokens: 1_000_000,
            compact_at: Some(967_000),
        };
        assert_eq!(window, expected);
        states_sent(&mut host);
        sup.on_work(Work::Context(window)).await;
        let states = states_sent(&mut host);
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].context, Some(expected));
        assert_eq!(states[0].status, SessionStatus::Idle);
        // The same window again changes nothing.
        sup.on_work(Work::Context(window)).await;
        assert!(states_sent(&mut host).is_empty());
        // Another model's.
        let smaller = ContextWindow {
            tokens: 200_000,
            compact_at: Some(167_000),
        };
        sup.on_work(Work::Context(smaller)).await;
        assert_eq!(states_sent(&mut host)[0].context, Some(smaller));
        Ok(())
    }

    #[tokio::test]
    async fn thinking_progress_goes_out_live_and_is_not_kept() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host) = in_a_turn(dir.path())?;
        let journaled = sup.journal.last_seq();
        // Claude Code 2.1.307 says how far its thinking got about once a second.
        let progress = json!({"type": "system", "subtype": "thinking_tokens", "estimated_tokens": 900,
                              "estimated_tokens_delta": 100, "session_id": "s", "uuid": "u"});
        sup.on_claude_message(progress.clone()).await;
        assert_eq!(sup.journal.last_seq(), journaled);
        sup.on_claude_message(bash("t1", "ls")).await;
        sup.flush();
        let (mut live, mut kept) = (vec![], vec![]);
        while let Ok(text) = host.try_recv() {
            match serde_json::from_str::<AgentToHost>(&text)? {
                AgentToHost::Stream { event } => live.push(event),
                AgentToHost::Events { events } => kept.extend(events.into_iter().map(|e| e.body)),
                _ => {}
            }
        }
        assert_eq!(live, [progress]);
        assert_eq!(kinds(&kept), ["claude"]);
        assert_eq!(sup.journal.last_seq(), journaled + 1);
        Ok(())
    }

    #[tokio::test]
    async fn the_idle_clock_waits_for_background_tasks() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _host) = in_a_turn(dir.path())?;
        sup.on_claude_message(task_started("a1", "Review the diff", "local_agent"))
            .await;
        sup.on_claude_message(task_started("b1", "Start the dev stack", "local_bash"))
            .await;
        // A foreground task that ends within the turn.
        sup.on_claude_message(task_started("c1", "Run the tests", "local_bash"))
            .await;
        sup.on_claude_message(task_ended("c1")).await;
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));

        sup.on_claude_message(result()).await;
        assert_eq!(
            waiting(&sup),
            (SessionStatus::Idle, Some("Waiting for 2 background tasks"))
        );
        assert!(sup.idle_since.is_none() && sup.waiting_since.is_some());
        // Well past the idle timeout, the session still waits.
        sup.waiting_since = Some(Instant::now() - Duration::from_secs(30 * 60));
        sup.on_tick().await;
        assert!(sup.stopping.is_none());

        sup.on_claude_message(task_ended("a1")).await;
        assert_eq!(
            waiting(&sup),
            (SessionStatus::Idle, Some("Waiting for a background task"))
        );
        assert!(sup.idle_since.is_none());
        sup.on_claude_message(task_ended("b1")).await;
        assert_eq!(
            waiting(&sup),
            (SessionStatus::Idle, Some("Waiting for a message"))
        );
        // The idle clock starts now, not at the end of the turn.
        assert!(
            sup.idle_since
                .is_some_and(|t| t.elapsed() < Duration::from_secs(5))
        );
        sup.on_tick().await;
        assert!(sup.stopping.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn a_turn_claude_starts_on_its_own_counts() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host) = in_a_turn(dir.path())?;
        sup.on_claude_message(turn_state("running")).await;
        sup.on_claude_message(task_started("a1", "Review the diff", "local_agent"))
            .await;
        sup.on_claude_message(result()).await;
        sup.on_claude_message(turn_state("idle")).await;
        sup.on_claude_message(task_ended("a1")).await;
        assert_eq!(
            waiting(&sup),
            (SessionStatus::Idle, Some("Waiting for a message"))
        );

        // Claude takes the result in a turn of its own.
        sup.on_claude_message(turn_state("running")).await;
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));
        assert!(sup.turn_active && sup.idle_since.is_none());
        sup.on_claude_message(result()).await;
        assert_eq!(
            waiting(&sup),
            (SessionStatus::Idle, Some("Waiting for a message"))
        );
        // The idle clock starts once Claude says it is done.
        assert!(!sup.turn_active && sup.idle_since.is_none());
        sup.on_claude_message(turn_state("idle")).await;
        assert!(sup.idle_since.is_some());

        // One that ends without a result ends all the same.
        sup.on_claude_message(turn_state("running")).await;
        sup.on_claude_message(turn_state("idle")).await;
        assert_eq!(waiting(&sup).0, SessionStatus::Idle);
        assert!(!sup.turn_active);
        // Claude's reports of its turns aren't kept.
        let kept = events_sent(&mut host);
        assert!(!kept.is_empty());
        assert!(kept.iter().all(
            |e| !matches!(e, EventBody::Claude(v) if v["subtype"] == "session_state_changed")
        ));

        // `idle` doesn't end a turn that answers a message: its result does.
        sup.turn_active = true;
        sup.set_status(SessionStatus::Running, None);
        sup.on_claude_message(turn_state("idle")).await;
        assert!(sup.turn_active);
        Ok(())
    }

    #[tokio::test]
    async fn the_first_message_of_a_turn_starts_it() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _host) = in_a_turn(dir.path())?;
        sup.on_claude_message(result()).await;
        assert_eq!(waiting(&sup).0, SessionStatus::Idle);
        // A background subagent works while the conversation waits.
        sup.on_claude_message(subagent_says("toolu_1", "Reviewing"))
            .await;
        assert_eq!(waiting(&sup).0, SessionStatus::Idle);
        let delta = json!({"type": "stream_event", "parent_tool_use_id": null,
            "event": {"type": "message_start", "message": {"id": "m2"}}});
        sup.on_claude_message(delta.clone()).await;
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));
        sup.on_claude_message(result()).await;
        assert_eq!(waiting(&sup).0, SessionStatus::Idle);

        // `system/init` starts each turn, before Claude streams.
        sup.on_claude_message(init()).await;
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));
        sup.on_claude_message(result()).await;

        // Claude Code that reports its state doesn't report every turn,
        // so the messages still tell.
        sup.on_claude_message(turn_state("idle")).await;
        sup.on_claude_message(delta).await;
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));
        Ok(())
    }

    #[tokio::test]
    async fn turns_that_take_subagents_reports_count_without_running() -> anyhow::Result<()> {
        // Claude Code 2.1.308's order with two subagents in the background:
        // it says `running` once, for the turn that starts them, and
        // `idle` once both have reported and the last turn has ended.
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host) = in_a_turn(dir.path())?;
        // As an earlier turn's `system/init` named it.
        sup.state.model = Some("claude-opus-5-5".into());
        sup.on_claude_message(turn_state("running")).await;
        sup.on_claude_message(init()).await;
        for (call, task, part) in [("toolu_a", "a1", "A"), ("toolu_b", "b1", "B")] {
            let description = format!("Review part {part}");
            sup.on_claude_message(agent(call, &description)).await;
            sup.on_claude_message(task_started(task, &description, "local_agent"))
                .await;
            sup.on_claude_message(tool_result(call)).await;
        }
        sup.on_claude_message(says("Waiting for the reviews."))
            .await;
        sup.on_claude_message(result()).await;
        assert_eq!(
            waiting(&sup),
            (SessionStatus::Idle, Some("Waiting for 2 background tasks"))
        );

        // A reports, and Claude Code takes the report in a turn of its own,
        // with no `running`.
        sup.on_claude_message(subagent_says("toolu_a", "Report A"))
            .await;
        sup.on_claude_message(task_ended("a1")).await;
        assert_eq!(
            waiting(&sup),
            (SessionStatus::Idle, Some("Waiting for a background task"))
        );
        sup.on_claude_message(init()).await;
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));
        assert!(sup.turn_active && sup.idle_since.is_none());
        sup.on_claude_message(bash("t3", "sleep 14")).await;
        // A shell command in the turn, which Claude Code reports as a task
        // once it has run for a few seconds.
        let mut command = task_started("c1", "Wait a while", "local_bash");
        command["is_backgrounded"] = json!(false);
        sup.on_claude_message(command).await;

        // B reports in the middle of that turn: the session runs on.
        sup.on_claude_message(subagent_says("toolu_b", "Report B"))
            .await;
        sup.on_claude_message(task_ended("b1")).await;
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));
        assert!(sup.idle_since.is_none() && sup.waiting_since.is_none());
        sup.on_tick().await;
        assert!(sup.stopping.is_none());

        // The turn takes B's report too, and ends. Claude Code says it is
        // done right after: only then does the idle clock start.
        sup.on_claude_message(task_ended("c1")).await;
        sup.on_claude_message(tool_result("t3")).await;
        sup.on_claude_message(says("Both reviews are in.")).await;
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));
        sup.on_claude_message(result()).await;
        assert_eq!(
            waiting(&sup),
            (SessionStatus::Idle, Some("Waiting for a message"))
        );
        assert!(!sup.turn_active && sup.idle_since.is_none());
        sup.on_claude_message(turn_state("idle")).await;
        assert!(
            sup.idle_since
                .is_some_and(|t| t.elapsed() < Duration::from_secs(5))
        );
        // The host saw the session run for the second turn, and wait
        // after it.
        let statuses: Vec<SessionStatus> =
            states_sent(&mut host).iter().map(|s| s.status).collect();
        assert_eq!(
            statuses,
            [
                SessionStatus::Running,
                SessionStatus::Idle,
                SessionStatus::Idle,
                SessionStatus::Running,
                SessionStatus::Idle,
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn work_claude_says_it_does_keeps_a_session_up_for_an_hour() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _host) = in_a_turn(dir.path())?;
        // Claude's turn ends, and it says it still works: for background
        // work that fctl-agent doesn't follow as a task, say.
        sup.on_claude_message(turn_state("running")).await;
        sup.on_claude_message(result()).await;
        assert_eq!(
            waiting(&sup),
            (SessionStatus::Idle, Some("Waiting for a message"))
        );
        assert!(sup.idle_since.is_none());
        sup.waiting_since = Some(Instant::now() - tasks::LIMIT + Duration::from_secs(60));
        sup.on_tick().await;
        assert!(sup.stopping.is_none());

        sup.waiting_since = Some(Instant::now() - tasks::LIMIT - Duration::from_secs(1));
        sup.on_tick().await;
        assert_eq!(
            waiting(&sup),
            (
                SessionStatus::Stopped,
                Some("Stopped after an hour waiting for background tasks")
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn background_work_keeps_a_session_up_for_an_hour() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host) = in_a_turn(dir.path())?;
        sup.on_claude_message(task_started("b1", "Start the dev stack", "local_bash"))
            .await;
        sup.on_claude_message(result()).await;
        sup.waiting_since = Some(Instant::now() - tasks::LIMIT + Duration::from_secs(60));
        sup.on_tick().await;
        assert!(sup.stopping.is_none());
        events_sent(&mut host);

        sup.waiting_since = Some(Instant::now() - tasks::LIMIT - Duration::from_secs(1));
        sup.on_tick().await;
        assert_eq!(
            waiting(&sup),
            (
                SessionStatus::Stopped,
                Some("Stopped after an hour waiting for background tasks")
            )
        );
        let notices: Vec<String> = events_sent(&mut host)
            .into_iter()
            .filter_map(|e| match e {
                EventBody::Notice(n) if n.code == "background_tasks_stopped" => Some(n.message),
                _ => None,
            })
            .collect();
        assert_eq!(
            notices,
            [
                "Stopped a background task that was still running: Start the dev stack (shell command)"
            ]
        );
        // The next run tells Claude.
        let (again, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        assert_eq!(
            again.persisted.unfinished,
            [Task {
                id: "b1".into(),
                description: "Start the dev stack".into(),
                kind: Some("local_bash".into()),
            }]
        );
        Ok(())
    }

    #[tokio::test]
    async fn the_next_message_says_which_tasks_did_not_finish() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _host) = in_a_turn(dir.path())?;
        sup.on_claude_message(task_started("a1", "Review the diff", "local_agent"))
            .await;
        sup.on_claude_message(result()).await;
        // A person stops the session.
        sup.on_frame(HostToAgent::Stop {
            reason: StopReason::User,
            grace_ms: 30_000,
        })
        .await;
        assert_eq!(waiting(&sup), (SessionStatus::Stopped, Some("Stopped")));

        let (mut next, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        next.claude = Some(stub_claude(dir.path(), &next)?);
        let bob = fctl_proto::session::UserRef {
            id: fctl_proto::UserId("usr_2".into()),
            name: "Bob".into(),
            avatar_url: None,
        };
        let by = prompt::Author {
            user: &bob,
            called: None,
        };
        next.send_turn("Please continue", Some(by), None, &[]).await;
        next.send_turn("And then this", None, None, &[]).await;
        let sent = sent_to_claude(dir.path(), 2).await?;
        let text = |i: usize| {
            sent[i]["message"]["content"][0]["text"]
                .as_str()
                .map(str::to_owned)
        };
        assert_eq!(
            text(0).as_deref(),
            Some(
                "The session stopped while these background tasks were still running, so they didn't finish:\n\
                 - Review the diff (subagent)\n\
                 Start again the ones you still need.\n\n\
                 Bob wrote:\n\nPlease continue"
            )
        );
        assert_eq!(text(1).as_deref(), Some("And then this"));
        let (later, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        assert!(later.persisted.unfinished.is_empty());
        Ok(())
    }

    /// The notices the host got: their codes and messages.
    fn notices_sent(host: &mut mpsc::UnboundedReceiver<String>) -> Vec<(String, String)> {
        events_sent(host)
            .into_iter()
            .filter_map(|e| match e {
                EventBody::Notice(n) => Some((n.code, n.message)),
                _ => None,
            })
            .collect()
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_supervisor_that_takes_over_stops_the_claude_left_running_and_carries_on()
    -> anyhow::Result<()> {
        use std::os::unix::process::CommandExt;
        let dir = tempfile::tempdir()?;
        // A supervisor started Claude, which leads a process group of its
        // own, and Claude works on a turn.
        let (mut sup, _host) = in_a_turn(dir.path())?;
        let mut left = std::process::Command::new("sleep")
            .arg("60")
            .process_group(0)
            .spawn()?;
        sup.persisted.claude = orphan::Process::of(left.id());
        sup.persisted.claude_started = true;
        sup.persist();
        assert!(sup.persisted.claude.is_some_and(|c| c.running()));
        // Then the supervisor dies, and Claude works on, unseen.
        drop(sup);

        let (mut next, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        next.take_over().await;
        let ended = tokio::task::spawn_blocking(move || left.wait()).await??;
        assert!(!ended.success(), "the Claude left running wasn't stopped");
        assert_eq!(next.persisted.cut_off, Some(CutOff::Unseen));
        let (saved, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        assert_eq!(
            (saved.persisted.claude, saved.persisted.working),
            (None, false)
        );
        // That supervisor dies too, before Claude ran again: the next one
        // still knows the turn was cut off.
        drop((next, saved));
        let (mut next, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        next.take_over().await;
        assert_eq!(next.persisted.cut_off, Some(CutOff::Unseen));

        // Once Claude runs again, the transcript says what happened, and
        // Claude carries on with the turn.
        next.config = Some(config(dir.path(), false)?);
        let (host_tx, mut host) = mpsc::unbounded_channel::<String>();
        next.host = Some((1, host_tx));
        next.claude = Some(stub_claude(dir.path(), &next)?);
        let cut_off = next
            .persisted
            .cut_off
            .take()
            .ok_or_else(|| anyhow::anyhow!("no cut-off turn"))?;
        next.carry_on(cut_off).await;
        assert_eq!(
            notices_sent(&mut host),
            vec![(
                "turn_cut_off".to_owned(),
                "fctl-agent restarted in the sandbox while Claude worked, so Claude's last steps aren't shown here, and fctl stopped that Claude to start it again on the conversation. Claude carries on with the turn, from where it left off.".to_owned()
            )]
        );
        let sent = sent_to_claude(dir.path(), 1).await?;
        let text = sent[0]["message"]["content"][0]["text"].as_str();
        assert_eq!(text, Some(prompt::cut_off_turn(true).as_str()));
        assert!(
            text.is_some_and(
                |t| t.starts_with("Your last turn was cut off: fctl-agent, which runs you")
            ),
            "{text:?}"
        );
        assert_eq!(next.state.status, SessionStatus::Running);

        // Cut off again soon after, it waits for a message instead.
        next.carry_on(CutOff::Stopped).await;
        next.flush();
        assert_eq!(
            notices_sent(&mut host),
            vec![(
                "turn_cut_off".to_owned(),
                "Claude's turn was cut off: fctl-agent stopped in the sandbox while Claude worked. It was cut off shortly before too, so Claude waits for a message.".to_owned()
            )]
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(sent_to_claude(dir.path(), 1).await?.len(), 1);
        // Said once.
        let (saved, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        assert_eq!(saved.persisted.cut_off, None);
        Ok(())
    }

    #[tokio::test]
    async fn only_a_turn_cut_off_short_is_carried_on() -> anyhow::Result<()> {
        // The sandbox stopped while Claude worked, and Claude with it.
        let dir = tempfile::tempdir()?;
        let (sup, _host) = in_a_turn(dir.path())?;
        assert!(sup.persisted.working);
        drop(sup);
        let (mut next, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        next.take_over().await;
        assert_eq!(next.persisted.cut_off, Some(CutOff::Stopped));

        // A session that waited, or that a person stopped, has nothing to
        // carry on, also with what Claude started still running.
        let dir = tempfile::tempdir()?;
        let (mut sup, _host) = in_a_turn(dir.path())?;
        sup.on_claude_message(result()).await;
        assert!(!sup.persisted.working);
        #[cfg(target_os = "linux")]
        let mut server = {
            use std::os::unix::process::CommandExt;
            let server = std::process::Command::new("sleep")
                .arg("60")
                .process_group(0)
                .spawn()?;
            sup.persisted.claude = orphan::Process::of(server.id());
            sup.persist();
            server
        };
        drop(sup);
        let (mut next, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        next.take_over().await;
        assert_eq!(next.persisted.cut_off, None);
        #[cfg(target_os = "linux")]
        {
            let ended = tokio::task::spawn_blocking(move || server.wait()).await??;
            assert!(!ended.success(), "what Claude left running wasn't stopped");
        }

        let dir = tempfile::tempdir()?;
        let (mut sup, _host) = in_a_turn(dir.path())?;
        sup.on_frame(HostToAgent::Stop {
            reason: StopReason::User,
            grace_ms: 30_000,
        })
        .await;
        // A person stopping it mid-turn cuts nothing short.
        assert_eq!(waiting(&sup).0, SessionStatus::Stopping);
        drop(sup);
        let (mut next, _claude, _work) = Supervisor::new(dir.path().join("state"), None)?;
        next.take_over().await;
        assert_eq!(next.persisted.cut_off, None);
        Ok(())
    }

    #[tokio::test]
    async fn turns_claude_starts_while_paused_or_stopping_are_interrupted() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _host) = in_a_turn(dir.path())?;
        sup.claude = Some(stub_claude(dir.path(), &sup)?);
        sup.on_claude_message(result()).await;
        sup.set_status(
            SessionStatus::Paused,
            Some("Waiting for the usage limit to reset".into()),
        );
        sup.on_claude_message(turn_state("running")).await;
        assert_eq!(waiting(&sup).0, SessionStatus::Paused);
        let sent = sent_to_claude(dir.path(), 3).await?;
        // Each turn's end asks which effort level Claude works at, and in
        // which context window.
        assert_eq!(sent[0]["request"]["subtype"], "get_settings");
        assert_eq!(sent[1]["request"]["subtype"], "get_context_usage");
        assert_eq!(sent[2]["request"]["subtype"], "interrupt");
        sup.on_claude_message(result()).await;
        assert_eq!(waiting(&sup).0, SessionStatus::Paused);

        // A stop interrupts the turn under way, and the one Claude starts
        // before the grace period ends.
        sup.turn_active = true;
        sup.set_status(SessionStatus::Running, None);
        sup.begin_stop("Stopped".into(), 30_000).await;
        sup.on_claude_message(result()).await;
        sup.on_claude_message(turn_state("running")).await;
        let sent = sent_to_claude(dir.path(), 9).await?;
        assert_eq!(sent[5]["request"]["subtype"], "interrupt");
        assert_eq!(sent[8]["request"]["subtype"], "interrupt");
        assert_eq!(waiting(&sup).0, SessionStatus::Stopping);
        Ok(())
    }

    /// A review agent's supervisor in the middle of its turn, with a host
    /// link. Review sessions wait a minute after a turn without a review.
    fn reviewing(
        dir: &std::path::Path,
    ) -> anyhow::Result<(Supervisor, mpsc::UnboundedReceiver<String>)> {
        let (mut sup, host) = in_a_turn(dir)?;
        if let Some(c) = sup.config.as_mut() {
            c.spec.idle_timeout_secs = 60;
            c.spec.review = Some(fctl_proto::runner::ReviewSpec {
                repo: fctl_proto::session::RepoRef {
                    owner: "acme".into(),
                    name: "widgets".into(),
                },
                number: 12,
                url: "https://github.com/acme/widgets/pull/12".into(),
                title: "Retry failed uploads".into(),
                head_sha: "3f2a1c9d00000000000000000000000000000000".into(),
                head_ref: "fctl/12-retry".into(),
                base_ref: "main".into(),
                previous_sha: None,
                instructions: String::new(),
            });
        }
        Ok((sup, host))
    }

    /// The agent runs `fctl review submit`: returns the call's id, which
    /// the host's answer names.
    fn submit_review(
        sup: &mut Supervisor,
        host: &mut mpsc::UnboundedReceiver<String>,
    ) -> anyhow::Result<(String, oneshot::Receiver<SessionReply>)> {
        let (tx, rx) = oneshot::channel();
        sup.on_session_call(local::Call {
            request: calling(AgentCall::SubmitReview {
                summary: None,
                findings: vec![],
                resolved: vec![],
            }),
            reply: tx,
        });
        let id = std::iter::from_fn(|| host.try_recv().ok())
            .find_map(|text| match serde_json::from_str(&text) {
                Ok(AgentToHost::Call { request_id, .. }) => Some(request_id),
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("the call didn't reach the host"))?;
        Ok((id, rx))
    }

    /// fctl posted the review: the control plane's answer to the call.
    fn posted(sup: &mut Supervisor, request_id: &str) {
        let review = fctl_proto::api::PostedReview {
            state: fctl_proto::api::ReviewState::Passed,
            url: Some("https://github.com/acme/widgets/pull/12#issuecomment-80".into()),
            comments: 0,
            in_text: 0,
            resolved: vec![],
            still_open: 0,
            again: false,
            sent_to: None,
        };
        sup.on_answer(request_id, Some(AgentAnswer::ReviewPosted { review }), None);
    }

    /// The frames the host has been sent since last asked.
    fn sent(host: &mut mpsc::UnboundedReceiver<String>) -> Vec<AgentToHost> {
        std::iter::from_fn(|| host.try_recv().ok())
            .filter_map(|text| serde_json::from_str(&text).ok())
            .collect()
    }

    #[tokio::test]
    async fn a_review_session_stops_as_the_turn_that_posted_ends() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host) = reviewing(dir.path())?;
        sup.on_claude_message(task_started("b1", "Run the upload tests", "local_bash"))
            .await;
        let (call, mut reply) = submit_review(&mut sup, &mut host)?;
        posted(&mut sup, &call);
        assert!(matches!(reply.try_recv(), Ok(SessionReply::Answer { .. })));
        // The agent says what it found: its turn runs until it ends.
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));
        sup.on_claude_message(says("Nothing to point out.")).await;
        sent(&mut host);

        sup.on_claude_message(result()).await;
        // It stops at once, without waiting for a message or its
        // background work, which the transcript names.
        assert_eq!(
            waiting(&sup),
            (
                SessionStatus::Stopped,
                Some("Finished: its review is posted")
            )
        );
        let frames = sent(&mut host);
        let statuses: Vec<SessionStatus> = frames
            .iter()
            .filter_map(|f| match f {
                AgentToHost::State { state } => Some(state.status),
                _ => None,
            })
            .collect();
        assert_eq!(statuses, [SessionStatus::Stopping, SessionStatus::Stopped]);
        let stopped_tasks = frames.iter().any(|f| {
            matches!(f, AgentToHost::Events { events } if events.iter().any(|e| matches!(
                &e.body,
                EventBody::Notice(n) if n.code == "background_tasks_stopped"
                    && n.message.ends_with("Run the upload tests (shell command)")
            )))
        });
        assert!(stopped_tasks, "{frames:?}");
        assert_eq!(
            frames.last(),
            Some(&AgentToHost::Exited {
                status: SessionStatus::Stopped,
                reason: Some("Finished: its review is posted".into()),
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_review_session_stops_as_a_turn_claude_starts_on_its_own_ends() -> anyhow::Result<()>
    {
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host) = reviewing(dir.path())?;
        // The agent hands parts of the review to subagents, and waits.
        sup.on_claude_message(turn_state("running")).await;
        sup.on_claude_message(task_started("a1", "Review the sync worker", "local_agent"))
            .await;
        sup.on_claude_message(result()).await;
        assert_eq!(
            waiting(&sup),
            (SessionStatus::Idle, Some("Waiting for a background task"))
        );
        sup.on_claude_message(task_ended("a1")).await;
        assert_eq!(
            waiting(&sup),
            (
                SessionStatus::Idle,
                Some("The review session stops in a minute")
            )
        );

        // Claude takes their report in a turn of its own, and posts.
        sup.on_claude_message(init()).await;
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));
        let (call, _reply) = submit_review(&mut sup, &mut host)?;
        posted(&mut sup, &call);
        sup.on_claude_message(says("Posted: one issue.")).await;
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));
        // The turn ends without a result.
        sup.on_claude_message(turn_state("idle")).await;
        assert_eq!(
            waiting(&sup),
            (
                SessionStatus::Stopped,
                Some("Finished: its review is posted")
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_review_agent_that_doesnt_post_is_reminded_then_stops_a_minute_later()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _host) = reviewing(dir.path())?;
        sup.claude = Some(stub_claude(dir.path(), &sup)?);
        sup.on_claude_message(result()).await;
        assert_eq!(
            waiting(&sup),
            (
                SessionStatus::Idle,
                Some("The review session stops in a minute")
            )
        );
        sup.on_tick().await;
        assert_eq!(waiting(&sup).0, SessionStatus::Idle);

        // Five seconds later, fctl-agent reminds it, once.
        sup.idle_since = Some(Instant::now() - REVIEW_REMINDER_AFTER);
        sup.on_tick().await;
        assert!(sup.persisted.review_nudged && sup.turn_active);
        assert_eq!(waiting(&sup), (SessionStatus::Running, None));
        let reminded = sent_to_claude(dir.path(), 3).await?;
        assert!(reminded.iter().any(|m| {
            m["message"]["content"][0]["text"]
                .as_str()
                .is_some_and(|t| t.starts_with("You haven't posted your review yet"))
        }));
        sup.on_claude_message(result()).await;
        sup.idle_since = Some(Instant::now() - REVIEW_REMINDER_AFTER);
        sup.on_tick().await;
        assert_eq!(
            waiting(&sup),
            (
                SessionStatus::Idle,
                Some("The review session stops in a minute")
            )
        );

        // Without a review, the session stops a minute after its turn.
        sup.idle_since = Some(Instant::now() - Duration::from_secs(61));
        sup.on_tick().await;
        assert_eq!(
            waiting(&sup),
            (
                SessionStatus::Stopped,
                Some("Finished without posting its review")
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_review_posted_while_the_session_waits_or_pauses_stops_it() -> anyhow::Result<()> {
        // The answer comes after the turn ended.
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host) = reviewing(dir.path())?;
        let (call, _reply) = submit_review(&mut sup, &mut host)?;
        sup.on_claude_message(result()).await;
        assert_eq!(waiting(&sup).0, SessionStatus::Idle);
        posted(&mut sup, &call);
        sup.on_tick().await;
        assert_eq!(
            waiting(&sup),
            (
                SessionStatus::Stopped,
                Some("Finished: its review is posted")
            )
        );

        // The usage limit ends the turn that posted it.
        let dir = tempfile::tempdir()?;
        let (mut sup, mut host) = reviewing(dir.path())?;
        let (call, _reply) = submit_review(&mut sup, &mut host)?;
        posted(&mut sup, &call);
        sup.set_status(
            SessionStatus::Paused,
            Some("Waiting for the usage limit to reset".into()),
        );
        sup.on_claude_message(result()).await;
        assert_eq!(waiting(&sup).0, SessionStatus::Paused);
        sup.on_tick().await;
        assert_eq!(
            waiting(&sup),
            (
                SessionStatus::Stopped,
                Some("Finished: its review is posted")
            )
        );
        Ok(())
    }

    #[test]
    fn posted_reviews_say_what_fctl_did_with_them() {
        let mut r = fctl_proto::api::PostedReview {
            state: fctl_proto::api::ReviewState::Commented,
            url: Some("https://github.com/acme/web/pull/12#pullrequestreview-80".into()),
            comments: 1,
            in_text: 1,
            resolved: vec!["PRRT_1".into()],
            still_open: 0,
            again: false,
            sent_to: None,
        };
        assert_eq!(
            posted_message(&r),
            "fctl posted the review: 2 issues (https://github.com/acme/web/pull/12#pullrequestreview-80). It marked 1 earlier finding fixed."
        );
        r.sent_to = Some(SessionId("ses_7".into()));
        assert!(posted_message(&r).ends_with(
            " fctl sent the findings to the agent of the session that opened the pull request."
        ));
    }

    fn summarize() -> AgentCall {
        AgentCall::SummarizePullRequest {
            summary: fctl_proto::api::PullRequestSummary {
                risk: fctl_proto::api::Risk::High,
                risk_reason: "It changes who may delete projects.".into(),
                overview: "Lets maintainers delete projects.".into(),
                highlight: None,
            },
        }
    }

    #[tokio::test]
    async fn only_review_agents_sum_their_pull_requests_up() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut sup, _host) = in_a_turn(dir.path())?;
        let refused = sup.call(None, summarize()).err().map(|e| e.message);
        assert_eq!(
            refused.as_deref(),
            Some(
                "This session doesn't review a pull request: `fctl review` is for review agents' sessions."
            )
        );
        let (mut sup, mut host) = reviewing(dir.path())?;
        assert!(sup.call(None, summarize()).is_ok());
        assert!(sent(&mut host).iter().any(|f| matches!(
            f,
            AgentToHost::Call {
                call: AgentCall::SummarizePullRequest { .. },
                ..
            }
        )));
        Ok(())
    }

    #[test]
    fn summaries_show_in_the_transcript() {
        let AgentCall::SummarizePullRequest { summary } = summarize() else {
            panic!("not a summary");
        };
        let mut summarized = fctl_proto::api::SummarizedPullRequest {
            repo: fctl_proto::session::RepoRef {
                owner: "acme".into(),
                name: "web".into(),
            },
            number: 12,
            url: "https://github.com/acme/web/pull/12".into(),
            summary,
            replaced: false,
        };
        let answer =
            |s: &fctl_proto::api::SummarizedPullRequest| AgentAnswer::PullRequestSummarized {
                summarized: s.clone(),
            };
        let notices = kept_notices(&answer(&summarized));
        let [n] = &notices[..] else {
            panic!("{notices:?}");
        };
        assert_eq!(n.code, "pull_request_summarized");
        assert_eq!(n.summarized.as_ref(), Some(&summarized));
        assert_eq!(
            n.message,
            "fctl put the review agent's summary at the end of the description of acme/web#12 (https://github.com/acme/web/pull/12): high risk."
        );
        summarized.replaced = true;
        assert!(summarized_message(&summarized).ends_with(
            "(https://github.com/acme/web/pull/12), in place of an earlier review's: high risk."
        ));
    }
}
