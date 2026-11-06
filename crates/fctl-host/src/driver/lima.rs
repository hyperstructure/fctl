//! The Lima driver (experimental): one VM per session, cloned from a
//! prepared base VM named `fctl-base` (`fctl host image build --driver lima`).
//! Heavier than containers, but a full VM on any Mac (Intel too) or Linux host.

use super::{
    AGENT_DIR, AGENT_DIR_SCRIPT, AGENT_PATH, AgentIo, COMMAND_TIMEOUT, SLOW_COMMAND_TIMEOUT,
    Sandbox, SandboxInfo, SandboxSpec, attach_as_root, attach_script, run_cmd_within,
    serve_as_root, serve_script, shell_words,
};
use anyhow::{Context, Result};
use fctl_proto::runner::DriverInfo;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub const BASE_VM: &str = "fctl-base";

/// What every VM fctl makes gets over Lima's template: none of its mounts,
/// which give the VM the host's home directory (with its fctl tokens and
/// SSH keys) and a `/tmp/lima` every VM writes to, and no port forwards,
/// which would put a session's services on the host's loopback, where the
/// other VMs reach them (`host.lima.internal`). The agent comes in with
/// `limactl copy` and runs through `limactl shell`, over SSH, which these
/// leave alone. A yq expression for `--set`: `limactl` takes only the last
/// `--set` it is given, so one expression holds them all.
pub const ISOLATED: &str = r#".mounts = [] | .portForwards = [{"guestIP": "0.0.0.0", "guestPortRange": [1, 65535], "proto": "any", "ignore": true}]"#;

/// The settings of a session's VM: its size, and [`ISOLATED`].
fn session_settings(cpus: u32, memory_mb: u32) -> String {
    format!(
        ".cpus = {} | .memory = \"{}MiB\" | {ISOLATED}",
        cpus.max(1),
        memory_mb.max(1024)
    )
}

/// Where Lima keeps its VMs and their disks: `$LIMA_HOME`, or `~/.lima`.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("LIMA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".lima")))
}

pub struct LimaDriver {
    bin: String,
    /// Whether it starts fctl-agent as root (`agent_as_root` in host.toml).
    agent_as_root: bool,
}

impl LimaDriver {
    pub fn new() -> Self {
        Self {
            bin: std::env::var("FCTL_LIMACTL_BIN").unwrap_or_else(|_| "limactl".into()),
            agent_as_root: true,
        }
    }

    /// The driver, starting fctl-agent as root (`on`, the default) or as
    /// the VM's user.
    pub fn agent_as_root(self, on: bool) -> Self {
        Self {
            agent_as_root: on,
            ..self
        }
    }

    async fn run(&self, args: &[&str]) -> Result<String> {
        // Cloning, booting and shutting down a VM take minutes.
        let limit = match args.first() {
            Some(&("clone" | "start" | "stop")) => SLOW_COMMAND_TIMEOUT,
            _ => COMMAND_TIMEOUT,
        };
        run_cmd_within(&self.bin, args, limit).await
    }

