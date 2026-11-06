//! Sandbox drivers: turn a session into a running Linux sandbox and give the
//! host a channel to its fctl-agent.
//!
//! | Driver | Isolation | Notes |
//! |---|---|---|
//! | `apple` | a lightweight VM per session | Apple `container`, Apple silicon, macOS 26+ |
//! | `docker` | a container per session | Docker, OrbStack, Colima, Podman (`docker` CLI) |
//! | `firecracker` | a microVM per session | Linux with KVM; experimental |
//! | `lima` | a VM per session, cloned from `fctl-base` | experimental |
//! | `process` | none | development and trusted hosts |

pub mod container;
#[cfg(test)]
pub(crate) mod fake;
pub mod firecracker;
pub mod lima;
pub mod process;

pub use container::ImageInfo;

use crate::caches::{self, Spot};
use crate::config::HostConfig;
use anyhow::Result;
use fctl_proto::runner::{DriverInfo, SandboxSize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

/// How long a driver command may run before the host kills it. Runtimes
/// answer in seconds; one that takes longer is stuck (an overloaded Docker
/// VM, say), and nothing that waits on it may hang with it.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// How long pulling an image, or cloning, booting or shutting down a VM,
/// may take.
pub const SLOW_COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// What to create.
#[derive(Clone, Debug)]
pub struct SandboxSpec {
    pub name: String,
    pub image: String,
    /// The image to use when `image` is not here and can't be pulled.
    pub fallback_image: Option<String>,
    pub cpus: u32,
    pub memory_mb: u32,
    /// The project's package caches, which a new sandbox mounts at
    /// [`caches::MOUNT`] (the process driver's agents use them where they
    /// are).
    pub cache: Option<Spot>,
}

/// An image to build from a Dockerfile.
#[derive(Clone, Debug)]
pub struct BuildJob<'a> {
    pub tag: &'a str,
    pub dockerfile: &'a Path,
    pub context: &'a Path,
    pub args: &'a std::collections::BTreeMap<String, String>,
    pub target: Option<&'a str>,
    /// How long the build may take.
    pub limit: Duration,
}

/// A sandbox, as the host knows it.
#[derive(Clone, Debug)]
pub struct Sandbox {
    pub name: String,
    /// Process driver: the sandbox's directory on the host.
    pub root: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct SandboxInfo {
    pub name: String,
    pub running: bool,
}

/// Where the host measures the disk its sandboxes take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskSpot {
    /// A directory on this machine.
    Dir(PathBuf),
    /// Inside the Docker daemon's VM, as on macOS: `df` runs in a
    /// short-lived container there.
    DockerVm,
}

impl std::fmt::Display for DiskSpot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dir(dir) => write!(f, "{}", dir.display()),
            Self::DockerVm => f.write_str("the Docker VM"),
        }
    }
}

/// A program running in a sandbox, with its input and output piped.
pub struct Running {
    pub stdin: Option<Box<dyn AsyncWrite + Unpin + Send>>,
    pub stdout: Option<Box<dyn AsyncRead + Unpin + Send>>,
    pub stderr: Option<Box<dyn AsyncRead + Unpin + Send>>,
    exit: Exit,
}

enum Exit {
    /// The runtime's command (`docker exec`, `limactl shell`), or the
    /// program itself with the process driver; killed when dropped.
    Child(tokio::process::Child),
    /// A program in a microVM, whose exit code comes over the vsock.
    Vm(tokio::sync::oneshot::Receiver<Option<i32>>),
}

impl Running {
    pub fn child(mut child: tokio::process::Child) -> Self {
        Self {
            stdin: child
                .stdin
                .take()
                .map(|s| Box::new(s) as Box<dyn AsyncWrite + Unpin + Send>),
            stdout: child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn AsyncRead + Unpin + Send>),
            stderr: child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn AsyncRead + Unpin + Send>),
            exit: Exit::Child(child),
        }
    }

    pub fn vm(started: firecracker::vsock::Started) -> Self {
        Self {
            stdin: Some(Box::new(started.stdin)),
            stdout: Some(Box::new(started.stdout)),
            stderr: Some(Box::new(started.stderr)),
            exit: Exit::Vm(started.exit),
        }
    }

    /// Waits for it to exit; its exit code, when it says one.
    pub async fn wait(&mut self) -> Result<Option<i32>> {
        match &mut self.exit {
            Exit::Child(child) => Ok(child.wait().await?.code()),
            Exit::Vm(exit) => Ok(exit.await.ok().flatten()),
        }
    }
}

/// A byte channel to fctl-agent.
pub struct AgentIo {
    pub reader: Box<dyn AsyncRead + Unpin + Send>,
    pub writer: Box<dyn AsyncWrite + Unpin + Send>,
    /// The `exec` process carrying the channel, if any.
    pub child: Option<tokio::process::Child>,
}

