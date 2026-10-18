//! Container drivers that shell out to a CLI: Apple `container` (one
//! lightweight VM per container) and the `docker` CLI (Docker, OrbStack,
//! Colima, Podman).
//!
//! Both get the same treatment: the sandbox is a long-lived container whose
//! main process just waits (`tail -f /dev/null`); fctl-agent is copied in and
//! started with a detached `exec`; the host talks to it through `exec -i
//! fctl-agent attach`.

use super::{
    AGENT_PATH, AgentIo, BuildJob, COMMAND_TIMEOUT, SLOW_COMMAND_TIMEOUT, Sandbox, SandboxInfo,
    SandboxSpec, TimedOut, attach_script, run_cmd_within, serve_script, timed_out,
};
use anyhow::{Context, Result};
use fctl_proto::runner::{DriverInfo, SandboxSize};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::Path;
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
        // A runtime that doesn't answer isn't missing the image.
        if e.is::<TimedOut>() || self.has_image(image).await {
            return Err(failed(e));
        }
        log(format!("Pulling {image}"));
        self.pull(image, false).await.map_err(Create::NoImage)?;
        self.run(&args).await.map(drop).map_err(failed)
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

    /// Removes an image; fails while a sandbox uses it.
    pub async fn remove_image(&self, image: &str) -> Result<()> {
        let args: &[&str] = match self.flavor {
            Flavor::Apple => &["image", "delete", image],
            Flavor::Docker => &["image", "rm", image],
        };
        self.run(args).await.map(drop)
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
        if self.has_image(image).await {
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
pub(crate) mod fake {
    use super::*;
    use anyhow::Result;
    use std::sync::{Mutex, PoisonError};

    /// A stand-in for the `docker` or `container` CLI with no sandboxes yet:
    /// `have` are the images on the host, `registry` those it can pull. It
    /// writes each call to `calls`; with `fail_run` every `run` fails, and with
    /// `fail_build` every `build`. A build lists its context in `context` and
    /// copies its Dockerfile to `dockerfile`; `size` holds the size images
    /// have (1000 bytes without it).
    pub struct FakeCli {
        pub dir: tempfile::TempDir,
    }

    impl FakeCli {
        pub fn new(have: &[&str], registry: &[&str]) -> Result<Self> {
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
                r##"#!/bin/sh
d='{}'
echo "$*" >> "$d/calls"
key() {{ printf '%s' "$1" | tr '/:' '__'; }}
for last in "$@"; do :; done
pull() {{
  [ -e "$d/registry/$(key "$last")" ] || {{ echo "$last: manifest unknown" >&2; exit 1; }}
  touch "$d/have/$(key "$last")"
}}
case "$1 $2" in
  "image inspect")
    [ -e "$d/have/$(key "$last")" ] || exit 1
    [ "$4" != '{{{{.Size}}}}' ] || cat "$d/size" 2>/dev/null || echo 1000 ;;
  "image rm" | "image delete") rm "$d/have/$(key "$last")" ;;
  "build "*)
    [ ! -e "$d/fail_build" ] || {{ echo "#3 ERROR: process did not complete successfully: exit code: 1" >&2; exit 1; }}
    echo "#1 [internal] load build definition from Dockerfile"
    echo "#5 [1/2] FROM debian" >&2
    echo "#5 0.1 noise"
    (cd "$last" && find . | sort) > "$d/context"
    cat "$5" > "$d/dockerfile"
    touch "$d/have/$(key "$3")" ;;
  "image pull" | "pull "*) pull ;;
  "inspect "*) exit 1 ;;
  "run "*)
    [ ! -e "$d/fail_run" ] || {{ echo "no room for it" >&2; exit 1; }}
    while [ $# -gt 0 ] && [ "$1" != tail ]; do shift; done
    [ -e "$d/have/$(key "$2")" ] || {{ echo "Unable to find image '$2' locally" >&2; exit 125; }} ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
"##,
                root.display()
            );
            let bin = root.join("cli");
            std::fs::write(&bin, script)?;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
            Ok(Self { dir })
        }

        pub fn driver(&self, flavor: Flavor) -> ContainerDriver {
            ContainerDriver {
                flavor,
                bin: self.dir.path().join("cli").display().to_string(),
            }
        }

        pub fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.dir.path().join("calls"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        /// Creates the sandbox: the image it started from, and the log.
        pub async fn ensure(
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
}

#[cfg(test)]
mod tests {
    use super::fake::FakeCli;
    use super::*;

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

    use std::sync::{Mutex, PoisonError};

    const VERSION_IMAGE: &str = "ghcr.io/hyperstructure/sandbox:0.3.0";
    const LATEST_IMAGE: &str = "ghcr.io/hyperstructure/sandbox:latest";

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
