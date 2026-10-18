//! The process driver: a directory and a process per session, no isolation.
//! For development, CI and hosts that are themselves disposable VMs.

use super::{AgentIo, Sandbox, SandboxInfo, SandboxSpec};
use anyhow::{Context, Result};
use fctl_proto::runner::DriverInfo;
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::net::UnixStream;

pub struct ProcessDriver {
    root: PathBuf,
    agent_bin: Option<PathBuf>,
}

impl ProcessDriver {
    pub fn new(data_dir: &Path, agent_bin: Option<PathBuf>) -> Self {
        Self {
            root: data_dir.join("sandboxes"),
            agent_bin,
        }
    }

    /// fctl-agent next to the running executable (or configured).
    fn agent(&self) -> Result<PathBuf> {
        if let Some(p) = &self.agent_bin {
            return Ok(p.clone());
        }
        let exe = std::env::current_exe()?;
        let sibling = exe.with_file_name("fctl-agent");
        if sibling.exists() {
            return Ok(sibling);
        }
        which("fctl-agent").context("fctl-agent was not found next to fctl or on PATH")
    }

    pub async fn probe(&self) -> DriverInfo {
        let agent = self.agent();
        DriverInfo {
            name: "process".into(),
            available: agent.is_ok(),
            isolation: "none".into(),
            version: None,
            detail: Some(match agent {
                Ok(p) => format!("agents run as local processes ({})", p.display()),
                Err(e) => e.to_string(),
            }),
        }
    }

    pub fn handle(&self, name: &str) -> Sandbox {
        Sandbox {
            name: name.to_owned(),
            root: Some(self.root.join(name)),
        }
    }

    pub async fn ensure(&self, spec: &SandboxSpec) -> Result<Sandbox> {
        let sb = self.handle(&spec.name);
        let root = sb.root.clone().context("process sandbox without a root")?;
        std::fs::create_dir_all(root.join("home/workspace"))?;
        std::fs::create_dir_all(root.join("home/.fctl"))?;
        Ok(sb)
    }

    /// HOME and the agent socket of a sandbox. Unix socket paths are limited
    /// to ~104 bytes (macOS), so sockets live in a short private directory
    /// under /tmp, named by a hash of the sandbox root.
    fn paths(sb: &Sandbox) -> Result<(PathBuf, PathBuf)> {
        let root = sb.root.clone().context("process sandbox without a root")?;
        let digest = hex::encode(sha2::Sha256::digest(root.display().to_string().as_bytes()));
        let user = std::env::var("USER").unwrap_or_else(|_| "fctl".into());
        let user: String = user
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(16)
            .collect();
        let dir = PathBuf::from(format!("/tmp/fctl-{user}"));
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok((
            root.join("home"),
            dir.join(format!("{}.sock", &digest[..16])),
        ))
    }

    pub async fn start_agent(&self, sb: &Sandbox) -> Result<()> {
        let (home, socket) = Self::paths(sb)?;
        if UnixStream::connect(&socket).await.is_ok() {
            return Ok(());
        }
        let agent = self.agent()?;
        let mut cmd = std::process::Command::new(&agent);
        cmd.args(["serve", "--socket"])
            .arg(&socket)
            .arg("--state")
            .arg(home.join(".fctl/state"))
            .arg("--log")
            .arg(home.join(".fctl/agent.log"))
            .env("HOME", &home)
            .current_dir(&home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Agents find this fctl first: `fctl edit` names their session.
        if let Some(path) = path_with_own_dir() {
            cmd.env("PATH", path);
        }
        // Its own process group: restarting the host (launchd, systemd) must not kill agents.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let child = cmd
            .spawn()
            .with_context(|| format!("starting {}", agent.display()))?;
        std::fs::write(home.join(".fctl/agent.pid"), child.id().to_string())?;
        // Reap it in the background so it never lingers as a zombie.
        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });
        Ok(())
    }

    pub async fn attach(&self, sb: &Sandbox) -> Result<AgentIo> {
        let (_, socket) = Self::paths(sb)?;
        let mut last = None;
        for _ in 0..100 {
            match UnixStream::connect(&socket).await {
                Ok(s) => {
                    let (r, w) = s.into_split();
                    return Ok(AgentIo {
                        reader: Box::new(r),
                        writer: Box::new(w),
                        child: None,
                    });
                }
                Err(e) => last = Some(e),
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(anyhow::anyhow!(
            "fctl-agent did not come up at {}: {:?}",
            socket.display(),
            last
        ))
    }

    pub async fn exec(&self, sb: &Sandbox, script: &str) -> Result<String> {
        let (home, _) = Self::paths(sb)?;
        let out = tokio::process::Command::new("sh")
            .args(["-c", script])
            .env("HOME", &home)
            .current_dir(&home)
            .output()
            .await?;
        if !out.status.success() {
            anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub async fn spawn(&self, sb: &Sandbox, script: &str) -> Result<tokio::process::Child> {
        let (home, _) = Self::paths(sb)?;
        Ok(tokio::process::Command::new("sh")
            .args(["-c", script])
            .env("HOME", &home)
            .env_remove("BROWSER")
            .current_dir(&home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?)
    }

    pub async fn stop(&self, sb: &Sandbox) -> Result<()> {
        let (home, _) = Self::paths(sb)?;
        if let Ok(pid) = std::fs::read_to_string(home.join(".fctl/agent.pid")) {
            let pid = pid.trim();
            if !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()) {
                let _ = tokio::process::Command::new("kill").arg(pid).status().await;
            }
        }
        Ok(())
    }

    pub async fn remove(&self, sb: &Sandbox) -> Result<()> {
        self.stop(sb).await?;
        if let Some(root) = &sb.root
            && root.starts_with(&self.root)
        {
            let _ = std::fs::remove_dir_all(root);
        }
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<SandboxInfo>> {
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir(&self.root) else {
            return Ok(out);
        };
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("fctl-") {
                continue;
            }
            let sb = Sandbox {
                name: name.clone(),
                root: Some(entry.path()),
            };
            let running = match Self::paths(&sb) {
                Ok((_, socket)) => UnixStream::connect(&socket).await.is_ok(),
                Err(_) => false,
            };
            out.push(SandboxInfo { name, running });
        }
        Ok(out)
    }
}

/// PATH with the running executable's directory in front.
fn path_with_own_dir() -> Option<std::ffi::OsString> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(std::iter::once(dir).chain(std::env::split_paths(&path))).ok()
}

/// Finds a program on PATH.
pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
}
