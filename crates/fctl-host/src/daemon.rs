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

use crate::caches;
use crate::claude_token;
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
    /// What the sweep removes, measures or empties next, one at a time,
    /// and why.
    removals: VecDeque<Removal>,
    /// Whether the sweep under way removed a sandbox.
    removed_some: bool,
    /// Whether a stopped sandbox or package caches may be left to remove:
    /// false from when a sweep plans until a session stops here, or the
    /// sweep leaves some for lack of need.
    removable: bool,
    /// Projects whose package caches a chore measures or empties: their
    /// sessions wait to start.
    clearing: HashSet<String>,
    /// Free disk (MiB) where sandboxes are, as last measured.
    disk_free_mb: Option<u64>,
    /// Sandboxes that chores stop or remove, with how many hold each.
    /// Sessions that start in one wait for it.
    busy: HashMap<String, u32>,
}

/// What the sweep removes, measures or empties.
enum Removal {
    Sandbox(String, Why),
    /// A project's package caches.
    Cache(String, caches::Why),
}

/// What a chore did with a project's package caches.
enum CacheDone {
    /// Measured them: they take this many MiB, no more than they may.
    Measured(u64),
    /// Emptied them, or removed them when `gone`, having found that they
    /// take this many MiB, when it measured them.
    Cleared { gone: bool, size_mb: Option<u64> },
}

/// What background work on sandboxes that no session runs in reports.
enum Chore {
    /// The sweep listed this host's sandboxes, and the projects whose
    /// package caches are here.
    Listed(Result<Vec<SandboxInfo>>, Vec<String>),
    /// The sweep removed a sandbox, or failed to, and measured the disk
    /// again.
    Removed {
        name: String,
        why: Why,
        result: Result<()>,
        free_mb: Option<u64>,
    },
    /// The sweep measured or emptied a project's package caches, or failed
    /// to, and measured the disk again.
    Cache {
        project: String,
        why: caches::Why,
        result: Result<CacheDone>,
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
        if let (Some(max), Some(_)) = (
            ctx.cfg.max_cache_mb(),
            ctx.driver.cache_spot("prj_0", &data),
        ) {
            let unused = match ctx.cfg.keep_caches_days {
                0 => "however long they go unused".to_owned(),
                days => format!("until {days} days after a session last started with them"),
            };
            tracing::info!(
                "keeping up to {} of package caches for each project that shares them, {unused} (max_cache_gb, keep_caches_days)",
                size::memory(max)
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
            clearing: HashSet::new(),
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
    /// than `keep_stopped_hours` (0 keeps them) and package caches unused
    /// for longer than `keep_caches_days`, emptying caches past
    /// `max_cache_gb`, and while the disk is low emptying the other caches
    /// and removing the other stopped sandboxes, unless a sweep is under
    /// way.
    fn start_sweep(&mut self) {
        if self.sweeping {
            return;
        }
        self.sweeping = true;
        let (ctx, chores) = (self.ctx.clone(), self.chores_tx.clone());
        tokio::spawn(async move {
            let listed = ctx.driver.list().await;
            let caches = match &listed {
                Ok(_) => ctx
                    .driver
                    .caches(&ctx.cfg.data_dir())
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!("listing the package caches: {e:#}");
                        Vec::new()
                    }),
                Err(_) => Vec::new(),
            };
            let _ = chores.send(Chore::Listed(listed, caches));
        });
    }

    /// Plans what the sweep removes (see `expiry` and `caches`), and starts
    /// removing it. Package caches go first: emptied, they cost the next
    /// session downloads, where a removed sandbox costs Claude's
    /// conversation.
    fn sweep_listed(&mut self, list: Vec<SandboxInfo>, here: Vec<String>) {
        let hours = self.ctx.cfg.keep_stopped_hours;
        let in_use: HashSet<String> = self
            .sessions
            .keys()
            .chain(self.pending_starts.keys())
            .map(|id| sandbox_name(id.as_str()))
            .collect();
        let data = self.ctx.cfg.data_dir();
        let path = expiry::path(&data);
        let now = expiry::now_ms();
        let keep_ms = (hours > 0).then(|| i64::from(hours) * 3_600_000);
        let plan = expiry::plan(&expiry::load(&path), &list, &in_use, now, keep_ms);
        if let Err(e) = expiry::save(&path, &plan.stopped) {
            tracing::warn!("saving {}: {e:#}", path.display());
        }
        // A host that keeps no caches now removes those it kept.
        let keep_caches_ms = match self.ctx.cfg.max_cache_mb() {
            Some(_) => self.ctx.cfg.keep_caches_ms(),
            None => Some(0),
        };
        let cache_path = caches::path(&data);
        let cached = caches::plan(
            &caches::load(&cache_path),
            &here,
            &list,
            &in_use,
            now,
            keep_caches_ms,
        );
        if let Err(e) = caches::save(&cache_path, &cached.records) {
            tracing::warn!("saving {}: {e:#}", cache_path.display());
        }
        let cache = |why: caches::Why| move |project: String| Removal::Cache(project, why);
        let sandbox = |why: Why| move |name: String| Removal::Sandbox(name, why);
        self.removals = cached
            .unused
            .into_iter()
            .map(cache(caches::Why::Unused))
            .chain(cached.grown.into_iter().map(cache(caches::Why::Grown)))
            .chain(plan.expired.into_iter().map(sandbox(Why::Expired)))
            .chain(
                cached
                    .least_recent_first
                    .into_iter()
                    .map(cache(caches::Why::LowDisk)),
            )
            .chain(plan.oldest_first.into_iter().map(sandbox(Why::LowDisk)))
            .collect();
        // A session that stops from now on sets it again.
        self.removable = false;
        self.remove_next();
    }

    /// Removes the next sandbox the sweep planned, or measures or empties
    /// the next package caches, in the background, or ends the sweep. One
    /// at a time: a session that starts in a sandbox meanwhile keeps it, or
    /// with the caches waits for them, and each one measures the disk
    /// again, so that removing for a low disk stops once there is room.
    fn remove_next(&mut self) {
        while let Some(removal) = self.removals.pop_front() {
            let for_room = matches!(
                removal,
                Removal::Sandbox(_, Why::LowDisk) | Removal::Cache(_, caches::Why::LowDisk)
            );
            if for_room && !self.low_on_disk() {
                // The rest stay, for when the disk runs low again.
                self.removals.clear();
                self.removable = true;
                break;
            }
            let (name, why) = match removal {
                Removal::Sandbox(name, why) => (name, why),
                Removal::Cache(project, why) => {
                    if self.clear_cache(project, why) {
                        return;
                    }
                    continue;
                }
            };
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
                "no stopped sandbox or package caches are left to remove: taking no new sessions until there is room ({})",
                self.below()
            );
        }
        if std::mem::take(&mut self.removed_some) {
            // Images that only the removed sandboxes used can go too.
            let ctx = self.ctx.clone();
            tokio::spawn(async move { clean_up_images(&ctx).await });
        }
    }

