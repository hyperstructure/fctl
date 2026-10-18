//! The host daemon: the control connection to FleetObject and every session.
//!
//! Sessions outlive the control connection (each has its own uplink) and
//! outlive the daemon itself: agents keep running in their sandboxes, and a
//! restarted daemon finds and re-attaches to them.
//!
//! The control connection never waits on a driver. The hello reports what
//! the drivers said when last probed, and sweeping or cleaning up sandboxes
//! and measuring the disk run in tasks of their own (`Chore`), so
//! heartbeats and pongs keep flowing while a runtime hangs.

use crate::config::HostConfig;
use crate::driver::{DiskSpot, Driver, SandboxInfo, sandbox_name, session_of, timed_out};
use crate::expiry::{self, Why};
use crate::login::{self, LoginCmd};
use crate::refresh;
use crate::session::{self, HostCtx, SessionCmd, SessionEvent, SessionHandle, Start};
use crate::sysinfo;
use anyhow::Result;
use fctl_client::ws;
use fctl_domain::disk;
use fctl_domain::image::ImageWant;
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
    /// What the sweep removes next, one at a time, and why.
    removals: VecDeque<(String, Why)>,
    /// Whether the sweep under way removed a sandbox.
    removed_some: bool,
    /// Whether a stopped sandbox may be left to remove: false from when a
    /// sweep plans until one stops here, or the sweep leaves some for lack
    /// of need.
    removable: bool,
    /// Free disk (MiB) where sandboxes are, as last measured.
    disk_free_mb: Option<u64>,
    /// Sandboxes that chores stop or remove, with how many hold each.
    /// Sessions that start in one wait for it.
    busy: HashMap<String, u32>,
}

/// What background work on sandboxes that no session runs in reports.
enum Chore {
    /// The sweep listed this host's sandboxes.
    Listed(Result<Vec<SandboxInfo>>),
    /// The sweep removed a sandbox, or failed to, and measured the disk
    /// again.
    Removed {
        name: String,
        why: Why,
        result: Result<()>,
        free_mb: Option<u64>,
    },
    /// A sandbox the control plane asked to stop is stopped (and removed,
    /// when it asked for that).
    Stopped { name: String, ended: HostToFleet },
    /// The free disk (MiB) where sandboxes are, as measured now.
    Disk(Option<u64>),
}

const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);
/// How often the host probes its drivers and measures what sandboxes run on,
/// and the disk inside the Docker VM, where measuring starts a container.
const FACTS_EVERY: Duration = Duration::from_secs(5 * 60);
/// How often the host measures its free disk elsewhere.
const DISK_EVERY: Duration = Duration::from_secs(30);
const HEARTBEAT_EVERY: Duration = Duration::from_secs(15);

