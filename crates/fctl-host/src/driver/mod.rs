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
pub mod lima;
pub mod process;

use anyhow::Result;
use fctl_proto::runner::DriverInfo;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncRead, AsyncWrite};

/// What to create.
#[derive(Clone, Debug)]
pub struct SandboxSpec {
    pub name: String,
    pub image: String,
    pub cpus: u32,
    pub memory_mb: u32,
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
        let mut out = vec![
            container::ContainerDriver::apple().probe().await,
            container::ContainerDriver::docker().probe().await,
            lima::LimaDriver::new().probe().await,
        ];
        out.push(process::ProcessDriver::new(data_dir, None).probe().await);
        out
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

    pub async fn probe(&self) -> DriverInfo {
        match self {
            Self::Process(d) => d.probe().await,
            Self::Container(d) => d.probe().await,
            Self::Lima(d) => d.probe().await,
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

/// Runs a host command, capturing output; errors carry stderr.
pub async fn run_cmd(program: &str, args: &[&str]) -> Result<String> {
    let out = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("running {program}: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "{program} {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

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
