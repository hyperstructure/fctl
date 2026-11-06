//! One task per session: sandbox, fctl-agent, and the bridge between the
//! agent and the session's uplink.

use crate::caches::{self, Spot};
use crate::claude_token::{self, Secrets};
use crate::config::{HostConfig, Images};
use crate::driver::{AgentIo, DiskSpot, Driver, Sandbox, SandboxSpec, sandbox_name};
use crate::files;
use crate::liveness::{Check, Health, Reach, Reached, Retry};
use crate::uplink::Uplink;
use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fctl_domain::disk;
use fctl_domain::image::ImageWant;
use fctl_domain::size::{self, Want};
use fctl_proto::agent::{AgentConfig, AgentToHost, HostToAgent, MAX_FRAME_BYTES, SandboxCache};
use fctl_proto::api::{AgentAnswer, AgentCall};
use fctl_proto::event::{LogLevel, LogLine, LogSource, Notice, NoticeLevel, PublishedFile};
use fctl_proto::runner::{
    ClaudeCredential, CredentialKind, DriverInfo, GithubCredential, SandboxLimits, SandboxSize,
    SessionSpec, StopReason,
};
use fctl_proto::session::SessionStatus;
use fctl_proto::uplink::{
    AgentUnreachable, Delivery, DeliveryKind, FEATURE_ATTACHMENTS, FEATURE_SETTINGS, SandboxPaused,
    SessionState, UplinkDown, UplinkUp,
};
use fctl_proto::{ApiError, DeliveryId, ErrorCode, Millis, RunnerId, SessionId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// What every session task shares.
pub struct HostCtx {
    pub cfg: HostConfig,
    pub driver: Driver,
    pub token: String,
    pub runner_id: Mutex<Option<RunnerId>>,
    /// What this host gives each sandbox, as last measured.
    pub sandbox: Mutex<SandboxLimits>,
    /// The drivers this host could use, as last probed.
    pub drivers: Mutex<Vec<DriverInfo>>,
    /// Where the disk that sandboxes take is measured, as last probed.
    pub disk_spot: Mutex<DiskSpot>,
    /// Held while an image builds: one build at a time.
    pub builds: tokio::sync::Mutex<()>,
    /// One per image being built: sessions that need the same one wait for
    /// the session that builds it.
    pub build_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Held while `builds.json` changes.
    pub build_record: Mutex<()>,
    /// The free disk (MiB) where sandboxes are, as last measured: sessions
    /// pause their sandboxes while it is nearly full
    /// (`fctl_domain::disk::holds`). `None` until measured, and while it
    /// can't be.
    pub disk: tokio::sync::watch::Sender<Option<u64>>,
}

impl HostCtx {
    /// Before [`refresh_facts`](Self::refresh_facts): sandboxes sized by
    /// this machine, no drivers probed, and the disk measured in the data
    /// directory.
    pub fn new(cfg: HostConfig, driver: Driver, token: String) -> Self {
        let sandbox = cfg.sandbox_limits(crate::driver::machine());
        let disk_spot = DiskSpot::Dir(cfg.data_dir());
        Self {
            cfg,
            driver,
            token,
            runner_id: Mutex::new(None),
            sandbox: Mutex::new(sandbox),
            drivers: Mutex::new(Vec::new()),
            disk_spot: Mutex::new(disk_spot),
            builds: tokio::sync::Mutex::new(()),
            build_locks: Mutex::new(HashMap::new()),
            build_record: Mutex::new(()),
            disk: tokio::sync::watch::channel(None).0,
        }
    }

    /// The lock of the build of `tag`, shared by the sessions that need it.
    pub fn build_lock(&self, tag: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .build_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        locks.retain(|_, lock| Arc::strong_count(lock) > 1);
        locks.entry(tag.to_owned()).or_default().clone()
    }

    /// The images and builds this host's sessions may start from.
    pub fn images(&self) -> Images {
        self.cfg.images(self.driver.builds_images())
    }

    pub fn runner_id(&self) -> RunnerId {
        self.runner_id
            .lock()
            .ok()
            .and_then(|r| r.clone())
            .unwrap_or_else(|| RunnerId("rnr_unknown".into()))
    }

    pub fn sandbox_limits(&self) -> SandboxLimits {
        *self.sandbox.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Why the driver this host uses isn't ready, as last probed; `None`
    /// when it is, or hasn't been probed.
    pub fn driver_problem(&self) -> Option<String> {
        let name = self.driver.name();
        self.drivers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|d| d.name == name && !d.available)
            .map(|d| d.detail.clone().unwrap_or_else(|| "not available".into()))
    }

    pub fn drivers(&self) -> Vec<DriverInfo> {
        self.drivers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether the drivers differ from `said`, what the control plane last
    /// heard of them.
    pub fn drivers_differ(&self, said: &[DriverInfo]) -> bool {
        *self.drivers.lock().unwrap_or_else(PoisonError::into_inner) != said
    }

    /// Probes the driver this host uses again, now, and notes what it found
    /// for the next heartbeat: why it isn't ready, or `None` when it is. A
    /// sandbox it failed to make may have failed for that: Docker stopped
    /// since the last probe, say.
    pub async fn recheck_driver(&self) -> Option<String> {
        let info = self.driver.probe().await;
        let why = (!info.available).then(|| {
            info.detail
                .clone()
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| "not available".into())
        });
        let mut drivers = self.drivers.lock().unwrap_or_else(PoisonError::into_inner);
        match drivers.iter_mut().find(|d| d.name == info.name) {
            Some(d) => *d = info,
            None => drivers.push(info),
        }
        why
    }

    /// Probes the drivers and measures what sandboxes run on again (Docker's
    /// VM may have been resized), and finds where their disk is. The
    /// runner's hello reports what this found last, so the control
    /// connection never waits on a runtime; a Docker daemon that doesn't
    /// answer leaves the limits and the disk's place as they were. First it
    /// starts Apple's `container` service when that is the driver and it
    /// isn't running (the Mac restarted, say), which may take minutes: when
    /// the host starts, it connects only after that, as it would take no
    /// sessions before.
    pub async fn refresh_facts(&self) {
        self.driver.start_service().await;
        let data_dir = self.cfg.data_dir();
        let (drivers, size, spot) = tokio::join!(
            Driver::probe_all(&self.cfg),
            self.driver.measure(),
            self.driver.disk_spot(&data_dir)
        );
        *self.drivers.lock().unwrap_or_else(PoisonError::into_inner) = drivers;
        if let Some(size) = size {
            *self.sandbox.lock().unwrap_or_else(PoisonError::into_inner) =
                self.cfg.sandbox_limits(size);
        }
        if let Some(spot) = spot {
            *self
                .disk_spot
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = spot;
        }
    }

    pub fn disk_spot(&self) -> DiskSpot {
        self.disk_spot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Free MiB where sandboxes keep their files, or `None` when it can't be
    /// measured. Inside the Docker VM this starts a container of the host's
    /// image.
    pub async fn disk_free_mb(&self) -> Option<u64> {
        let images = self.cfg.own_images();
        self.driver.disk_free_mb(&self.disk_spot(), &images).await
    }

    /// The CPUs and memory a session's sandbox gets on this host.
    pub fn sandbox_size(&self, spec: &SessionSpec) -> SandboxSize {
        Want::of(&spec.project.settings).on(&self.sandbox_limits())
    }

    /// Where a session's sandbox finds its project's package caches here:
    /// none when the project doesn't share them, or for a review session,
    /// whose pull request could steer its agent to leave things there for
    /// the project's other sessions; why not, when the project shares them
    /// and this host keeps none.
    pub fn cache_of(&self, spec: &SessionSpec) -> std::result::Result<Option<Spot>, String> {
        if !spec.project.settings.package_caches || spec.review.is_some() {
            return Ok(None);
        }
        let without = "so this session's package managers download what they need";
        if self.cfg.max_cache_mb().is_none() {
            return Err(format!(
                "{} keeps no package caches (max_cache_gb = 0 in its host.toml), {without}",
                self.cfg.name
            ));
        }
        let project = spec.project.id.as_str();
        if !caches::valid(project) {
            return Err(format!(
                "The project's id, {project}, can't name its package caches, {without}"
            ));
        }
        match self.driver.cache_spot(project, &self.cfg.data_dir()) {
            Some(spot) => Ok(Some(spot)),
            None => Err(format!(
                "{} keeps no package caches with the {} driver, {without}",
                self.cfg.name,
                self.driver.name()
            )),
        }
    }

    pub fn uplink_url(&self, session: &SessionId) -> String {
        format!(
            "{}/api/v1/runner/sessions/{}/uplink",
            self.cfg.server.trim_end_matches('/'),
            session
        )
    }
}

pub enum Start {
    New(Box<SessionSpec>),
    /// A sandbox found running after a host restart.
    Recover(SessionId),
}

pub enum SessionCmd {
    Stop {
        reason: StopReason,
        destroy: bool,
    },
    /// A newer run of this session waits to start: skip the final status.
    Superseded,
    Credentials {
        request_id: Option<String>,
        claude: Option<ClaudeCredential>,
        github: Option<GithubCredential>,
        github_read: Option<GithubCredential>,
        error: Option<String>,
    },
}

pub enum SessionEvent {
    CredentialRequest {
        session_id: SessionId,
        request_id: String,
        kind: CredentialKind,
    },
    /// The run ended. `Queued`: it never started, since the driver isn't
    /// ready (`reason` says so), and the control plane queues it again.
    Ended {
        session_id: SessionId,
        status: SessionStatus,
        reason: Option<String>,
        run_id: Option<String>,
    },
}

#[derive(Clone)]
pub struct Report {
    pub status: SessionStatus,
    pub agent_seq: u64,
}

pub struct SessionHandle {
    pub tx: mpsc::UnboundedSender<SessionCmd>,
    pub report: Arc<Mutex<Report>>,
}

pub fn spawn(
    ctx: Arc<HostCtx>,
    start: Start,
    events: mpsc::UnboundedSender<SessionEvent>,
) -> SessionHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    let report = Arc::new(Mutex::new(Report {
        status: SessionStatus::Provisioning,
        agent_seq: 0,
    }));
    let r = report.clone();
    tokio::spawn(async move {
        let (id, run_id) = match &start {
            Start::New(spec) => (spec.session_id.clone(), spec.run_id.clone()),
            Start::Recover(id) => (id.clone(), None),
        };
        let (status, reason) = run(ctx, start, rx, events.clone(), r).await;
        let _ = events.send(SessionEvent::Ended {
            session_id: id,
            status,
            reason,
            run_id,
        });
    });
    SessionHandle { tx, report }
}

/// A framed connection to fctl-agent. Dropping it kills the exec that
/// carries it.
struct AgentLink {
    tx: mpsc::UnboundedSender<HostToAgent>,
    rx: mpsc::UnboundedReceiver<AgentToHost>,
    health: Health,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    _child: Option<tokio::process::Child>,
}

impl Drop for AgentLink {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

impl AgentLink {
    /// Frames from the agent go without `secrets`: Claude tokens, which
    /// must not reach the control plane in what it sends there.
    fn open(io: AgentIo, sandbox: &str, secrets: Secrets) -> Self {
        let (tx, mut out_rx) = mpsc::unbounded_channel::<HostToAgent>();
        let (in_tx, rx) = mpsc::unbounded_channel::<AgentToHost>();
        let mut tasks = Vec::new();
        let mut child = io.child;
        // Why an exec failed: the runtime's errors, or fctl-agent's.
        if let Some(stderr) = child.as_mut().and_then(|c| c.stderr.take()) {
            let sandbox = sandbox.to_owned();
            tasks.push(tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        tracing::warn!("attaching to {sandbox}: {}", line.trim());
                    }
                }
            }));
        }
        let mut writer = io.writer;
        tasks.push(tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                let Ok(mut line) = serde_json::to_string(&msg) else {
                    continue;
                };
                line.push('\n');
                if writer.write_all(line.as_bytes()).await.is_err() || writer.flush().await.is_err()
                {
                    break;
                }
            }
        }));
        let reader = io.reader;
        tasks.push(tokio::spawn(async move {
            let mut reader = BufReader::with_capacity(256 * 1024, reader);
            let mut line = String::new();
            loop {
                line.clear();
                // What runs in the sandbox can write to the link: a line
                // without an end mustn't fill the host's memory, where other
                // sessions run.
                let mut frame = (&mut reader).take(MAX_FRAME_BYTES as u64 + 1);
                match frame.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) if n > MAX_FRAME_BYTES => break,
                    Ok(_) => {}
                }
                if line.trim().is_empty() {
                    continue;
                }
                let line = secrets.redact(&line);
                match serde_json::from_str::<AgentToHost>(line.trim()) {
                    Ok(m) => {
                        if in_tx.send(m).is_err() {
                            break;
                        }
                    }
                    Err(e) => tracing::warn!("malformed frame from fctl-agent: {e}"),
                }
            }
        }));
        Self {
            tx,
            rx,
            health: Health::new(Instant::now()),
            tasks,
            _child: child,
        }
    }

    fn send(&self, msg: HostToAgent) {
        let _ = self.tx.send(msg);
    }
}