impl Daemon {
    pub async fn new(cfg: HostConfig, token: String) -> Result<Self> {
        let data = cfg.data_dir();
        std::fs::create_dir_all(&data)?;
        // What image builds left behind when the host last stopped.
        let _ = std::fs::remove_dir_all(data.join("builds"));
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
        if let Some(floor) = ctx.cfg.min_free_disk_mb() {
            tracing::info!(
                "keeping {} of disk free at {} (min_free_disk_gb)",
                size::memory(floor),
                ctx.disk_spot()
            );
        }
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
            removals: VecDeque::new(),
            removed_some: false,
            removable: true,
            disk_free_mb: None,
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
    /// than `keep_stopped_hours` (0 keeps them), and while the disk is low
    /// the others, unless a sweep is under way.
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

    /// Plans what the sweep removes (see `expiry`), and starts removing it.
    fn sweep_listed(&mut self, list: Vec<SandboxInfo>) {
        let hours = self.ctx.cfg.keep_stopped_hours;
        let in_use: HashSet<String> = self
            .sessions
            .keys()
            .chain(self.pending_starts.keys())
            .map(|id| sandbox_name(id.as_str()))
            .collect();
        let path = expiry::path(&self.ctx.cfg.data_dir());
        let now = expiry::now_ms();
        let keep_ms = (hours > 0).then(|| i64::from(hours) * 3_600_000);
        let plan = expiry::plan(&expiry::load(&path), &list, &in_use, now, keep_ms);
        if let Err(e) = expiry::save(&path, &plan.stopped) {
            tracing::warn!("saving {}: {e:#}", path.display());
        }
        self.removals = plan.removals().collect();
        // A session that stops from now on sets it again.
        self.removable = false;
        self.remove_next();
    }

    /// Removes the next sandbox the sweep planned, in the background, or
    /// ends the sweep. One at a time: a session that starts in one meanwhile
    /// keeps it, and each removal measures the disk again, so that removing
    /// for a low disk stops once there is room.
    fn remove_next(&mut self) {
        while let Some((name, why)) = self.removals.pop_front() {
            if why == Why::LowDisk && !self.low_on_disk() {
                // The rest stay, for when the disk runs low again.
                self.removals.clear();
                self.removable = true;
                break;
            }
            // A session started in it meanwhile, or a chore holds it: it
            // waits for the next sweep.
            if self.in_use(&name) || self.busy.contains_key(&name) {
                continue;
            }
            self.hold(&name);
            let (ctx, chores) = (self.ctx.clone(), self.chores_tx.clone());
            tokio::spawn(async move {
                let result = ctx.driver.remove(&ctx.driver.handle(&name)).await;
                let free_mb = ctx.disk_free_mb().await;
                let _ = chores.send(Chore::Removed {
                    name,
                    why,
                    result,
                    free_mb,
                });
            });
            return;
        }
        self.sweeping = false;
        if self.low_on_disk() && !self.removable {
            tracing::warn!(
                "no stopped sandbox is left to remove: taking no new sessions until there is room ({})",
                self.below()
            );
        }
        if std::mem::take(&mut self.removed_some) {
            // Images that only the removed sandboxes used can go too.
            let ctx = self.ctx.clone();
            tokio::spawn(async move { clean_up_images(&ctx).await });
        }
    }

    fn on_chore(&mut self, chore: Chore) {
        match chore {
            Chore::Listed(Ok(list)) => self.sweep_listed(list),
            Chore::Listed(Err(e)) => {
                tracing::warn!("listing sandboxes: {e:#}");
                self.sweeping = false;
            }
            Chore::Removed {
                name,
                why,
                result,
                free_mb,
            } => {
                match result {
                    Ok(()) => {
                        self.removed_some = true;
                        match why {
                            Why::Expired => {
                                let hours = self.ctx.cfg.keep_stopped_hours;
                                tracing::info!(
                                    "removed {name}: stopped for more than {hours} hours"
                                );
                            }
                            Why::LowDisk => tracing::info!(
                                "removed {name} to make room on the disk ({})",
                                self.below()
                            ),
                        }
                        let path = expiry::path(&self.ctx.cfg.data_dir());
                        if let Err(e) = expiry::forget(&path, &name) {
                            tracing::warn!("saving {}: {e:#}", path.display());
                        }
                    }
                    Err(e) => tracing::warn!("removing {name}: {e:#}"),
                }
                self.note_disk(free_mb);
                self.release(&name);
                self.remove_next();
            }
            Chore::Stopped { name, ended } => {
                self.outbox.push_back(ended);
                self.removable = true;
                self.release(&name);
            }
            Chore::Disk(free_mb) => {
                self.note_disk(free_mb);
                // Below the floor, stopped sandboxes go before their time.
                if self.low_on_disk() && self.removable {
                    self.start_sweep();
                }
            }
        }
    }

    /// Whether the disk has less free space than the host keeps
    /// (`min_free_disk_gb`): stopped sandboxes go early, and no new session
    /// starts here.
    fn low_on_disk(&self) -> bool {
        disk::low(self.disk_free_mb, self.ctx.cfg.min_free_disk_mb())
    }

    /// How free the disk is, against the floor the host keeps.
    fn below(&self) -> String {
        match (self.disk_free_mb, self.ctx.cfg.min_free_disk_mb()) {
            (Some(free), Some(floor)) => disk::below(free, floor),
            (Some(free), None) => format!("{} of disk free", size::memory(free)),
            (None, _) => "free disk unknown".into(),
        }
    }

    /// Remembers what a measurement found, and says in the log when the
    /// disk goes below the floor or comes back above it.
    fn note_disk(&mut self, free_mb: Option<u64>) {
        let was_low = self.low_on_disk();
        self.disk_free_mb = free_mb;
        match (was_low, self.low_on_disk(), free_mb) {
            (false, true, _) => tracing::warn!(
                "{} at {}: removing stopped sandboxes, those stopped longest first, and taking no new sessions until there is room",
                self.below(),
                self.ctx.disk_spot()
            ),
            (true, false, Some(free)) => tracing::info!(
                "{} of disk free again: taking new sessions",
                size::memory(free)
            ),
            (true, false, None) => tracing::warn!(
                "can't measure the free disk at {}: taking new sessions again",
                self.ctx.disk_spot()
            ),
            _ => {}
        }
    }

    /// Whether `sandbox` is the sandbox of a session that runs here or
    /// waits to start.
    fn in_use(&self, sandbox: &str) -> bool {
        self.sessions
            .keys()
            .chain(self.pending_starts.keys())
            .any(|id| sandbox_name(id.as_str()) == sandbox)
    }

    /// Whether the session's sandbox is here, stopped, as the host last
    /// noted it.
    fn has_stopped_sandbox(&self, id: &SessionId) -> bool {
        let stopped = expiry::load(&expiry::path(&self.ctx.cfg.data_dir()));
        stopped.contains_key(&sandbox_name(id.as_str()))
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
        let images = self.ctx.images();
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
            image: Some(images.own),
            sandbox: Some(self.ctx.sandbox_limits()),
            allowed_images: images.allowed,
            allowed_builds: images.builds,
            min_free_disk_mb: self.ctx.cfg.min_free_disk_mb(),
        }
    }