    async fn instances(&self) -> Vec<(String, String)> {
        let Ok(out) = self.run(&["list", "--format", "json"]).await else {
            return vec![];
        };
        // One JSON object per line.
        out.lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| {
                Some((
                    v.get("name")?.as_str()?.to_owned(),
                    v.get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown")
                        .to_owned(),
                ))
            })
            .collect()
    }

    pub async fn probe(&self) -> DriverInfo {
        let version = self
            .run(&["--version"])
            .await
            .ok()
            .map(|s| s.trim().to_owned());
        let has_base = self.instances().await.iter().any(|(n, _)| n == BASE_VM);
        DriverInfo {
            name: "lima".into(),
            available: version.is_some() && has_base,
            isolation: "vm".into(),
            detail: Some(match (version.is_some(), has_base) {
                (false, _) => "limactl is not installed (install Lima: https://lima-vm.io)".into(),
                (true, false) => {
                    format!("no {BASE_VM} VM yet (run `fctl host image build --driver lima`)")
                }
                (true, true) => "ready".into(),
            }),
            version,
        }
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
        let all = self.instances().await;
        match all.iter().find(|(n, _)| n == &spec.name) {
            Some((_, status)) if status == "Running" => return Ok(sb),
            Some(_) => {
                // A VM cloned before fctl cleared Lima's mounts would boot
                // with the host's home directory: it boots without them, or
                // a new one takes its place.
                match self
                    .run(&["edit", "--tty=false", "--set", ISOLATED, &spec.name])
                    .await
                {
                    Ok(_) => {
                        log(format!("Starting VM {}", spec.name));
                        self.run(&["start", "--tty=false", &spec.name]).await?;
                        return Ok(sb);
                    }
                    Err(e) => {
                        log(format!(
                            "Replacing VM {}, whose mounts couldn't be removed: {e:#}",
                            spec.name
                        ));
                        self.run(&["delete", "--force", &spec.name]).await?;
                    }
                }
            }
            None => {}
        }
        if !all.iter().any(|(n, _)| n == BASE_VM) {
            anyhow::bail!(
                "the {BASE_VM} VM does not exist: run `fctl host image build --driver lima`"
            );
        }
        log(format!("Cloning {BASE_VM} into {}", spec.name));
        let settings = session_settings(spec.cpus, spec.memory_mb);
        self.run(&[
            "clone",
            "--tty=false",
            "--set",
            &settings,
            BASE_VM,
            &spec.name,
        ])
        .await?;
        log(format!("Booting {}", spec.name));
        self.run(&["start", "--tty=false", &spec.name]).await?;
        Ok(sb)
    }

    pub async fn install_agent(&self, sb: &Sandbox, binary: &Path) -> Result<()> {
        let tmp = "/tmp/fctl-agent.new";
        self.run(&[
            "copy",
            &binary.display().to_string(),
            &format!("{}:{tmp}", sb.name),
        ])
        .await
        .context("copying fctl-agent into the VM")?;
        self.exec(
            sb,
            &format!(
                "sudo sh -c '{AGENT_DIR_SCRIPT}' && sudo install -m 755 {tmp} {AGENT_PATH} && rm -f {tmp}"
            ),
        )
        .await?;
        Ok(())
    }

    /// Starts fctl-agent as root with `sudo`, and Claude and its commands
    /// as the VM's user, who can't stop it then; as that user where `sudo`
    /// asks for a password.
    pub async fn start_agent(&self, sb: &Sandbox) -> Result<()> {
        // One that ran as root before keeps doing so, rather than start
        // another beside one that may run.
        let wanted =
            self.agent_as_root || self.exec(sb, &format!("test -d {AGENT_DIR}")).await.is_ok();
        let root = wanted && self.exec(sb, "sudo -n true").await.is_ok();
        if !wanted {
            tracing::info!(
                "fctl-agent runs as {}'s user (agent_as_root = false), so the agent's commands can stop it",
                sb.name
            );
        } else if !root {
            tracing::warn!(
                "fctl-agent runs as {}'s user, so the agent's commands can stop it: sudo asks for a password there",
                sb.name
            );
        }
        self.exec(sb, &serve_script_for_lima(root)).await?;
        Ok(())
    }

    pub async fn attach(&self, sb: &Sandbox) -> Result<AgentIo> {
        let mut child = self.spawn(sb, &attach_script_for_lima()).await?;
        let stdout = child.stdout.take().context("shell stdout")?;
        let stdin = child.stdin.take().context("shell stdin")?;
        Ok(AgentIo {
            reader: Box::new(stdout),
            writer: Box::new(stdin),
            child: Some(child),
        })
    }

    pub async fn exec(&self, sb: &Sandbox, script: &str) -> Result<String> {
        self.run(&[
            "shell",
            "--workdir",
            "/",
            &sb.name,
            "--",
            "sh",
            "-c",
            script,
        ])
        .await
    }

    pub async fn spawn(&self, sb: &Sandbox, script: &str) -> Result<tokio::process::Child> {
        Ok(tokio::process::Command::new(&self.bin)
            .args([
                "shell",
                "--workdir",
                "/",
                &sb.name,
                "--",
                "sh",
                "-c",
                script,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?)
    }

    pub async fn stop(&self, sb: &Sandbox) -> Result<()> {
        let _ = self.run(&["stop", &sb.name]).await;
        Ok(())
    }

    pub async fn remove(&self, sb: &Sandbox) -> Result<()> {
        let _ = self.run(&["delete", "--force", &sb.name]).await;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<SandboxInfo>> {
        Ok(self
            .instances()
            .await
            .into_iter()
            .filter(|(n, _)| n.starts_with("fctl-") && n != BASE_VM)
            .map(|(name, status)| SandboxInfo {
                name,
                running: status == "Running",
            })
            .collect())
    }
}

/// Starts fctl-agent in the background: as root with `sudo` (`root`), or
/// as the VM's user.
fn serve_script_for_lima(root: bool) -> String {
    if root {
        // sudo resets the PATH, which Claude and its commands need.
        let serve = shell_words(&serve_as_root("USER_ID")).replace("'USER_ID'", "\"$(id -u)\"");
        format!("setsid sudo -n env \"PATH=$PATH\" {serve} >/dev/null 2>&1 < /dev/null &")
    } else {
        format!(
            "setsid sh -c '{}' >/dev/null 2>&1 < /dev/null &",
            serve_script(AGENT_PATH).replace('\'', "'\\''")
        )
    }
}

/// Attaches to fctl-agent as root where it runs so, as the VM's user where
/// it runs as the user: without `sudo` in a VM where it never ran as root.
fn attach_script_for_lima() -> String {
    let socket = format!("{AGENT_DIR}/agent.sock");
    format!(
        "if [ -d {AGENT_DIR} ] && sudo -n test -S {socket} 2>/dev/null; then exec sudo -n {}; fi; {}",
        shell_words(&attach_as_root()),
        attach_script(AGENT_PATH)
    )
}

impl Default for LimaDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fctl_agent_runs_as_root_where_sudo_lets_it() {
        assert_eq!(
            serve_script_for_lima(true),
            "setsid sudo -n env \"PATH=$PATH\" '/opt/fctl/bin/fctl-agent' 'serve' '--user' \"$(id -u)\" '--socket' '/var/lib/fctl-agent/agent.sock' '--state' '/var/lib/fctl-agent/state' '--log' '/var/lib/fctl-agent/agent.log' >/dev/null 2>&1 < /dev/null &"
        );
        assert!(serve_script_for_lima(false).starts_with(
            "setsid sh -c 'mkdir -p \"$HOME/.fctl\" && exec /opt/fctl/bin/fctl-agent serve --socket \"$HOME/.fctl/agent.sock\""
        ));
        assert_eq!(
            attach_script_for_lima(),
            "if [ -d /var/lib/fctl-agent ] && sudo -n test -S /var/lib/fctl-agent/agent.sock 2>/dev/null; then exec sudo -n '/opt/fctl/bin/fctl-agent' 'attach' '--socket' '/var/lib/fctl-agent/agent.sock'; fi; exec /opt/fctl/bin/fctl-agent attach --socket \"$HOME/.fctl/agent.sock\""
        );
    }

    #[test]
    fn session_vms_get_their_size_and_no_mounts_in_one_expression() {
        assert_eq!(
            session_settings(2, 4096),
            r#".cpus = 2 | .memory = "4096MiB" | .mounts = [] | .portForwards = [{"guestIP": "0.0.0.0", "guestPortRange": [1, 65535], "proto": "any", "ignore": true}]"#
        );
        assert!(session_settings(0, 0).starts_with(r#".cpus = 1 | .memory = "1024MiB" | "#));
    }
}