fn state(status: SessionStatus, reason: &str) -> SessionState {
    SessionState {
        status,
        reason: Some(reason.to_owned()),
        resume_at: None,
        branch: None,
        model: None,
        effort: None,
        claude_session_id: None,
        remote_control: None,
        stats: None,
        title: None,
        description: None,
        context: None,
    }
}

/// A delivery from the control plane: messages wait for their files (in
/// order), interrupts go straight to the agent.
fn route(
    delivery: Delivery,
    jobs: &mpsc::UnboundedSender<files::Job>,
    link: Option<&AgentLink>,
    queued: &mut Vec<HostToAgent>,
) {
    if matches!(delivery.kind, DeliveryKind::Message { .. }) {
        let _ = jobs.send(files::Job::Deliver(delivery));
        return;
    }
    match link {
        Some(l) => l.send(HostToAgent::Deliver { delivery }),
        None => queued.push(HostToAgent::Deliver { delivery }),
    }
}

/// A change of model or effort level (`DeliveryKind::Settings`): an agent
/// that starts again in this run (`agent_config`) starts Claude on it.
fn follow_settings(spec: &mut SessionSpec, delivery: &Delivery) {
    if let DeliveryKind::Settings { model, effort, .. } = &delivery.kind {
        if let Some(m) = model {
            spec.model = Some(m.clone());
        }
        if let Some(e) = effort {
            spec.effort = Some(e.clone());
        }
    }
}

/// Takes the changes of model or effort level out of what waits for an
/// agent that said hello without them, which couldn't read them, and
/// returns their ids.
fn refuse_settings(
    unconfirmed: &mut Vec<Delivery>,
    queued: &mut Vec<HostToAgent>,
) -> Vec<DeliveryId> {
    let mut refused: Vec<DeliveryId> = unconfirmed
        .iter()
        .filter(|d| d.kind.is_settings())
        .map(|d| d.id.clone())
        .collect();
    unconfirmed.retain(|d| !d.kind.is_settings());
    queued.retain(|f| match f {
        HostToAgent::Deliver { delivery } if delivery.kind.is_settings() => {
            if !refused.contains(&delivery.id) {
                refused.push(delivery.id.clone());
            }
            false
        }
        _ => true,
    });
    refused
}

/// What the control plane hears of a change of model or effort level that
/// the agent in the sandbox can't make.
fn older_agent(delivery_id: DeliveryId) -> UplinkUp {
    UplinkUp::Delivered {
        delivery_id,
        ok: false,
        error: Some(
            "The agent in this sandbox started on an older fctl and can't change the model or the effort level while Claude runs. Stop and resume the session, then change them again.".into(),
        ),
    }
}

/// Sends the deliveries the agent hasn't confirmed over a new link, but
/// not those already waiting for one.
fn resend(
    unconfirmed: &[Delivery],
    jobs: &mpsc::UnboundedSender<files::Job>,
    link: &AgentLink,
    queued: &mut Vec<HostToAgent>,
) {
    let waiting = |d: &Delivery| {
        queued
            .iter()
            .any(|f| matches!(f, HostToAgent::Deliver { delivery } if delivery.id == d.id))
    };
    let again: Vec<Delivery> = unconfirmed
        .iter()
        .filter(|d| !waiting(d))
        .cloned()
        .collect();
    for d in again {
        route(d, jobs, Some(link), queued);
    }
}

/// Keeps `delivery` until the agent confirms it (the latest copy of each).
fn remember(unconfirmed: &mut Vec<Delivery>, delivery: &Delivery) {
    match unconfirmed.iter_mut().find(|d| d.id == delivery.id) {
        Some(d) => *d = delivery.clone(),
        None => unconfirmed.push(delivery.clone()),
    }
}