    fn heartbeat(&self) -> HostHeartbeat {
        HostHeartbeat {
            sessions: self.reports(),
            load1: sysinfo::load1(),
            memory_available_mb: sysinfo::memory_available_mb(),
            disk_available_mb: self.disk_free_mb,
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
        watch_disk(self.ctx.clone(), self.chores_tx.clone());
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
                // Its sandbox is one a low disk can make room in.
                self.removable = true;
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
        let want = ImageWant::of(&spec.project.settings, spec.project.repo.as_ref());
        if !self.ctx.images().policy().takes(&want) {
            return Err(match want {
                ImageWant::Build(repo) => format!(
                    "{} builds no images from {} (allowed_builds in its host.toml)",
                    self.ctx.cfg.name,
                    repo.full_name()
                ),
                ImageWant::Image(image) => format!(
                    "{} doesn't run {image} (allowed_images in its host.toml)",
                    self.ctx.cfg.name
                ),
                ImageWant::Default => format!("{} can't take the session", self.ctx.cfg.name),
            });
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
        // Low on disk, it takes only the sessions whose sandboxes it has:
        // they need no new one. The control plane sends no others, but it
        // learns the free disk from heartbeats, which come every 15 s.
        if self.low_on_disk() && !self.has_stopped_sandbox(&id) {
            return Err(format!(
                "{} has {} (min_free_disk_gb in its host.toml)",
                self.ctx.cfg.name,
                self.below()
            ));
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

/// Measures the free disk where sandboxes are, now and then, in the
/// background, and tells the daemon: every [`DISK_EVERY`], or inside the
/// Docker VM, where measuring starts a container, every [`FACTS_EVERY`].
fn watch_disk(ctx: Arc<HostCtx>, chores: mpsc::UnboundedSender<Chore>) {
    tokio::spawn(async move {
        loop {
            let free_mb = ctx.disk_free_mb().await;
            if chores.send(Chore::Disk(free_mb)).is_err() {
                return;
            }
            let every = match ctx.disk_spot() {
                DiskSpot::DockerVm => FACTS_EVERY,
                DiskSpot::Dir(_) => DISK_EVERY,
            };
            tokio::time::sleep(every).await;
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    const GIB: u64 = 1024;
    const MINUTE: i64 = 60_000;

    /// A process-driver daemon whose data directory has stopped session
    /// sandboxes, each stopped `minutes` ago.
    async fn daemon_with(stopped: &[(&str, i64)]) -> Result<(tempfile::TempDir, Daemon)> {
        let dir = tempfile::tempdir()?;
        let path = expiry::path(dir.path());
        for (name, minutes) in stopped {
            std::fs::create_dir_all(dir.path().join("sandboxes").join(name))?;
            expiry::note_stopped(&path, name, expiry::now_ms() - minutes * MINUTE)?;
        }
        let cfg = HostConfig {
            driver: "process".into(),
            data_dir: Some(dir.path().to_owned()),
            ..HostConfig::default()
        };
        Ok((dir, Daemon::new(cfg, "fctl_rt_test".into()).await?))
    }

    /// Runs the sweep under way to its end, as if the disk had
    /// `free_after(n)` MiB free after its `n`th removal: what it removed,
    /// in order.
    async fn finish_sweep(
        d: &mut Daemon,
        free_after: impl Fn(usize) -> u64,
    ) -> Result<Vec<String>> {
        let mut removed = Vec::new();
        while d.sweeping {
            let next = tokio::time::timeout(Duration::from_secs(10), d.chores_rx.recv());
            let mut chore = next.await?.context("no more chores")?;
            if let Chore::Removed { name, free_mb, .. } = &mut chore {
                removed.push(name.clone());
                *free_mb = Some(free_after(removed.len()));
            }
            d.on_chore(chore);
        }
        Ok(removed)
    }

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

    #[tokio::test]
    async fn a_low_disk_removes_stopped_sandboxes_oldest_first_until_there_is_room() -> Result<()> {
        let (newest, oldest, middle, held) = (
            "fctl-ses-01ccc",
            "fctl-ses-01aaa",
            "fctl-ses-01bbb",
            "fctl-ses-01ddd",
        );
        let (dir, mut d) =
            daemon_with(&[(newest, 1), (oldest, 30), (middle, 10), (held, 20)]).await?;
        // A chore stops that one: it waits for the next sweep.
        d.hold(held);
        // 17 GiB free, below the 20 GiB it keeps: a sweep starts.
        d.on_chore(Chore::Disk(Some(17 * GIB)));
        assert!(d.sweeping && d.low_on_disk());
        let removed = finish_sweep(&mut d, |n| [19, 21][n - 1] * GIB).await?;
        // Two make room; the newest stays, for a resume.
        assert_eq!(removed, [oldest, middle]);
        assert!(!d.low_on_disk() && d.removable);
        let sandboxes = dir.path().join("sandboxes");
        assert!(sandboxes.join(newest).is_dir() && sandboxes.join(held).is_dir());
        assert!(!sandboxes.join(oldest).exists() && !sandboxes.join(middle).exists());
        let stopped = expiry::load(&expiry::path(dir.path()));
        assert_eq!(
            stopped.keys().map(String::as_str).collect::<Vec<_>>(),
            [newest, held]
        );
        Ok(())
    }

    #[tokio::test]
    async fn with_nothing_left_to_remove_only_sessions_with_a_sandbox_here_start() -> Result<()> {
        let (_dir, mut d) = daemon_with(&[("fctl-ses-01aaa", 30)]).await?;
        d.on_chore(Chore::Disk(Some(10 * GIB)));
        let removed = finish_sweep(&mut d, |_| 11 * GIB).await?;
        assert_eq!(removed, ["fctl-ses-01aaa"]);
        assert!(d.low_on_disk() && !d.removable);
        // A new session needs a new sandbox.
        let refused = d
            .start_session(spec("ses_01NEW")?)
            .err()
            .unwrap_or_default();
        assert!(
            refused.ends_with(
                "has 11 GiB of disk free, below the 20 GiB it keeps free (min_free_disk_gb in its host.toml)"
            ),
            "{refused}"
        );
        // One whose sandbox is here resumes in it (once a chore on it ends).
        let path = expiry::path(&d.ctx.cfg.data_dir());
        expiry::note_stopped(&path, "fctl-ses-01own", expiry::now_ms())?;
        d.hold("fctl-ses-01own");
        assert_eq!(d.start_session(spec("ses_01OWN")?), Ok(()));
        assert!(
            d.pending_starts
                .contains_key(&SessionId("ses_01OWN".into()))
        );
        // Measuring again lists nothing while nothing has stopped here…
        d.on_chore(Chore::Disk(Some(11 * GIB)));
        assert!(!d.sweeping);
        // …and a session that stops is one to make room with.
        d.on_session_event(SessionEvent::Ended {
            session_id: SessionId("ses_01EEE".into()),
            status: SessionStatus::Stopped,
            reason: None,
            run_id: None,
        });
        d.on_chore(Chore::Disk(Some(11 * GIB)));
        assert!(d.sweeping);
        finish_sweep(&mut d, |_| 11 * GIB).await?;
        Ok(())
    }

    #[tokio::test]
    async fn a_session_that_stops_during_a_sweep_leaves_one_to_remove() -> Result<()> {
        let (dir, mut d) = daemon_with(&[("fctl-ses-01aaa", 30)]).await?;
        d.on_chore(Chore::Disk(Some(10 * GIB)));
        // The sweep has listed the sandboxes, and removes the one it found…
        let listed = tokio::time::timeout(Duration::from_secs(10), d.chores_rx.recv())
            .await?
            .context("no more chores")?;
        d.on_chore(listed);
        // …when another session stops.
        std::fs::create_dir_all(dir.path().join("sandboxes/fctl-ses-01eee"))?;
        d.on_session_event(SessionEvent::Ended {
            session_id: SessionId("ses_01EEE".into()),
            status: SessionStatus::Stopped,
            reason: None,
            run_id: None,
        });
        assert_eq!(
            finish_sweep(&mut d, |_| 11 * GIB).await?,
            ["fctl-ses-01aaa"]
        );
        // Still low: the next measurement starts a sweep that removes it.
        assert!(d.low_on_disk() && d.removable);
        d.on_chore(Chore::Disk(Some(11 * GIB)));
        assert_eq!(
            finish_sweep(&mut d, |_| 13 * GIB).await?,
            ["fctl-ses-01eee"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn expired_sandboxes_go_whatever_the_disk_has() -> Result<()> {
        let (_dir, mut d) =
            daemon_with(&[("fctl-ses-01old", 100 * 60), ("fctl-ses-01new", 1)]).await?;
        d.on_chore(Chore::Disk(Some(500 * GIB)));
        assert!(!d.sweeping);
        // The hourly sweep.
        d.start_sweep();
        let removed = finish_sweep(&mut d, |_| 500 * GIB).await?;
        assert_eq!(removed, ["fctl-ses-01old"]);
        assert!(d.removable);
        Ok(())
    }
}