/// The sandbox name for a session id: `fctl-ses-01ab…` (lowercase, dashes).
pub fn sandbox_name(session_id: &str) -> String {
    format!("fctl-{}", name_part(session_id))
}

/// The sandbox name of a Claude sign-in: `fctl-login-01ab…`.
pub fn login_name(job_id: &str) -> String {
    format!("fctl-login-{}", name_part(job_id))
}

/// An id the control plane sent, as part of a sandbox name: in lowercase,
/// with a dash for anything else than a letter or a digit. Ids are those
/// already, `_` aside; whatever else a control plane sends stays one name
/// for the container runtime, and one part of a path for the process
/// driver, which keeps each sandbox in a directory of that name.
fn name_part(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// The session id back from a sandbox name.
pub fn session_of(name: &str) -> Option<String> {
    let rest = name.strip_prefix("fctl-ses-")?;
    Some(format!("ses_{}", rest.to_uppercase()))
}

/// Where fctl-agent lives inside container and VM sandboxes.
pub const AGENT_PATH: &str = "/opt/fctl/bin/fctl-agent";

/// Readies `/opt/fctl/bin` for fctl-agent, as root: root's, where an image
/// gives it to its user (the default image did), since fctl-agent runs as
/// root from there, and its user must not put another in its place.
pub const AGENT_DIR_SCRIPT: &str = "mkdir -p /opt/fctl/bin && chown 0:0 /opt/fctl /opt/fctl/bin && chmod 755 /opt/fctl /opt/fctl/bin";

/// Where fctl-agent keeps its socket, state and log in container and VM
/// sandboxes, where it runs as root: a directory only root may enter, out
/// of reach of the user Claude runs as.
pub const AGENT_DIR: &str = "/var/lib/fctl-agent";

/// Says `root` in a sandbox where fctl-agent ran as root ([`AGENT_DIR`] is
/// there), `user` elsewhere, as any user: an answer either way, so that a
/// command that failed isn't taken for one ([`ran_as_root`]).
pub const RAN_AS_ROOT_SCRIPT: &str =
    "if [ -d /var/lib/fctl-agent ]; then echo root; else echo user; fi";

/// What [`RAN_AS_ROOT_SCRIPT`] said: whether fctl-agent ran as root in the
/// sandbox, which keeps it so rather than start a second supervisor beside
/// one that may run.
pub fn ran_as_root(said: &str) -> Result<bool> {
    match said.trim() {
        "root" => Ok(true),
        "user" => Ok(false),
        other => {
            anyhow::bail!("asked whether fctl-agent ran as root in the sandbox, it said {other:?}")
        }
    }
}

/// `fctl-agent serve` as root, with Claude and its commands as `user` (a
/// name or a uid), who can't stop it then.
pub fn serve_as_root(user: &str) -> Vec<String> {
    [
        AGENT_PATH,
        "serve",
        "--user",
        user,
        "--socket",
        &format!("{AGENT_DIR}/agent.sock"),
        "--state",
        &format!("{AGENT_DIR}/state"),
        "--log",
        &format!("{AGENT_DIR}/agent.log"),
    ]
    .map(str::to_owned)
    .to_vec()
}

/// `fctl-agent attach` to a supervisor that runs as root, as root.
pub fn attach_as_root() -> Vec<String> {
    [
        AGENT_PATH,
        "attach",
        "--socket",
        &format!("{AGENT_DIR}/agent.sock"),
    ]
    .map(str::to_owned)
    .to_vec()
}

/// fctl-agent in a sandbox where it runs as the user, who puts it there
/// itself, without root: in its `~/.fctl/bin`. A shell word inside double
/// quotes.
pub const USER_AGENT: &str = "$HOME/.fctl/bin/fctl-agent";

/// Puts fctl-agent, read from its input, in the user's `~/.fctl/bin`,
/// beside the one that may run and then in its place, as the user.
pub fn install_as_user_script() -> String {
    format!(
        r#"mkdir -p "$HOME/.fctl/bin" && cat > "{USER_AGENT}.new" && chmod 755 "{USER_AGENT}.new" && mv -f "{USER_AGENT}.new" "{USER_AGENT}""#
    )
}

/// Starts fctl-agent as the user, from its `~/.fctl/bin`.
pub fn serve_as_user_script() -> String {
    serve_script(&format!("\"{USER_AGENT}\""))
}

/// Attaches to fctl-agent as the user: with the user's own fctl-agent, or
/// the one in `/opt/fctl/bin` of a sandbox that a host put it in before.
pub fn attach_as_user_script() -> String {
    format!(
        r#"a="{USER_AGENT}"; [ -x "$a" ] || a={AGENT_PATH}; exec "$a" attach --socket "$HOME/.fctl/agent.sock""#
    )
}

/// The shell snippet that starts the supervisor inside a sandbox as the
/// sandbox's user, where it can't run as root.
pub fn serve_script(agent: &str) -> String {
    format!(
        "mkdir -p \"$HOME/.fctl\" && exec {agent} serve --socket \"$HOME/.fctl/agent.sock\" --state \"$HOME/.fctl/state\" --log \"$HOME/.fctl/agent.log\""
    )
}

pub fn attach_script(agent: &str) -> String {
    format!("exec {agent} attach --socket \"$HOME/.fctl/agent.sock\"")
}

/// Arguments as one line of `sh`, each quoted.
pub fn shell_words(args: &[String]) -> String {
    args.iter()
        .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ")
}

pub enum Driver {
    Process(process::ProcessDriver),
    Container(container::ContainerDriver),
    Firecracker(firecracker::FirecrackerDriver),
    Lima(lima::LimaDriver),
}

impl Driver {
    /// The driver `name`, with `cfg`'s settings (`auto` picks the best
    /// available; it never picks Firecracker, which hosts choose).
    pub async fn select(name: &str, cfg: &HostConfig) -> Result<Self> {
        let data_dir = cfg.data_dir();
        let data_dir = data_dir.as_path();
        let agent_bin = cfg.agent_bin.clone();
        let pick = |n: &str| -> Option<Driver> {
            match n {
                "process" => Some(Driver::Process(process::ProcessDriver::new(
                    data_dir,
                    agent_bin.clone(),
                ))),
                "apple" => Some(Driver::Container(
                    container::ContainerDriver::apple().agent_as_root(cfg.agent_as_root),
                )),
                "docker" => Some(Driver::Container(
                    container::ContainerDriver::docker().agent_as_root(cfg.agent_as_root),
                )),
                "firecracker" => Some(Driver::Firecracker(firecracker::FirecrackerDriver::new(
                    cfg,
                ))),
                "lima" => Some(Driver::Lima(
                    lima::LimaDriver::new().agent_as_root(cfg.agent_as_root),
                )),
                _ => None,
            }
        };
        if name != "auto" {
            return pick(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown driver `{name}` (use apple, docker, firecracker, lima, process or auto)"
                )
            });
        }
        for candidate in ["apple", "docker", "lima"] {
            if let Some(d) = pick(candidate)
                && d.probe().await.available
            {
                return Ok(d);
            }
        }
        tracing::warn!("no VM or container runtime found; using the process driver (no isolation)");
        Ok(Driver::Process(process::ProcessDriver::new(
            data_dir, agent_bin,
        )))
    }

    /// Every driver this host could use, for the runner's hello, each with
    /// `cfg`'s settings, as the one it uses has them: the control plane
    /// starts no session here while that one isn't ready.
    pub async fn probe_all(cfg: &HostConfig) -> Vec<DriverInfo> {
        let (apple, docker, firecracker, lima, process) = (
            container::ContainerDriver::apple(),
            container::ContainerDriver::docker(),
            firecracker::FirecrackerDriver::new(cfg),
            lima::LimaDriver::new(),
            process::ProcessDriver::new(&cfg.data_dir(), cfg.agent_bin.clone()),
        );
        let (apple, docker, firecracker, lima, process) = tokio::join!(
            apple.probe(),
            docker.probe(),
            firecracker.probe(),
            lima.probe(),
            process.probe()
        );
        vec![apple, docker, firecracker, lima, process]
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Process(_) => "process",
            Self::Container(c) => c.flavor_name(),
            Self::Firecracker(_) => "firecracker",
            Self::Lima(_) => "lima",
        }
    }

    /// Whether agents inside need a Linux fctl-agent binary copied in.
    pub fn needs_linux_agent(&self) -> bool {
        !matches!(self, Self::Process(_))
    }

    /// Whether sandboxes are made from images (Lima clones its base VM, and
    /// the process driver has none).
    pub fn uses_images(&self) -> bool {
        matches!(self, Self::Container(_) | Self::Firecracker(_))
    }

    /// Whether it builds images from Dockerfiles (Firecracker pulls them
    /// only).
    pub fn builds_images(&self) -> bool {
        matches!(self, Self::Container(_))
    }

    /// Whether `image` is on this host.
    pub async fn has_image(&self, image: &str) -> bool {
        match self {
            Self::Container(d) => d.has_image(image).await,
            Self::Firecracker(d) => d.has_image(image).await,
            _ => false,
        }
    }

    /// Pulls `image` from its registry; `progress` shows the CLI's own output.
    pub async fn pull(&self, image: &str, progress: bool) -> Result<()> {
        match self {
            Self::Container(d) => d.pull(image, progress).await,
            Self::Firecracker(d) => d.pull(image, progress).await,
            _ => anyhow::bail!("the {} driver does not use images", self.name()),
        }
    }

    /// Pulls `image`, or `fallback` when `image` can't be pulled and isn't
    /// here either; returns the image it pulled.
    pub async fn pull_or_fallback(
        &self,
        image: &str,
        fallback: Option<&str>,
        progress: bool,
        log: &(dyn Fn(String) + Send + Sync),
    ) -> Result<String> {
        match self {
            Self::Container(d) => d.pull_or_fallback(image, fallback, progress, log).await,
            Self::Firecracker(d) => d.pull_or_fallback(image, fallback, progress, log).await,
            _ => anyhow::bail!("the {} driver does not use images", self.name()),
        }
    }

    /// What the runtime says about `image`; `None` when it isn't here.
    pub async fn image(&self, image: &str) -> Result<Option<ImageInfo>> {
        match self {
            Self::Container(d) => d.image(image).await,
            Self::Firecracker(d) => d.image(image).await,
            _ => Ok(None),
        }
    }

    /// Gives `image` another name, `name`.
    pub async fn tag_image(&self, image: &str, name: &str) -> Result<()> {
        match self {
            Self::Container(d) => d.tag_image(image, name).await,
            Self::Firecracker(d) => d.tag_image(image, name).await,
            _ => anyhow::bail!("the {} driver does not use images", self.name()),
        }
    }

    /// The names (`repository:tag`) of `repository`'s images on this host.
    pub async fn images(&self, repository: &str) -> Result<Vec<String>> {
        match self {
            Self::Container(d) => d.images(repository).await,
            Self::Firecracker(d) => d.images(repository).await,
            _ => Ok(Vec::new()),
        }
    }

    /// Builds an image, saying in `log` what the build does.
    pub async fn build(
        &self,
        job: &BuildJob<'_>,
        log: &(dyn Fn(String) + Send + Sync),
    ) -> Result<()> {
        match self {
            Self::Container(d) => d.build(job, log).await,
            Self::Firecracker(_) => anyhow::bail!("the firecracker driver builds no images"),
            _ => anyhow::bail!("the {} driver does not use images", self.name()),
        }
    }

    /// An image's size in bytes, when the runtime says.
    pub async fn image_size(&self, image: &str) -> Option<u64> {
        match self {
            Self::Container(d) => d.image_size(image).await,
            Self::Firecracker(d) => d.image_size(image).await,
            _ => None,
        }
    }

    /// Removes the name `image`, and the image once no other name refers to
    /// it, without force: it fails while a sandbox uses the image (Docker).
    pub async fn remove_image(&self, image: &str) -> Result<()> {
        match self {
            Self::Container(d) => d.remove_image(image).await,
            Self::Firecracker(d) => d.remove_image(image).await,
            _ => Ok(()),
        }
    }

    /// Whether the sandbox `name` exists, running or not.
    pub async fn sandbox_exists(&self, name: &str) -> Result<bool> {
        match self {
            Self::Container(d) => d.exists(name).await,
            Self::Firecracker(d) => d.exists(name).await,
            _ => Ok(false),
        }
    }

    /// Starts the driver's runtime when it isn't running and the host may
    /// start it (Apple's `container` service), and logs what came of it.
    pub async fn start_service(&self) {
        let Self::Container(d) = self else { return };
        match d.start_service().await {
            Ok(true) => tracing::info!("started Apple's `container` service, which wasn't running"),
            Ok(false) => {}
            Err(e) => tracing::warn!("couldn't start Apple's `container` service: {e:#}"),
        }
    }

    pub async fn probe(&self) -> DriverInfo {
        match self {
            Self::Process(d) => d.probe().await,
            Self::Container(d) => d.probe().await,
            Self::Firecracker(d) => d.probe().await,
            Self::Lima(d) => d.probe().await,
        }
    }

    /// The CPUs and memory sandboxes run on: the Docker daemon's (on macOS
    /// the VM of OrbStack, Docker Desktop or Colima), otherwise this
    /// machine's. Memory is 0 when unknown.
    pub async fn resources(&self) -> SandboxSize {
        self.measure().await.unwrap_or_else(machine)
    }

    /// [`resources`](Self::resources), or `None` when the Docker daemon
    /// doesn't say.
    pub async fn measure(&self) -> Option<SandboxSize> {
        match self {
            Self::Container(d) if d.flavor_name() == "docker" => d.resources().await,
            _ => Some(machine()),
        }
    }

    /// Where sandboxes keep their files: Apple's `container` its app root,
    /// Lima its home, Docker its root directory (on macOS inside its VM),
    /// Firecracker and the process driver `data_dir`. `None` when the Docker
    /// daemon doesn't answer.
    pub async fn disk_spot(&self, data_dir: &Path) -> Option<DiskSpot> {
        match self {
            Self::Process(_) => Some(DiskSpot::Dir(data_dir.to_owned())),
            Self::Container(d) => d.disk_spot(data_dir).await,
            Self::Firecracker(d) => Some(d.disk_spot(data_dir)),
            Self::Lima(_) => Some(DiskSpot::Dir(
                lima::home()
                    .filter(|h| h.is_dir())
                    .unwrap_or_else(|| data_dir.to_owned()),
            )),
        }
    }

    /// Free MiB at `spot`. Inside the Docker VM it runs `df` in a
    /// short-lived container of the first of `images` that is here. `None`
    /// when it can't be measured.
    pub async fn disk_free_mb(&self, spot: &DiskSpot, images: &[String]) -> Option<u64> {
        match (spot, self) {
            (DiskSpot::Dir(dir), _) => crate::sysinfo::disk_available_mb(dir).await,
            (DiskSpot::DockerVm, Self::Container(d)) => d.vm_disk_free_mb(images).await,
            (DiskSpot::DockerVm, _) => None,
        }
    }

    /// Whether the driver holds sandboxes to their size (the process driver
    /// does not).
    pub fn enforces_size(&self) -> bool {
        !matches!(self, Self::Process(_))
    }

    /// Where `project`'s package caches are with this driver: a Docker
    /// volume, or a directory in `data_dir`, which Apple's `container`
    /// mounts and the process driver's agents use. `None` with Lima and
    /// Firecracker, whose VMs share no files with the host, and for a
    /// directory that Apple's `container` can't mount, whose path has a
    /// comma.
    pub fn cache_spot(&self, project: &str, data_dir: &Path) -> Option<Spot> {
        let dir = caches::dir(data_dir, project);
        match self {
            Self::Container(d) if d.flavor_name() == "docker" => {
                Some(Spot::Volume(caches::volume(project)))
            }
            Self::Container(_) => (!dir.to_string_lossy().contains(',')).then_some(Spot::Dir(dir)),
            Self::Process(_) => Some(Spot::Dir(dir)),
            Self::Firecracker(_) | Self::Lima(_) => None,
        }
    }

    /// Where a sandbox finds the caches at `spot`.
    pub fn cache_in_sandbox(&self, spot: &Spot) -> String {
        match (self, spot) {
            (Self::Process(_), Spot::Dir(dir)) => dir.display().to_string(),
            _ => caches::MOUNT.to_owned(),
        }
    }

    /// The projects whose package caches are on this host.
    pub async fn caches(&self, data_dir: &Path) -> Result<Vec<String>> {
        match self {
            Self::Container(d) if d.flavor_name() == "docker" => d.cache_volumes().await,
            Self::Container(_) | Self::Process(_) => Ok(caches::dirs(data_dir)),
            Self::Firecracker(_) | Self::Lima(_) => Ok(Vec::new()),
        }
    }

    /// What the caches at `spot` take, in MiB. A Docker volume is measured
    /// in a short-lived container of the first of `images` that is here.
    pub async fn cache_size_mb(&self, spot: &Spot, images: &[String]) -> Result<u64> {
        match (spot, self) {
            (Spot::Volume(volume), Self::Container(d)) => d.volume_size_mb(volume, images).await,
            (Spot::Volume(_), _) => anyhow::bail!("the {} driver keeps no volumes", self.name()),
            (Spot::Dir(dir), _) => {
                let dir = dir.clone();
                Ok(tokio::task::spawn_blocking(move || caches::dir_size_mb(&dir)).await??)
            }
        }
    }

    /// Removes the caches at `spot`, or with `keep` only what they hold: a
    /// sandbox here may mount them again. Docker keeps a volume that a
    /// container mounts, stopped ones too, and empties it in a short-lived
    /// container of the first of `images` that is here. Whether they are
    /// gone.
    pub async fn clear_cache(&self, spot: &Spot, keep: bool, images: &[String]) -> Result<bool> {
        match (spot, self) {
            (Spot::Volume(volume), Self::Container(d)) => {
                d.clear_volume(volume, keep, images).await
            }
            (Spot::Volume(_), _) => anyhow::bail!("the {} driver keeps no volumes", self.name()),
            (Spot::Dir(dir), _) => {
                let dir = dir.clone();
                tokio::task::spawn_blocking(move || caches::clear_dir(&dir, keep)).await??;
                Ok(!keep)
            }
        }
    }

    /// Creates the sandbox, or starts an existing one with the same name.
    pub async fn ensure(
        &self,
        spec: &SandboxSpec,
        log: &(dyn Fn(String) + Send + Sync),
    ) -> Result<Sandbox> {
        match self {
            Self::Process(d) => d.ensure(spec).await,
            Self::Container(d) => d.ensure(spec, log).await,
            Self::Firecracker(d) => d.ensure(spec, log).await,
            Self::Lima(d) => d.ensure(spec, log).await,
        }
    }

    /// Puts the fctl-agent binary into the sandbox (no-op for processes).
    pub async fn install_agent(&self, sb: &Sandbox, binary: &Path) -> Result<()> {
        match self {
            Self::Process(_) => Ok(()),
            Self::Container(d) => d.install_agent(sb, binary).await,
            Self::Firecracker(d) => d.install_agent(sb, binary).await,
            Self::Lima(d) => d.install_agent(sb, binary).await,
        }
    }

    /// Starts the supervisor (detached; harmless if already running).
    pub async fn start_agent(&self, sb: &Sandbox) -> Result<()> {
        match self {
            Self::Process(d) => d.start_agent(sb).await,
            Self::Container(d) => d.start_agent(sb).await,
            Self::Firecracker(d) => d.start_agent(sb).await,
            Self::Lima(d) => d.start_agent(sb).await,
        }
    }

    /// Opens a channel to the supervisor.
    pub async fn attach(&self, sb: &Sandbox) -> Result<AgentIo> {
        match self {
            Self::Process(d) => d.attach(sb).await,
            Self::Container(d) => d.attach(sb).await,
            Self::Firecracker(d) => d.attach(sb).await,
            Self::Lima(d) => d.attach(sb).await,
        }
    }

    /// Runs a command in the sandbox and returns its stdout.
    pub async fn exec(&self, sb: &Sandbox, script: &str) -> Result<String> {
        match self {
            Self::Process(d) => d.exec(sb, script).await,
            Self::Container(d) => d.exec(sb, script).await,
            Self::Firecracker(d) => d.exec(sb, script).await,
            Self::Lima(d) => d.exec(sb, script).await,
        }
    }

    /// Runs an interactive command (stdin/stdout piped).
    pub async fn spawn(&self, sb: &Sandbox, script: &str) -> Result<Running> {
        match self {
            Self::Process(d) => d.spawn(sb, script).await.map(Running::child),
            Self::Container(d) => d.spawn(sb, script).await.map(Running::child),
            Self::Firecracker(d) => d.spawn(sb, script).await,
            Self::Lima(d) => d.spawn(sb, script).await.map(Running::child),
        }
    }

    /// Whether it pauses sandboxes while the disk is nearly full: VMs whose
    /// disk is a file on the host's, which lose what they write when the
    /// host has no room for it. Docker's containers write to the host's
    /// disk, and see that it is full.
    pub fn pauses(&self) -> bool {
        matches!(self, Self::Firecracker(_))
    }

    /// Pauses a sandbox ([`pauses`](Self::pauses)).
    pub async fn pause(&self, sb: &Sandbox) -> Result<()> {
        match self {
            Self::Firecracker(d) => d.pause(sb).await,
            _ => anyhow::bail!("the {} driver doesn't pause sandboxes", self.name()),
        }
    }

    /// Resumes a paused sandbox.
    pub async fn resume(&self, sb: &Sandbox) -> Result<()> {
        match self {
            Self::Firecracker(d) => d.resume(sb).await,
            _ => anyhow::bail!("the {} driver doesn't pause sandboxes", self.name()),
        }
    }

    /// Whether a sandbox is paused.
    pub async fn paused(&self, sb: &Sandbox) -> bool {
        match self {
            Self::Firecracker(d) => d.paused(sb).await,
            _ => false,
        }
    }

    /// Lets the states that stopped sandboxes saved go, but those of
    /// `keep`, to make room on the disk (Firecracker saves its VMs' as they
    /// stop); what that freed, in MiB.
    pub async fn drop_saved_states(&self, keep: &std::collections::HashSet<String>) -> u64 {
        match self {
            Self::Firecracker(d) => d.drop_saved_states(keep).await,
            _ => 0,
        }
    }

    /// Stops the sandbox, which keeps its files. An Apple `container`
    /// sandbox first gives back the disk space of the files deleted in it.
    pub async fn stop(&self, sb: &Sandbox) -> Result<()> {
        match self {
            Self::Process(d) => d.stop(sb).await,
            Self::Container(d) => d.stop(sb).await,
            Self::Firecracker(d) => d.stop(sb).await,
            Self::Lima(d) => d.stop(sb).await,
        }
    }

    /// Stops the sandbox, and with `destroy` removes it; what each came to.
    /// A Firecracker VM that goes isn't stopped first, which would save the
    /// state of a VM nothing resumes; others are, as Apple's `container`
    /// sometimes needs its stop's second try.
    pub async fn finish(&self, sb: &Sandbox, destroy: bool) -> (Result<()>, Result<()>) {
        if destroy && matches!(self, Self::Firecracker(_)) {
            return (Ok(()), self.remove(sb).await);
        }
        let stopped = self.stop(sb).await;
        let removed = if destroy {
            self.remove(sb).await
        } else {
            Ok(())
        };
        (stopped, removed)
    }

    pub async fn remove(&self, sb: &Sandbox) -> Result<()> {
        match self {
            Self::Process(d) => d.remove(sb).await,
            Self::Container(d) => d.remove(sb).await,
            Self::Firecracker(d) => d.remove(sb).await,
            Self::Lima(d) => d.remove(sb).await,
        }
    }

    /// Sandboxes this host created (names starting with `fctl-`).
    pub async fn list(&self) -> Result<Vec<SandboxInfo>> {
        match self {
            Self::Process(d) => d.list().await,
            Self::Container(d) => d.list().await,
            Self::Firecracker(d) => d.list().await,
            Self::Lima(d) => d.list().await,
        }
    }

    /// A handle for an existing sandbox by name.
    pub fn handle(&self, name: &str) -> Sandbox {
        match self {
            Self::Process(d) => d.handle(name),
            _ => Sandbox {
                name: name.to_owned(),
                root: None,
            },
        }
    }

    /// The workdir path agents see.
    pub fn workdir(&self, sb: &Sandbox) -> String {
        match self {
            Self::Process(_) => sb.root.as_ref().map_or_else(
                || "~/workspace".into(),
                |r| r.join("home/workspace").display().to_string(),
            ),
            _ => "~/workspace".into(),
        }
    }

    /// HOME for agents, when the host decides it.
    pub fn home(&self, sb: &Sandbox) -> Option<String> {
        match self {
            Self::Process(_) => sb
                .root
                .as_ref()
                .map(|r| r.join("home").display().to_string()),
            _ => None,
        }
    }
}