/// Sends a frame to the agent. An agent older than attachments gets its
/// messages without their files, and the transcript says so.
fn to_agent(l: &AgentLink, frame: HostToAgent, agent_files: bool, uplink: &Uplink) {
    if !agent_files {
        match &frame {
            HostToAgent::File { .. } => return,
            HostToAgent::Deliver {
                delivery:
                    Delivery {
                        kind: DeliveryKind::Message { attachments, .. },
                        ..
                    },
            } if !attachments.is_empty() => host_log(
                uplink,
                LogLevel::Warn,
                "The agent in this sandbox started on an older fctl and cannot take files, so it gets this message without its attachments. Stop and resume the session to update it.".into(),
            ),
            _ => {}
        }
    }
    l.send(frame);
}

/// What publishing a screenshot came to, for the agent that asked.
type Published = (String, std::result::Result<PublishedFile, ApiError>);

/// The most screenshots and calls one session's agent has the host send at
/// once. What runs in a sandbox can pose as its agent, and each holds up
/// to a frame's worth of memory on a host that runs other sessions too.
const MAX_IN_FLIGHT: usize = 8;

/// Why the host doesn't take another screenshot or call from a session's
/// agent now: it has [`MAX_IN_FLIGHT`] of them under way.
fn busy(in_flight: usize) -> Option<ApiError> {
    (in_flight >= MAX_IN_FLIGHT).then(|| {
        ApiError::new(
            ErrorCode::RateLimited,
            "The host is still sending this session's other screenshots and calls. Try again in a moment.",
        )
    })
}

/// Why the host doesn't upload a screenshot: its bytes (`data`, base64)
/// are more than the control plane takes, which fctl-agent checks too.
fn oversized(data: &str) -> Option<ApiError> {
    let most = 4 * (fctl_domain::published::MAX_BYTES as usize).div_ceil(3);
    (data.len() > most).then(|| {
        ApiError::validation(
            "file",
            format!(
                "Screenshots can be at most {}",
                fctl_domain::files::human_size(fctl_domain::published::MAX_BYTES)
            ),
        )
    })
}

/// Uploads a screenshot the agent publishes (`fctl publish`), retrying what
/// may pass, and hands the answer back for the agent.
fn publish(
    api: fctl_client::Api,
    session: SessionId,
    request_id: String,
    name: String,
    data: String,
    done: mpsc::UnboundedSender<Published>,
) {
    tokio::spawn(async move {
        let result = match STANDARD.decode(data.as_bytes()) {
            Ok(bytes) => upload(&api, &session, &name, bytes).await,
            Err(e) => Err(ApiError::bad_request(format!(
                "The screenshot's bytes are not base64: {e}"
            ))),
        };
        match &result {
            Ok(file) => tracing::info!("published {} for {session}", file.name),
            Err(e) => tracing::warn!("publishing {name} for {session}: {}", e.message),
        }
        let _ = done.send((request_id, result));
    });
}

async fn upload(
    api: &fctl_client::Api,
    session: &SessionId,
    name: &str,
    bytes: Vec<u8>,
) -> std::result::Result<PublishedFile, ApiError> {
    let mut delay = Duration::from_secs(1);
    let mut attempt = 1;
    loop {
        match api
            .runner_publish(session.as_str(), name, bytes.clone())
            .await
        {
            Ok(file) => return Ok(file),
            Err(e) if e.is_transient() && attempt < 3 => {
                tracing::warn!("publishing {name} for {session} (attempt {attempt}): {e}");
                tokio::time::sleep(delay).await;
                delay *= 3;
                attempt += 1;
            }
            Err(e) => {
                return Err(e.api().cloned().unwrap_or_else(|| {
                    ApiError::new(
                        ErrorCode::Unavailable,
                        format!("The host could not reach the control plane: {e}"),
                    )
                }));
            }
        }
    }
}

/// What the control plane answered an agent's call, for the agent that
/// asked.
type Answered = (String, std::result::Result<AgentAnswer, ApiError>);

/// Sends what the agent asks about its project (`fctl memory`, `fctl
/// retro`) to the control plane, and hands the answer back for the agent.
fn call(
    api: fctl_client::Api,
    session: SessionId,
    request_id: String,
    call: AgentCall,
    done: mpsc::UnboundedSender<Answered>,
) {
    tokio::spawn(async move {
        let result = ask(&api, &session, &call).await;
        if let Err(e) = &result {
            tracing::warn!("a call of the agent of {session}: {}", e.message);
        }
        let _ = done.send((request_id, result));
    });
}

/// Retries what may pass, where asking twice changes nothing: reading the
/// memory, the session's retrospectives, a review's threads or the
/// project's tickets, marking a note stale, updating a retrospective,
/// resolving a review thread and putting a review's summary in its pull
/// request's description, which takes the place of the one there. A
/// new note, retrospective or review that may have reached the control
/// plane isn't sent again, so that none is kept or posted twice; the agent
/// hears what went wrong and can ask again.
async fn ask(
    api: &fctl_client::Api,
    session: &SessionId,
    call: &AgentCall,
) -> std::result::Result<AgentAnswer, ApiError> {
    let again = match call {
        AgentCall::Memories { .. }
        | AgentCall::MarkStale { .. }
        | AgentCall::Retrospectives
        | AgentCall::UpdateRetrospective { .. }
        | AgentCall::ResolveThread { .. }
        | AgentCall::ReviewThreads
        | AgentCall::SummarizePullRequest { .. } => true,
        // What reads the tracker; a request this host doesn't know may write.
        AgentCall::Ticket { request } => {
            serde_json::from_value::<fctl_proto::tickets::TicketRequest>(request.clone())
                .is_ok_and(|r| r.reads())
        }
        _ => false,
    };
    let mut delay = Duration::from_secs(1);
    let mut attempt = 1;
    loop {
        match api.runner_call(session.as_str(), call).await {
            Ok(answer) => return Ok(answer),
            Err(e) if again && e.is_transient() && attempt < 3 => {
                tracing::warn!("a call of the agent of {session} (attempt {attempt}): {e}");
                tokio::time::sleep(delay).await;
                delay *= 3;
                attempt += 1;
            }
            Err(e) => {
                return Err(e.api().cloned().unwrap_or_else(|| {
                    ApiError::new(
                        ErrorCode::Unavailable,
                        format!("The host could not reach the control plane: {e}"),
                    )
                }));
            }
        }
    }
}

/// How long a sandbox that stayed paused as it was resumed waits before the
/// host tries again.
const RESUME_AGAIN_AFTER: Duration = Duration::from_secs(10);

fn host_log(uplink: &Uplink, level: LogLevel, text: String) {
    uplink.log(LogLine {
        source: LogSource::Host,
        level,
        text,
    });
}

/// Tells the session's viewers whether the host reaches the agent, and
/// whether it holds the sandbox paused, which each such message says.
fn report_reach(uplink: &Uplink, reached: Reached, paused: Option<&SandboxPaused>) {
    let unreachable = match reached {
        Reached::Not { since, reason } => Some(AgentUnreachable {
            since: wall_ms(since),
            reason,
        }),
        Reached::Again => None,
    };
    uplink.send(UplinkUp::AgentLink {
        unreachable,
        paused: paused.cloned(),
    });
}

/// `t` as milliseconds since the Unix epoch.
fn wall_ms(t: Instant) -> Millis {
    let ago = Instant::now().saturating_duration_since(t).as_millis();
    crate::expiry::now_ms() - i64::try_from(ago).unwrap_or(i64::MAX)
}

/// Why a session couldn't start, as its status says: a full disk by name,
/// ahead of what went wrong.
fn failure(runner: &str, e: &anyhow::Error) -> String {
    if disk::out_of_space(e.as_ref()) {
        format!("Runner {runner} ran out of disk: {e:#}")
    } else {
        format!("{e:#}")
    }
}

/// `with 800 MiB free`, or nothing for a disk not measured yet.
fn with_free(free_mb: Option<u64>) -> String {
    free_mb.map_or_else(String::new, |f| format!(", with {} free", size::memory(f)))
}

/// What the session's viewers hear of a sandbox the host paused.
fn paused_for(free_mb: Option<u64>) -> SandboxPaused {
    SandboxPaused {
        since: crate::expiry::now_ms(),
        reason: format!("it is nearly out of disk{}", with_free(free_mb)),
    }
}

