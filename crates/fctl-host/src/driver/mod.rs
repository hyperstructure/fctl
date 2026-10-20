//! Sandbox drivers: turn a session into a running Linux sandbox and give the
//! host a channel to its fctl-agent.
//!
//! | Driver | Isolation | Notes |
//! |---|---|---|
//! | `apple` | a lightweight VM per session | Apple `container`, Apple silicon, macOS 26+ |
//! | `docker` | a container per session | Docker, OrbStack, Colima, Podman (`docker` CLI) |
//! | `lima` | a VM per session, cloned from `fctl-base` | experimental |
//! | `process` | none | development and trusted hosts |

pub mod container;
#[cfg(test)]
pub(crate) mod fake;
pub mod lima;
pub mod process;

pub use container::ImageInfo;

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

/// A byte channel to fctl-agent.
pub struct AgentIo {
    pub reader: Box<dyn AsyncRead + Unpin + Send>,
    pub writer: Box<dyn AsyncWrite + Unpin + Send>,
    /// The `exec` process carrying the channel, if any.
    pub child: Option<tokio::process::Child>,
}

/// The sandbox name for a session id: `fctl-ses-01ab…` (lowercase, dashes).
pub fn sandbox_name(session_id: &str) -> String {
    format!("fctl-{}", session_id.to_lowercase().replace('_', "-"))
}

/// The session id back from a sandbox name.
pub fn session_of(name: &str) -> Option<String> {
    let rest = name.strip_prefix("fctl-ses-")?;
    Some(format!("ses_{}", rest.to_uppercase()))
}

/// Where fctl-agent lives inside container and VM sandboxes.
pub const AGENT_PATH: &str = "/opt/fctl/bin/fctl-agent";

/// The shell snippet that starts the supervisor inside a sandbox.
pub fn serve_script(agent: &str) -> String {
    format!(
        "mkdir -p \"$HOME/.fctl\" && exec {agent} serve --socket \"$HOME/.fctl/agent.sock\" --state \"$HOME/.fctl/state\" --log \"$HOME/.fctl/agent.log\""
    )
}

pub fn attach_script(agent: &str) -> String {
    format!("exec {agent} attach --socket \"$HOME/.fctl/agent.sock\"")
}

pub enum Driver {
    Process(process::ProcessDriver),
    Container(container::ContainerDriver),
    Lima(lima::LimaDriver),
}

