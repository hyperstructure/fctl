//! Container drivers that shell out to a CLI: Apple `container` (one
//! lightweight VM per container) and the `docker` CLI (Docker, OrbStack,
//! Colima, Podman).
//!
//! Both get the same treatment: the sandbox is a long-lived container whose
//! main process just waits (`tail -f /dev/null`); fctl-agent is copied in and
//! started with a detached `exec`; the host talks to it through `exec -i
//! fctl-agent attach`.

use super::{
    AGENT_PATH, AgentIo, COMMAND_TIMEOUT, SLOW_COMMAND_TIMEOUT, Sandbox, SandboxInfo, SandboxSpec,
    TimedOut, attach_script, run_cmd_within, serve_script, timed_out,
};
use anyhow::{Context, Result};
use fctl_proto::runner::{DriverInfo, SandboxSize};
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Apple,
    Docker,
}

pub struct ContainerDriver {
    flavor: Flavor,
    bin: String,
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

    /// `running`, `stopped`, … or `None` when it does not exist. An error
    /// only when the runtime doesn't answer.
    async fn state(&self, name: &str) -> Result<Option<String>> {
        let inspect: &[&str] = match self.flavor {
            Flavor::Apple => &["inspect", name],
            Flavor::Docker => &["inspect", "--format", "{{.State.Status}}", name],
        };
        let out = match self.run(inspect).await {
            Ok(out) => out,
            Err(e) if e.is::<TimedOut>() => return Err(e),
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
            Some("running") => return Ok(sb),
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
        let cpus = spec.cpus.max(1).to_string();
        let memory = match self.flavor {
            Flavor::Apple => format!("{}M", spec.memory_mb.max(512)),
            Flavor::Docker => format!("{}m", spec.memory_mb.max(512)),
        };
        let label = "sh.fctl.sandbox=1".to_owned();
        let args: Vec<&str> = vec![
            "run",
            "-d",
            "--name",
            &spec.name,
            "--label",
            &label,
            "--cpus",
            &cpus,
            "--memory",
            &memory,
            "--init",
            "--entrypoint",
            "tail",
            &spec.image,
            "-f",
            "/dev/null",
        ];
        if let Err(e) = self.run(&args).await {
            // A missing image: pull it and try once more.
            log(format!("Pulling {}", spec.image));
            let pull = match self.flavor {
                Flavor::Apple => self.run(&["image", "pull", &spec.image]).await,
                Flavor::Docker => self.run(&["pull", &spec.image]).await,
            };
            if pull.is_err() {
                return Err(e).with_context(|| {
                    format!(
                        "starting {} (is the image {} built? run `fctl host image build`)",
                        spec.name, spec.image
                    )
                });
            }
            self.run(&args).await?;
        }
        Ok(sb)
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

    pub async fn stop(&self, sb: &Sandbox) -> Result<()> {
        if self.state(&sb.name).await?.as_deref() == Some("running") {
            self.run(&["stop", "-t", "20", &sb.name]).await?;
        }
        Ok(())
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
}
