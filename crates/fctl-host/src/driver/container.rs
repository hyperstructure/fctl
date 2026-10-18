//! Container drivers that shell out to a CLI: Apple `container` (one
//! lightweight VM per container) and the `docker` CLI (Docker, OrbStack,
//! Colima, Podman).
//!
//! Both get the same treatment: the sandbox is a long-lived container whose
//! main process just waits (`tail -f /dev/null`); fctl-agent is copied in and
//! started with a detached `exec`; the host talks to it through `exec -i
//! fctl-agent attach`.

use super::{
    AGENT_PATH, AgentIo, Sandbox, SandboxInfo, SandboxSpec, attach_script, run_cmd, serve_script,
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

    pub fn flavor_name(&self) -> &'static str {
        match self.flavor {
            Flavor::Apple => "apple",
            Flavor::Docker => "docker",
        }
    }

    async fn run(&self, args: &[&str]) -> Result<String> {
        run_cmd(&self.bin, args).await
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
        let version = match self.flavor {
            Flavor::Apple => self
                .run(&["system", "version", "--format", "json"])
                .await
                .ok()
                .and_then(|s| {
                    let v: Value = serde_json::from_str(&s).ok()?;
                    v.get("version")
                        .or_else(|| v.pointer("/0/version"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(Some(s.trim().to_owned()))
                }),
            Flavor::Docker => self
                .run(&["version", "--format", "{{.Server.Version}}"])
                .await
                .ok()
                .map(|s| s.trim().to_owned()),
        };
        // The service must be up, not just the CLI installed.
        let ready = match self.flavor {
            Flavor::Apple => self.run(&["system", "status"]).await.is_ok(),
            Flavor::Docker => version.is_some(),
        };
        DriverInfo {
            name: self.flavor_name().into(),
            available: ready,
            isolation: isolation.into(),
            detail: Some(match (self.flavor, ready) {
                (_, true) => "ready".into(),
                (Flavor::Apple, false) => {
                    "the `container` service is not running (run `container system start`)".into()
                }
                (Flavor::Docker, false) => "the docker daemon is not running".into(),
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

    /// `running`, `stopped`, … or `None` when it does not exist.
    async fn state(&self, name: &str) -> Option<String> {
        match self.flavor {
            Flavor::Apple => {
                let out = self.run(&["inspect", name]).await.ok()?;
                let v: Value = serde_json::from_str(&out).ok()?;
                v.pointer("/0/status/state")
                    .or_else(|| v.pointer("/0/status"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }
            Flavor::Docker => {
                let out = self
                    .run(&["inspect", "--format", "{{.State.Status}}", name])
                    .await
                    .ok()?;
                Some(out.trim().to_owned())
            }
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
        match self.state(&spec.name).await.as_deref() {
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
            image,
            "-f",
            "/dev/null",
        ];
        let failed =
            |e: anyhow::Error| Create::Failed(e.context(format!("starting {}", spec.name)));
        let Err(e) = self.run(&args).await else {
            return Ok(());
        };
        if self.has_image(image).await {
            return Err(failed(e));
        }
        log(format!("Pulling {image}"));
        self.pull(image, false).await.map_err(Create::NoImage)?;
        self.run(&args).await.map(drop).map_err(failed)
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
        if self.state(&sb.name).await.as_deref() == Some("running") {
            self.run(&["stop", "-t", "20", &sb.name]).await?;
        }
        Ok(())
    }

    pub async fn remove(&self, sb: &Sandbox) -> Result<()> {
        let args: &[&str] = match self.flavor {
            Flavor::Apple => &["delete", "--force", &sb.name],
            Flavor::Docker => &["rm", "-f", &sb.name],
        };
        if self.state(&sb.name).await.is_some() {
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

/// An architecture as images name it: `arm64` for Rust's `aarch64`.
fn linux_arch(arch: &str) -> &str {
    match arch {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
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

    /// A stand-in for the `docker` or `container` CLI with no sandboxes yet:
    /// `have` are the images on the host, `registry` those it can pull. It
    /// writes each call to `calls`, and with `fail_run` every `run` fails.
    struct FakeCli {
        dir: tempfile::TempDir,
    }

    impl FakeCli {
        fn new(have: &[&str], registry: &[&str]) -> Result<Self> {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir()?;
            let root = dir.path();
            for (sub, images) in [("have", have), ("registry", registry)] {
                std::fs::create_dir(root.join(sub))?;
                for image in images {
                    std::fs::write(root.join(sub).join(image.replace(['/', ':'], "_")), "")?;
                }
            }
            let script = format!(
                r#"#!/bin/sh
d='{}'
echo "$*" >> "$d/calls"
key() {{ printf '%s' "$1" | tr '/:' '__'; }}
for last in "$@"; do :; done
pull() {{
  [ -e "$d/registry/$(key "$last")" ] || {{ echo "$last: manifest unknown" >&2; exit 1; }}
  touch "$d/have/$(key "$last")"
}}
case "$1 $2" in
  "image inspect") [ -e "$d/have/$(key "$last")" ] ;;
  "image pull" | "pull "*) pull ;;
  "inspect "*) exit 1 ;;
  "run "*)
    [ ! -e "$d/fail_run" ] || {{ echo "no room for it" >&2; exit 1; }}
    while [ $# -gt 0 ] && [ "$1" != tail ]; do shift; done
    [ -e "$d/have/$(key "$2")" ] || {{ echo "Unable to find image '$2' locally" >&2; exit 125; }} ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
"#,
                root.display()
            );
            let bin = root.join("cli");
            std::fs::write(&bin, script)?;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
            Ok(Self { dir })
        }

        fn driver(&self, flavor: Flavor) -> ContainerDriver {
            ContainerDriver {
                flavor,
                bin: self.dir.path().join("cli").display().to_string(),
            }
        }

        fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.dir.path().join("calls"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        /// Creates the sandbox: the image it started from, and the log.
        async fn ensure(
            &self,
            flavor: Flavor,
            spec: &SandboxSpec,
        ) -> Result<(String, Vec<String>)> {
            let lines = Mutex::new(Vec::new());
            let log = |line: String| {
                lines
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(line)
            };
            self.driver(flavor).ensure(spec, &log).await?;
            let run = self
                .calls()
                .into_iter()
                .rev()
                .find(|c| c.starts_with("run "));
            let image = run
                .as_deref()
                .and_then(|c| c.split(" tail ").nth(1)?.split(' ').next())
                .unwrap_or_default()
                .to_owned();
            Ok((
                image,
                lines.into_inner().unwrap_or_else(PoisonError::into_inner),
            ))
        }
    }

    use std::sync::{Mutex, PoisonError};

    const VERSION_IMAGE: &str = "ghcr.io/hyperstructure/fctl-agent:0.3.0";
    const LATEST_IMAGE: &str = "ghcr.io/hyperstructure/fctl-agent:latest";

    fn spec(image: &str, fallback: Option<&str>) -> SandboxSpec {
        SandboxSpec {
            name: "fctl-ses-1".into(),
            image: image.into(),
            fallback_image: fallback.map(str::to_owned),
            cpus: 2,
            memory_mb: 2048,
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
    async fn other_failures_are_not_about_the_image() -> Result<()> {
        let cli = FakeCli::new(&[VERSION_IMAGE], &[LATEST_IMAGE])?;
        std::fs::write(cli.dir.path().join("fail_run"), "")?;
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