/// What the transcript says as the host pauses the sandbox.
fn paused_notice(runner: &str, free_mb: Option<u64>) -> Notice {
    Notice::new(
        NoticeLevel::Warn,
        "sandbox_paused",
        format!(
            "Runner {runner} is nearly out of disk{}, so its host paused this session's sandbox, where what the agent writes would be lost. The sandbox keeps what it holds, and the agent carries on where it was once there is room.",
            with_free(free_mb)
        ),
    )
}

/// What the transcript says as the host resumes the sandbox: it has room
/// again, or the session stops.
fn resumed_notice(runner: &str, free_mb: Option<u64>, stopping: bool) -> Notice {
    let message = if stopping {
        format!("Runner {runner} resumed this session's sandbox to stop it.")
    } else {
        format!(
            "Runner {runner} has room on its disk again{}, so its host resumed this session's sandbox.",
            with_free(free_mb)
        )
    };
    Notice::new(NoticeLevel::Info, "sandbox_resumed", message)
}

/// What the transcript says as the host starts fctl-agent again while its
/// disk is below its floor, which may be why the agent stopped answering.
/// Nothing with room.
fn restart_notice(runner: &str, free_mb: Option<u64>, min_free_mb: Option<u64>) -> Option<Notice> {
    let free = size::memory(free_mb?);
    let what = match disk::room(free_mb, min_free_mb)? {
        fctl_proto::runner::DiskRoom::Out => format!(
            "fctl-agent stopped answering in the sandbox, perhaps because runner {runner} is nearly out of disk, with {free} free"
        ),
        fctl_proto::runner::DiskRoom::Low => format!(
            "fctl-agent stopped answering in the sandbox while runner {runner} is low on disk, with {free} free, below the {} its host keeps free",
            size::memory(min_free_mb?)
        ),
    };
    Some(Notice::new(
        NoticeLevel::Warn,
        "agent_restarted",
        format!("{what}. Its host is starting it again."),
    ))
}

/// What the transcript says when starting fctl-agent again failed for want
/// of disk.
fn start_failed_notice(runner: &str, e: &anyhow::Error) -> Notice {
    Notice::new(
        NoticeLevel::Error,
        "agent_restarted",
        format!(
            "Its host couldn't start fctl-agent in the sandbox again: runner {runner} ran out of disk ({e:#}). It tries again."
        ),
    )
}

/// Makes the session's sandbox, or starts the one it has, and starts
/// fctl-agent there: the sandbox, and where the agent finds the project's
/// package caches.
async fn prepare(
    ctx: &HostCtx,
    spec: &SessionSpec,
    uplink: &Uplink,
) -> Result<(Sandbox, Option<SandboxCache>)> {
    let name = sandbox_name(spec.session_id.as_str());
    let log_uplink = |line: String| host_log(uplink, LogLevel::Info, line);
    let want = ImageWant::of(&spec.project.settings, spec.project.repo.as_ref());
    let (image, fallback_image) = match want {
        // A sandbox that is here already keeps the image it was made from.
        ImageWant::Build(_)
            if ctx.driver.builds_images() && !ctx.driver.sandbox_exists(&name).await? =>
        {
            uplink.state(state(
                SessionStatus::Provisioning,
                &format!("Getting the sandbox image ready on {}", ctx.cfg.name),
            ));
            (crate::build::image(ctx, spec, &log_uplink).await?, None)
        }
        // The control plane checked the reference; the host takes none that
        // its runtime could read as anything else (`-…`, a flag).
        ImageWant::Image(image) => match fctl_domain::image::ImageRef::parse(&image) {
            Some(_) if !image.starts_with('-') => (image, None),
            _ => anyhow::bail!("The project's image isn't an image reference: {image}"),
        },
        _ => (ctx.cfg.image(), ctx.cfg.fallback_image()),
    };
    let size = ctx.sandbox_size(spec);
    let cache = match ctx.cache_of(spec) {
        Ok(cache) => cache,
        Err(why) => {
            log_uplink(why);
            None
        }
    };
    // Apple's `container` mounts a directory that must be there, also for
    // a sandbox made with the caches of a project that no longer shares
    // them, and the process driver's agents use it directly. Without it,
    // the session runs without caches.
    let make = |spot: &Spot| {
        match spot {
        Spot::Dir(dir) => std::fs::create_dir_all(dir).map_err(|e| {
            format!(
                "Making the package caches' directory {} failed ({e}), so this session's package managers download what they need",
                dir.display()
            )
        }),
        Spot::Volume(_) => Ok(()),
    }
    };
    let data = ctx.cfg.data_dir();
    let mounted = caches::load(&caches::path(&data))
        .sandboxes
        .get(&name)
        .and_then(|project| ctx.driver.cache_spot(project, &data));
    if let Some(Err(why)) = mounted.as_ref().map(make) {
        tracing::warn!("{why}");
    }
    let cache = cache.filter(|spot| match make(spot) {
        Ok(()) => true,
        Err(why) => {
            log_uplink(why);
            false
        }
    });
    let for_agent = cache.as_ref().map(|spot| SandboxCache {
        dir: ctx.driver.cache_in_sandbox(spot),
        max_mb: ctx.cfg.max_cache_mb(),
    });
    let sspec = SandboxSpec {
        name,
        image,
        fallback_image,
        cpus: size.cpus,
        memory_mb: size.memory_mb,
        cache,
    };
    let sb = ctx.driver.ensure(&sspec, &log_uplink).await?;
    if ctx.driver.needs_linux_agent() {
        let bin = crate::agentbin::linux_agent(&ctx.cfg)
            .await
            .context("getting fctl-agent for the sandbox")?;
        ctx.driver.install_agent(&sb, &bin).await?;
    }
    ctx.driver.start_agent(&sb).await?;
    Ok((sb, for_agent))
}

/// Why this host takes no session now: `studio's docker driver isn't ready:
/// the Docker daemon isn't running`.
pub fn not_ready(ctx: &HostCtx, why: &str) -> String {
    format!(
        "{}'s {} driver isn't ready: {why}",
        ctx.cfg.name,
        ctx.driver.name()
    )
}

/// Whether someone asked to stop the session, of what came for it so far.
fn stop_asked(cmds: &mut mpsc::UnboundedReceiver<SessionCmd>) -> bool {
    let mut asked = false;
    while let Ok(cmd) = cmds.try_recv() {
        asked |= matches!(cmd, SessionCmd::Stop { .. });
    }
    asked
}

/// Starts fctl-agent in a sandbox that has one, perhaps of an older host,
/// which may not run as root (`serve --user`): this host's goes in first.
/// One that runs keeps running.
async fn start_again(ctx: &HostCtx, sb: &Sandbox) -> Result<()> {
    if ctx.driver.needs_linux_agent() {
        let installed = async {
            let bin = crate::agentbin::linux_agent(&ctx.cfg)
                .await
                .context("getting fctl-agent for the sandbox")?;
            ctx.driver.install_agent(sb, &bin).await
        };
        if let Err(e) = installed.await {
            tracing::warn!("putting this host's fctl-agent in {}: {e:#}", sb.name);
        }
    }
    ctx.driver.start_agent(sb).await
}

