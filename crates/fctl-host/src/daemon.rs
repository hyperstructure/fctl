//! The host daemon: the control connection to FleetObject and every session.
//!
//! Sessions outlive the control connection (each has its own uplink) and
//! outlive the daemon itself: agents keep running in their sandboxes, and a
//! restarted daemon finds and re-attaches to them.

use crate::config::HostConfig;
use crate::driver::{Driver, session_of};
use crate::login::{self, LoginCmd};
use crate::session::{self, HostCtx, SessionCmd, SessionEvent, SessionHandle, Start};
use crate::sysinfo;
use anyhow::Result;
use fctl_client::ws;
use fctl_proto::runner::{FleetToHost, HostHeartbeat, HostHello, HostSessionReport, HostToFleet};
use fctl_proto::{LoginJobId, PROTOCOL_VERSION, SessionId};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Daemon {
    ctx: Arc<HostCtx>,
    sessions: HashMap<SessionId, SessionHandle>,
    /// request id → session waiting for a credential.
    pending_creds: HashMap<String, (SessionId, String)>,
    logins: HashMap<LoginJobId, mpsc::UnboundedSender<LoginCmd>>,
    events_tx: mpsc::UnboundedSender<SessionEvent>,
    events_rx: mpsc::UnboundedReceiver<SessionEvent>,
    login_tx: mpsc::UnboundedSender<HostToFleet>,
    login_rx: mpsc::UnboundedReceiver<HostToFleet>,
    /// Messages waiting for the control connection.
    outbox: Vec<HostToFleet>,
    /// Starts that wait for the previous run of the same session to finish.
    pending_starts: HashMap<SessionId, Box<fctl_proto::runner::SessionSpec>>,
    next_request: u64,
}

impl Daemon {
    pub async fn new(cfg: HostConfig, token: String) -> Result<Self> {
        let data = cfg.data_dir();
        std::fs::create_dir_all(&data)?;
        let driver = Driver::select(&cfg.driver, &data, cfg.agent_bin.clone()).await?;
        tracing::info!(
            "fctl host {VERSION} on {} using the {} driver, capacity {}",
            cfg.name,
            driver.name(),
            cfg.capacity
        );
        let ctx = Arc::new(HostCtx {
            cfg,
            driver,
            token,
            runner_id: Mutex::new(None),
        });
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (login_tx, login_rx) = mpsc::unbounded_channel();
        Ok(Self {
            ctx,
            sessions: HashMap::new(),
            pending_creds: HashMap::new(),
            logins: HashMap::new(),
            events_tx,
            events_rx,
            login_tx,
            login_rx,
            outbox: Vec::new(),
            pending_starts: HashMap::new(),
            next_request: 0,
        })
    }

    /// Re-attaches to sandboxes that survived a restart.
    async fn recover(&mut self) {
        match self.ctx.driver.list().await {
            Ok(list) => {
                for sb in list.into_iter().filter(|s| s.running) {
                    if let Some(id) = session_of(&sb.name).map(SessionId) {
                        tracing::info!("recovering {id} from {}", sb.name);
                        let h = session::spawn(
                            self.ctx.clone(),
                            Start::Recover(id.clone()),
                            self.events_tx.clone(),
                        );
                        self.sessions.insert(id, h);
                    }
                }
            }
            Err(e) => tracing::warn!("listing sandboxes: {e:#}"),
        }
    }

    fn reports(&self) -> Vec<HostSessionReport> {
        self.sessions
            .iter()
            .filter_map(|(id, h)| {
                let r = h.report.lock().ok()?.clone();
                Some(HostSessionReport {
                    session_id: id.clone(),
                    status: r.status,
                    agent_seq: r.agent_seq,
                })
            })
            .collect()
    }

    async fn hello(&self) -> HostHello {
        HostHello {
            protocol: PROTOCOL_VERSION,
            version: VERSION.into(),
            hostname: crate::config::hostname(),
            os: sysinfo::os().into(),
            arch: sysinfo::arch().into(),
            cpus: sysinfo::cpus(),
            memory_mb: sysinfo::memory_mb(),
            drivers: Driver::probe_all(&self.ctx.cfg.data_dir()).await,
            driver: self.ctx.driver.name().into(),
            capacity: self.ctx.cfg.capacity,
            labels: self.ctx.cfg.labels.clone(),
            sessions: self.reports(),
            image: Some(self.ctx.cfg.image.clone()),
        }
    }