/// This machine's CPUs and memory.
pub fn machine() -> SandboxSize {
    SandboxSize {
        cpus: crate::sysinfo::cpus(),
        memory_mb: u32::try_from(crate::sysinfo::memory_mb()).unwrap_or(u32::MAX),
    }
}

/// Runs a host command, capturing output; errors carry stderr. One still
/// running after [`COMMAND_TIMEOUT`] is killed.
pub async fn run_cmd(program: &str, args: &[&str]) -> Result<String> {
    run_cmd_within(program, args, COMMAND_TIMEOUT).await
}

/// [`run_cmd`] with a time limit of its own.
pub async fn run_cmd_within(program: &str, args: &[&str], limit: Duration) -> Result<String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).stdin(Stdio::null());
    let out = output_within(&mut cmd, program, || command_line(program, args), limit).await?;
    if !out.status.success() {
        anyhow::bail!(
            "{program} {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Runs `cmd` to its end and captures its output. One still running after
/// `limit` is killed, and the host logs which (`what`).
pub async fn output_within(
    cmd: &mut tokio::process::Command,
    program: &str,
    what: impl FnOnce() -> String,
    limit: Duration,
) -> Result<std::process::Output> {
    // Dropping the unfinished `output()` kills the process.
    let run = cmd.kill_on_drop(true).output();
    match tokio::time::timeout(limit, run).await {
        Ok(out) => out.map_err(|e| spawn_error(program, e)),
        Err(_) => {
            let e = TimedOut {
                command: what(),
                after: limit,
            };
            tracing::warn!("{e}");
            Err(e.into())
        }
    }
}

/// A driver command that ran past its time limit, and was killed.
#[derive(Debug)]
pub struct TimedOut {
    pub command: String,
    pub after: Duration,
}

impl std::fmt::Display for TimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let after = match self.after.as_secs() {
            0 => format!("{} ms", self.after.as_millis()),
            s if s >= 120 && s % 60 == 0 => format!("{} minutes", s / 60),
            s => format!("{s} seconds"),
        };
        write!(
            f,
            "`{}` did not finish within {after}, so the host stopped it",
            self.command
        )
    }
}

