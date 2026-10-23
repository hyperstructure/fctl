//! Container drivers that shell out to a CLI: Apple `container` (one
//! lightweight VM per container) and the `docker` CLI (Docker, OrbStack,
//! Colima, Podman).
//!
//! Both get the same treatment: the sandbox is a long-lived container whose
//! main process just waits (`tail -f /dev/null`); fctl-agent is copied in and
//! started with a detached `exec`; the host talks to it through `exec -i
//! fctl-agent attach`.

use super::{
    AGENT_PATH, AgentIo, BuildJob, COMMAND_TIMEOUT, DiskSpot, NotStarted, SLOW_COMMAND_TIMEOUT,
    Sandbox, SandboxInfo, SandboxSpec, TimedOut, attach_script, run_cmd_within, serve_script,
    timed_out, unanswered,
};
use crate::caches::{self, Spot};
use anyhow::{Context, Result};
use fctl_proto::runner::{DriverInfo, SandboxSize};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Apple,
    Docker,
}

pub struct ContainerDriver {
    flavor: Flavor,
    bin: String,
}

/// An image on this host, as its runtime describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageInfo {
    /// Docker's image ID, or with Apple's `container` the digest of the
    /// image's index: a pull that brings a newer build changes it. Empty
    /// when the runtime's description can't be read.
    pub id: String,
    /// Its labels (with Apple's `container`, those of each platform).
    pub labels: BTreeMap<String, String>,
}

/// The network Docker sandboxes join: a bridge whose containers reach the
/// internet but not each other (`enable_icc=false`). On Docker's default
/// network a session reaches what other sessions on the host serve, which
/// may be another workspace's.
pub const SANDBOX_NETWORK: &str = "fctl-sandboxes";
/// Whether a network lets its containers reach each other, as `docker
/// network inspect --format` prints it.
const ICC_FORMAT: &str = r#"{{index .Options "com.docker.network.bridge.enable_icc"}}"#;

/// Why a sandbox wasn't created.
enum Create {
    /// Its image isn't on this host and pulling it failed.
    NoImage(anyhow::Error),
    Failed(anyhow::Error),
}

impl ContainerDriver {
    pub fn apple() -> Self {
        Self {
            flavor: Flavor::Apple,
            bin: std::env::var("FCTL_CONTAINER_BIN").unwrap_or_else(|_| "container".into()),
        }
    }

    pub fn docker() -> Self {
        Self {
            flavor: Flavor::Docker,
            bin: std::env::var("FCTL_DOCKER_BIN").unwrap_or_else(|_| "docker".into()),
        }
    }

    /// A driver whose CLI is `bin`, a stand-in.
    #[cfg(test)]
    pub(crate) fn with_bin(flavor: Flavor, bin: String) -> Self {
        Self { flavor, bin }
    }