    pub async fn run(mut self) -> Result<()> {
        keep_awake();
        self.recover().await;
        let url = format!(
            "{}/api/v1/runner/connect",
            self.ctx.cfg.server.trim_end_matches('/')
        );
        let mut backoff = Duration::from_secs(1);
        loop {
            match ws::connect(&url, Some(&self.ctx.token)).await {
                Ok(socket) => {
                    backoff = Duration::from_secs(1);
                    tracing::info!("connected to {}", self.ctx.cfg.server);
                    if let Err(e) = self.serve(socket).await {
                        tracing::warn!("control connection: {e:#}");
                    }
                }
                Err(e) => tracing::warn!("cannot reach {}: {e:#}", self.ctx.cfg.server),
            }
            // Keep sessions moving while disconnected.
            let deadline = tokio::time::sleep(jitter(backoff));
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    _ = &mut deadline => break,
                    Some(ev) = self.events_rx.recv() => { let _ = self.on_session_event(ev); }
                    Some(msg) = self.login_rx.recv() => self.outbox.push(msg),
                    _ = shutdown_signal() => return Ok(()),
                }
            }
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }

    async fn serve(&mut self, socket: ws::Socket) -> Result<()> {
        let (mut sink, mut stream) = socket.split();
        ws::send_json(&mut sink, &HostToFleet::Hello(Box::new(self.hello().await))).await?;
        for m in std::mem::take(&mut self.outbox) {
            ws::send_json(&mut sink, &m).await?;
        }
        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
        heartbeat.tick().await;
        loop {
            tokio::select! {
                msg = ws::next_text(&mut stream) => {
                    let text = match msg {
                        Some(Ok(t)) => t,
                        Some(Err(e)) => return Err(e),
                        None => return Ok(()),
                    };
                    let msg: FleetToHost = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => { tracing::warn!("unknown message from the control plane: {e}"); continue; }
                    };
                    for reply in self.on_fleet(msg).await {
                        ws::send_json(&mut sink, &reply).await?;
                    }
                }
                Some(ev) = self.events_rx.recv() => {
                    if let Some(m) = self.on_session_event(ev) {
                        ws::send_json(&mut sink, &m).await?;
                    }
                    for m in std::mem::take(&mut self.outbox) {
                        ws::send_json(&mut sink, &m).await?;
                    }
                }
                Some(msg) = self.login_rx.recv() => ws::send_json(&mut sink, &msg).await?,
                _ = heartbeat.tick() => {
                    let hb = HostHeartbeat {
                        sessions: self.reports(),
                        load1: sysinfo::load1(),
                        memory_available_mb: sysinfo::memory_available_mb(),
                        disk_available_mb: sysinfo::disk_available_mb(&self.ctx.cfg.data_dir()),
                    };
                    ws::send_json(&mut sink, &HostToFleet::Heartbeat(hb)).await?;
                }
                _ = shutdown_signal() => {
                    tracing::info!("shutting down; sessions keep running in their sandboxes");
                    std::process::exit(0);
                }
            }
        }
    }

    /// Handles a session event; returns a message for the control plane (or queues it).
    fn on_session_event(&mut self, ev: SessionEvent) -> Option<HostToFleet> {
        match ev {
            SessionEvent::CredentialRequest {
                session_id,
                request_id,
                kind,
            } => {
                self.next_request += 1;
                let rid = format!("h{}", self.next_request);
                self.pending_creds
                    .insert(rid.clone(), (session_id.clone(), request_id));
                let m = HostToFleet::CredentialRequest {
                    request_id: rid,
                    session_id,
                    kind,
                };
                self.outbox.push(m);
                None
            }
            SessionEvent::Ended {
                session_id,
                status,
                reason,
                run_id,
            } => {
                self.sessions.remove(&session_id);
                tracing::info!(
                    "session {session_id} ended: {status:?} {}",
                    reason.clone().unwrap_or_default()
                );
                self.outbox.push(HostToFleet::SessionEnded {
                    session_id: session_id.clone(),
                    status,
                    reason,
                    run_id,
                });
                if let Some(spec) = self.pending_starts.remove(&session_id) {
                    tracing::info!("starting the next run of {session_id}");
                    let h =
                        session::spawn(self.ctx.clone(), Start::New(spec), self.events_tx.clone());
                    self.sessions.insert(session_id, h);
                }
                None
            }
        }
    }

    async fn on_fleet(&mut self, msg: FleetToHost) -> Vec<HostToFleet> {
        match msg {
            FleetToHost::Welcome { runner, .. } => {
                tracing::info!("registered as {} ({})", runner.name, runner.id);
                if let Ok(mut r) = self.ctx.runner_id.lock() {
                    *r = Some(runner.id);
                }
                vec![]
            }
            FleetToHost::Ping => vec![HostToFleet::Pong],
            FleetToHost::StartSession { spec } => {
                let id = spec.session_id.clone();
                if let Some(existing) = self.sessions.get(&id) {
                    let winding_down = existing
                        .report
                        .lock()
                        .map(|r| {
                            r.status.is_terminal()
                                || r.status == fctl_proto::session::SessionStatus::Stopping
                        })
                        .unwrap_or(false);
                    if winding_down {
                        // A resume while the last run still shuts down: start it right after.
                        let _ = existing.tx.send(SessionCmd::Superseded);
                        self.pending_starts.insert(id.clone(), spec);
                    }
                    return vec![HostToFleet::StartResult {
                        session_id: id,
                        accepted: true,
                        error: None,
                    }];
                }
                if self.sessions.len() as u32 >= self.ctx.cfg.capacity {
                    return vec![HostToFleet::StartResult {
                        session_id: id,
                        accepted: false,
                        error: Some("the host is at capacity".into()),
                    }];
                }
                tracing::info!("starting {id}: {}", spec.title);
                let h = session::spawn(self.ctx.clone(), Start::New(spec), self.events_tx.clone());
                self.sessions.insert(id.clone(), h);
                vec![HostToFleet::StartResult {
                    session_id: id,
                    accepted: true,
                    error: None,
                }]
            }
            FleetToHost::StopSession {
                session_id,
                reason,
                destroy,
            } => {
                match self.sessions.get(&session_id) {
                    Some(h) => {
                        let _ = h.tx.send(SessionCmd::Stop { reason, destroy });
                        vec![]
                    }
                    None => {
                        // Not running here: clean up any leftover sandbox.
                        let sb = self
                            .ctx
                            .driver
                            .handle(&crate::driver::sandbox_name(session_id.as_str()));
                        let _ = self.ctx.driver.stop(&sb).await;
                        if destroy {
                            let _ = self.ctx.driver.remove(&sb).await;
                        }
                        vec![HostToFleet::SessionEnded {
                            session_id,
                            status: fctl_proto::session::SessionStatus::Stopped,
                            reason: Some("Stopped".into()),
                            run_id: None,
                        }]
                    }
                }
            }
            FleetToHost::Credential {
                request_id,
                claude,
                github,
                error,
            } => {
                if let Some((sid, agent_request)) = self.pending_creds.remove(&request_id)
                    && let Some(h) = self.sessions.get(&sid) {
                        let _ = h.tx.send(SessionCmd::Credentials {
                            request_id: Some(agent_request),
                            claude,
                            github,
                            error,
                        });
                    }
                vec![]
            }
            FleetToHost::ClaudeLoginStart { job_id, user } => {
                let (tx, rx) = mpsc::unbounded_channel();
                self.logins.insert(job_id.clone(), tx);
                tracing::info!("starting a Claude sign-in for {}", user.name);
                tokio::spawn(login::run(
                    self.ctx.clone(),
                    job_id,
                    rx,
                    self.login_tx.clone(),
                ));
                vec![]
            }
            FleetToHost::ClaudeLoginCode { job_id, code } => {
                if let Some(tx) = self.logins.get(&job_id) {
                    let _ = tx.send(LoginCmd::Code(code));
                }
                vec![]
            }
            FleetToHost::ClaudeLoginCancel { job_id } => {
                if let Some(tx) = self.logins.remove(&job_id) {
                    let _ = tx.send(LoginCmd::Cancel);
                }
                vec![]
            }
        }
    }
}

fn jitter(d: Duration) -> Duration {
    let ms = d.as_millis() as u64;
    let r = fctl_domain::random_bytes::<2>();
    let spread = (u16::from_le_bytes(r) as u64 % (ms / 4 + 1)) as i64 - (ms / 8) as i64;
    Duration::from_millis((ms as i64 + spread).max(100) as u64)
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let (Ok(mut term), Ok(mut int)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) {
            tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// On macOS, keeps the machine from idle-sleeping while the daemon runs.
fn keep_awake() {
    if cfg!(target_os = "macos") {
        let pid = std::process::id().to_string();
        match std::process::Command::new("caffeinate")
            .args(["-i", "-s", "-w", &pid])
            .spawn()
        {
            Ok(_) => tracing::info!("holding a no-idle-sleep assertion (caffeinate)"),
            Err(e) => tracing::warn!("caffeinate unavailable: {e}"),
        }
    }
}
