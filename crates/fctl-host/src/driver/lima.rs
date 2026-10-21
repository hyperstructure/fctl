//! The Lima driver (experimental): one VM per session, cloned from a
//! prepared base VM named `fctl-base` (`fctl host image build --driver lima`).
//! Heavier than containers, but a full VM on any Mac (Intel too) or Linux host.

use super::{
    AGENT_PATH, AgentIo, COMMAND_TIMEOUT, SLOW_COMMAND_TIMEOUT, Sandbox, SandboxInfo, SandboxSpec,
    attach_script, run_cmd_within, serve_script,
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
}

impl LimaDriver {
    pub fn new() -> Self {
        Self {
            bin: std::env::var("FCTL_LIMACTL_BIN").unwrap_or_else(|_| "limactl".into()),
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
        self.exec(sb, &format!("sudo mkdir -p /opt/fctl/bin && sudo install -m 755 {tmp} {AGENT_PATH} && rm -f {tmp}")).await?;
        Ok(())
    }

    pub async fn start_agent(&self, sb: &Sandbox) -> Result<()> {
        let script = format!(
            "setsid sh -c '{}' >/dev/null 2>&1 < /dev/null &",
            serve_script(AGENT_PATH).replace('\'', "'\\''")
        );
        self.exec(sb, &script).await?;
        Ok(())
    }

    pub async fn attach(&self, sb: &Sandbox) -> Result<AgentIo> {
        let mut child = self.spawn(sb, &attach_script(AGENT_PATH)).await?;
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

impl Default for LimaDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_vms_get_their_size_and_no_mounts_in_one_expression() {
        assert_eq!(
            session_settings(2, 4096),
            r#".cpus = 2 | .memory = "4096MiB" | .mounts = [] | .portForwards = [{"guestIP": "0.0.0.0", "guestPortRange": [1, 65535], "proto": "any", "ignore": true}]"#
        );
        assert!(session_settings(0, 0).starts_with(r#".cpus = 1 | .memory = "1024MiB" | "#));
    }
}