impl Driver {
    /// The driver named in the configuration (`auto` picks the best available).
    pub async fn select(name: &str, data_dir: &Path, agent_bin: Option<PathBuf>) -> Result<Self> {
        let pick = |n: &str| -> Option<Driver> {
            match n {
                "process" => Some(Driver::Process(process::ProcessDriver::new(
                    data_dir,
                    agent_bin.clone(),
                ))),
                "apple" => Some(Driver::Container(container::ContainerDriver::apple())),
                "docker" => Some(Driver::Container(container::ContainerDriver::docker())),
                "lima" => Some(Driver::Lima(lima::LimaDriver::new())),
                _ => None,
            }
        };
        if name != "auto" {
            return pick(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown driver `{name}` (use apple, docker, lima, process or auto)"
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

    /// Every driver this host could use, for the runner's hello.
    pub async fn probe_all(data_dir: &Path) -> Vec<DriverInfo> {
        let (apple, docker, lima, process) = (
            container::ContainerDriver::apple(),
            container::ContainerDriver::docker(),
            lima::LimaDriver::new(),
            process::ProcessDriver::new(data_dir, None),
        );
        let (apple, docker, lima, process) =
            tokio::join!(apple.probe(), docker.probe(), lima.probe(), process.probe());
        vec![apple, docker, lima, process]
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Process(_) => "process",
            Self::Container(c) => c.flavor_name(),
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
        matches!(self, Self::Container(_))
    }

    /// Whether `image` is on this host.
    pub async fn has_image(&self, image: &str) -> bool {
        match self {
            Self::Container(d) => d.has_image(image).await,
            _ => false,
        }
    }

    /// Pulls `image` from its registry; `progress` shows the CLI's own output.
    pub async fn pull(&self, image: &str, progress: bool) -> Result<()> {
        match self {
            Self::Container(d) => d.pull(image, progress).await,
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
            _ => anyhow::bail!("the {} driver does not use images", self.name()),
        }
    }

    /// What the runtime says about `image`; `None` when it isn't here.
    pub async fn image(&self, image: &str) -> Result<Option<ImageInfo>> {
        match self {
            Self::Container(d) => d.image(image).await,
            _ => Ok(None),
        }
    }

    /// Gives `image` another name, `name`.
    pub async fn tag_image(&self, image: &str, name: &str) -> Result<()> {
        match self {
            Self::Container(d) => d.tag_image(image, name).await,
            _ => anyhow::bail!("the {} driver does not use images", self.name()),
        }
    }

    /// The names (`repository:tag`) of `repository`'s images on this host.
    pub async fn images(&self, repository: &str) -> Result<Vec<String>> {
        match self {
            Self::Container(d) => d.images(repository).await,
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
            _ => anyhow::bail!("the {} driver does not use images", self.name()),
        }
    }

    /// An image's size in bytes, when the runtime says.
    pub async fn image_size(&self, image: &str) -> Option<u64> {
        match self {
            Self::Container(d) => d.image_size(image).await,
            _ => None,
        }
    }

    /// Removes the name `image`, and the image once no other name refers to
    /// it, without force: it fails while a sandbox uses the image (Docker).
    pub async fn remove_image(&self, image: &str) -> Result<()> {
        match self {
            Self::Container(d) => d.remove_image(image).await,
            _ => Ok(()),
        }
    }

    /// Whether the sandbox `name` exists, running or not.
    pub async fn sandbox_exists(&self, name: &str) -> Result<bool> {
        match self {
            Self::Container(d) => d.exists(name).await,
            _ => Ok(false),
        }
    }

    pub async fn probe(&self) -> DriverInfo {
        match self {
            Self::Process(d) => d.probe().await,
            Self::Container(d) => d.probe().await,
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
    /// and the process driver `data_dir`. `None` when the Docker daemon
    /// doesn't answer.
    pub async fn disk_spot(&self, data_dir: &Path) -> Option<DiskSpot> {
        match self {
            Self::Process(_) => Some(DiskSpot::Dir(data_dir.to_owned())),
            Self::Container(d) => d.disk_spot(data_dir).await,
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

    /// Creates the sandbox, or starts an existing one with the same name.
    pub async fn ensure(
        &self,
        spec: &SandboxSpec,
        log: &(dyn Fn(String) + Send + Sync),
    ) -> Result<Sandbox> {
        match self {
            Self::Process(d) => d.ensure(spec).await,
            Self::Container(d) => d.ensure(spec, log).await,
            Self::Lima(d) => d.ensure(spec, log).await,
        }
    }

    /// Puts the fctl-agent binary into the sandbox (no-op for processes).
    pub async fn install_agent(&self, sb: &Sandbox, binary: &Path) -> Result<()> {
        match self {
            Self::Process(_) => Ok(()),
            Self::Container(d) => d.install_agent(sb, binary).await,
            Self::Lima(d) => d.install_agent(sb, binary).await,
        }
    }

    /// Starts the supervisor (detached; harmless if already running).
    pub async fn start_agent(&self, sb: &Sandbox) -> Result<()> {
        match self {
            Self::Process(d) => d.start_agent(sb).await,
            Self::Container(d) => d.start_agent(sb).await,
            Self::Lima(d) => d.start_agent(sb).await,
        }
    }

    /// Opens a channel to the supervisor.
    pub async fn attach(&self, sb: &Sandbox) -> Result<AgentIo> {
        match self {
            Self::Process(d) => d.attach(sb).await,
            Self::Container(d) => d.attach(sb).await,
            Self::Lima(d) => d.attach(sb).await,
        }
    }

    /// Runs a command in the sandbox and returns its stdout.
    pub async fn exec(&self, sb: &Sandbox, script: &str) -> Result<String> {
        match self {
            Self::Process(d) => d.exec(sb, script).await,
            Self::Container(d) => d.exec(sb, script).await,
            Self::Lima(d) => d.exec(sb, script).await,
        }
    }

    /// Runs an interactive command (stdin/stdout piped).
    pub async fn spawn(&self, sb: &Sandbox, script: &str) -> Result<tokio::process::Child> {
        match self {
            Self::Process(d) => d.spawn(sb, script).await,
            Self::Container(d) => d.spawn(sb, script).await,
            Self::Lima(d) => d.spawn(sb, script).await,
        }
    }

    /// Stops the sandbox, which keeps its files. An Apple `container`
    /// sandbox first gives back the disk space of the files deleted in it.
    pub async fn stop(&self, sb: &Sandbox) -> Result<()> {
        match self {
            Self::Process(d) => d.stop(sb).await,
            Self::Container(d) => d.stop(sb).await,
            Self::Lima(d) => d.stop(sb).await,
        }
    }

    pub async fn remove(&self, sb: &Sandbox) -> Result<()> {
        match self {
            Self::Process(d) => d.remove(sb).await,
            Self::Container(d) => d.remove(sb).await,
            Self::Lima(d) => d.remove(sb).await,
        }
    }

    /// Sandboxes this host created (names starting with `fctl-`).
    pub async fn list(&self) -> Result<Vec<SandboxInfo>> {
        match self {
            Self::Process(d) => d.list().await,
            Self::Container(d) => d.list().await,
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
/// couldn't be started, or ran out of time. Any other failure is the
/// runtime's answer, like "No such image".
pub fn unanswered(e: &anyhow::Error) -> bool {
    e.is::<NotStarted>() || e.is::<TimedOut>()
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
    fn names_roundtrip() {
        let n = sandbox_name("ses_01M553XWYGXA3FE9YKD72G5Y8T");
        assert_eq!(n, "fctl-ses-01m553xwygxa3fe9ykd72g5y8t");
        assert_eq!(
            session_of(&n).as_deref(),
            Some("ses_01M553XWYGXA3FE9YKD72G5Y8T")
        );
        assert_eq!(session_of("fctl-login-x"), None);
    }
}
