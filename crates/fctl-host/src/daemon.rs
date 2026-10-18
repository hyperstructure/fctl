//! The host daemon: the control connection to FleetObject and every session.
//!
//! Sessions outlive the control connection (each has its own uplink) and
//! outlive the daemon itself: agents keep running in their sandboxes, and a
//! restarted daemon finds and re-attaches to them.
//!
//! The control connection never waits on a driver. The hello reports what
//! the drivers said when last probed, and sweeping or cleaning up sandboxes
//! runs in tasks of their own (`Chore`), so heartbeats and pongs keep
//! flowing while a runtime hangs.

use crate::config::HostConfig;
use crate::driver::{Driver, SandboxInfo, sandbox_name, session_of, timed_out};
use crate::expiry;
use crate::login::{self, LoginCmd};
use crate::refresh;
use crate::session::{self, HostCtx, SessionCmd, SessionEvent, SessionHandle, Start};
use crate::sysinfo;
use anyhow::Result;
use fctl_client::ws;
use fctl_domain::size::{self, Want};
use fctl_proto::runner::{
    FleetToHost, HostHeartbeat, HostHello, HostSessionReport, HostToFleet, SessionSpec, StopReason,
};
use fctl_proto::session::SessionStatus;
use fctl_proto::{LoginJobId, PROTOCOL_VERSION, SessionId};
use futures::StreamExt;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

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
    chores_tx: mpsc::UnboundedSender<Chore>,
    chores_rx: mpsc::UnboundedReceiver<Chore>,
    /// Messages waiting for the control connection.
    outbox: VecDeque<HostToFleet>,
    /// Starts that wait for the previous run of the same session to finish,
    /// or for a chore on its sandbox.
    pending_starts: HashMap<SessionId, Box<SessionSpec>>,
    next_request: u64,
    /// When to remove expired stopped sandboxes (see `expiry`).
    sweep: tokio::time::Interval,
    /// Whether a sweep is under way.
    sweeping: bool,
    /// Sandboxes that chores stop or remove, with how many hold each.
    /// Sessions that start in one wait for it.
    busy: HashMap<String, u32>,
}

/// What background work on sandboxes that no session runs in reports.
enum Chore {
    /// The sweep listed this host's sandboxes.
    Listed(Result<Vec<SandboxInfo>>),
    /// The sweep removed a sandbox, or failed to.
    Removed { name: String, result: Result<()> },
    /// The sweep is over.
    Swept,
    /// A sandbox the control plane asked to stop is stopped (and removed,
    /// when it asked for that).
    Stopped { name: String, ended: HostToFleet },
}

const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);
/// How often the host probes its drivers and measures what sandboxes run on.
const FACTS_EVERY: Duration = Duration::from_secs(5 * 60);
const HEARTBEAT_EVERY: Duration = Duration::from_secs(15);

impl Daemon {
    pub async fn new(cfg: HostConfig, token: String) -> Result<Self> {
        let data = cfg.data_dir();
        std::fs::create_dir_all(&data)?;
        let driver = Driver::select(&cfg.driver, &data, cfg.agent_bin.clone()).await?;
        let ctx = Arc::new(HostCtx::new(cfg, driver, token));
        ctx.refresh_facts().await;
        tracing::info!(
            "fctl host {VERSION} on {} using the {} driver, capacity {}, sandboxes {}",
            ctx.cfg.name,
            ctx.driver.name(),
            ctx.cfg.capacity,
            size::limits(&ctx.sandbox_limits())
        );
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (login_tx, login_rx) = mpsc::unbounded_channel();
        let (chores_tx, chores_rx) = mpsc::unbounded_channel();
        Ok(Self {
            ctx,
            sessions: HashMap::new(),
            pending_creds: HashMap::new(),
            logins: HashMap::new(),
            events_tx,
            events_rx,
            login_tx,
            login_rx,
            chores_tx,
            chores_rx,
            outbox: VecDeque::new(),
            pending_starts: HashMap::new(),
            next_request: 0,
            sweep: tokio::time::interval_at(tokio::time::Instant::now() + SWEEP_EVERY, SWEEP_EVERY),
            sweeping: false,
            busy: HashMap::new(),
        })
    }

