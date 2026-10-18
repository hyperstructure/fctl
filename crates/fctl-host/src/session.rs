//! One task per session: sandbox, fctl-agent, and the bridge between the
//! agent and the session's uplink.

use crate::config::HostConfig;
use crate::driver::{Driver, Sandbox, SandboxSpec, sandbox_name};
use crate::files;
use crate::uplink::Uplink;
use anyhow::{Context, Result};
use fctl_domain::size::Want;
use fctl_proto::agent::{AgentConfig, AgentToHost, HostToAgent, MAX_FRAME_BYTES};
use fctl_proto::event::{LogLevel, LogLine, LogSource};
use fctl_proto::runner::{
    ClaudeCredential, CredentialKind, GithubCredential, SandboxLimits, SandboxSize, SessionSpec,
    StopReason,
};
use fctl_proto::session::SessionStatus;
use fctl_proto::uplink::{Delivery, DeliveryKind, FEATURE_ATTACHMENTS, SessionState, UplinkDown};
use fctl_proto::{RunnerId, SessionId};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// What every session task shares.
pub struct HostCtx {
    pub cfg: HostConfig,
    pub driver: Driver,
    pub token: String,
    pub runner_id: Mutex<Option<RunnerId>>,
    /// What this host gives each sandbox, as of the last hello.
    pub sandbox: Mutex<SandboxLimits>,
}

impl HostCtx {
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

    /// Measures what the sandboxes run on again (the Docker VM may have
    /// been resized) and returns the limits that follow.
    pub async fn refresh_sandbox_limits(&self) -> SandboxLimits {
        let limits = self.cfg.sandbox_limits(self.driver.resources().await);
        *self.sandbox.lock().unwrap_or_else(PoisonError::into_inner) = limits;
        limits
    }

    /// The CPUs and memory a session's sandbox gets on this host.
    pub fn sandbox_size(&self, spec: &SessionSpec) -> SandboxSize {
        Want::of(&spec.project.settings).on(&self.sandbox_limits())
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
        error: Option<String>,
    },
}