    /// Measures or empties `project`'s package caches in the background,
    /// for the sweep: whether it does, which it doesn't while a session here
    /// uses them, or when they hold nothing to free for a low disk. A
    /// session that starts with them meanwhile waits.
    fn clear_cache(&mut self, project: String, why: caches::Why) -> bool {
        let data = self.ctx.cfg.data_dir();
        let records = caches::load(&caches::path(&data));
        if self.caches_in_use(&records).contains(&project) {
            return false;
        }
        // A sandbox here may mount them again: they stay, emptied.
        let keep = records.mounted(&project);
        // Emptied since their last use, they have nothing to free, and
        // stay while a sandbox may mount them.
        let empty = records.projects.get(&project).is_some_and(|r| r.empty());
        if empty && (why == caches::Why::LowDisk || keep) {
            return false;
        }
        let Some(spot) = self.ctx.driver.cache_spot(&project, &data) else {
            return false;
        };
        let max_mb = self.ctx.cfg.max_cache_mb();
        self.clearing.insert(project.clone());
        let (ctx, chores) = (self.ctx.clone(), self.chores_tx.clone());
        tokio::spawn(async move {
            let images = ctx.cfg.own_images();
            let result = match why {
                caches::Why::Grown => match ctx.driver.cache_size_mb(&spot, &images).await {
                    Ok(size) if max_mb.is_some_and(|max| size > max) => ctx
                        .driver
                        .clear_cache(&spot, keep, &images)
                        .await
                        .map(|gone| CacheDone::Cleared {
                            gone,
                            size_mb: Some(size),
                        }),
                    Ok(size) => Ok(CacheDone::Measured(size)),
                    Err(e) => Err(e),
                },
                caches::Why::Unused | caches::Why::LowDisk => ctx
                    .driver
                    .clear_cache(&spot, keep, &images)
                    .await
                    .map(|gone| CacheDone::Cleared {
                        gone,
                        size_mb: None,
                    }),
            };
            let free_mb = ctx.disk_free_mb().await;
            let _ = chores.send(Chore::Cache {
                project,
                why,
                result,
                free_mb,
            });
        });
        true
    }