    pub fn flavor_name(&self) -> &'static str {
        match self.flavor {
            Flavor::Apple => "apple",
            Flavor::Docker => "docker",
        }
    }

    async fn run(&self, args: &[&str]) -> Result<String> {
        // Pulling an image takes minutes, and so may `run`, which pulls one
        // that is missing.
        let limit = match args {
            ["pull", ..] | ["image", "pull", ..] | ["run", ..] => SLOW_COMMAND_TIMEOUT,
            _ => COMMAND_TIMEOUT,
        };
        run_cmd_within(&self.bin, args, limit).await
    }

    pub async fn probe(&self) -> DriverInfo {
        let isolation = match self.flavor {
            Flavor::Apple => "vm",
            Flavor::Docker => "container",
        };
        // Elsewhere only a stand-in (`FCTL_CONTAINER_BIN`, tests) can play Apple's CLI.
        if cfg!(not(target_os = "macos")) && self.flavor == Flavor::Apple && self.bin == "container"
        {
            return DriverInfo {
                name: "apple".into(),
                available: false,
                isolation: isolation.into(),
                version: None,
                detail: Some("Apple container needs macOS 26 on Apple silicon".into()),
            };
        }
        // Not installed and not running need different fixes.
        if !installed(&self.bin) {
            return DriverInfo {
                name: self.flavor_name().into(),
                available: false,
                isolation: isolation.into(),
                version: None,
                detail: Some(match self.flavor {
                    Flavor::Apple => "Apple's `container` is not installed (get it from https://github.com/apple/container/releases, then run `container system start`)".into(),
                    Flavor::Docker => "the docker CLI is not installed".into(),
                }),
            };
        }
        // The service must be up, not just the CLI installed.
        let (version, ready, hung) = match self.flavor {
            Flavor::Apple => {
                let version = self.run(&["system", "version", "--format", "json"]).await;
                // A service that doesn't answer one command won't answer the next.
                let (ready, hung) = if timed_out(&version) {
                    (false, true)
                } else {
                    let status = self.run(&["system", "status"]).await;
                    (status.is_ok(), timed_out(&status))
                };
                let version = version.ok().and_then(|s| {
                    let v: Value = serde_json::from_str(&s).ok()?;
                    v.get("version")
                        .or_else(|| v.pointer("/0/version"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(Some(s.trim().to_owned()))
                });
                (version, ready, hung)
            }
            Flavor::Docker => {
                let version = self
                    .run(&["version", "--format", "{{.Server.Version}}"])
                    .await;
                let hung = timed_out(&version);
                let version = version.ok().map(|s| s.trim().to_owned());
                (version.clone(), version.is_some(), hung)
            }
        };
        DriverInfo {
            name: self.flavor_name().into(),
            available: ready,
            isolation: isolation.into(),
            detail: Some(match (self.flavor, ready, hung) {
                (_, true, _) => "ready".into(),
                (Flavor::Apple, false, true) => "the `container` service is not answering".into(),
                (Flavor::Apple, false, false) => {
                    "the `container` service is not running (run `container system start`)".into()
                }
                (Flavor::Docker, false, true) => "the docker daemon is not answering".into(),
                (Flavor::Docker, false, false) => "the docker daemon is not running".into(),
            }),
            version,
        }
    }

    /// The Docker daemon's CPUs and memory: on macOS, its VM's rather than
    /// the Mac's. `None` for Apple's `container`, whose sandboxes are VMs of
    /// their own.
    pub async fn resources(&self) -> Option<SandboxSize> {
        if self.flavor != Flavor::Docker {
            return None;
        }
        let out = self
            .run(&["info", "--format", "{{.NCPU}} {{.MemTotal}}"])
            .await
            .ok()?;
        docker_resources(&out)
    }

    /// Where the runtime keeps its sandboxes' disks. Apple's `container`
    /// keeps each one's disk image in its app root (or, when that can't be
    /// found, `data_dir`'s disk is the Mac's), Docker its containers in its
    /// root directory, which on macOS is inside its VM. `None` when the
    /// Docker daemon doesn't answer.
    pub async fn disk_spot(&self, data_dir: &Path) -> Option<DiskSpot> {
        match self.flavor {
            Flavor::Apple => {
                let status = self.run(&["system", "status", "--format", "json"]).await;
                let root = status
                    .ok()
                    .as_deref()
                    .and_then(apple_app_root)
                    .or_else(|| std::env::var_os("CONTAINER_APP_ROOT").map(PathBuf::from))
                    .or_else(|| {
                        std::env::var_os("HOME").map(|h| {
                            PathBuf::from(h).join("Library/Application Support/com.apple.container")
                        })
                    })
                    .filter(|root| root.is_dir());
                Some(DiskSpot::Dir(root.unwrap_or_else(|| data_dir.to_owned())))
            }
            Flavor::Docker => {
                let out = self
                    .run(&["info", "--format", "{{.DockerRootDir}}"])
                    .await
                    .ok()?;
                let root = PathBuf::from(out.trim());
                // On macOS the path is the VM's; on Linux the daemon may run
                // elsewhere too (a VM, DOCKER_HOST), and then it isn't here.
                Some(
                    if cfg!(target_os = "linux") && root.is_absolute() && root.is_dir() {
                        DiskSpot::Dir(root)
                    } else {
                        DiskSpot::DockerVm
                    },
                )
            }
        }
    }

    /// Free MiB where the Docker daemon keeps containers, from `df` in a
    /// short-lived container of the first of `images` that is here: its root
    /// is an overlay on that disk. `None` when none can run.
    pub async fn vm_disk_free_mb(&self, images: &[String]) -> Option<u64> {
        for image in images {
            let args = [
                "run",
                "--rm",
                "--pull",
                "never",
                "--network",
                "none",
                "--entrypoint",
                "df",
                image,
                "-Pk",
                "/",
            ];
            match run_cmd_within(&self.bin, &args, COMMAND_TIMEOUT).await {
                Ok(out) => return crate::sysinfo::df_available_mb(&out),
                Err(e) if e.is::<TimedOut>() => return None,
                Err(_) => {}
            }
        }
        None
    }

    /// `running`, `stopped`, … or `None` when it does not exist. An error
    /// only when the runtime doesn't answer.
    async fn state(&self, name: &str) -> Result<Option<String>> {
        let inspect: &[&str] = match self.flavor {
            Flavor::Apple => &["inspect", name],
            Flavor::Docker => &["inspect", "--format", "{{.State.Status}}", name],
        };
        let out = match self.run(inspect).await {
            Ok(out) => out,
            Err(e) if unanswered(&e) => return Err(e),
            Err(_) => return Ok(None),
        };
        Ok(match self.flavor {
            Flavor::Apple => serde_json::from_str::<Value>(&out).ok().and_then(|v| {
                v.pointer("/0/status/state")
                    .or_else(|| v.pointer("/0/status"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }),
            Flavor::Docker => Some(out.trim().to_owned()),
        })
    }

    pub async fn ensure(
        &self,
        spec: &SandboxSpec,
        log: &(dyn Fn(String) + Send + Sync),
    ) -> Result<Sandbox> {
        let sb = Sandbox {
            name: spec.name.clone(),
            root: None,
        };
        match self.state(&spec.name).await?.as_deref() {
            Some("running") => {
                // Apple's `container` lists a sandbox it failed to stop as
                // running, and refuses commands in it (see `halt`). A
                // runtime that doesn't answer says nothing about that.
                let e = match self.run(&["exec", &spec.name, "true"]).await {
                    Ok(_) => return Ok(sb),
                    Err(e) if unanswered(&e) => return Err(e),
                    Err(e) => e,
                };
                tracing::warn!(
                    "{} is listed as running but takes no commands, so the host starts it again: {e:#}",
                    spec.name
                );
                log(format!(
                    "Starting the sandbox {} again: it is listed as running but takes no commands",
                    spec.name
                ));
                self.halt(&spec.name).await?;
                self.run(&["start", &spec.name]).await?;
                return Ok(sb);
            }
            Some(_) => {
                log(format!("Starting the existing sandbox {}", spec.name));
                self.run(&["start", &spec.name]).await?;
                return Ok(sb);
            }
            None => {}
        }
        log(format!(
            "Creating sandbox {} from {}",
            spec.name, spec.image
        ));
        let pull_error = match self.create(spec, &spec.image, log).await {
            Ok(()) => return Ok(sb),
            Err(Create::Failed(e)) => return Err(e),
            Err(Create::NoImage(e)) => e,
        };
        let missing = format!("{} is not on this host and can't be pulled", spec.image);
        if let Some(fallback) = &spec.fallback_image {
            log(format!("{missing}; starting from {fallback} instead"));
            match self.create(spec, fallback, log).await {
                Ok(()) => return Ok(sb),
                Err(Create::Failed(e)) => return Err(e),
                Err(Create::NoImage(_)) => {}
            }
            return Err(pull_error.context(format!(
                "{missing} (`fctl host image build` builds it here)"
            )));
        }
        Err(pull_error.context(missing))
    }

    /// Starts the sandbox from `image`, pulling the image when it isn't here.
    async fn create(
        &self,
        spec: &SandboxSpec,
        image: &str,
        log: &(dyn Fn(String) + Send + Sync),
    ) -> Result<(), Create> {
        self.start_new(spec, image, log).await?;
        if let Some(Spot::Volume(_)) = &spec.cache {
            self.give_cache_to_user(&spec.name).await;
        }
        Ok(())
    }

    async fn start_new(
        &self,
        spec: &SandboxSpec,
        image: &str,
        log: &(dyn Fn(String) + Send + Sync),
    ) -> Result<(), Create> {
        let cpus = spec.cpus.max(1).to_string();
        let memory = match self.flavor {
            Flavor::Apple => format!("{}M", spec.memory_mb.max(512)),
            Flavor::Docker => format!("{}m", spec.memory_mb.max(512)),
        };
        let label = "sh.fctl.sandbox=1".to_owned();
        let mut args: Vec<&str> = vec![
            "run", "-d", "--name", &spec.name, "--label", &label, "--cpus", &cpus, "--memory",
            &memory, "--init",
        ];
        if let Some(network) = self.sandbox_network().await {
            args.extend(["--network", network]);
        }
        let cache = spec.cache.as_ref().map(cache_mount);
        if let Some(mount) = &cache {
            args.extend(["--mount", mount.as_str()]);
        }
        args.extend(["--entrypoint", "tail", image, "-f", "/dev/null"]);
        let failed =
            |e: anyhow::Error| Create::Failed(e.context(format!("starting {}", spec.name)));
        let Err(e) = self.run(&args).await else {
            return Ok(());
        };
        // A runtime that doesn't answer isn't missing the image.
        if unanswered(&e) || self.has_image(image).await {
            return Err(failed(e));
        }
        log(format!("Pulling {image}"));
        self.pull(image, false).await.map_err(Create::NoImage)?;
        self.run(&args).await.map(drop).map_err(failed)
    }

    /// The network a new Docker sandbox joins ([`SANDBOX_NETWORK`]), made
    /// the first time. `None` with Apple's `container`, and with a runtime
    /// that can't make one, which leaves sandboxes on its default network:
    /// the host says so in its log.
    async fn sandbox_network(&self) -> Option<&'static str> {
        if self.flavor != Flavor::Docker {
            return None;
        }
        let apart = || async {
            self.run(&[
                "network",
                "inspect",
                "--format",
                ICC_FORMAT,
                SANDBOX_NETWORK,
            ])
            .await
            .map(|out| out.trim() == "false")
        };
        match apart().await {
            Ok(true) => return Some(SANDBOX_NETWORK),
            Ok(false) => {
                tracing::warn!(
                    "the {SANDBOX_NETWORK} network lets its containers reach each other: new sandboxes start on Docker's default network, where they reach each other too. Remove it (`docker network rm {SANDBOX_NETWORK}`) and fctl makes it again."
                );
                return None;
            }
            Err(_) => {}
        }
        let made = self
            .run(&[
                "network",
                "create",
                "--driver",
                "bridge",
                "--opt",
                "com.docker.network.bridge.enable_icc=false",
                SANDBOX_NETWORK,
            ])
            .await;
        match made {
            Ok(_) => Some(SANDBOX_NETWORK),
            // Made meanwhile, for another sandbox.
            Err(_) if matches!(apart().await, Ok(true)) => Some(SANDBOX_NETWORK),
            Err(e) => {
                tracing::warn!(
                    "sandboxes start on Docker's default network, where they reach each other: making the {SANDBOX_NETWORK} network failed: {e:#}"
                );
                None
            }
        }
    }

    /// Gives the user of the new Docker sandbox `name` the package caches
    /// it mounts: Docker makes a new volume's root root's. Caches that
    /// another image's user made are given over whole. Where this fails,
    /// fctl-agent finds it can't write to them, and the session's package
    /// managers keep their own.
    async fn give_cache_to_user(&self, name: &str) {
        let ids = self
            .run(&["exec", name, "sh", "-c", r#"echo "$(id -u):$(id -g)""#])
            .await;
        let owner = match ids {
            Ok(out) => out.trim().to_owned(),
            Err(e) => {
                tracing::warn!("{name} may not write to its package caches: {e:#}");
                return;
            }
        };
        let numeric = owner
            .split(':')
            .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !numeric || owner.split(':').count() != 2 || owner.starts_with("0:") {
            return;
        }
        let mount = caches::MOUNT;
        let script = format!(r#"[ "$(stat -c %u:%g {mount})" = "$1" ] || chown -R "$1" {mount}"#);
        let given = self
            .run(&["exec", "-u", "0", name, "sh", "-c", &script, "sh", &owner])
            .await;
        if let Err(e) = given {
            tracing::warn!("{name} may not write to its package caches: {e:#}");
        }
    }

    /// The projects whose package caches have a Docker volume here.
    pub async fn cache_volumes(&self) -> Result<Vec<String>> {
        let out = self.run(&["volume", "ls", "--format", "{{.Name}}"]).await?;
        let mut projects: Vec<String> = out
            .lines()
            .filter_map(|l| caches::project_of(l.trim()))
            .map(str::to_owned)
            .collect();
        projects.sort();
        Ok(projects)
    }

    /// What the Docker volume `volume` takes, in MiB: `du` in a
    /// short-lived container of the first of `images` that is here.
    pub async fn volume_size_mb(&self, volume: &str, images: &[String]) -> Result<u64> {
        let out = self
            .in_volume(volume, false, images, "du", &["-sk", "/cache"])
            .await?;
        let kib: u64 = out
            .split_whitespace()
            .next()
            .and_then(|k| k.parse().ok())
            .with_context(|| format!("du printed {:?}", out.trim()))?;
        Ok(kib.div_ceil(1024))
    }

    /// Removes the Docker volume `volume`, or only what it holds: with
    /// `keep`, and when a container mounts it, which Docker counts even
    /// when it is stopped. Whether the volume is gone.
    pub async fn clear_volume(&self, volume: &str, keep: bool, images: &[String]) -> Result<bool> {
        if !keep {
            match self.run(&["volume", "rm", volume]).await {
                Ok(_) => return Ok(true),
                Err(e) if unanswered(&e) => return Err(e),
                Err(_) => {}
            }
        }
        // Mounting a volume that isn't here would make it.
        match self.run(&["volume", "inspect", volume]).await {
            Ok(_) => {}
            Err(e) if unanswered(&e) => return Err(e),
            Err(_) => return Ok(true),
        }
        self.in_volume(
            volume,
            true,
            images,
            "find",
            &["/cache", "-mindepth", "1", "-delete"],
        )
        .await?;
        Ok(false)
    }

    /// Runs `program` as root in a short-lived container of the first of
    /// `images` that is here, with no network and `volume` at `/cache`
    /// (read-only unless it `writes`), and returns what it printed.
    async fn in_volume(
        &self,
        volume: &str,
        writes: bool,
        images: &[String],
        program: &str,
        args: &[&str],
    ) -> Result<String> {
        let ro = if writes { "" } else { ",readonly" };
        let mount = format!("type=volume,source={volume},target=/cache{ro}");
        let mut failed = None;
        for image in images {
            let mut run = vec![
                "run",
                "--rm",
                "--pull",
                "never",
                "--network",
                "none",
                "--user",
                "0",
                "--mount",
                &mount,
                "--entrypoint",
                program,
                image,
            ];
            run.extend(args);
            match self.run(&run).await {
                Ok(out) => return Ok(out),
                Err(e) if unanswered(&e) => return Err(e),
                Err(e) => failed = Some(e),
            }
        }
        Err(failed.unwrap_or_else(|| anyhow::anyhow!("no image is here to run {program} in")))
    }

    /// Whether the sandbox `name` exists. An error only when the runtime
    /// doesn't answer.
    pub async fn exists(&self, name: &str) -> Result<bool> {
        Ok(self.state(name).await?.is_some())
    }

    /// Builds `job` with `docker build` or `container build`, saying in
    /// `log` which step it is at. One still running after `job.limit` is
    /// stopped.
    pub async fn build(
        &self,
        job: &BuildJob<'_>,
        log: &(dyn Fn(String) + Send + Sync),
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "build".into(),
            "--tag".into(),
            job.tag.into(),
            "--file".into(),
            job.dockerfile.display().to_string(),
        ];
        for (k, v) in job.args {
            args.extend(["--build-arg".into(), format!("{k}={v}")]);
        }
        if let Some(target) = job.target {
            args.extend(["--target".into(), target.into()]);
        }
        args.push(job.context.display().to_string());
        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if self.flavor == Flavor::Docker {
            // Lines rather than a redrawn screen; other builders ignore it.
            cmd.env("BUILDKIT_PROGRESS", "plain");
        }
        let mut child = cmd.spawn().map_err(|e| super::spawn_error(&self.bin, e))?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        if let Some(out) = child.stdout.take() {
            forward_lines(out, tx.clone());
        }
        if let Some(err) = child.stderr.take() {
            forward_lines(err, tx.clone());
        }
        drop(tx);
        let mut tail: VecDeque<String> = VecDeque::new();
        let run = async {
            while let Some(line) = rx.recv().await {
                if build_step(&line) {
                    log(short(&line));
                }
                if tail.len() == 20 {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
            child.wait().await
        };
        let status = match tokio::time::timeout(job.limit, run).await {
            Ok(status) => status.with_context(|| format!("running {} build", self.bin))?,
            Err(_) => {
                let _ = child.kill().await;
                anyhow::bail!(
                    "The image build took longer than {} minutes, so the host stopped it (build_timeout_minutes in host.toml)",
                    job.limit.as_secs() / 60
                );
            }
        };
        if !status.success() {
            for line in &tail {
                log(line.clone());
            }
            let why = tail
                .iter()
                .rev()
                .find(|l| l.contains("ERROR") || l.contains("error"))
                .or(tail.back())
                .map(|l| format!(": {}", l.trim()))
                .unwrap_or_default();
            anyhow::bail!("Building the image failed ({status}){why}");
        }
        Ok(())
    }

    /// An image's size in bytes: Docker's, or with Apple's `container` the
    /// size of its variant for this machine.
    pub async fn image_size(&self, image: &str) -> Option<u64> {
        match self.flavor {
            Flavor::Docker => self
                .run(&["image", "inspect", "--format", "{{.Size}}", image])
                .await
                .ok()?
                .trim()
                .parse()
                .ok(),
            Flavor::Apple => {
                let out = self.run(&["image", "inspect", image]).await.ok()?;
                apple_image_size(&out, linux_arch(std::env::consts::ARCH))
            }
        }
    }

    pub async fn has_image(&self, image: &str) -> bool {
        let inspect: &[&str] = match self.flavor {
            Flavor::Apple => &["image", "inspect", image],
            Flavor::Docker => &["image", "inspect", "--format", "{{.Id}}", image],
        };
        self.run(inspect).await.is_ok()
    }

    /// Pulls `image` for the platform sandboxes run on: without one, Apple's
    /// `container` pulls every platform an image has.
    pub async fn pull(&self, image: &str, progress: bool) -> Result<()> {
        let platform = format!("linux/{}", linux_arch(std::env::consts::ARCH));
        let args: &[&str] = match self.flavor {
            Flavor::Apple => &["image", "pull", "--platform", &platform, image],
            Flavor::Docker => &["pull", image],
        };
        if !progress {
            return self.run(args).await.map(drop);
        }
        let status = tokio::process::Command::new(&self.bin)
            .args(args)
            .stdin(Stdio::null())
            .status()
            .await
            .map_err(|e| super::spawn_error(&self.bin, e))?;
        if !status.success() {
            anyhow::bail!("{} {} failed ({status})", self.bin, args.join(" "));
        }
        Ok(())
    }

    /// Pulls `image`, or `fallback` when `image` can't be pulled and isn't
    /// here either: what sandboxes start from then. Returns the image it
    /// pulled, and says in `log` when it falls back.
    pub async fn pull_or_fallback(
        &self,
        image: &str,
        fallback: Option<&str>,
        progress: bool,
        log: &(dyn Fn(String) + Send + Sync),
    ) -> Result<String> {
        let error = match self.pull(image, progress).await {
            Ok(()) => return Ok(image.to_owned()),
            Err(e) => e,
        };
        let Some(fallback) = fallback else {
            return Err(error);
        };
        // A CLI that couldn't start won't pull the fallback either.
        if error.is::<NotStarted>() || self.has_image(image).await {
            return Err(error);
        }
        log(format!(
            "{image} can't be pulled; pulling {fallback} instead"
        ));
        match self.pull(fallback, progress).await {
            Ok(()) => Ok(fallback.to_owned()),
            Err(_) => Err(error.context(format!(
                "{image} can't be pulled, and neither can {fallback} (`fctl host image build` builds it here)"
            ))),
        }
    }

    /// What the runtime says about `image`: `None` when it isn't here, and
    /// an empty [`ImageInfo`] when its description can't be read. An error
    /// only when the runtime doesn't answer.
    pub async fn image(&self, image: &str) -> Result<Option<ImageInfo>> {
        match self.run(&["image", "inspect", image]).await {
            Ok(out) => Ok(Some(image_info(&out).unwrap_or_default())),
            Err(e) if unanswered(&e) => Err(e),
            Err(_) => Ok(None),
        }
    }

    /// Gives `image` another name, `name`.
    pub async fn tag_image(&self, image: &str, name: &str) -> Result<()> {
        self.run(&["image", "tag", image, name]).await.map(drop)
    }

    /// Removes the name `image`, and the image once no other name refers to
    /// it, without force: Docker refuses while a container uses it, stopped
    /// ones included. Apple's `container` doesn't check, but each of its
    /// containers has a copy of the image's filesystem, so it still starts.
    pub async fn remove_image(&self, image: &str) -> Result<()> {
        let args: &[&str] = match self.flavor {
            Flavor::Apple => &["image", "delete", image],
            Flavor::Docker => &["image", "rm", image],
        };
        self.run(args).await.map(drop)
    }

    /// The names (`repository:tag`) of `repository`'s images on this host.
    pub async fn images(&self, repository: &str) -> Result<Vec<String>> {
        let out = match self.flavor {
            Flavor::Apple => self.run(&["image", "list", "--quiet"]).await?,
            Flavor::Docker => {
                let format = "{{.Repository}}:{{.Tag}}";
                self.run(&["image", "ls", "--format", format, repository])
                    .await?
            }
        };
        Ok(names_in(&out, repository))
    }

    pub async fn install_agent(&self, sb: &Sandbox, binary: &Path) -> Result<()> {
        let src = binary.display().to_string();
        let dest = format!("{}:{AGENT_PATH}", sb.name);
        if self.flavor == Flavor::Docker {
            let _ = self
                .run(&["exec", "-u", "0", &sb.name, "mkdir", "-p", "/opt/fctl/bin"])
                .await;
        }
        self.run(&["cp", &src, &dest])
            .await
            .context("copying fctl-agent into the sandbox")?;
        let _ = self
            .run(&["exec", "-u", "0", &sb.name, "chmod", "755", AGENT_PATH])
            .await;
        Ok(())
    }

    pub async fn start_agent(&self, sb: &Sandbox) -> Result<()> {
        let script = serve_script(AGENT_PATH);
        self.run(&["exec", "-d", &sb.name, "sh", "-c", &script])
            .await?;
        Ok(())
    }

    pub async fn attach(&self, sb: &Sandbox) -> Result<AgentIo> {
        let mut child = self.spawn(sb, &attach_script(AGENT_PATH)).await?;
        let stdout = child.stdout.take().context("exec stdout")?;
        let stdin = child.stdin.take().context("exec stdin")?;
        Ok(AgentIo {
            reader: Box::new(stdout),
            writer: Box::new(stdin),
            child: Some(child),
        })
    }

    pub async fn exec(&self, sb: &Sandbox, script: &str) -> Result<String> {
        self.run(&["exec", &sb.name, "sh", "-c", script]).await
    }

    pub async fn spawn(&self, sb: &Sandbox, script: &str) -> Result<tokio::process::Child> {
        tokio::process::Command::new(&self.bin)
            .args(["exec", "-i", &sb.name, "sh", "-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("running {} exec", self.bin))
    }

    /// Stops the sandbox, which keeps its files. With Apple's `container`
    /// it first gives back the disk space of the files deleted in it.
    pub async fn stop(&self, sb: &Sandbox) -> Result<()> {
        if self.state(&sb.name).await?.as_deref() == Some("running") {
            if self.flavor == Flavor::Apple {
                self.give_back_space(&sb.name).await;
            }
            self.halt(&sb.name).await?;
        }
        Ok(())
    }

    /// Stops the running sandbox `name`. Apple's `container` can fail to,
    /// and leave it half stopped: listed as running, but refusing `exec`,
    /// `cp` and `clean`, while `start` does nothing. It does when a program
    /// in the sandbox made cgroups of its own (Podman, say) that the VM's
    /// init fails to kill. A second `stop` finishes the job, so the host
    /// stops a sandbox again while it is still listed as running.
    async fn halt(&self, name: &str) -> Result<()> {
        let stop = ["stop", "-t", "20", name];
        let e = match self.run(&stop).await {
            Ok(_) => return Ok(()),
            Err(e) if unanswered(&e) => return Err(e),
            Err(e) => e,
        };
        if self.state(name).await?.as_deref() != Some("running") {
            return Ok(());
        }
        tracing::warn!("{name} did not stop, so the host stops it again: {e:#}");
        self.run(&stop).await.map(drop)
    }

    /// Gives the Mac back the space of the files deleted in the running
    /// sandbox `name`. Its root filesystem is a sparse disk image, which
    /// shrinks only where the guest discards blocks, and the guest mounts
    /// it without `discard`. The container can't trim it, having no
    /// `CAP_SYS_ADMIN`, but the VM's init can: `container clean`, from
    /// Apple's `container` 1.4. ext4 trims only the blocks whose release
    /// its journal has committed, so `sync` goes first. Without `clean`,
    /// the sandbox keeps the space until the host removes it.
    async fn give_back_space(&self, name: &str) {
        // A VM that doesn't answer one command won't answer the next.
        if timed_out(&self.run(&["exec", name, "sync"]).await) {
            return;
        }
        if let Err(e) = self.run(&["clean", name]).await {
            let e = format!("{e:#}");
            let why = e.lines().next().unwrap_or_default();
            tracing::info!("{name} keeps the disk space of the files deleted in it: {why}");
        }
    }

    pub async fn remove(&self, sb: &Sandbox) -> Result<()> {
        let args: &[&str] = match self.flavor {
            Flavor::Apple => &["delete", "--force", &sb.name],
            Flavor::Docker => &["rm", "-f", &sb.name],
        };
        if self.state(&sb.name).await?.is_some() {
            self.run(args).await?;
        }
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<SandboxInfo>> {
        match self.flavor {
            Flavor::Apple => {
                let out = self.run(&["list", "--all", "--format", "json"]).await?;
                let v: Value = serde_json::from_str(&out).unwrap_or(Value::Array(vec![]));
                Ok(v.as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|c| {
                                let name = c
                                    .get("id")
                                    .or_else(|| c.pointer("/configuration/id"))
                                    .and_then(Value::as_str)?;
                                let state = c
                                    .pointer("/status/state")
                                    .or_else(|| c.get("status"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown");
                                name.starts_with("fctl-").then(|| SandboxInfo {
                                    name: name.to_owned(),
                                    running: state == "running",
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default())
            }
            Flavor::Docker => {
                let out = self
                    .run(&[
                        "ps",
                        "-a",
                        "--filter",
                        "label=sh.fctl.sandbox=1",
                        "--format",
                        "{{.Names}}\t{{.State}}",
                    ])
                    .await?;
                Ok(out
                    .lines()
                    .filter_map(|l| {
                        let (name, state) = l.split_once('\t')?;
                        name.starts_with("fctl-").then(|| SandboxInfo {
                            name: name.to_owned(),
                            running: state == "running",
                        })
                    })
                    .collect())
            }
        }
    }
}

/// How a sandbox mounts its project's package caches: a Docker volume, or
/// with Apple's `container` a directory on the Mac.
fn cache_mount(spot: &Spot) -> String {
    match spot {
        Spot::Volume(name) => format!("type=volume,source={name},target={}", caches::MOUNT),
        Spot::Dir(dir) => format!(
            "type=bind,source={},target={}",
            dir.display(),
            caches::MOUNT
        ),
    }
}

/// Sends each line `r` gives to `tx`.
fn forward_lines<R: AsyncRead + Unpin + Send + 'static>(
    r: R,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(r).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
}

/// Whether a line of a build's output starts one of its steps, or says it
/// failed: BuildKit's `#5 [2/4] RUN …`, the older builder's `Step 2/4 :`,
/// Podman's `STEP 2/4:`.
fn build_step(line: &str) -> bool {
    let l = line.trim_start();
    let buildkit = l.strip_prefix('#').is_some_and(|rest| {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && rest[digits..].starts_with(" [")
    });
    buildkit || l.starts_with("Step ") || l.starts_with("STEP ") || l.starts_with("ERROR")
}

/// A line of a build's output, cut short when it is long.
fn short(line: &str) -> String {
    let line = line.trim_end();
    match line.char_indices().nth(240) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_owned(),
    }
}

/// The size of an image's variant for `arch` in `container image inspect`'s
/// JSON, or of its largest one.
fn apple_image_size(json: &str, arch: &str) -> Option<u64> {
    let v: Value = serde_json::from_str(json).ok()?;
    let image = v.get(0).unwrap_or(&v);
    let variants = image.get("variants")?.as_array()?;
    let size = |v: &Value| v.get("size").and_then(Value::as_u64);
    variants
        .iter()
        .find(|v| v.pointer("/platform/architecture").and_then(Value::as_str) == Some(arch))
        .and_then(size)
        .or_else(|| variants.iter().filter_map(size).max())
}

/// The app root in `container system status --format json`: under `paths`
/// from Apple's `container` 1.4, at the top before.
fn apple_app_root(json: &str) -> Option<PathBuf> {
    let v: Value = serde_json::from_str(json).ok()?;
    v.pointer("/paths/appRoot")
        .or_else(|| v.get("appRoot"))
        .and_then(Value::as_str)
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
}

/// An architecture as images name it: `arm64` for Rust's `aarch64`.
fn linux_arch(arch: &str) -> &str {
    match arch {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    }
}

/// An image's ID and labels from `image inspect`: Docker's, or Apple's,
/// whose `index` became `configuration.descriptor` in its 1.0.
fn image_info(out: &str) -> Option<ImageInfo> {
    let v: Value = serde_json::from_str(out).ok()?;
    let image = v.get(0)?;
    let id = image
        .get("Id")
        .or_else(|| image.pointer("/configuration/descriptor/digest"))
        .or_else(|| image.pointer("/index/digest"))
        .and_then(Value::as_str)?;
    let variants = image.get("variants").and_then(Value::as_array);
    let configs = image.pointer("/Config/Labels").into_iter().chain(
        variants
            .into_iter()
            .flatten()
            .filter_map(|v| v.pointer("/config/config/Labels")),
    );
    let mut labels = BTreeMap::new();
    for (k, v) in configs.filter_map(Value::as_object).flatten() {
        if let Some(v) = v.as_str() {
            labels.insert(k.clone(), v.to_owned());
        }
    }
    Some(ImageInfo {
        id: id.to_owned(),
        labels,
    })
}

/// The tagged names of `repository` in a list of image names, one per line.
fn names_in(out: &str, repository: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|name| {
            name.strip_prefix(repository)
                .and_then(|rest| rest.strip_prefix(':'))
                .is_some_and(|tag| !tag.is_empty() && tag != "<none>")
        })
        .map(str::to_owned)
        .collect()
}

/// `docker info --format '{{.NCPU}} {{.MemTotal}}'`: CPUs, and memory in bytes.
fn docker_resources(out: &str) -> Option<SandboxSize> {
    let mut words = out.split_whitespace();
    let cpus: u32 = words.next()?.parse().ok()?;
    let bytes: u64 = words.next()?.parse().ok()?;
    let memory_mb = u32::try_from(bytes / 1024 / 1024).unwrap_or(u32::MAX);
    (cpus > 0 && memory_mb > 0).then_some(SandboxSize { cpus, memory_mb })
}

/// Whether `bin` can be started: a path to a file, or a program on the PATH.
fn installed(bin: &str) -> bool {
    if bin.contains('/') {
        Path::new(bin).is_file()
    } else {
        super::process::which(bin).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::fake::{FakeCli, write_script};
    use std::sync::{Mutex, PoisonError};

    #[test]
    fn build_logs_show_the_steps() {
        for step in [
            "#5 [2/4] RUN apt-get update",
            "#12 [build 3/7] COPY . .",
            "Step 2/4 : RUN make",
            "STEP 2/4: RUN make",
            "ERROR: failed to solve: process \"/bin/sh -c make\" did not complete successfully",
        ] {
            assert!(build_step(step), "{step}");
        }
        for noise in [
            "#5 0.312 Get:1 http://deb.debian.org",
            "#5 DONE 3.1s",
            "",
            "#x [1/2]",
        ] {
            assert!(!build_step(noise), "{noise}");
        }
    }

    #[test]
    fn reads_the_size_of_apples_images() {
        let json = r#"[{"name":"fctl-build-prj_1:abc","index":{"size":1024},"variants":[
            {"platform":{"os":"linux","architecture":"amd64"},"size":900},
            {"platform":{"os":"linux","architecture":"arm64"},"size":800}]}]"#;
        assert_eq!(apple_image_size(json, "arm64"), Some(800));
        assert_eq!(apple_image_size(json, "riscv64"), Some(900));
        assert_eq!(apple_image_size("[{\"name\":\"x\"}]", "arm64"), None);
        assert_eq!(apple_image_size("not json", "arm64"), None);
    }

    #[test]
    fn reads_the_docker_vms_size() {
        // OrbStack's VM in #14's session: 10 CPUs and 15.6 GiB.
        assert_eq!(
            docker_resources("10 16766767104\n"),
            Some(SandboxSize {
                cpus: 10,
                memory_mb: 15_990
            })
        );
        for bad in [
            "",
            "10",
            "ten 1024",
            "0 16766767104",
            "<no value> <no value>",
        ] {
            assert_eq!(docker_resources(bad), None, "{bad}");
        }
    }

    #[test]
    fn reads_an_images_id_and_labels() {
        let labels = BTreeMap::from([
            (
                "org.opencontainers.image.revision".to_owned(),
                "12685ad".to_owned(),
            ),
            (
                "sh.fctl.claude-code.version".to_owned(),
                "2.1.302".to_owned(),
            ),
        ]);
        let published = |id: &str| ImageInfo {
            id: id.into(),
            labels: labels.clone(),
        };
        let docker = r#"[{
            "Id": "sha256:4f1c",
            "RepoTags": ["ghcr.io/hyperstructure/sandbox:0.5.0"],
            "Config": {"User": "agent", "Labels": {
                "org.opencontainers.image.revision": "12685ad",
                "sh.fctl.claude-code.version": "2.1.302"
            }}
        }]"#;
        assert_eq!(image_info(docker), Some(published("sha256:4f1c")));
        let built = r#"[{"Id": "sha256:77aa", "Config": {"User": "agent", "Labels": null}}]"#;
        assert_eq!(
            image_info(built),
            Some(ImageInfo {
                id: "sha256:77aa".into(),
                labels: BTreeMap::new(),
            })
        );
        // Apple's `container` from 1.0: an attestation has no image config.
        let apple = r#"[{
            "id": "9a8b",
            "configuration": {
                "name": "ghcr.io/hyperstructure/sandbox:0.5.0",
                "descriptor": {"digest": "sha256:9a8b", "size": 1609}
            },
            "variants": [
                {"platform": {"os": "linux", "architecture": "arm64"}, "digest": "sha256:01", "size": 1,
                 "config": {"os": "linux", "architecture": "arm64", "config": {"User": "agent", "Labels": {
                    "org.opencontainers.image.revision": "12685ad",
                    "sh.fctl.claude-code.version": "2.1.302"
                 }}}},
                {"platform": {"os": "unknown", "architecture": "unknown"}, "digest": "sha256:02", "size": 1,
                 "config": {"os": "unknown", "architecture": "unknown"}}
            ]
        }]"#;
        assert_eq!(image_info(apple), Some(published("sha256:9a8b")));
        // …and before it.
        let apple_0 = r#"[{
            "name": "ghcr.io/hyperstructure/sandbox:0.5.0",
            "index": {"digest": "sha256:9a8b", "size": 1609},
            "variants": [{"platform": {"os": "linux", "architecture": "arm64"}, "size": 1,
                "config": {"os": "linux", "architecture": "arm64", "config": {"Labels": {
                    "org.opencontainers.image.revision": "12685ad",
                    "sh.fctl.claude-code.version": "2.1.302"
                }}}}]
        }]"#;
        assert_eq!(image_info(apple_0), Some(published("sha256:9a8b")));
        for unreadable in ["", "[]", "{}", "[{\"Config\": {}}]", "Error: no such image"] {
            assert_eq!(image_info(unreadable), None, "{unreadable}");
        }
    }

    #[test]
    fn lists_the_tagged_names_of_a_repository() {
        let repository = "ghcr.io/hyperstructure/sandbox";
        // `docker image ls` lists a build without a tag as `<none>`, and
        // Apple's `container image list --quiet` lists every image.
        let out = "ghcr.io/hyperstructure/sandbox:0.5.0\n\
                   ghcr.io/hyperstructure/sandbox:<none>\n\
                   ghcr.io/hyperstructure/sandbox-dev:1\n\
                   ghcr.io/hyperstructure/sandbox@sha256:9a8b\n\
                   docker.io/library/debian:bookworm-slim\n\
                   ghcr.io/hyperstructure/sandbox:latest\n";
        assert_eq!(
            names_in(out, repository),
            [
                "ghcr.io/hyperstructure/sandbox:0.5.0",
                "ghcr.io/hyperstructure/sandbox:latest"
            ]
        );
    }

    const VERSION_IMAGE: &str = "ghcr.io/hyperstructure/sandbox:0.3.0";
    const LATEST_IMAGE: &str = "ghcr.io/hyperstructure/sandbox:latest";

    fn spec(image: &str, fallback: Option<&str>) -> SandboxSpec {
        SandboxSpec {
            name: "fctl-ses-1".into(),
            image: image.into(),
            fallback_image: fallback.map(str::to_owned),
            cpus: 2,
            memory_mb: 2048,
            cache: None,
        }
    }

    fn error_of<T: std::fmt::Debug>(result: Result<T>) -> String {
        match result {
            Ok(v) => format!("no error: {v:?}"),
            Err(e) => format!("{e:#}"),
        }
    }

    #[tokio::test]
    async fn sandboxes_pull_their_image_when_it_is_not_here() -> Result<()> {
        let default = spec(VERSION_IMAGE, Some(LATEST_IMAGE));
        let cli = FakeCli::new(&[VERSION_IMAGE], &[])?;
        let (image, log) = cli.ensure(Flavor::Docker, &default).await?;
        assert_eq!(image, VERSION_IMAGE);
        assert_eq!(
            log,
            [format!("Creating sandbox fctl-ses-1 from {VERSION_IMAGE}")]
        );

        let cli = FakeCli::new(&[], &[VERSION_IMAGE, LATEST_IMAGE])?;
        let (image, log) = cli.ensure(Flavor::Docker, &default).await?;
        assert_eq!(image, VERSION_IMAGE);
        assert_eq!(log[1], format!("Pulling {VERSION_IMAGE}"));
        assert!(cli.calls().contains(&format!("pull {VERSION_IMAGE}")));

        // Apple's CLI pulls every platform of an image unless told one.
        let cli = FakeCli::new(&[], &[VERSION_IMAGE])?;
        let (image, _) = cli
            .ensure(Flavor::Apple, &spec(VERSION_IMAGE, None))
            .await?;
        assert_eq!(image, VERSION_IMAGE);
        let platform = format!("linux/{}", linux_arch(std::env::consts::ARCH));
        let pull = format!("image pull --platform {platform} {VERSION_IMAGE}");
        assert!(cli.calls().contains(&pull), "{:?}", cli.calls());
        Ok(())
    }

    #[tokio::test]
    async fn docker_sandboxes_join_a_network_that_keeps_them_apart() -> Result<()> {
        let default = spec(VERSION_IMAGE, None);
        let cli = FakeCli::new(&[VERSION_IMAGE], &[])?;
        cli.ensure(Flavor::Docker, &default).await?;
        let calls = cli.calls();
        let create = format!(
            "network create --driver bridge --opt com.docker.network.bridge.enable_icc=false {SANDBOX_NETWORK}"
        );
        assert!(calls.contains(&create), "{calls:?}");
        let run = calls.iter().find(|c| c.starts_with("run ")).cloned();
        assert!(
            run.as_deref()
                .is_some_and(|r| r.contains(&format!(" --network {SANDBOX_NETWORK} "))),
            "{run:?}"
        );
        // The next sandbox finds it.
        let other = SandboxSpec {
            name: "fctl-ses-2".into(),
            ..default.clone()
        };
        cli.ensure(Flavor::Docker, &other).await?;
        assert_eq!(
            cli.calls().iter().filter(|c| **c == create).count(),
            1,
            "{:?}",
            cli.calls()
        );

        // A runtime that can't make it still starts sandboxes, as before.
        let cli = FakeCli::new(&[VERSION_IMAGE], &[])?;
        cli.no_networks()?;
        cli.ensure(Flavor::Docker, &default).await?;
        let run = cli.calls().into_iter().find(|c| c.starts_with("run "));
        assert!(
            run.as_deref().is_some_and(|r| !r.contains("--network")),
            "{run:?}"
        );

        // Apple's VMs don't take Docker's networks.
        let cli = FakeCli::new(&[VERSION_IMAGE], &[])?;
        cli.ensure(Flavor::Apple, &default).await?;
        assert!(
            !cli.calls().iter().any(|c| c.contains("network")),
            "{:?}",
            cli.calls()
        );
        Ok(())
    }

    #[tokio::test]
    async fn the_default_image_falls_back_to_the_latest_published_one() -> Result<()> {
        // A release's image is published a few minutes after the release.
        let default = spec(VERSION_IMAGE, Some(LATEST_IMAGE));
        let cli = FakeCli::new(&[], &[LATEST_IMAGE])?;
        let (image, log) = cli.ensure(Flavor::Docker, &default).await?;
        assert_eq!(image, LATEST_IMAGE);
        let note = format!(
            "{VERSION_IMAGE} is not on this host and can't be pulled; starting from {LATEST_IMAGE} instead"
        );
        assert!(log.contains(&note), "{log:?}");

        let cli = FakeCli::new(&[], &[])?;
        let error = error_of(cli.ensure(Flavor::Docker, &default).await);
        let missing = format!("{VERSION_IMAGE} is not on this host and can't be pulled");
        assert!(
            error.starts_with(&format!(
                "{missing} (`fctl host image build` builds it here): "
            )),
            "{error}"
        );
        assert!(
            error.ends_with(&format!("{VERSION_IMAGE}: manifest unknown")),
            "{error}"
        );

        // A project's own image has nothing to fall back to.
        let project = spec("ghcr.io/acme/agent:1", None);
        let error = error_of(cli.ensure(Flavor::Docker, &project).await);
        let missing = "ghcr.io/acme/agent:1 is not on this host and can't be pulled: ";
        assert!(error.starts_with(missing), "{error}");
        Ok(())
    }

    #[tokio::test]
    async fn pulling_the_default_image_falls_back_to_the_latest_published_one() -> Result<()> {
        let lines = Mutex::new(Vec::new());
        let log = |line: String| {
            lines
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line)
        };
        let logged = || lines.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let fallback = Some(LATEST_IMAGE);

        let cli = FakeCli::new(&[], &[VERSION_IMAGE, LATEST_IMAGE])?;
        let docker = cli.driver(Flavor::Docker);
        let pulled = docker
            .pull_or_fallback(VERSION_IMAGE, fallback, false, &log)
            .await?;
        assert_eq!(pulled, VERSION_IMAGE);
        assert_eq!(cli.calls(), [format!("pull {VERSION_IMAGE}")]);
        assert!(logged().is_empty());

        // A release's image is published within the hour after the release.
        let cli = FakeCli::new(&[], &[LATEST_IMAGE])?;
        let docker = cli.driver(Flavor::Docker);
        let pulled = docker
            .pull_or_fallback(VERSION_IMAGE, fallback, false, &log)
            .await?;
        assert_eq!(pulled, LATEST_IMAGE);
        assert!(
            cli.calls().contains(&format!("pull {LATEST_IMAGE}")),
            "{:?}",
            cli.calls()
        );
        let note = format!("{VERSION_IMAGE} can't be pulled; pulling {LATEST_IMAGE} instead");
        assert_eq!(logged(), [note]);

        // Sessions start from an image that is here, a build of it say.
        let cli = FakeCli::new(&[VERSION_IMAGE], &[LATEST_IMAGE])?;
        let docker = cli.driver(Flavor::Docker);
        let error = error_of(
            docker
                .pull_or_fallback(VERSION_IMAGE, fallback, false, &log)
                .await,
        );
        assert!(
            error.ends_with(&format!("{VERSION_IMAGE}: manifest unknown")),
            "{error}"
        );
        assert!(!cli.calls().contains(&format!("pull {LATEST_IMAGE}")));

        let cli = FakeCli::new(&[], &[])?;
        let docker = cli.driver(Flavor::Docker);
        let error = error_of(
            docker
                .pull_or_fallback(VERSION_IMAGE, fallback, false, &log)
                .await,
        );
        assert!(
            error.starts_with(&format!(
                "{VERSION_IMAGE} can't be pulled, and neither can {LATEST_IMAGE} (`fctl host image build` builds it here): "
            )),
            "{error}"
        );
        assert!(
            error.ends_with(&format!("{VERSION_IMAGE}: manifest unknown")),
            "{error}"
        );

        // An image set in host.toml has nothing to fall back to.
        let error = error_of(
            docker
                .pull_or_fallback("ghcr.io/acme/agent:1", None, false, &log)
                .await,
        );
        assert!(
            error.ends_with("ghcr.io/acme/agent:1: manifest unknown"),
            "{error}"
        );
        assert_eq!(
            cli.calls(),
            [
                format!("pull {VERSION_IMAGE}"),
                format!("image inspect --format {{{{.Id}}}} {VERSION_IMAGE}"),
                format!("pull {LATEST_IMAGE}"),
                "pull ghcr.io/acme/agent:1".to_owned(),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn other_failures_are_not_about_the_image() -> Result<()> {
        let cli = FakeCli::new(&[VERSION_IMAGE], &[LATEST_IMAGE])?;
        cli.fail_runs()?;
        let default = spec(VERSION_IMAGE, Some(LATEST_IMAGE));
        let error = error_of(cli.ensure(Flavor::Docker, &default).await);
        assert!(error.starts_with("starting fctl-ses-1: "), "{error}");
        assert!(error.contains("no room for it"), "{error}");
        assert!(
            !cli.calls().iter().any(|c| c.contains("pull")),
            "{:?}",
            cli.calls()
        );
        Ok(())
    }

    #[tokio::test]
    async fn nothing_is_missing_when_the_cli_cannot_start() -> Result<()> {
        // The image is here and published, but the system won't run the CLI.
        let cli = FakeCli::new(&[VERSION_IMAGE], &[VERSION_IMAGE, LATEST_IMAGE])?;
        cli.wont_start()?;
        let refused = format!(
            "running {}: Permission denied (os error 13)",
            cli.dir.path().join("cli").display()
        );
        let docker = cli.driver(Flavor::Docker);
        let not_started = |e: anyhow::Error| e.is::<NotStarted>();
        assert!(docker.image(VERSION_IMAGE).await.is_err_and(not_started));
        // A sandbox it can't stop or remove isn't gone.
        let sb = Sandbox {
            name: "fctl-ses-1".into(),
            root: None,
        };
        assert!(docker.exists(&sb.name).await.is_err_and(not_started));
        assert!(docker.stop(&sb).await.is_err_and(not_started));
        assert!(docker.remove(&sb).await.is_err_and(not_started));

        // The host pulls neither the image nor `latest` in its place.
        let default = spec(VERSION_IMAGE, Some(LATEST_IMAGE));
        assert_eq!(
            error_of(cli.ensure(Flavor::Docker, &default).await),
            refused
        );
        let lines = Mutex::new(Vec::new());
        let log = |line: String| {
            lines
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line)
        };
        let created = docker.create(&default, VERSION_IMAGE, &log).await;
        assert!(matches!(&created, Err(Create::Failed(e)) if e.is::<NotStarted>()));
        let pulled = docker
            .pull_or_fallback(VERSION_IMAGE, Some(LATEST_IMAGE), false, &log)
            .await;
        assert_eq!(error_of(pulled), refused);
        assert!(
            lines
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        );
        Ok(())
    }

    /// A stand-in CLI that prints `info` for `docker info` and for
    /// `container system status`, and `df` for a `run` of an image in
    /// `have`, logging every call in `calls`.
    fn disk_cli(dir: &Path, info: &str, have: &str) -> Result<ContainerDriver> {
        let df = "Filesystem 1024-blocks Used Available Capacity Mounted on\\noverlay 61202244 40000000 18066668 69%% /\\n";
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> '{calls}'\ncase \"$1\" in\n  info|system) echo '{info}' ;;\n  run) case \" $* \" in *\" {have} -Pk / \"*) printf '{df}' ;; *) echo 'Unable to find image' >&2; exit 125 ;; esac ;;\n  *) exit 2 ;;\nesac\n",
            calls = dir.join("calls").display(),
        );
        let bin = dir.join("cli");
        write_script(&bin, &script)?;
        Ok(ContainerDriver::with_bin(
            Flavor::Docker,
            bin.display().to_string(),
        ))
    }

    #[test]
    fn apples_app_root_comes_from_its_status() {
        let now = r#"{"status": "running", "paths": {"appRoot": "/Volumes/Big/container/", "installRoot": "/usr/local/"}}"#;
        let before = r#"{"status": "running", "appRoot": "/Users/ada/Library/Application Support/com.apple.container/"}"#;
        assert_eq!(
            apple_app_root(now),
            Some(PathBuf::from("/Volumes/Big/container/"))
        );
        assert_eq!(
            apple_app_root(before),
            Some(PathBuf::from(
                "/Users/ada/Library/Application Support/com.apple.container/"
            ))
        );
        assert_eq!(apple_app_root(r#"{"status": "unregistered"}"#), None);
        assert_eq!(apple_app_root(r#"{"appRoot": ""}"#), None);
        assert_eq!(apple_app_root("apiserver is not running"), None);
    }

    #[tokio::test]
    async fn apple_sandboxes_are_measured_in_the_app_root() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("container");
        std::fs::create_dir(&root)?;
        let status = format!(
            r#"{{"status": "running", "paths": {{"appRoot": "{}"}}}}"#,
            root.display()
        );
        let mut driver = disk_cli(dir.path(), &status, "-")?;
        driver.flavor = Flavor::Apple;
        let data = dir.path().join("data");
        assert_eq!(driver.disk_spot(&data).await, Some(DiskSpot::Dir(root)));
        // A runtime too old to say measures the Mac's disk anyway.
        let older = ContainerDriver::with_bin(Flavor::Apple, "false".into());
        assert!(matches!(
            older.disk_spot(&data).await,
            Some(DiskSpot::Dir(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn docker_is_measured_where_it_keeps_containers() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let data = dir.path().join("data");
        // Its root directory, when it is on this machine.
        let here = dir.path().display().to_string();
        let driver = disk_cli(dir.path(), &here, "-")?;
        let spot = driver.disk_spot(&data).await;
        if cfg!(target_os = "linux") {
            assert_eq!(spot, Some(DiskSpot::Dir(dir.path().to_owned())));
        } else {
            assert_eq!(spot, Some(DiskSpot::DockerVm));
        }
        // Otherwise inside its VM, as on macOS.
        let driver = disk_cli(dir.path(), "/var/lib/fctl-test-not-here", "-")?;
        assert_eq!(driver.disk_spot(&data).await, Some(DiskSpot::DockerVm));
        // A daemon that doesn't answer leaves it as it was.
        let down = ContainerDriver::with_bin(Flavor::Docker, "false".into());
        assert_eq!(down.disk_spot(&data).await, None);
        Ok(())
    }

    #[tokio::test]
    async fn the_docker_vm_is_measured_in_a_short_lived_container() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let latest = "ghcr.io/hyperstructure/sandbox:latest";
        let driver = disk_cli(dir.path(), "-", latest)?;
        // The host's image isn't here yet; the one it falls back to is.
        let images = [
            "ghcr.io/hyperstructure/sandbox:0.6.0".to_owned(),
            latest.to_owned(),
        ];
        assert_eq!(driver.vm_disk_free_mb(&images).await, Some(17_643));
        let calls = std::fs::read_to_string(dir.path().join("calls"))?;
        let calls: Vec<&str> = calls.lines().collect();
        assert_eq!(
            calls,
            [
                "run --rm --pull never --network none --entrypoint df ghcr.io/hyperstructure/sandbox:0.6.0 -Pk /",
                "run --rm --pull never --network none --entrypoint df ghcr.io/hyperstructure/sandbox:latest -Pk /",
            ]
        );
        assert_eq!(driver.vm_disk_free_mb(&images[..1]).await, None);
        Ok(())
    }

    /// A stand-in CLI whose one sandbox runs, logging every call in
    /// `calls`. Its `clean` runs `clean`.
    fn running_cli(dir: &Path, flavor: Flavor, clean: &str) -> Result<ContainerDriver> {
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> '{calls}'\ncase \"$1\" in\n  inspect) if [ \"$2\" = --format ]; then echo running; else echo '[{{\"status\": {{\"state\": \"running\"}}}}]'; fi ;;\n  exec|stop) ;;\n  clean) {clean} ;;\n  *) exit 2 ;;\nesac\n",
            calls = dir.join("calls").display(),
        );
        let bin = dir.join("cli");
        write_script(&bin, &script)?;
        Ok(ContainerDriver::with_bin(flavor, bin.display().to_string()))
    }

    fn calls_in(dir: &Path) -> Result<Vec<String>> {
        let calls = std::fs::read_to_string(dir.join("calls"))?;
        Ok(calls.lines().map(str::to_owned).collect())
    }

    #[tokio::test]
    async fn apple_sandboxes_give_back_the_space_of_deleted_files_as_they_stop() -> Result<()> {
        let sb = Sandbox {
            name: "fctl-ses-1".into(),
            root: None,
        };
        let cleans = "echo \"$2\"";
        // Apple's `container` before 1.4 looks for a plugin.
        let has_no_clean = "echo \"Error: Plugin 'container-clean' not found.\" >&2; exit 64";
        for clean in [cleans, has_no_clean] {
            let dir = tempfile::tempdir()?;
            running_cli(dir.path(), Flavor::Apple, clean)?
                .stop(&sb)
                .await?;
            // While it runs, after a `sync`: ext4 trims only what its
            // journal has committed.
            assert_eq!(
                calls_in(dir.path())?,
                [
                    "inspect fctl-ses-1",
                    "exec fctl-ses-1 sync",
                    "clean fctl-ses-1",
                    "stop -t 20 fctl-ses-1"
                ]
            );
        }
        // Docker's containers share a filesystem, which frees what they delete.
        let dir = tempfile::tempdir()?;
        running_cli(dir.path(), Flavor::Docker, cleans)?
            .stop(&sb)
            .await?;
        assert_eq!(
            calls_in(dir.path())?,
            [
                "inspect --format {{.State.Status}} fctl-ses-1",
                "stop -t 20 fctl-ses-1"
            ]
        );
        Ok(())
    }

    /// A stand-in for Apple's `container` with one sandbox, whose state its
    /// API server (`listed`) and the VM's runtime (`runtime`) keep apart, as
    /// Apple's do. With `wedge` there, a `stop` fails as Apple's did when
    /// the VM's init couldn't kill the cgroups Podman had made in the
    /// sandbox: the runtime is left stopping, and the API server lists the
    /// sandbox as running. Then `exec`, `cp` and `clean` are refused, and
    /// `start` does nothing, until a `stop` succeeds. With `stuck` there,
    /// every `stop` fails. With `lost` there, it can't be started again once
    /// it has answered an `inspect`.
    const APPLE: &str = r##"#!/bin/sh
d=$(dirname "$0")
echo "$*" >> "$d/calls"
for last in "$@"; do :; done
listed=$(cat "$d/listed")
runtime=$(cat "$d/runtime")
refuse() {
  printf 'Error: failed to %s (cause: "invalidState: "cannot %s: container is not running"")\n' "$1" "$2" >&2
  exit 1
}
case "$1" in
  inspect)
    printf '[{"configuration": {"id": "%s"}, "status": "%s"}]\n' "$last" "$listed"
    [ ! -e "$d/lost" ] || chmod 644 "$0" ;;
  exec) [ "$runtime" = running ] || refuse "create process in container" exec ;;
  cp) [ "$runtime" = running ] || refuse "copy into container" copyIn ;;
  clean) [ "$runtime" = running ] || refuse "clean container" clean ;;
  stop)
    if [ -e "$d/stuck" ] || { [ -e "$d/wedge" ] && [ "$runtime" = running ]; }; then
      [ "$runtime" != running ] || echo stopping > "$d/runtime"
      printf 'Error: internalError: "failed to stop container" (cause: "internalError: "failed to stop container %s (cause: "internalError: "failed to delete process (cause: "internalError: "deleteProcess: failed with errno 95: failed to write to /sys/fs/cgroup/container/%s/libpod_parent/cgroup.kill"")"")"")\n' "$last" "$last" >&2
      exit 1
    fi
    echo stopped > "$d/listed"
    echo stopped > "$d/runtime"
    echo "$last" ;;
  start)
    if [ "$listed" != running ]; then
      echo running > "$d/listed"
      echo running > "$d/runtime"
    fi
    echo "$last" ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
"##;

    /// [`APPLE`] in `dir`, its sandbox `listed` as it says and its runtime
    /// in the state `runtime`.
    fn apple_cli(dir: &Path, listed: &str, runtime: &str) -> Result<ContainerDriver> {
        std::fs::write(dir.join("listed"), listed)?;
        std::fs::write(dir.join("runtime"), runtime)?;
        let bin = dir.join("cli");
        write_script(&bin, APPLE)?;
        Ok(ContainerDriver::with_bin(
            Flavor::Apple,
            bin.display().to_string(),
        ))
    }

    /// Readies the sandbox of [`spec`] with `driver`, and puts fctl-agent
    /// in it: what it said in the session's log.
    async fn ready(driver: &ContainerDriver) -> Result<Vec<String>> {
        let lines = Mutex::new(Vec::new());
        let log = |line: String| {
            lines
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line)
        };
        let sb = driver.ensure(&spec(VERSION_IMAGE, None), &log).await?;
        driver.install_agent(&sb, Path::new("fctl-agent")).await?;
        Ok(lines.into_inner().unwrap_or_else(PoisonError::into_inner))
    }

    #[tokio::test]
    async fn apple_sandboxes_that_fail_to_stop_are_stopped_again() -> Result<()> {
        let sb = Sandbox {
            name: "fctl-ses-1".into(),
            root: None,
        };
        let dir = tempfile::tempdir()?;
        let apple = apple_cli(dir.path(), "running", "running")?;
        std::fs::write(dir.path().join("wedge"), "")?;
        apple.stop(&sb).await?;
        assert_eq!(
            calls_in(dir.path())?,
            [
                "inspect fctl-ses-1",
                "exec fctl-ses-1 sync",
                "clean fctl-ses-1",
                "stop -t 20 fctl-ses-1",
                "inspect fctl-ses-1",
                "stop -t 20 fctl-ses-1"
            ]
        );
        // The session resumes in it.
        assert_eq!(
            ready(&apple).await?,
            ["Starting the existing sandbox fctl-ses-1"]
        );

        // One that doesn't stop the second time either is reported.
        let dir = tempfile::tempdir()?;
        let apple = apple_cli(dir.path(), "running", "running")?;
        std::fs::write(dir.path().join("stuck"), "")?;
        let error = error_of(apple.stop(&sb).await);
        assert!(
            error.ends_with("libpod_parent/cgroup.kill\"\")\"\")\"\")"),
            "{error}"
        );
        let stops = calls_in(dir.path())?
            .into_iter()
            .filter(|c| c.starts_with("stop "))
            .count();
        assert_eq!(stops, 2);
        Ok(())
    }

    #[tokio::test]
    async fn sandboxes_listed_as_running_that_take_no_commands_start_again() -> Result<()> {
        // As hosts before this left the sandboxes they failed to stop.
        let dir = tempfile::tempdir()?;
        let apple = apple_cli(dir.path(), "running", "stopping")?;
        assert_eq!(
            ready(&apple).await?,
            [
                "Starting the sandbox fctl-ses-1 again: it is listed as running but takes no commands"
            ]
        );
        assert_eq!(
            calls_in(dir.path())?[..4],
            [
                "inspect fctl-ses-1",
                "exec fctl-ses-1 true",
                "stop -t 20 fctl-ses-1",
                "start fctl-ses-1"
            ]
        );

        // One that runs is taken as it is.
        let dir = tempfile::tempdir()?;
        let apple = apple_cli(dir.path(), "running", "running")?;
        assert!(ready(&apple).await?.is_empty());
        assert_eq!(
            calls_in(dir.path())?[..2],
            ["inspect fctl-ses-1", "exec fctl-ses-1 true"]
        );
        assert!(!calls_in(dir.path())?.contains(&"start fctl-ses-1".to_owned()));

        // One whose runtime stops answering is left as it is.
        let dir = tempfile::tempdir()?;
        let apple = apple_cli(dir.path(), "running", "stopping")?;
        std::fs::write(dir.path().join("lost"), "")?;
        let lines = Mutex::new(Vec::new());
        let log = |line: String| {
            lines
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line)
        };
        let ensured = apple.ensure(&spec(VERSION_IMAGE, None), &log).await;
        assert!(ensured.is_err_and(|e| e.is::<NotStarted>()));
        assert!(
            lines
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn says_whether_the_runtime_is_missing_or_stopped() {
        // `true` and `false` stand in for a CLI whose every command succeeds or fails.
        for (flavor, bin, available, detail) in [
            (
                Flavor::Apple,
                "fctl-test-missing",
                false,
                "Apple's `container` is not installed",
            ),
            (
                Flavor::Apple,
                "false",
                false,
                "the `container` service is not running",
            ),
            (Flavor::Apple, "true", true, "ready"),
            (
                Flavor::Docker,
                "fctl-test-missing",
                false,
                "the docker CLI is not installed",
            ),
            (
                Flavor::Docker,
                "false",
                false,
                "the docker daemon is not running",
            ),
        ] {
            let driver = ContainerDriver {
                flavor,
                bin: bin.into(),
            };
            let info = driver.probe().await;
            assert_eq!(info.available, available, "{bin}");
            let got = info.detail.unwrap_or_default();
            assert!(got.starts_with(detail), "{bin}: {got}");
        }
    }

    #[tokio::test]
    async fn docker_sandboxes_mount_their_projects_caches_and_get_to_write_them() -> Result<()> {
        let cli = FakeCli::new(&[VERSION_IMAGE], &[])?;
        let cached = SandboxSpec {
            cache: Some(Spot::Volume(caches::volume("prj_1"))),
            ..spec(VERSION_IMAGE, None)
        };
        cli.ensure(Flavor::Docker, &cached).await?;
        let calls = cli.calls();
        let run = calls
            .iter()
            .find(|c| c.starts_with("run "))
            .cloned()
            .unwrap_or_default();
        assert!(
            run.contains(
                " --mount type=volume,source=fctl-cache-prj_1,target=/var/cache/fctl --entrypoint tail "
            ),
            "{run}"
        );
        // Docker makes a new volume root's: the sandbox's user gets it.
        let given = calls.iter().find(|c| c.starts_with("exec -u 0 ")).cloned();
        assert_eq!(
            given.as_deref(),
            Some(
                r#"exec -u 0 fctl-ses-1 sh -c [ "$(stat -c %u:%g /var/cache/fctl)" = "$1" ] || chown -R "$1" /var/cache/fctl sh 1000:1000"#
            )
        );
        assert_eq!(cli.volumes()?, [("fctl-cache-prj_1".to_owned(), 0)]);

        // An image whose user is root has nothing to give.
        let cli = FakeCli::new(&[VERSION_IMAGE], &[])?;
        cli.user("0:0")?;
        cli.ensure(Flavor::Docker, &cached).await?;
        assert!(!cli.calls().iter().any(|c| c.starts_with("exec -u 0")));

        // Without caches, nothing is mounted or given.
        let cli = FakeCli::new(&[VERSION_IMAGE], &[])?;
        cli.ensure(Flavor::Docker, &spec(VERSION_IMAGE, None))
            .await?;
        assert!(
            !cli.calls()
                .iter()
                .any(|c| c.contains("--mount") || c.starts_with("exec"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn apple_sandboxes_mount_their_projects_cache_directory() -> Result<()> {
        let cli = FakeCli::new(&[VERSION_IMAGE], &[])?;
        let dir = PathBuf::from("/Users/ada/Library/Application Support/fctl/host/caches/prj_1");
        let cached = SandboxSpec {
            cache: Some(Spot::Dir(dir.clone())),
            ..spec(VERSION_IMAGE, None)
        };
        cli.ensure(Flavor::Apple, &cached).await?;
        let run = cli
            .calls()
            .into_iter()
            .find(|c| c.starts_with("run "))
            .unwrap_or_default();
        let mount = format!(
            " --mount type=bind,source={},target=/var/cache/fctl ",
            dir.display()
        );
        assert!(run.contains(&mount), "{run}");
        // A share of the Mac's needs no owner.
        assert!(!cli.calls().iter().any(|c| c.starts_with("exec")));
        Ok(())
    }

    #[tokio::test]
    async fn docker_volumes_are_listed_measured_and_cleared() -> Result<()> {
        let cli = FakeCli::new(&[VERSION_IMAGE], &[])?;
        let docker = cli.driver(Flavor::Docker);
        let images = [LATEST_IMAGE.to_owned(), VERSION_IMAGE.to_owned()];
        cli.volume("fctl-cache-prj_1", 3 * 1024 * 1024 + 1)?;
        cli.volume("fctl-cache-prj_2", 10)?;
        cli.volume("buildx_buildkit_state", 10)?;
        assert_eq!(docker.cache_volumes().await?, ["prj_1", "prj_2"]);
        // In the first of the host's images that is here, without its network.
        assert_eq!(
            docker.volume_size_mb("fctl-cache-prj_1", &images).await?,
            3073
        );
        let du = cli
            .calls()
            .into_iter()
            .rfind(|c| c.contains(" du "))
            .unwrap_or_default();
        assert_eq!(
            du,
            format!(
                "run --rm --pull never --network none --user 0 --mount type=volume,source=fctl-cache-prj_1,target=/cache,readonly --entrypoint du {VERSION_IMAGE} -sk /cache"
            )
        );
        // A sandbox mounts prj_1's, stopped or not: it stays, emptied.
        cli.uses("fctl-ses-1", "sha256:1")?;
        std::fs::write(
            cli.dir.path().join("mounts/fctl-ses-1"),
            "fctl-cache-prj_1\n",
        )?;
        assert!(
            !docker
                .clear_volume("fctl-cache-prj_1", false, &images)
                .await?
        );
        // Nothing mounts prj_2's: it goes.
        assert!(
            docker
                .clear_volume("fctl-cache-prj_2", false, &images)
                .await?
        );
        assert_eq!(
            cli.volumes()?,
            [
                ("buildx_buildkit_state".to_owned(), 10),
                ("fctl-cache-prj_1".to_owned(), 0)
            ]
        );
        // Kept, it is only emptied; gone, nothing makes it again.
        cli.volume("fctl-cache-prj_2", 10)?;
        assert!(
            !docker
                .clear_volume("fctl-cache-prj_2", true, &images)
                .await?
        );
        assert_eq!(cli.volumes()?[2], ("fctl-cache-prj_2".to_owned(), 0));
        assert!(
            docker
                .clear_volume("fctl-cache-prj_9", true, &images)
                .await?
        );
        assert_eq!(cli.volumes()?.len(), 3);
        Ok(())
    }
}