pub enum SessionEvent {
    CredentialRequest {
        session_id: SessionId,
        request_id: String,
        kind: CredentialKind,
    },
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

/// A framed connection to fctl-agent.
struct AgentLink {
    tx: mpsc::UnboundedSender<HostToAgent>,
    rx: mpsc::UnboundedReceiver<AgentToHost>,
    _child: Option<tokio::process::Child>,
}

impl AgentLink {
    fn open(io: crate::driver::AgentIo) -> Self {
        let (tx, mut out_rx) = mpsc::unbounded_channel::<HostToAgent>();
        let (in_tx, rx) = mpsc::unbounded_channel::<AgentToHost>();
        let mut writer = io.writer;
        tokio::spawn(async move {
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
        });
        let reader = io.reader;
        tokio::spawn(async move {
            let mut reader = BufReader::with_capacity(256 * 1024, reader);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) if n > MAX_FRAME_BYTES => break,
                    Ok(_) => {}
                }
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<AgentToHost>(line.trim()) {
                    Ok(m) => {
                        if in_tx.send(m).is_err() {
                            break;
                        }
                    }
                    Err(e) => tracing::warn!("malformed frame from fctl-agent: {e}"),
                }
            }
        });
        Self {
            tx,
            rx,
            _child: io.child,
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
        claude_session_id: None,
        remote_control: None,
        stats: None,
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

fn host_log(uplink: &Uplink, level: LogLevel, text: String) {
    uplink.log(LogLine {
        source: LogSource::Host,
        level,
        text,
    });
}

async fn prepare(ctx: &HostCtx, spec: &SessionSpec, uplink: &Uplink) -> Result<Sandbox> {
    let image = spec
        .project
        .settings
        .image
        .clone()
        .unwrap_or_else(|| ctx.cfg.image.clone());
    let size = ctx.sandbox_size(spec);
    let sspec = SandboxSpec {
        name: sandbox_name(spec.session_id.as_str()),
        image,
        cpus: size.cpus,
        memory_mb: size.memory_mb,
    };
    let log_uplink = |line: String| host_log(uplink, LogLevel::Info, line);
    let sb = ctx.driver.ensure(&sspec, &log_uplink).await?;
    if ctx.driver.needs_linux_agent() {
        let bin = crate::agentbin::linux_agent(&ctx.cfg)
            .await
            .context("getting fctl-agent for the sandbox")?;
        ctx.driver.install_agent(&sb, &bin).await?;
    }
    ctx.driver.start_agent(&sb).await?;
    Ok(sb)
}

async fn run(
    ctx: Arc<HostCtx>,
    start: Start,
    mut cmds: mpsc::UnboundedReceiver<SessionCmd>,
    events: mpsc::UnboundedSender<SessionEvent>,
    report: Arc<Mutex<Report>>,
) -> (SessionStatus, Option<String>) {
    let (session_id, spec) = match start {
        Start::New(spec) => (spec.session_id.clone(), Some(spec)),
        Start::Recover(id) => (id, None),
    };
    let (uplink, mut down) = Uplink::start(
        ctx.uplink_url(&session_id),
        ctx.token.clone(),
        ctx.runner_id(),
    );
    let (jobs, mut ready) = files::start(
        fctl_client::Api::new(&ctx.cfg.server, Some(ctx.token.clone())),
        session_id.clone(),
    );
    let set_report = |status: SessionStatus, seq: Option<u64>| {
        if let Ok(mut r) = report.lock() {
            r.status = status;
            if let Some(s) = seq {
                r.agent_seq = r.agent_seq.max(s);
            }
        }
    };
    let sb = match &spec {
        Some(spec) => {
            uplink.state(state(
                SessionStatus::Provisioning,
                &format!("Creating the sandbox on {}", ctx.cfg.name),
            ));
            match prepare(&ctx, spec, &uplink).await {
                Ok(sb) => sb,
                Err(e) => {
                    let msg = format!("{e:#}");
                    host_log(&uplink, LogLevel::Error, msg.clone());
                    uplink.state(state(SessionStatus::Failed, &msg));
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    return (SessionStatus::Failed, Some(msg));
                }
            }
        }
        None => {
            let sb = ctx.driver.handle(&sandbox_name(session_id.as_str()));
            if let Err(e) = ctx.driver.start_agent(&sb).await {
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
    let mut exited: Option<(SessionStatus, Option<String>)> = None;
    let mut stop: Option<(bool, Instant)> = None;
    let mut attach_failures = 0u32;
    let mut superseded = false;
    // A fresh agent for a resumed session numbers its events after the ones
    // the control plane has, so it waits for the uplink's Welcome.
    let mut configure_when_welcomed = false;
    let mut tick = tokio::time::interval(Duration::from_secs(1));

    let outcome = loop {
        if link.is_none() && exited.is_none() {
            match ctx.driver.attach(&sb).await {
                Ok(io) => {
                    attach_failures = 0;
                    link = Some(AgentLink::open(io));
                }
                Err(e) => {
                    attach_failures += 1;
                    tracing::warn!("attaching to {}: {e:#}", sb.name);
                    if attach_failures >= 5 {
                        let msg = "fctl-agent stopped responding in the sandbox".to_owned();
                        uplink.state(state(SessionStatus::Failed, &msg));
                        break (SessionStatus::Failed, Some(msg));
                    }
                    let _ = ctx.driver.start_agent(&sb).await;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            }
        }
        tokio::select! {
            msg = async { match link.as_mut() { Some(l) => l.rx.recv().await, None => std::future::pending().await } } => {
                let Some(msg) = msg else {
                    link = None;
                    if let Some(done) = exited.clone() {
                        break done;
                    }
                    if stop.is_some() {
                        break (SessionStatus::Stopped, Some("Stopped".into()));
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                };
                let Some(l) = link.as_ref() else { continue };
                match msg {
                    AgentToHost::Hello(h) => {
                        set_report(h.status, Some(h.agent_seq));
                        agent_files = h.features.iter().any(|f| f == FEATURE_ATTACHMENTS);
                        if !h.configured {
                            match &spec {
                                Some(spec) if spec.resume && !welcomed => configure_when_welcomed = true,
                                Some(spec) => {
                                    let _ = jobs.send(files::Job::Configure(Box::new(agent_config(&ctx, &sb, spec, acked))));
                                }
                                None => {
                                    let msg = "The sandbox lost the session's state when the host restarted. Resume the session to continue.".to_owned();
                                    uplink.state(state(SessionStatus::Stopped, &msg));
                                    break (SessionStatus::Stopped, Some(msg));
                                }
                            }
                        }
                        if welcomed {
                            l.send(HostToAgent::Replay { after_agent_seq: acked });
                            for frame in queued.drain(..) {
                                to_agent(l, frame, agent_files, &uplink);
                            }
                        }
                    }
                    AgentToHost::Events { events: evs } => {
                        set_report(report.lock().map(|r| r.status).unwrap_or(SessionStatus::Running), evs.last().map(|e| e.agent_seq));
                        uplink.events(evs);
                    }
                    AgentToHost::Stream { event } => uplink.send(fctl_proto::uplink::UplinkUp::Stream { event }),
                    AgentToHost::State { state: st } => {
                        set_report(st.status, None);
                        uplink.state(st);
                    }
                    AgentToHost::Log { line } => uplink.log(line),
                    AgentToHost::Delivered { delivery_id, ok, error } => {
                        uplink.send(fctl_proto::uplink::UplinkUp::Delivered { delivery_id, ok, error });
                    }
                    AgentToHost::CredentialRequest { request_id, kind } => {
                        let _ = events.send(SessionEvent::CredentialRequest { session_id: session_id.clone(), request_id, kind });
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
                            let _ = jobs.send(files::Job::Configure(Box::new(agent_config(&ctx, &sb, spec, acked))));
                        }
                        if let Some(l) = &link {
                            l.send(HostToAgent::Replay { after_agent_seq: acked });
                        }
                        for p in pending {
                            route(p, &jobs, link.as_ref(), &mut queued);
                        }
                    }
                    UplinkDown::Ack { agent_seq } => {
                        acked = acked.max(agent_seq);
                        if let Some(l) = &link {
                            l.send(HostToAgent::Ack { agent_seq });
                        }
                    }
                    UplinkDown::Deliver { delivery } => route(delivery, &jobs, link.as_ref(), &mut queued),
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
            c = cmds.recv() => match c {
                Some(SessionCmd::Stop { reason, destroy }) => {
                    if stop.is_none() {
                        stop = Some((destroy, Instant::now() + Duration::from_secs(60)));
                        match &link {
                            Some(l) => l.send(HostToAgent::Stop { reason, grace_ms: 30_000 }),
                            None => break (SessionStatus::Stopped, Some("Stopped".into())),
                        }
                    }
                }
                Some(SessionCmd::Superseded) => superseded = true,
                Some(SessionCmd::Credentials { request_id, claude, github, error }) => {
                    if let Some(l) = &link {
                        l.send(HostToAgent::Credentials { request_id, claude, github, error });
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
            }
        }
    };

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

fn agent_config(
    ctx: &HostCtx,
    sb: &Sandbox,
    spec: &SessionSpec,
    first_agent_seq: u64,
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
    }
}