impl std::error::Error for TimedOut {}

/// Whether `r` failed because its command ran out of time.
pub fn timed_out<T>(r: &Result<T>) -> bool {
    r.as_ref().is_err_and(|e| e.is::<TimedOut>())
}

/// A driver command that couldn't be started: its program is missing, or
/// the system wouldn't run it.
#[derive(Debug)]
pub struct NotStarted {
    pub program: String,
    pub error: std::io::Error,
}

impl std::fmt::Display for NotStarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.error.kind() == std::io::ErrorKind::NotFound {
            write!(
                f,
                "`{}` is not installed (not found on the PATH)",
                self.program
            )
        } else {
            write!(f, "running {}: {}", self.program, self.error)
        }
    }
}

impl std::error::Error for NotStarted {}

/// Whether `e` leaves what its command asked unanswered: the command
/// couldn't be started, ran out of time, or couldn't reach the Docker
/// daemon. Any other failure is the runtime's answer, like "No such image".
pub fn unanswered(e: &anyhow::Error) -> bool {
    e.is::<NotStarted>() || e.is::<TimedOut>() || e.is::<crate::docker_access::Unreachable>()
}

/// A command as the log shows it: arguments with spaces quoted, and long
/// ones (scripts) cut short.
pub(crate) fn command_line(program: &str, args: &[&str]) -> String {
    let mut line = program.to_owned();
    for a in args {
        line.push(' ');
        if a.is_empty() || a.contains(char::is_whitespace) {
            line.push_str(&format!("'{a}'"));
        } else {
            line.push_str(a);
        }
    }
    match line.char_indices().nth(160) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line,
    }
}