    /// The projects whose package caches the sessions here use, and those
    /// of the sessions waiting to start.
    fn caches_in_use(&self, records: &caches::Records) -> HashSet<String> {
        let mut projects =
            records.projects_of(self.sessions.keys().map(|id| sandbox_name(id.as_str())));
        projects.extend(
            self.pending_starts
                .values()
                .filter(|spec| matches!(self.ctx.cache_of(spec), Ok(Some(_))))
                .map(|spec| spec.project.id.as_str().to_owned()),
        );
        projects
    }

    /// Records what a chore did with a project's package caches, and says
    /// so in the log.
    fn note_cache(&mut self, project: &str, why: caches::Why, done: CacheDone) {
        let path = caches::path(&self.ctx.cfg.data_dir());
        let mut records = caches::load(&path);
        let now = expiry::now_ms();
        let max = self.ctx.cfg.max_cache_mb().map(size::memory);
        match done {
            CacheDone::Measured(size_mb) => {
                tracing::info!(
                    "the package caches of {project} take {}",
                    size::memory(size_mb)
                );
                if let Some(r) = records.projects.get_mut(project) {
                    r.size_mb = Some(size_mb);
                    r.measured_at = Some(now);
                }
            }
            CacheDone::Cleared { gone, size_mb } => {
                let what = if gone { "removed" } else { "emptied" };
                match (why, size_mb, max) {
                    (caches::Why::Grown, Some(size), Some(max)) => tracing::info!(
                        "{what} the package caches of {project}: they took {}, more than the {max} a project's may (max_cache_gb)",
                        size::memory(size)
                    ),
                    (caches::Why::LowDisk, ..) => tracing::info!(
                        "{what} the package caches of {project} to make room on the disk ({})",
                        self.below()
                    ),
                    (_, _, None) => tracing::info!(
                        "{what} the package caches of {project}: this host keeps none (max_cache_gb = 0)"
                    ),
                    _ => tracing::info!(
                        "{what} the package caches of {project}: no session has started with them for {} days (keep_caches_days)",
                        self.ctx.cfg.keep_caches_days
                    ),
                }
                if gone {
                    records.projects.remove(project);
                } else if let Some(r) = records.projects.get_mut(project) {
                    r.size_mb = Some(0);
                    r.measured_at = Some(now);
                }
            }
        }
        if let Err(e) = caches::save(&path, &records) {
            tracing::warn!("saving {}: {e:#}", path.display());
        }
    }