async fn run(
    ctx: Arc<HostCtx>,
    start: Start,
    mut cmds: mpsc::UnboundedReceiver<SessionCmd>,
    events: mpsc::UnboundedSender<SessionEvent>,
    report: Arc<Mutex<Report>>,
) -> (SessionStatus, Option<String>) {
    let (session_id, mut spec) = match start {
        Start::New(spec) => (spec.session_id.clone(), Some(spec)),
        Start::Recover(id) => (id, None),
    };
    // The Claude tokens its agent has: the one it starts with, those it
    // gets later, and the one this host keeps (a session found running
    // after a restart may have it), kept out of what the agent sends.
    let secrets = Secrets::default();
    if let Some(spec) = &spec {
        secrets.add(&spec.credentials.claude.access_token);
    }
    if let Ok(Some(kept)) = claude_token::load(&ctx.cfg.claude_token()) {
        secrets.add(&kept.token);
    }
    // Claude streams what it writes in pieces, which may split a token.
    let mut streaming = claude_token::StreamRedactor::default();
    let (uplink, mut down) = Uplink::start(
        ctx.uplink_url(&session_id),
        ctx.token.clone(),
        ctx.runner_id(),
    );
    let api = fctl_client::Api::new(&ctx.cfg.server, Some(ctx.token.clone()));
    let (jobs, mut ready) = files::start(api.clone(), session_id.clone());
    let (published_tx, mut published_rx) = mpsc::unbounded_channel::<Published>();
    let (answered_tx, mut answered_rx) = mpsc::unbounded_channel::<Answered>();
    // Screenshots and calls under way (`MAX_IN_FLIGHT`).
    let mut in_flight: usize = 0;
    let set_report = |status: SessionStatus, seq: Option<u64>| {
        if let Ok(mut r) = report.lock() {
            r.status = status;
            if let Some(s) = seq {
                r.agent_seq = r.agent_seq.max(s);
            }
        }
    };
    // Where the agent finds the project's package caches.
    let mut cache = None;
    // A sandbox found paused as the host restarted.
    let mut found_paused = false;
    let sb = match &spec {
        Some(spec) => {
            uplink.state(state(
                SessionStatus::Provisioning,
                &format!("Creating the sandbox on {}", ctx.cfg.name),
            ));
            // A stop calls off the preparation: a runtime that hangs may
            // keep it waiting until its commands time out.
            let preparing = prepare(&ctx, spec, &uplink);
            tokio::pin!(preparing);
            let mut listening = true;
            let prepared = loop {
                tokio::select! {
                    r = &mut preparing => break Ok(r),
                    c = cmds.recv(), if listening => match c {
                        Some(SessionCmd::Stop { destroy, .. }) => break Err(destroy),
                        // Nothing else is for an agent that isn't there yet.
                        Some(_) => {}
                        None => listening = false,
                    },
                }
            };
            match prepared {
                Ok(Ok((sb, for_agent))) => {
                    cache = for_agent;
                    sb
                }
                Ok(Err(e)) => {
                    let msg = failure(&ctx.cfg.name, &e);
                    // The driver may have stopped being ready since the host
                    // last probed it (Docker quit, say): then the session
                    // goes back to the control plane's queue, to start once
                    // a runner can start it, rather than fail. Not when
                    // someone stopped it meanwhile.
                    if let Some(why) = ctx.recheck_driver().await
                        && !stop_asked(&mut cmds)
                    {
                        let reason = not_ready(&ctx, &why);
                        tracing::warn!("{session_id} goes back to the queue: {reason} ({msg})");
                        host_log(
                            &uplink,
                            LogLevel::Warn,
                            format!(
                                "{reason}, so the session waits for a runner that can start it"
                            ),
                        );
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        return (SessionStatus::Queued, Some(reason));
                    }
                    host_log(&uplink, LogLevel::Error, msg.clone());
                    uplink.state(state(SessionStatus::Failed, &msg));
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    return (SessionStatus::Failed, Some(msg));
                }
                Err(destroy) => {
                    let sb = ctx.driver.handle(&sandbox_name(session_id.as_str()));
                    if let Err(e) = ctx.driver.stop(&sb).await {
                        tracing::warn!("stopping {}: {e:#}", sb.name);
                    }
                    if destroy && let Err(e) = ctx.driver.remove(&sb).await {
                        tracing::warn!("removing {}: {e:#}", sb.name);
                    }
                    let msg = "Stopped before it started".to_owned();
                    uplink.state(state(SessionStatus::Stopped, &msg));
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    return (SessionStatus::Stopped, Some(msg));
                }
            }
        }
        None => {
            let sb = ctx.driver.handle(&sandbox_name(session_id.as_str()));
            // Paused while the disk was nearly full: it stays so until the
            // disk has room, and its agent runs on once it resumes.
            if ctx.driver.pauses() && ctx.driver.paused(&sb).await {
                tracing::info!("{} is paused until there is room on the disk", sb.name);
                found_paused = true;
            } else if let Err(e) = start_again(&ctx, &sb).await {
                tracing::warn!("restarting fctl-agent in {}: {e:#}", sb.name);
            }
            sb
        }
    };

    let mut acked: u64 = 0;
    let mut welcomed = false;
    let mut link: Option<AgentLink> = None;
    // Frames for the agent while none is attached, in order.
    let mut queued: Vec<HostToAgent> = Vec::new();
    // Whether the attached agent takes `File` frames.
    let mut agent_files = false;
    // Whether the attached agent changes the model and effort level Claude
    // works at; until it says hello, it may.
    let mut agent_settings = true;
    let mut exited: Option<(SessionStatus, Option<String>)> = None;
    let mut stop: Option<(bool, Instant)> = None;
    let mut superseded = false;
    // A fresh agent for a resumed session numbers its events after the ones
    // the control plane has, so it waits for the uplink's Welcome.
    let mut configure_when_welcomed = false;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    // Attaching runs in a task of its own, so that a runtime that hangs
    // doesn't hold up the session; after a failure it waits, and starts
    // fctl-agent again first in case it is gone (harmless when it isn't).
    // It gives why starting it failed, if it did, and the link.
    let mut attaching: Option<tokio::task::JoinHandle<(Option<anyhow::Error>, Result<AgentIo>)>> =
        None;
    let mut attach_at = Some(Instant::now());
    let mut restart_agent = false;
    let mut retry = Retry::default();
    // Deliveries the agent hasn't confirmed. Frames sent over a link that
    // died may have died with it, so a new link gets these again (the agent
    // ignores the ones it has).
    let mut unconfirmed: Vec<Delivery> = Vec::new();
    let mut said_hello = false;
    // Whether the agent can be reached, for the session's viewers.
    let mut reach = Reach::default();
    reach.down(
        Instant::now(),
        Instant::now(),
        "no answer from fctl-agent yet",
    );
    let gave_up = || {
        let msg = "fctl-agent stopped responding in the sandbox".to_owned();
        uplink.state(state(SessionStatus::Failed, &msg));
        (SessionStatus::Failed, Some(msg))
    };
    // The free disk where sandboxes are, which pauses the sandbox while it
    // is nearly full (`disk::holds`): looked at once before anything else.
    let mut free_disk = ctx.disk.subscribe();
    free_disk.mark_changed();
    let mut measuring = true;
    // The sandbox, while the host holds it paused for want of disk.
    let mut held: Option<SandboxPaused> = None;
    if found_paused {
        let p = paused_for(*free_disk.borrow());
        uplink.send(UplinkUp::AgentLink {
            unreachable: None,
            paused: Some(p.clone()),
        });
        held = Some(p);
    }
    // Pausing failed while the disk is nearly full: tried once.
    let mut pause_failed = false;
    // Resuming failed, and the sandbox is still paused: tried again then.
    let mut resume_again_at: Option<Instant> = None;
    // Whether the transcript said why the host starts fctl-agent again,
    // since the agent last said hello.
    let mut told_restart = false;

    let outcome = loop {
        if link.is_none()
            && attaching.is_none()
            && exited.is_none()
            && held.is_none()
            && attach_at.is_some_and(|at| Instant::now() >= at)
        {
            attach_at = None;
            let restart = std::mem::take(&mut restart_agent);
            if restart
                && !told_restart
                && let Some(n) = restart_notice(
                    &ctx.cfg.name,
                    *free_disk.borrow(),
                    ctx.cfg.min_free_disk_mb(),
                )
            {
                uplink.send(UplinkUp::Notice { notice: n });
                told_restart = true;
            }
            let (ctx, sb) = (ctx.clone(), sb.clone());
            attaching = Some(tokio::spawn(async move {
                let started = if restart {
                    start_again(&ctx, &sb).await.err()
                } else {
                    None
                };
                if let Some(e) = &started {
                    tracing::warn!("starting fctl-agent in {}: {e:#}", sb.name);
                }
                (started, ctx.driver.attach(&sb).await)
            }));
        }
        tokio::select! {
            r = async { match attaching.as_mut() { Some(a) => a.await, None => std::future::pending().await } } => {
                attaching = None;
                let (started, attached) = match r {
                    Ok((started, attached)) => (started, attached),
                    Err(e) => (None, Err(anyhow::Error::from(e))),
                };
                if let Some(e) = started
                    && !told_restart
                    && disk::out_of_space(e.as_ref())
                {
                    uplink.send(UplinkUp::Notice { notice: start_failed_notice(&ctx.cfg.name, &e) });
                    told_restart = true;
                }
                match attached {
                    Ok(io) => link = Some(AgentLink::open(io, &sb.name, secrets.clone())),
                    Err(e) => {
                        tracing::warn!("attaching to {}: {e:#}", sb.name);
                        reach.down(Instant::now(), Instant::now(), format!("{e:#}"));
                        let Some(wait) = retry.failed(Instant::now()) else { break gave_up() };
                        attach_at = Some(Instant::now() + wait);
                        restart_agent = true;
                    }
                }
            }
            msg = async { match link.as_mut() { Some(l) => l.rx.recv().await, None => std::future::pending().await } } => {
                let Some(msg) = msg else {
                    let closed = link.take();
                    let answered = closed.as_ref().is_some_and(|l| l.health.answered());
                    if let Some(done) = exited.clone() {
                        break done;
                    }
                    if stop.is_some() {
                        break (SessionStatus::Stopped, Some("Stopped".into()));
                    }
                    if answered {
                        tracing::info!("the link to fctl-agent in {} closed; attaching again", sb.name);
                        let heard = closed.as_ref().map_or_else(Instant::now, |l| l.health.last_heard());
                        reach.down(heard, Instant::now(), "the link to fctl-agent closed");
                        attach_at = Some(Instant::now() + Duration::from_millis(500));
                    } else {
                        tracing::warn!("attaching to {}: the link closed before fctl-agent said hello", sb.name);
                        reach.down(Instant::now(), Instant::now(), "the link closed before fctl-agent said hello");
                        let Some(wait) = retry.failed(Instant::now()) else { break gave_up() };
                        attach_at = Some(Instant::now() + wait);
                        restart_agent = true;
                    }
                    continue;
                };
                let Some(l) = link.as_mut() else { continue };
                l.health.heard(Instant::now());
                let l = &*l;
                match msg {
                    AgentToHost::Hello(h) => {
                        retry.reset();
                        told_restart = false;
                        if let Some(r) = reach.up() {
                            report_reach(&uplink, r, held.as_ref());
                        }
                        tracing::info!(
                            "attached to fctl-agent {} in {} (journal at agent_seq {})",
                            h.version,
                            sb.name,
                            h.agent_seq
                        );
                        set_report(h.status, Some(h.agent_seq));
                        agent_files = h.features.iter().any(|f| f == FEATURE_ATTACHMENTS);
                        agent_settings = h.features.iter().any(|f| f == FEATURE_SETTINGS);
                        if !h.configured {
                            match &spec {
                                Some(spec) if spec.resume && !welcomed => configure_when_welcomed = true,
                                Some(spec) => {
                                    let _ = jobs.send(files::Job::Configure(Box::new(agent_config(&ctx, &sb, spec, acked, cache.clone()))));
                                }
                                None => {
                                    let msg = "The sandbox lost the session's state when the host restarted. Resume the session to continue.".to_owned();
                                    uplink.state(state(SessionStatus::Stopped, &msg));
                                    break (SessionStatus::Stopped, Some(msg));
                                }
                            }
                        }
                        if !agent_settings {
                            for id in refuse_settings(&mut unconfirmed, &mut queued) {
                                uplink.send(older_agent(id));
                            }
                        }
                        if welcomed {
                            l.send(HostToAgent::Replay { after_agent_seq: acked });
                            if said_hello {
                                resend(&unconfirmed, &jobs, l, &mut queued);
                            }
                            for frame in queued.drain(..) {
                                to_agent(l, frame, agent_files, &uplink);
                            }
                        }
                        said_hello = true;
                    }
                    AgentToHost::Events { events: evs } => {
                        set_report(report.lock().map(|r| r.status).unwrap_or(SessionStatus::Running), evs.last().map(|e| e.agent_seq));
                        uplink.events(evs);
                    }
                    AgentToHost::Stream { event } => {
                        for event in streaming.apply(&secrets, event) {
                            uplink.send(UplinkUp::Stream { event });
                        }
                    }
                    AgentToHost::State { state: st } => {
                        set_report(st.status, None);
                        uplink.state(st);
                    }
                    AgentToHost::Log { line } => uplink.log(line),
                    AgentToHost::Diff { files } => uplink.diffs(files),
                    AgentToHost::Delivered { delivery_id, ok, error } => {
                        unconfirmed.retain(|d| d.id != delivery_id);
                        uplink.send(UplinkUp::Delivered { delivery_id, ok, error });
                    }
                    AgentToHost::CredentialRequest { request_id, kind } => {
                        let _ = events.send(SessionEvent::CredentialRequest { session_id: session_id.clone(), request_id, kind });
                    }
                    AgentToHost::Publish { request_id, name, data } => {
                        if let Some(error) = busy(in_flight).or_else(|| oversized(&data)) {
                            l.send(HostToAgent::Published { request_id, file: None, error: Some(error) });
                        } else {
                            in_flight += 1;
                            publish(api.clone(), session_id.clone(), request_id, name, data, published_tx.clone());
                        }
                    }
                    AgentToHost::Call { request_id, call: c } => {
                        if let Some(error) = busy(in_flight) {
                            l.send(HostToAgent::Answer { request_id, answer: None, error: Some(error) });
                        } else {
                            in_flight += 1;
                            call(api.clone(), session_id.clone(), request_id, c, answered_tx.clone());
                        }
                    }
                    AgentToHost::Exited { status, reason } => {
                        set_report(status, None);
                        exited = Some((status, reason));
                    }
                    AgentToHost::Pong => {}
                }
            }
            d = down.recv() => {
                let Some(d) = d else { break (SessionStatus::Failed, Some("uplink task ended".into())) };
                match d {
                    UplinkDown::Welcome { acked_agent_seq, pending } => {
                        acked = acked_agent_seq;
                        welcomed = true;
                        if configure_when_welcomed
                            && let (Some(spec), Some(_)) = (&spec, &link)
                        {
                            configure_when_welcomed = false;
                            let _ = jobs.send(files::Job::Configure(Box::new(agent_config(&ctx, &sb, spec, acked, cache.clone()))));
                        }
                        if let Some(l) = &link {
                            l.send(HostToAgent::Replay { after_agent_seq: acked });
                        }
                        for p in pending {
                            if let Some(s) = spec.as_mut() {
                                follow_settings(s, &p);
                            }
                            if p.kind.is_settings() && !agent_settings {
                                uplink.send(older_agent(p.id));
                                continue;
                            }
                            remember(&mut unconfirmed, &p);
                            route(p, &jobs, link.as_ref(), &mut queued);
                        }
                    }
                    UplinkDown::Ack { agent_seq } => {
                        acked = acked.max(agent_seq);
                        if let Some(l) = &link {
                            l.send(HostToAgent::Ack { agent_seq });
                        }
                    }
                    UplinkDown::Deliver { delivery } => {
                        if let Some(s) = spec.as_mut() {
                            follow_settings(s, &delivery);
                        }
                        if delivery.kind.is_settings() && !agent_settings {
                            uplink.send(older_agent(delivery.id));
                        } else {
                            remember(&mut unconfirmed, &delivery);
                            route(delivery, &jobs, link.as_ref(), &mut queued);
                        }
                    }
                    UplinkDown::Pong => {}
                }
            }
            Some(r) = ready.recv() => match r {
                files::Ready::File(frame) => match &link {
                    Some(l) => to_agent(l, *frame, agent_files, &uplink),
                    None => queued.push(*frame),
                },
                files::Ready::Failed(att, why) => host_log(
                    &uplink,
                    LogLevel::Warn,
                    format!("Could not fetch {} for the agent: {why}", att.name),
                ),
                // Without an agent the next one's hello configures it again.
                files::Ready::Job(files::Job::Configure(config)) => {
                    if let Some(l) = &link {
                        l.send(HostToAgent::Configure { config });
                    }
                }
                files::Ready::Job(files::Job::Deliver(delivery)) => match &link {
                    Some(l) => to_agent(l, HostToAgent::Deliver { delivery }, agent_files, &uplink),
                    None => queued.push(HostToAgent::Deliver { delivery }),
                },
            },
            Some((request_id, result)) = published_rx.recv() => {
                in_flight = in_flight.saturating_sub(1);
                let (file, error) = match result {
                    Ok(file) => (Some(file), None),
                    Err(e) => (None, Some(e)),
                };
                let frame = HostToAgent::Published { request_id, file, error };
                match &link {
                    Some(l) => l.send(frame),
                    None => queued.push(frame),
                }
            }
            Some((request_id, result)) = answered_rx.recv() => {
                in_flight = in_flight.saturating_sub(1);
                let (answer, error) = match result {
                    Ok(answer) => (Some(answer), None),
                    Err(e) => (None, Some(e)),
                };
                let frame = HostToAgent::Answer { request_id, answer, error };
                match &link {
                    Some(l) => l.send(frame),
                    None => queued.push(frame),
                }
            }
            changed = free_disk.changed(), if measuring => {
                if changed.is_err() {
                    measuring = false;
                    continue;
                }
                let free = *free_disk.borrow_and_update();
                // A session that stops wants its sandbox to run, room or not.
                let hold = stop.is_none() && disk::holds(held.is_some(), free, ctx.cfg.min_free_disk_mb());
                if !hold {
                    pause_failed = false;
                }
                match (&held, hold) {
                    (None, true)
                        if ctx.driver.pauses()
                            && stop.is_none()
                            && exited.is_none()
                            && !pause_failed =>
                    {
                        let paused = match ctx.driver.pause(&sb).await {
                            Ok(()) => Ok(()),
                            // Firecracker may have paused it before its
                            // answer failed to come.
                            Err(e) if ctx.driver.paused(&sb).await => {
                                tracing::warn!("pausing {}: {e:#}, but it is paused", sb.name);
                                Ok(())
                            }
                            Err(e) => Err(e),
                        };
                        match paused {
                            Ok(()) => {
                                let p = paused_for(free);
                                tracing::warn!("paused {}: {}", sb.name, p.reason);
                                uplink.send(UplinkUp::AgentLink { unreachable: None, paused: Some(p.clone()) });
                                uplink.send(UplinkUp::Notice { notice: paused_notice(&ctx.cfg.name, free) });
                                held = Some(p);
                                // The viewers hear of the pause now, not of the link.
                                reach.told_nothing(Instant::now());
                                // An attach under way waits for the sandbox.
                                if let Some(a) = attaching.take() {
                                    a.abort();
                                    attach_at = Some(Instant::now());
                                }
                            }
                            Err(e) => {
                                pause_failed = true;
                                tracing::warn!("pausing {}: {e:#}", sb.name);
                                host_log(&uplink, LogLevel::Warn, format!("Runner {} is nearly out of disk, but its host couldn't pause this session's sandbox: {e:#}", ctx.cfg.name));
                            }
                        }
                    }
                    (Some(_), false) => {
                        let notice = resumed_notice(&ctx.cfg.name, free, stop.is_some());
                        if resume(&ctx, &sb, &uplink, notice).await {
                            held = None;
                            reach.told_nothing(Instant::now());
                            if let Some(l) = link.as_mut() {
                                l.health.heard(Instant::now());
                            }
                        } else {
                            resume_again_at = Some(Instant::now() + RESUME_AGAIN_AFTER);
                        }
                    }
                    _ => {}
                }
            }
            c = cmds.recv() => match c {
                Some(SessionCmd::Stop { reason, destroy }) => {
                    // The agent stops what it does, and the sandbox writes
                    // out its disk, only once it runs: one that stays paused
                    // is resumed again until it does, or until the deadline
                    // stops it, which resumes it first.
                    if held.is_some() {
                        if resume(&ctx, &sb, &uplink, resumed_notice(&ctx.cfg.name, None, true)).await {
                            held = None;
                            resume_again_at = None;
                            reach.told_nothing(Instant::now());
                            if let Some(l) = link.as_mut() {
                                l.health.heard(Instant::now());
                            }
                        } else {
                            resume_again_at = Some(Instant::now() + RESUME_AGAIN_AFTER);
                        }
                    }
                    if stop.is_none() {
                        // Without a link, the agent gets it once the host
                        // attaches again, or its sandbox stops at the deadline.
                        stop = Some((destroy, Instant::now() + Duration::from_secs(60)));
                        let frame = HostToAgent::Stop { reason, grace_ms: 30_000 };
                        match &link {
                            Some(l) => l.send(frame),
                            None => queued.push(frame),
                        }
                    }
                }
                Some(SessionCmd::Superseded) => superseded = true,
                Some(SessionCmd::Credentials { request_id, claude, github, github_read, error }) => {
                    if let Some(c) = &claude {
                        secrets.add(&c.access_token);
                    }
                    if let Some(l) = &link {
                        l.send(HostToAgent::Credentials { request_id, claude, github, github_read, error });
                    }
                }
                None => break (SessionStatus::Stopped, Some("host shutting down".into())),
            },
            _ = tick.tick() => {
                if let Some((_, deadline)) = stop
                    && Instant::now() > deadline {
                        break exited.clone().unwrap_or((SessionStatus::Stopped, Some("Stopped (forced)".into())));
                    }
                if exited.is_some() && link.is_none()
                    && let Some(done) = exited.clone() {
                        break done;
                    }
                // A sandbox that stayed paused as it was resumed: the disk
                // is looked at again, which resumes it if there is room.
                if resume_again_at.is_some_and(|at| Instant::now() >= at) {
                    resume_again_at = None;
                    free_disk.mark_changed();
                }
                // A link that goes quiet is replaced. A paused sandbox's
                // says nothing, and the viewers know why.
                if held.is_some() {
                    continue;
                }
                if let Some(l) = link.as_mut() {
                    match l.health.check(Instant::now()) {
                        Check::Fine => {}
                        Check::Ping => l.send(HostToAgent::Ping),
                        Check::Dead(why) => {
                            reach.down(l.health.last_heard(), Instant::now(), why);
                            let wait = retry.stalled();
                            tracing::warn!(
                                "{}: {why}; attaching again in {} s",
                                sb.name,
                                wait.as_secs()
                            );
                            link = None;
                            attach_at = Some(Instant::now() + wait);
                            restart_agent = true;
                        }
                    }
                }
                if let Some(r) = reach.check(Instant::now()) {
                    report_reach(&uplink, r, held.as_ref());
                }
            }
        }
    };
    if let Some(a) = attaching.take() {
        a.abort();
    }

    // Give the uplink a moment to deliver the agent's last events.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let destroy = stop.map(|(d, _)| d).unwrap_or(false);
    if let Err(e) = ctx.driver.stop(&sb).await {
        tracing::warn!("stopping {}: {e:#}", sb.name);
    }
    if destroy && let Err(e) = ctx.driver.remove(&sb).await {
        tracing::warn!("removing {}: {e:#}", sb.name);
    }
    let (status, reason) = outcome;
    // Drain a late Superseded so the next run's status is not overwritten.
    while let Ok(c) = cmds.try_recv() {
        if matches!(c, SessionCmd::Superseded) {
            superseded = true;
        }
    }
    if !superseded {
        uplink.state(state(status, reason.as_deref().unwrap_or("Stopped")));
    }
    set_report(status, None);
    tokio::time::sleep(Duration::from_millis(500)).await;
    (status, reason)
}