    /// Pulls the host's sandbox image in the background, now and every
    /// [`refresh::REFRESH_EVERY`]: when it isn't here yet, so that the first
    /// session after an upgrade doesn't wait for it, and on the default
    /// image again, so that new sessions start from its newest build. Then
    /// it removes the images sessions no longer start from.
    fn refresh_image(&self) {
        if !self.ctx.driver.uses_images() {
            return;
        }
        let ctx = self.ctx.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(refresh::REFRESH_EVERY);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                refresh::refresh(&ctx.driver, &ctx.cfg).await;
                clean_up_images(&ctx).await;
            }
        });
    }

    /// Re-attaches to sandboxes that survived a restart. False when the
    /// runtime doesn't answer: the hello must not tell the control plane
    /// that sessions are gone (it would stop them) when they may still run.
    async fn recover(&mut self) -> bool {
        let listed = self.ctx.driver.list().await;
        if timed_out(&listed) {
            return false;
        }
        match listed {
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
        true
    }

    /// Starts removing session sandboxes that have been stopped for longer
    /// than `keep_stopped_hours` (0 keeps them), unless a sweep is under way.
    fn start_sweep(&mut self) {
        if self.sweeping {
            return;
        }
        self.sweeping = true;
        let (ctx, chores) = (self.ctx.clone(), self.chores_tx.clone());
        tokio::spawn(async move {
            let _ = chores.send(Chore::Listed(ctx.driver.list().await));
        });
    }

    /// Picks the sandboxes the sweep removes, and removes them one after
    /// another in the background.
    fn sweep_listed(&mut self, list: Vec<SandboxInfo>) {
        let hours = self.ctx.cfg.keep_stopped_hours;
        let in_use: HashSet<String> = self
            .sessions
            .keys()
            .chain(self.pending_starts.keys())
            .map(|id| sandbox_name(id.as_str()))
            .collect();
        let path = expiry::path(&self.ctx.cfg.data_dir());
        let stopped = expiry::load(&path);
        let now = expiry::now_ms();
        let keep_ms = (hours > 0).then(|| i64::from(hours) * 3_600_000);
        let (expired, mut keep) = expiry::plan(&stopped, &list, &in_use, now, keep_ms);
        // They stay on the list until they are gone, for a host that stops
        // halfway; one that a chore holds waits for the next sweep.
        let mut remove = Vec::new();
        for name in expired {
            keep.insert(name.clone(), stopped.get(&name).copied().unwrap_or(now));
            if !self.busy.contains_key(&name) {
                remove.push(name);
            }
        }
        if let Err(e) = expiry::save(&path, &keep) {
            tracing::warn!("saving {}: {e:#}", path.display());
        }
        if remove.is_empty() {
            self.sweeping = false;
            return;
        }
        for name in &remove {
            self.hold(name);
        }
        let (ctx, chores) = (self.ctx.clone(), self.chores_tx.clone());
        tokio::spawn(async move {
            for name in remove {
                let result = ctx.driver.remove(&ctx.driver.handle(&name)).await;
                let _ = chores.send(Chore::Removed { name, result });
            }
            let _ = chores.send(Chore::Swept);
        });
    }

    fn on_chore(&mut self, chore: Chore) {
        match chore {
            Chore::Listed(Ok(list)) => self.sweep_listed(list),
            Chore::Listed(Err(e)) => {
                tracing::warn!("listing sandboxes: {e:#}");
                self.sweeping = false;
            }
            Chore::Removed { name, result } => {
                match result {
                    Ok(()) => {
                        let hours = self.ctx.cfg.keep_stopped_hours;
                        tracing::info!("removed {name}: stopped for more than {hours} hours");
                        let path = expiry::path(&self.ctx.cfg.data_dir());
                        if let Err(e) = expiry::forget(&path, &name) {
                            tracing::warn!("saving {}: {e:#}", path.display());
                        }
                    }
                    Err(e) => tracing::warn!("removing {name}: {e:#}"),
                }
                self.release(&name);
            }
            Chore::Swept => {
                self.sweeping = false;
                // Images that only the removed sandboxes used can go too.
                let ctx = self.ctx.clone();
                tokio::spawn(async move { clean_up_images(&ctx).await });
            }
            Chore::Stopped { name, ended } => {
                self.outbox.push_back(ended);
                self.release(&name);
            }
        }
    }

    /// Holds a sandbox for a chore: sessions that start in it wait.
    fn hold(&mut self, name: &str) {
        *self.busy.entry(name.to_owned()).or_default() += 1;
    }

    /// Ends a chore's hold on a sandbox, and starts the session that waited
    /// for it.
    fn release(&mut self, name: &str) {
        if let Some(n) = self.busy.get_mut(name)
            && *n > 1
        {
            *n -= 1;
            return;
        }
        self.busy.remove(name);
        if let Some(id) = session_of(name).map(SessionId)
            && !self.sessions.contains_key(&id)
            && let Some(spec) = self.pending_starts.remove(&id)
        {
            tracing::info!("starting {id}: {}", spec.title);
            self.spawn_session(spec);
        }
    }

    fn spawn_session(&mut self, spec: Box<SessionSpec>) {
        let id = spec.session_id.clone();
        let h = session::spawn(self.ctx.clone(), Start::New(spec), self.events_tx.clone());
        self.sessions.insert(id, h);
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

    fn hello(&self) -> HostHello {
        HostHello {
            protocol: PROTOCOL_VERSION,
            version: VERSION.into(),
            hostname: crate::config::hostname(),
            os: sysinfo::os().into(),
            arch: sysinfo::arch().into(),
            cpus: sysinfo::cpus(),
            memory_mb: sysinfo::memory_mb(),
            drivers: self.ctx.drivers(),
            driver: self.ctx.driver.name().into(),
            capacity: self.ctx.cfg.capacity,
            labels: self.ctx.cfg.labels.clone(),
            sessions: self.reports(),
            image: Some(self.ctx.cfg.image()),
            sandbox: Some(self.ctx.sandbox_limits()),
        }
    }

    fn heartbeat(&self) -> HostHeartbeat {
        HostHeartbeat {
            sessions: self.reports(),
            load1: sysinfo::load1(),
            memory_available_mb: sysinfo::memory_available_mb(),
            disk_available_mb: sysinfo::disk_available_mb(&self.ctx.cfg.data_dir()),
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let mut shutdown = shutdown_signal();
        keep_awake();
        let mut wait = Duration::from_secs(5);
        loop {
            let recovered = tokio::select! {
                r = self.recover() => r,
                _ = &mut shutdown => return Ok(()),
            };
            if recovered {
                break;
            }
            tracing::warn!(
                "the {} driver does not answer; connecting once it lists the sandboxes (next try in {} s)",
                self.ctx.driver.name(),
                wait.as_secs()
            );
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = &mut shutdown => return Ok(()),
            }
            wait = (wait * 2).min(Duration::from_secs(60));
        }
        // Once the runtime answers.
        self.refresh_image();
        self.start_sweep();
        refresh_facts_every(self.ctx.clone(), FACTS_EVERY);
        let url = format!(
            "{}/api/v1/runner/connect",
            self.ctx.cfg.server.trim_end_matches('/')
        );
        let mut backoff = Duration::from_secs(1);
        loop {
            let connected = tokio::select! {
                c = ws::connect(&url, Some(&self.ctx.token)) => c,
                _ = &mut shutdown => return Ok(()),
            };
            match connected {
                Ok(socket) => {
                    backoff = Duration::from_secs(1);
                    tracing::info!("connected to {}", self.ctx.cfg.server);
                    match self.serve(socket, &mut shutdown).await {
                        Ok(()) => tracing::info!("the control plane closed the connection"),
                        Err(e) => tracing::warn!("control connection: {e:#}"),
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
                    Some(ev) = self.events_rx.recv() => self.on_session_event(ev),
                    Some(msg) = self.login_rx.recv() => self.outbox.push_back(msg),
                    Some(chore) = self.chores_rx.recv() => self.on_chore(chore),
                    _ = self.sweep.tick() => self.start_sweep(),
                    _ = &mut shutdown => return Ok(()),
                }
            }
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }

    async fn serve(
        &mut self,
        socket: ws::Socket,
        shutdown: &mut oneshot::Receiver<()>,
    ) -> Result<()> {
        let (mut sink, mut stream) = socket.split();
        ws::send_json(&mut sink, &HostToFleet::Hello(Box::new(self.hello()))).await?;
        let mut heartbeat = tokio::time::interval(HEARTBEAT_EVERY);
        heartbeat.tick().await;
        loop {
            // What the last step queued, and what waited for a connection.
            // A message that can't be sent waits for the next one.
            while let Some(m) = self.outbox.pop_front() {
                if let Err(e) = ws::send_json(&mut sink, &m).await {
                    self.outbox.push_front(m);
                    return Err(e);
                }
            }
            tokio::select! {
                msg = ws::next_text(&mut stream) => {
                    let text = match msg {
                        Some(Ok(t)) => t,
                        Some(Err(e)) => return Err(e),
                        None => return Ok(()),
                    };
                    match serde_json::from_str::<FleetToHost>(&text) {
                        Ok(msg) => self.on_fleet(msg),
                        Err(e) => tracing::warn!("unknown message from the control plane: {e}"),
                    }
                }
                Some(ev) = self.events_rx.recv() => self.on_session_event(ev),
                Some(msg) = self.login_rx.recv() => self.outbox.push_back(msg),
                Some(chore) = self.chores_rx.recv() => self.on_chore(chore),
                _ = self.sweep.tick() => self.start_sweep(),
                _ = heartbeat.tick() => {
                    let hb = self.heartbeat();
                    self.outbox.push_back(HostToFleet::Heartbeat(hb));
                }
                _ = &mut *shutdown => {
                    tracing::info!("shutting down; sessions keep running in their sandboxes");
                    std::process::exit(0);
                }
            }
        }
    }

    /// Handles a session event; what it has for the control plane goes to
    /// the outbox.
    fn on_session_event(&mut self, ev: SessionEvent) {
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
                self.outbox.push_back(HostToFleet::CredentialRequest {
                    request_id: rid,
                    session_id,
                    kind,
                });
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
                // Its sandbox (if kept) expires keep_stopped_hours from now.
                let stopped = expiry::path(&self.ctx.cfg.data_dir());
                let name = sandbox_name(session_id.as_str());
                if let Err(e) = expiry::note_stopped(&stopped, &name, expiry::now_ms()) {
                    tracing::warn!("saving {}: {e:#}", stopped.display());
                }
                self.outbox.push_back(HostToFleet::SessionEnded {
                    session_id: session_id.clone(),
                    status,
                    reason,
                    run_id,
                });
                if let Some(spec) = self.pending_starts.remove(&session_id) {
                    tracing::info!("starting the next run of {session_id}");
                    self.spawn_session(spec);
                }
            }
        }
    }

    /// Handles a message from the control plane; replies go to the outbox.
    fn on_fleet(&mut self, msg: FleetToHost) {
        match msg {
            FleetToHost::Welcome { runner, .. } => {
                tracing::info!("registered as {} ({})", runner.name, runner.id);
                if let Ok(mut r) = self.ctx.runner_id.lock() {
                    *r = Some(runner.id);
                }
            }
            FleetToHost::Ping => self.outbox.push_back(HostToFleet::Pong),
            FleetToHost::StartSession { spec } => {
                let id = spec.session_id.clone();
                let error = self.start_session(spec).err();
                self.outbox.push_back(HostToFleet::StartResult {
                    session_id: id,
                    accepted: error.is_none(),
                    error,
                });
            }
            FleetToHost::StopSession {
                session_id,
                reason,
                destroy,
            } => self.stop_session(session_id, reason, destroy),
            FleetToHost::Credential {
                request_id,
                claude,
                github,
                github_read,
                error,
            } => {
                if let Some((sid, agent_request)) = self.pending_creds.remove(&request_id)
                    && let Some(h) = self.sessions.get(&sid)
                {
                    let _ = h.tx.send(SessionCmd::Credentials {
                        request_id: Some(agent_request),
                        claude,
                        github,
                        github_read,
                        error,
                    });
                }
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
            }
            FleetToHost::ClaudeLoginCode { job_id, code } => {
                if let Some(tx) = self.logins.get(&job_id) {
                    let _ = tx.send(LoginCmd::Code(code));
                }
            }
            FleetToHost::ClaudeLoginCancel { job_id } => {
                if let Some(tx) = self.logins.remove(&job_id) {
                    let _ = tx.send(LoginCmd::Cancel);
                }
            }
        }
    }

    /// Takes on a session, or says why not.
    fn start_session(&mut self, spec: Box<SessionSpec>) -> Result<(), String> {
        let id = spec.session_id.clone();
        // The control plane only sends sessions that fit; an older one may
        // not know this host's limits.
        let limits = self.ctx.sandbox_limits();
        if !Want::of(&spec.project.settings).fits(&limits) {
            return Err(format!(
                "{} gives a sandbox at most {} and {} of memory",
                self.ctx.cfg.name,
                size::cpus(limits.max.cpus),
                size::memory(limits.max.memory_mb.into())
            ));
        }
        if let Some(existing) = self.sessions.get(&id) {
            let winding_down = existing
                .report
                .lock()
                .map(|r| r.status.is_terminal() || r.status == SessionStatus::Stopping)
                .unwrap_or(false);
            if winding_down {
                // A resume while the last run still shuts down: start it right after.
                let _ = existing.tx.send(SessionCmd::Superseded);
                self.pending_starts.insert(id, spec);
            }
            return Ok(());
        }
        if let Some(waiting) = self.pending_starts.get_mut(&id) {
            *waiting = spec;
            return Ok(());
        }
        let waiting = self
            .pending_starts
            .keys()
            .filter(|id| !self.sessions.contains_key(*id))
            .count();
        if (self.sessions.len() + waiting) as u32 >= self.ctx.cfg.capacity {
            return Err("the host is at capacity".into());
        }
        if self.busy.contains_key(&sandbox_name(id.as_str())) {
            tracing::info!("{id} starts once its sandbox is stopped or removed");
            self.pending_starts.insert(id, spec);
            return Ok(());
        }
        tracing::info!("starting {id}: {}", spec.title);
        self.spawn_session(spec);
        Ok(())
    }

    fn stop_session(&mut self, session_id: SessionId, reason: StopReason, destroy: bool) {
        if let Some(h) = self.sessions.get(&session_id) {
            let _ = h.tx.send(SessionCmd::Stop { reason, destroy });
            return;
        }
        // Not running here: a start that waits for its sandbox is called
        // off, and a leftover sandbox is stopped in the background.
        self.pending_starts.remove(&session_id);
        let name = sandbox_name(session_id.as_str());
        self.hold(&name);
        let (ctx, chores) = (self.ctx.clone(), self.chores_tx.clone());
        tokio::spawn(async move {
            let sb = ctx.driver.handle(&name);
            if let Err(e) = ctx.driver.stop(&sb).await {
                tracing::warn!("stopping {name}: {e:#}");
            }
            if destroy && let Err(e) = ctx.driver.remove(&sb).await {
                tracing::warn!("removing {name}: {e:#}");
            }
            let ended = HostToFleet::SessionEnded {
                session_id,
                status: SessionStatus::Stopped,
                reason: Some("Stopped".into()),
                run_id: None,
            };
            let _ = chores.send(Chore::Stopped { name, ended });
        });
    }
}

/// Removes the sandbox images that sessions no longer start from, saying
/// which in the log.
async fn clean_up_images(ctx: &HostCtx) {
    for name in refresh::clean_up(&ctx.driver, &ctx.cfg).await {
        let what = refresh::describe(&name);
        tracing::info!("removed {what}: sessions no longer start from it");
    }
}

/// Probes the drivers and measures what sandboxes run on every `period`, in
/// the background.
fn refresh_facts_every(ctx: Arc<HostCtx>, period: Duration) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            ctx.refresh_facts().await;
        }
    });
}

fn jitter(d: Duration) -> Duration {
    let ms = d.as_millis() as u64;
    let r = fctl_domain::random_bytes::<2>();
    let spread = (u16::from_le_bytes(r) as u64 % (ms / 4 + 1)) as i64 - (ms / 8) as i64;
    Duration::from_millis((ms as i64 + spread).max(100) as u64)
}

/// Fires on SIGTERM or SIGINT. It listens from the start, so that a signal
/// that comes while the daemon is busy elsewhere is not lost.
fn shutdown_signal() -> oneshot::Receiver<()> {
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        signalled().await;
        let _ = tx.send(());
    });
    rx
}

async fn signalled() {
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