    /// Ends a chore on a project's package caches, and starts the sessions
    /// that waited for it.
    fn release_cache(&mut self, project: &str) {
        self.clearing.remove(project);
        let waiting: Vec<SessionId> = self
            .pending_starts
            .iter()
            .filter(|(id, spec)| {
                spec.project.id.as_str() == project
                    && !self.sessions.contains_key(*id)
                    && !self.busy.contains_key(&sandbox_name(id.as_str()))
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in waiting {
            if let Some(spec) = self.pending_starts.remove(&id) {
                tracing::info!("starting {id}: {}", spec.title);
                self.spawn_session(spec);
            }
        }
    }

    /// Whether a session's project's package caches are being measured or
    /// emptied, which it waits for.
    fn waits_for_caches(&self, spec: &SessionSpec) -> bool {
        matches!(self.ctx.cache_of(spec), Ok(Some(_)))
            && self.clearing.contains(spec.project.id.as_str())
    }

    fn on_chore(&mut self, chore: Chore) {
        match chore {
            Chore::Listed(Ok(list), caches) => self.sweep_listed(list, caches),
            Chore::Listed(Err(e), _) => {
                tracing::warn!("listing sandboxes: {e:#}");
                self.sweeping = false;
            }
            Chore::Cache {
                project,
                why,
                result,
                free_mb,
            } => {
                match result {
                    Ok(done) => self.note_cache(&project, why, done),
                    Err(e) => tracing::warn!(
                        "{} the package caches of {project}: {e:#}",
                        match why {
                            caches::Why::Grown => "measuring",
                            caches::Why::Unused => "removing",
                            caches::Why::LowDisk => "emptying",
                        }
                    ),
                }
                self.note_disk(free_mb);
                self.release_cache(&project);
                self.remove_next();
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
            && self
                .pending_starts
                .get(&id)
                .is_some_and(|spec| !self.waits_for_caches(spec))
            && let Some(spec) = self.pending_starts.remove(&id)
        {
            tracing::info!("starting {id}: {}", spec.title);
            self.spawn_session(spec);
        }
    }

    fn spawn_session(&mut self, spec: Box<SessionSpec>) {
        let id = spec.session_id.clone();
        // Its sandbox uses the project's package caches from now on, also
        // after a restart, while the host knows no spec for it.
        if let Ok(Some(_)) = self.ctx.cache_of(&spec) {
            let path = caches::path(&self.ctx.cfg.data_dir());
            let (sandbox, project) = (sandbox_name(id.as_str()), spec.project.id.as_str());
            if let Err(e) = caches::note_used(&path, &sandbox, project, expiry::now_ms()) {
                tracing::warn!("saving {}: {e:#}", path.display());
            }
        }
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
            // Its agents, of its own version, run review agents' sessions.
            features: vec![fctl_proto::runner::FEATURE_REVIEWS.to_owned()],
            claude_token: Some(claude_token::announce(&self.ctx.cfg.claude_token())),
        }
    }

    fn heartbeat(&self) -> HostHeartbeat {
        HostHeartbeat {
            sessions: self.reports(),
            load1: sysinfo::load1(),
            memory_available_mb: sysinfo::memory_available_mb(),
            disk_available_mb: self.disk_free_mb,
            // Its owner may have saved or removed it since.
            claude_token: Some(claude_token::announce(&self.ctx.cfg.claude_token())),
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
                mut claude,
                github,
                github_read,
                mut error,
            } => {
                // A token kept on this runner comes without it: the host
                // puts its own in, or says why it can't.
                if let Some(c) = claude.as_mut()
                    && let Err(why) =
                        claude_token::fill(&self.ctx.cfg.claude_token(), c, &self.ctx.cfg.name)
                {
                    claude = None;
                    error = Some(why);
                }
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
    fn start_session(&mut self, mut spec: Box<SessionSpec>) -> Result<(), String> {
        let id = spec.session_id.clone();
        // Its person keeps their Claude token on their runners: this one's.
        claude_token::fill(
            &self.ctx.cfg.claude_token(),
            &mut spec.credentials.claude,
            &self.ctx.cfg.name,
        )?;
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
        if self.waits_for_caches(&spec) {
            tracing::info!("{id} starts once its project's package caches are measured or emptied");
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
        daemon_where(stopped, |_| {}).await
    }

    /// [`daemon_with`], with `host.toml` as `configure` leaves it.
    async fn daemon_where(
        stopped: &[(&str, i64)],
        configure: impl FnOnce(&mut HostConfig),
    ) -> Result<(tempfile::TempDir, Daemon)> {
        let dir = tempfile::tempdir()?;
        let path = expiry::path(dir.path());
        for (name, minutes) in stopped {
            std::fs::create_dir_all(dir.path().join("sandboxes").join(name))?;
            expiry::note_stopped(&path, name, expiry::now_ms() - minutes * MINUTE)?;
        }
        let mut cfg = HostConfig {
            driver: "process".into(),
            data_dir: Some(dir.path().to_owned()),
            ..HostConfig::default()
        };
        configure(&mut cfg);
        Ok((dir, Daemon::new(cfg, "fctl_rt_test".into()).await?))
    }

    /// Gives `project` package caches in the data directory, with a file,
    /// that a session last started with `minutes` ago, and that the host
    /// measured since when `measured`.
    fn cache(dir: &std::path::Path, project: &str, minutes: i64, measured: bool) -> Result<()> {
        let files = caches::dir(dir, project).join("npm/_cacache");
        std::fs::create_dir_all(&files)?;
        std::fs::write(files.join("tarball"), vec![7u8; 64 * 1024])?;
        let path = caches::path(dir);
        let mut records = caches::load(&path);
        let used_at = expiry::now_ms() - minutes * MINUTE;
        records.projects.insert(
            project.into(),
            caches::Record {
                used_at,
                size_mb: measured.then_some(1),
                measured_at: measured.then_some(used_at + 1),
            },
        );
        caches::save(&path, &records)
    }

    /// Notes that `sandbox` was made with `project`'s caches.
    fn mounts(dir: &std::path::Path, sandbox: &str, project: &str) -> Result<()> {
        let path = caches::path(dir);
        let mut records = caches::load(&path);
        records.sandboxes.insert(sandbox.into(), project.into());
        caches::save(&path, &records)
    }

    /// A session of `project`, which shares its package caches.
    fn cached_spec(session_id: &str, project: &str) -> Result<Box<SessionSpec>> {
        let mut spec = spec(session_id)?;
        spec.project.id = fctl_proto::ProjectId(project.into());
        spec.project.settings.package_caches = true;
        Ok(spec)
    }

    /// A session that runs here, as far as the sweep can tell.
    fn running(d: &mut Daemon, session_id: &str) {
        let (tx, _rx) = mpsc::unbounded_channel();
        let report = Arc::new(std::sync::Mutex::new(session::Report {
            status: SessionStatus::Running,
            agent_seq: 0,
        }));
        d.sessions
            .insert(SessionId(session_id.into()), SessionHandle { tx, report });
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
            match &mut chore {
                Chore::Removed { name, free_mb, .. } => {
                    removed.push(name.clone());
                    *free_mb = Some(free_after(removed.len()));
                }
                Chore::Cache {
                    project,
                    result: Ok(CacheDone::Cleared { .. }),
                    free_mb,
                    ..
                } => {
                    removed.push(format!("caches of {project}"));
                    *free_mb = Some(free_after(removed.len()));
                }
                Chore::Cache { free_mb, .. } => *free_mb = d.disk_free_mb,
                _ => {}
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

    const CLAUDE_TOKEN: &str = "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJ-Xy3Q";

    /// A spec whose Claude connection keeps its token on its owner's
    /// runners: the control plane sends it without one.
    fn spec_on_runners(session_id: &str) -> Result<Box<SessionSpec>> {
        let mut spec = spec(session_id)?;
        spec.credentials.claude = serde_json::from_value(serde_json::json!({
            "id": "crd_kept", "kind": "setup_token", "access_token": "",
            "scopes": ["user:inference"], "on_runners": true
        }))?;
        Ok(spec)
    }

    #[tokio::test]
    async fn sessions_on_a_token_kept_here_get_it_and_others_keep_theirs() -> Result<()> {
        let tokens = tempfile::tempdir()?;
        let file = tokens.path().join("claude-token");
        let kept = file.clone();
        let (_dir, mut d) = daemon_where(&[], move |cfg| {
            cfg.name = "studio-mac".into();
            cfg.claude_token_file = Some(kept);
        })
        .await?;
        // It says it can keep one, and that it keeps none.
        assert_eq!(d.hello().claude_token, Some(Default::default()));
        let refused = d
            .start_session(spec_on_runners("ses_01KEPT")?)
            .err()
            .unwrap_or_default();
        assert_eq!(
            refused,
            "studio-mac keeps no Claude token: run `fctl host claude set-token` on it"
        );
        // Saved while it runs: its next heartbeat says so, and sessions get it.
        claude_token::save(&file, CLAUDE_TOKEN)?;
        assert!(d.heartbeat().claude_token.is_some_and(|t| t.kept()));
        assert!(d.hello().claude_token.is_some_and(|t| t.kept()));
        d.hold("fctl-ses-01kept");
        d.hold("fctl-ses-01else");
        assert_eq!(d.start_session(spec_on_runners("ses_01KEPT")?), Ok(()));
        assert_eq!(d.start_session(spec("ses_01ELSE")?), Ok(()));
        let token_of = |d: &Daemon, id: &str| {
            d.pending_starts
                .get(&SessionId(id.into()))
                .map(|s| s.credentials.claude.access_token.clone())
        };
        assert_eq!(token_of(&d, "ses_01KEPT").as_deref(), Some(CLAUDE_TOKEN));
        // A session on a token the control plane keeps runs on that one.
        assert_eq!(token_of(&d, "ses_01ELSE").as_deref(), Some("a"));
        Ok(())
    }

    #[tokio::test]
    async fn fresh_credentials_for_a_token_kept_here_come_from_here() -> Result<()> {
        let tokens = tempfile::tempdir()?;
        let file = tokens.path().join("claude-token");
        let kept = file.clone();
        let (_dir, mut d) = daemon_where(&[], move |cfg| cfg.claude_token_file = Some(kept)).await?;
        let sid = SessionId("ses_01KEPT".into());
        let (tx, mut rx) = mpsc::unbounded_channel();
        d.sessions.insert(
            sid.clone(),
            SessionHandle {
                tx,
                report: Arc::new(std::sync::Mutex::new(session::Report {
                    status: SessionStatus::Running,
                    agent_seq: 0,
                })),
            },
        );
        let placeholder = spec_on_runners("ses_01KEPT")?.credentials.claude;
        let mut answer = |d: &mut Daemon, request: &str| {
            d.pending_creds
                .insert(request.into(), (sid.clone(), format!("agent-{request}")));
            d.on_fleet(FleetToHost::Credential {
                request_id: request.into(),
                claude: Some(placeholder.clone()),
                github: None,
                github_read: None,
                error: None,
            });
            rx.try_recv().ok()
        };
        // Replaced on the runner since the session started: it gets the new one.
        claude_token::save(&file, CLAUDE_TOKEN)?;
        let Some(SessionCmd::Credentials { claude, error, .. }) = answer(&mut d, "h1") else {
            anyhow::bail!("no credentials for the agent");
        };
        assert_eq!(claude.map(|c| c.access_token).as_deref(), Some(CLAUDE_TOKEN));
        assert_eq!(error, None);
        // Removed: the agent hears why, and gets no empty token.
        claude_token::remove(&file)?;
        let Some(SessionCmd::Credentials { claude, error, .. }) = answer(&mut d, "h2") else {
            anyhow::bail!("no answer for the agent");
        };
        assert_eq!(claude, None);
        assert!(error.is_some_and(|e| e.contains("keeps no Claude token")));
        Ok(())
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

    #[tokio::test]
    async fn a_low_disk_empties_package_caches_before_it_removes_stopped_sandboxes() -> Result<()> {
        let (dir, mut d) = daemon_with(&[("fctl-ses-01old", 30), ("fctl-ses-01new", 1)]).await?;
        cache(dir.path(), "prj_recent", 10, true)?;
        cache(dir.path(), "prj_least", 60, true)?;
        d.on_chore(Chore::Disk(Some(17 * GIB)));
        let removed = finish_sweep(&mut d, |n| [18, 19, 21][n - 1] * GIB).await?;
        // The caches used least recently first, then the sandbox stopped
        // longest; the newest sandbox stays, for a resume.
        assert_eq!(
            removed,
            [
                "caches of prj_least",
                "caches of prj_recent",
                "fctl-ses-01old"
            ]
        );
        assert!(!caches::dir(dir.path(), "prj_least").exists());
        assert!(caches::load(&caches::path(dir.path())).projects.is_empty());
        assert!(dir.path().join("sandboxes/fctl-ses-01new").is_dir());
        Ok(())
    }

    #[tokio::test]
    async fn caches_in_use_stay_and_sessions_wait_while_theirs_are_emptied() -> Result<()> {
        // On a low disk, only sessions whose sandboxes are here resume.
        let (dir, mut d) = daemon_with(&[("fctl-ses-01wait", 5)]).await?;
        cache(dir.path(), "prj_busy", 60, true)?;
        cache(dir.path(), "prj_idle", 10, true)?;
        mounts(dir.path(), "fctl-ses-01run", "prj_busy")?;
        running(&mut d, "ses_01RUN");
        d.on_chore(Chore::Disk(Some(17 * GIB)));
        // The sweep lists the caches here, and empties the idle ones…
        let listed = tokio::time::timeout(Duration::from_secs(10), d.chores_rx.recv())
            .await?
            .context("no more chores")?;
        d.on_chore(listed);
        assert!(d.clearing.contains("prj_idle"));
        // …while a session of their project waits.
        let waiting = cached_spec("ses_01WAIT", "prj_idle")?;
        let sandbox = sandbox_name(waiting.session_id.as_str());
        d.hold(&sandbox);
        assert_eq!(d.start_session(waiting.clone()), Ok(()));
        assert!(d.waits_for_caches(&waiting));
        let removed = finish_sweep(&mut d, |_| 18 * GIB).await?;
        assert_eq!(removed, ["caches of prj_idle"]);
        assert!(
            caches::dir(dir.path(), "prj_busy")
                .join("npm/_cacache/tarball")
                .exists()
        );
        // It starts once its sandbox is free too.
        assert!(!d.waits_for_caches(&waiting));
        assert!(d.pending_starts.contains_key(&waiting.session_id));
        Ok(())
    }

    #[tokio::test]
    async fn unused_caches_go_whatever_the_disk_has_unless_a_sandbox_mounts_them() -> Result<()> {
        let day = 24 * 60;
        let (dir, mut d) = daemon_with(&[("fctl-ses-01stop", 60)]).await?;
        cache(dir.path(), "prj_old", 15 * day, true)?;
        cache(dir.path(), "prj_kept", 15 * day, true)?;
        cache(dir.path(), "prj_recent", day, true)?;
        mounts(dir.path(), "fctl-ses-01stop", "prj_kept")?;
        // The hourly sweep.
        d.start_sweep();
        let removed = finish_sweep(&mut d, |_| 500 * GIB).await?;
        assert_eq!(removed, ["caches of prj_kept", "caches of prj_old"]);
        assert!(!caches::dir(dir.path(), "prj_old").exists());
        // The stopped sandbox mounts its directory as it starts.
        let kept = caches::dir(dir.path(), "prj_kept");
        assert!(kept.is_dir() && std::fs::read_dir(&kept)?.next().is_none());
        let records = caches::load(&caches::path(dir.path()));
        assert_eq!(
            records
                .projects
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["prj_kept", "prj_recent"]
        );
        assert!(records.projects["prj_kept"].empty());
        // Emptied, it waits for its sandbox to go; then it goes too.
        d.start_sweep();
        assert!(finish_sweep(&mut d, |_| 500 * GIB).await?.is_empty());
        std::fs::remove_dir_all(dir.path().join("sandboxes/fctl-ses-01stop"))?;
        d.start_sweep();
        assert_eq!(
            finish_sweep(&mut d, |_| 500 * GIB).await?,
            ["caches of prj_kept"]
        );
        assert!(!kept.exists());
        Ok(())
    }

    #[tokio::test]
    async fn a_host_that_keeps_no_caches_removes_those_it_kept() -> Result<()> {
        let (dir, mut d) = daemon_where(&[], |cfg| cfg.max_cache_gb = 0).await?;
        cache(dir.path(), "prj_1", 1, true)?;
        let spec = cached_spec("ses_01NEW", "prj_1")?;
        assert_eq!(
            d.ctx.cache_of(&spec),
            Err(format!(
                "{} keeps no package caches (max_cache_gb = 0 in its host.toml), so this session's package managers download what they need",
                d.ctx.cfg.name
            ))
        );
        d.start_sweep();
        assert_eq!(
            finish_sweep(&mut d, |_| 500 * GIB).await?,
            ["caches of prj_1"]
        );
        assert!(!caches::dir(dir.path(), "prj_1").exists());
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn caches_that_grew_past_the_limit_are_emptied() -> Result<()> {
        let (dir, mut d) = daemon_where(&[], |cfg| cfg.max_cache_gb = 1).await?;
        cache(dir.path(), "prj_big", 10, false)?;
        cache(dir.path(), "prj_small", 10, false)?;
        cache(dir.path(), "prj_measured", 10, true)?;
        // 1.1 GiB at once, without writing it.
        let big = caches::dir(dir.path(), "prj_big").join("uv/torch.whl");
        std::fs::create_dir_all(big.parent().context("no parent")?)?;
        let allocated = std::process::Command::new("fallocate")
            .args(["-l", "1100M"])
            .arg(&big)
            .status();
        if !allocated.is_ok_and(|s| s.success()) {
            eprintln!("skipped: fallocate can't allocate here");
            return Ok(());
        }
        d.start_sweep();
        let removed = finish_sweep(&mut d, |_| 500 * GIB).await?;
        assert_eq!(removed, ["caches of prj_big"]);
        let records = caches::load(&caches::path(dir.path()));
        assert!(!records.projects.contains_key("prj_big"));
        // Measured, within the limit; one measured since its last use isn't
        // measured again.
        let small = records.projects["prj_small"];
        assert_eq!(small.size_mb, Some(1));
        assert!(!small.grown());
        assert_eq!(records.projects["prj_measured"].size_mb, Some(1));
        Ok(())
    }
}