/// Resumes the sandbox the host held paused, and says so. False while it
/// stays paused: resuming failed, and the host tries again. One that isn't
/// paused after a failure (its VM may have resumed before the answer failed
/// to come, or stopped) is no longer held, and the agent's link says which;
/// only one that resumed is said to have resumed.
async fn resume(ctx: &HostCtx, sb: &Sandbox, uplink: &Uplink, notice: Notice) -> bool {
    let resumed = match ctx.driver.resume(sb).await {
        Ok(()) => {
            tracing::info!("resumed {}", sb.name);
            true
        }
        Err(e) if ctx.driver.paused(sb).await => {
            tracing::warn!("resuming {}: {e:#}; trying again", sb.name);
            return false;
        }
        Err(e) => {
            tracing::warn!("resuming {}: {e:#}, but it isn't paused", sb.name);
            false
        }
    };
    uplink.send(UplinkUp::AgentLink {
        unreachable: None,
        paused: None,
    });
    if resumed {
        uplink.send(UplinkUp::Notice { notice });
    }
    true
}

fn agent_config(
    ctx: &HostCtx,
    sb: &Sandbox,
    spec: &SessionSpec,
    first_agent_seq: u64,
    cache: Option<SandboxCache>,
) -> AgentConfig {
    let process = matches!(ctx.driver, Driver::Process(_));
    AgentConfig {
        spec: spec.clone(),
        workdir: ctx.driver.workdir(sb),
        home: ctx.driver.home(sb),
        claude_bin: if process {
            ctx.cfg.claude_bin.clone()
        } else {
            None
        },
        claude_args: ctx.cfg.claude_args.clone(),
        env: Default::default(),
        push_on_commit: ctx.cfg.push_on_commit,
        first_agent_seq: (first_agent_seq > 0).then_some(first_agent_seq),
        sandbox: ctx.driver.enforces_size().then(|| ctx.sandbox_size(spec)),
        cache,
        diffs: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session of a project that sets nothing.
    fn spec(session_id: &str) -> Result<Box<SessionSpec>> {
        Ok(serde_json::from_value(serde_json::json!({
            "session_id": session_id,
            "number": 1,
            "title": "T",
            "project": {
                "id": "prj_1", "slug": "app", "name": "App",
                "repo": { "owner": "acme", "name": "app" },
                "default_branch": "main", "settings": {}
            },
            "branch": "fctl/1", "base_branch": "main",
            "author": { "id": "usr_1", "name": "Ada" },
            "claude_session_id": "00000000-0000-0000-0000-000000000000",
            "resume": false, "secrets": {},
            "credentials": {
                "claude": { "id": "crd_1", "kind": "oauth", "access_token": "a", "scopes": [] },
                "github": { "token": "ghs_w", "expires_at": 1 }
            },
            "git_author": { "name": "bot", "email": "bot@example.com" },
            "co_authors": [], "web_url": "https://fctl.test/s/ses_1", "idle_timeout_secs": 60
        }))?)
    }

    /// Starts a new run of `session_id` and waits for it to end: how, and why.
    async fn run_to_end(
        ctx: &Arc<HostCtx>,
        session_id: &str,
    ) -> Result<(SessionStatus, Option<String>)> {
        let (events, mut ended) = mpsc::unbounded_channel();
        let _handle = spawn(ctx.clone(), Start::New(spec(session_id)?), events);
        loop {
            let event = tokio::time::timeout(Duration::from_secs(60), ended.recv()).await?;
            match event.context("the run ended without saying so")? {
                SessionEvent::Ended { status, reason, .. } => return Ok((status, reason)),
                SessionEvent::CredentialRequest { .. } => {}
            }
        }
    }

    #[tokio::test]
    async fn a_session_whose_driver_stopped_being_ready_goes_back_to_the_queue() -> Result<()> {
        use crate::driver::container::Flavor;
        use crate::driver::fake::FakeCli;
        let data = tempfile::tempdir()?;
        let cfg = HostConfig {
            name: "studio".into(),
            driver: "docker".into(),
            // Nothing answers there: the session's uplink tries in vain.
            server: "http://127.0.0.1:9".into(),
            data_dir: Some(data.path().to_owned()),
            ..HostConfig::default()
        };
        let cli = FakeCli::new(&[&cfg.image()], &[])?;
        let ctx = Arc::new(HostCtx::new(
            cfg,
            cli.host_driver(Flavor::Docker),
            "fctl_rt_test".into(),
        ));
        // When the host last probed it, Docker answered; then it stopped.
        *ctx.drivers.lock().unwrap_or_else(PoisonError::into_inner) =
            vec![ctx.driver.probe().await];
        assert_eq!(ctx.driver_problem(), None);
        cli.refuse(
            "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?",
        )?;
        let (status, reason) = run_to_end(&ctx, "ses_01DOWN").await?;
        assert_eq!(status, SessionStatus::Queued);
        let reason = reason.unwrap_or_default();
        assert!(
            reason
                .starts_with("studio's docker driver isn't ready: the Docker daemon isn't running"),
            "{reason}"
        );
        // The next heartbeat says so.
        let why = ctx.driver_problem().unwrap_or_default();
        assert!(why.starts_with("the Docker daemon isn't running"), "{why}");
        // Docker answers again, but the sandbox can't be made for another
        // reason: that session fails, as before.
        cli.answer()?;
        cli.fail_runs()?;
        let (status, reason) = run_to_end(&ctx, "ses_01FULL").await?;
        assert_eq!(status, SessionStatus::Failed);
        assert!(
            reason
                .as_deref()
                .unwrap_or_default()
                .contains("no room for it"),
            "{reason:?}"
        );
        assert_eq!(ctx.driver_problem(), None);
        Ok(())
    }

    fn delivery(id: &str, kind: DeliveryKind) -> Delivery {
        Delivery {
            id: DeliveryId(id.into()),
            kind,
        }
    }

    fn settings(id: &str) -> Delivery {
        delivery(
            id,
            DeliveryKind::Settings {
                model: None,
                effort: Some("medium".into()),
                by: None,
            },
        )
    }

    #[tokio::test]
    async fn a_session_that_fails_for_a_full_disk_says_so() -> Result<()> {
        use crate::driver::container::Flavor;
        use crate::driver::fake::FakeCli;
        const IMAGE: &str = "ghcr.io/hyperstructure/sandbox:0.3.0";
        let spec = SandboxSpec {
            name: "fctl-ses-1".into(),
            image: IMAGE.into(),
            fallback_image: None,
            cpus: 2,
            memory_mb: 2048,
            cache: None,
        };
        let cli = FakeCli::new(&[IMAGE], &[])?;
        cli.full_disk()?;
        let Err(e) = cli.ensure(Flavor::Docker, &spec).await else {
            panic!("the sandbox was made on a full disk");
        };
        let said = failure("now", &e);
        assert!(
            said.starts_with("Runner now ran out of disk: starting fctl-ses-1: "),
            "{said}"
        );
        assert!(said.ends_with("no space left on device."), "{said}");
        // Other failures say only what went wrong.
        let other = FakeCli::new(&[IMAGE], &[])?;
        other.fail_runs()?;
        let Err(e) = other.ensure(Flavor::Docker, &spec).await else {
            panic!("the sandbox was made");
        };
        assert_eq!(failure("now", &e), format!("{e:#}"));
        Ok(())
    }

    #[test]
    fn the_transcript_says_why_the_sandbox_pauses_and_the_agent_starts_again() {
        let paused = paused_notice("now", Some(800));
        assert_eq!(
            (paused.level, paused.code.as_str()),
            (NoticeLevel::Warn, "sandbox_paused")
        );
        assert_eq!(
            paused.message,
            "Runner now is nearly out of disk, with 800 MiB free, so its host paused this session's sandbox, where what the agent writes would be lost. The sandbox keeps what it holds, and the agent carries on where it was once there is room."
        );
        assert_eq!(paused_for(None).reason, "it is nearly out of disk");
        assert_eq!(
            resumed_notice("now", Some(5 * 1024), false).message,
            "Runner now has room on its disk again, with 5 GiB free, so its host resumed this session's sandbox."
        );
        assert_eq!(
            resumed_notice("now", None, true).message,
            "Runner now resumed this session's sandbox to stop it."
        );
        let floor = Some(20 * 1024);
        // With room, a restart is no news here.
        assert_eq!(restart_notice("now", Some(30 * 1024), floor), None);
        assert_eq!(restart_notice("now", None, floor), None);
        let low = restart_notice("now", Some(12_595), floor).map(|n| n.message);
        assert_eq!(
            low.as_deref(),
            Some(
                "fctl-agent stopped answering in the sandbox while runner now is low on disk, with 12.3 GiB free, below the 20 GiB its host keeps free. Its host is starting it again."
            )
        );
        let out = restart_notice("now", Some(300), floor).map(|n| n.message);
        assert_eq!(
            out.as_deref(),
            Some(
                "fctl-agent stopped answering in the sandbox, perhaps because runner now is nearly out of disk, with 300 MiB free. Its host is starting it again."
            )
        );
        let full = anyhow::Error::from(std::io::Error::from_raw_os_error(28))
            .context("copying fctl-agent into the sandbox");
        assert_eq!(
            start_failed_notice("now", &full).message,
            "Its host couldn't start fctl-agent in the sandbox again: runner now ran out of disk (copying fctl-agent into the sandbox: No space left on device (os error 28)). It tries again."
        );
    }

    #[test]
    fn an_older_agent_gets_no_change_of_settings() {
        let interrupt = delivery("dlv_1", DeliveryKind::Interrupt { by: None });
        let mut unconfirmed = vec![interrupt.clone(), settings("dlv_2")];
        let mut queued = vec![
            HostToAgent::Deliver {
                delivery: settings("dlv_2"),
            },
            HostToAgent::Deliver {
                delivery: settings("dlv_3"),
            },
            HostToAgent::Deliver {
                delivery: interrupt.clone(),
            },
        ];
        let refused = refuse_settings(&mut unconfirmed, &mut queued);
        assert_eq!(
            refused,
            vec![DeliveryId("dlv_2".into()), DeliveryId("dlv_3".into())]
        );
        assert_eq!(unconfirmed, vec![interrupt.clone()]);
        assert_eq!(
            queued,
            vec![HostToAgent::Deliver {
                delivery: interrupt
            }]
        );
        let UplinkUp::Delivered { ok, error, .. } = older_agent(DeliveryId("dlv_2".into())) else {
            panic!("expected a delivery's outcome");
        };
        assert!(!ok);
        assert!(error.is_some_and(|e| e.contains("older fctl")));
    }
}