/// Why `program` could not be started, naming a missing program plainly.
pub fn spawn_error(program: &str, e: std::io::Error) -> anyhow::Error {
    NotStarted {
        program: program.to_owned(),
        error: e,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_missing_program_is_named() {
        let e = run_cmd("fctl-test-missing-program", &["--version"])
            .await
            .err();
        assert!(e.as_ref().is_some_and(unanswered));
        assert_eq!(
            e.map(|e| e.to_string()).as_deref(),
            Some("`fctl-test-missing-program` is not installed (not found on the PATH)")
        );
    }

    #[tokio::test]
    async fn a_command_that_hangs_is_killed() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pid = dir.path().join("pid");
        let script = format!("echo $$ > '{}'; exec sleep 30", pid.display());
        let started = std::time::Instant::now();
        let r = run_cmd_within("sh", &["-c", &script], Duration::from_millis(500)).await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(timed_out(&r));
        assert!(r.as_ref().is_err_and(unanswered));
        let e = r.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            e.starts_with(&format!("`sh -c 'echo $$ > '{}'", pid.display())),
            "{e}"
        );
        assert!(
            e.ends_with("did not finish within 500 ms, so the host stopped it"),
            "{e}"
        );
        // Gone, or a zombie until the runtime reaps it.
        let pid = std::fs::read_to_string(&pid)?;
        let mut stat = String::new();
        for _ in 0..20 {
            let ps = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", pid.trim()])
                .output()?;
            stat = String::from_utf8_lossy(&ps.stdout).trim().to_owned();
            if stat.is_empty() || stat.starts_with('Z') {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        anyhow::bail!("sleep {} still runs: {stat}", pid.trim())
    }

    #[tokio::test]
    async fn quick_commands_keep_their_output_and_errors() {
        let ok = run_cmd("sh", &["-c", "echo hi"]).await.ok();
        assert_eq!(ok.as_deref(), Some("hi\n"));
        let failed = run_cmd("sh", &["-c", "echo nope >&2; exit 3"]).await;
        assert!(failed.as_ref().is_err_and(|e| !unanswered(e)));
        let e = failed.err().map(|e| e.to_string());
        assert_eq!(e.as_deref(), Some("sh -c failed: nope"));
    }

    #[test]
    fn long_commands_are_cut_short_in_the_log() {
        assert_eq!(
            command_line("docker", &["ps", "--format", "{{.Names}}\t{{.State}}"]),
            "docker ps --format '{{.Names}}\t{{.State}}'"
        );
        let script = "x".repeat(300);
        let line = command_line("sh", &["-c", &script]);
        assert_eq!(line.chars().count(), 161);
        assert!(line.ends_with('…'));
        let e = TimedOut {
            command: "docker pull x".into(),
            after: SLOW_COMMAND_TIMEOUT,
        };
        assert_eq!(
            e.to_string(),
            "`docker pull x` did not finish within 30 minutes, so the host stopped it"
        );
    }

    #[test]
    fn only_a_clear_answer_says_where_fctl_agent_ran() {
        assert!(ran_as_root("root\n").is_ok_and(|r| r));
        assert!(ran_as_root("user\n").is_ok_and(|r| !r));
        assert!(ran_as_root("").is_err());
        assert!(ran_as_root("1000:1000").is_err());
    }

    #[test]
    fn names_roundtrip() {
        let n = sandbox_name("ses_01M553XWYGXA3FE9YKD72G5Y8T");
        assert_eq!(n, "fctl-ses-01m553xwygxa3fe9ykd72g5y8t");
        assert_eq!(
            session_of(&n).as_deref(),
            Some("ses_01M553XWYGXA3FE9YKD72G5Y8T")
        );
        assert_eq!(session_of("fctl-login-x"), None);
        assert_eq!(login_name("lgn_01ABC"), "fctl-login-lgn-01abc");
    }

    #[test]
    fn names_stay_one_name_whatever_the_control_plane_sends() {
        for id in [
            "ses_/../../etc",
            "ses_..",
            "lgn_a b\\c",
            "ses_x\0y",
            "ses_é",
        ] {
            for name in [sandbox_name(id), login_name(id)] {
                assert!(
                    name.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                    "{name}"
                );
            }
        }
    }
}
