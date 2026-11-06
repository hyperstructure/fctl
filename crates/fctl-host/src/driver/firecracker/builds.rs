//! Images built from a project's Dockerfile, with BuildKit in a VM of the
//! project's own, which keeps BuildKit's cache on its disk between builds.
//! The host sends the build's context and Dockerfile in as archives over the
//! VM's vsock, BuildKit writes the image as an OCI archive, and the host
//! keeps it as a pull would: its layers unpacked in another VM. No file of
//! the repository or the image is run or unpacked on the host.

use super::{FirecrackerDriver, Log, images, oci, vsock};
use crate::driver::{BuildJob, SandboxSpec};
use anyhow::{Context, Result, bail};
use fctl_proto::vm::Exec;
use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// BuildKit's image, which builder VMs boot from, pinned by digest.
pub const BUILDKIT_IMAGE: &str = "docker.io/moby/buildkit:v0.34.2@sha256:c5f8862f52499773af85bc07695b2379ef08e8002352b0df2c4864c91d78dabe";

/// Where BuildKit keeps its state in a builder VM: on the VM's own disk,
/// ext4, where its overlay snapshots go (they can't on the overlay that is
/// the VM's root), and where it stays between builds.
const STATE: &str = "/run/fctl/disk/buildkit";

/// Where a build's context, Dockerfile and image are in a builder VM.
const WORK: &str = "/run/fctl/disk/build";

/// buildkitd's socket in a builder VM.
const SOCKET: &str = "unix:///run/buildkit/buildkitd.sock";

/// The builder VM of the image `tag` builds (`fctl-build-<project>:…`): one
/// for each project, so that no project's build uses another's cache.
pub fn builder_name(tag: &str) -> String {
    let repository = tag.split(['@', ':']).next().unwrap_or(tag);
    let project = repository.strip_prefix("fctl-build-").unwrap_or(repository);
    let part: String = project
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(48)
        .collect();
    format!("fctl-builder-{part}")
}

/// `buildctl build`'s arguments for `job`.
fn buildctl_args(job: &BuildJob<'_>) -> Vec<String> {
    let mut argv: Vec<String> = [
        "buildctl",
        "--addr",
        SOCKET,
        "build",
        "--progress",
        "plain",
        "--frontend",
        "dockerfile.v0",
        "--local",
        &format!("context={WORK}/context"),
        "--local",
        &format!("dockerfile={WORK}/dockerfile"),
        "--opt",
        "filename=Dockerfile",
        "--output",
        &format!("type=oci,dest={WORK}/image.tar"),
    ]
    .map(str::to_owned)
    .to_vec();
    if let Some(target) = job.target {
        argv.extend(["--opt".into(), format!("target={target}")]);
    }
    for (k, v) in job.args {
        argv.extend(["--opt".into(), format!("build-arg:{k}={v}")]);
    }
    argv
}

impl FirecrackerDriver {
    /// Builds `job`'s image with BuildKit in the project's builder VM,
    /// within `job.limit`, saying in `log` what the build does, and keeps
    /// it under `job.tag`. The VM stops after, keeping BuildKit's cache.
    pub async fn build(&self, job: &BuildJob<'_>, log: Log<'_>) -> Result<()> {
        let name = builder_name(job.tag);
        let spec = SandboxSpec {
            name: name.clone(),
            image: BUILDKIT_IMAGE.into(),
            fallback_image: None,
            cpus: self.host.cpus.max(1),
            memory_mb: self.host.memory_mb.max(2048),
            cache: None,
        };
        log(format!(
            "Starting the image builder, BuildKit in a VM ({name})"
        ));
        let sb = self
            .ensure(&spec, log)
            .await
            .context("starting the image builder")?;
        let built = tokio::time::timeout(job.limit, self.build_in(&sb.name, job, log)).await;
        if let Err(e) = self.stop(&sb).await {
            tracing::warn!("stopping the image builder {name}: {e:#}");
        }
        match built {
            Ok(result) => result,
            Err(_) => bail!(
                "The image build took longer than {} minutes, so the host stopped it (build_timeout_minutes in host.toml)",
                job.limit.as_secs() / 60
            ),
        }
    }

    /// A program for the builder VM `name`, as root.
    fn as_root(&self, name: &str, argv: Vec<String>) -> Exec {
        Exec {
            argv,
            user: Some("root".into()),
            cwd: Some("/".into()),
            ..self.exec_for(name, Vec::new())
        }
    }

    /// Runs `script` as root in the builder VM `name`; what it printed, or
    /// why it failed.
    async fn builder_sh(&self, name: &str, script: &str) -> Result<String> {
        let vm = self.vm(name)?;
        let exec = self.as_root(name, vec!["sh".into(), "-c".into(), script.into()]);
        let out = tokio::time::timeout(
            crate::driver::COMMAND_TIMEOUT,
            vsock::run(&vm.vsock(), exec, None),
        )
        .await
        .map_err(|_| anyhow::anyhow!("`{script}` didn't finish in the image builder"))??;
        if out.code != 0 {
            bail!(
                "`{script}` failed in the image builder: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    async fn build_in(&self, name: &str, job: &BuildJob<'_>, log: Log<'_>) -> Result<()> {
        self.start_buildkitd(name).await?;
        // The files the build reads, sent as archives: the context, and the
        // Dockerfile with the ignore file beside it.
        self.builder_sh(
            name,
            &format!("rm -rf {WORK} && mkdir -p {WORK}/context {WORK}/dockerfile"),
        )
        .await?;
        let scratch = job
            .dockerfile
            .parent()
            .context("the Dockerfile has no directory")?
            .join("vm");
        std::fs::create_dir_all(&scratch)?;
        let context_tar = scratch.join("context.tar");
        let dockerfile_tar = scratch.join("dockerfile.tar");
        let (context, dockerfile) = (job.context.to_owned(), job.dockerfile.to_owned());
        let (to_context, to_dockerfile) = (context_tar.clone(), dockerfile_tar.clone());
        tokio::task::spawn_blocking(move || {
            archive_tree(&context, &to_context)?;
            archive_dockerfile(&dockerfile, &to_dockerfile)
        })
        .await??;
        self.send_archive(name, &context_tar, &format!("{WORK}/context"))
            .await
            .context("sending the build's context to the image builder")?;
        self.send_archive(name, &dockerfile_tar, &format!("{WORK}/dockerfile"))
            .await
            .context("sending the Dockerfile to the image builder")?;
        self.run_buildctl(name, job, log).await?;
        // The image, back as an OCI archive, kept as a pull would keep it:
        // beside the images, on their filesystem.
        let downloads = self.root.join("downloads").join(format!(
            "build-{}",
            hex::encode(fctl_domain::random_bytes::<6>())
        ));
        std::fs::create_dir_all(&downloads)?;
        let kept = async {
            let image_tar = downloads.join("image.tar");
            self.receive_file(name, &format!("{WORK}/image.tar"), &image_tar)
                .await
                .context("getting the image from the image builder")?;
            let _ = self.builder_sh(name, &format!("rm -rf {WORK}")).await;
            let blobs = downloads.join("blobs");
            let image = tokio::task::spawn_blocking(move || {
                oci::read(&image_tar, &blobs, std::env::consts::ARCH)
            })
            .await??;
            self.keep_built(job.tag, image, &downloads, log).await
        }
        .await;
        let _ = std::fs::remove_dir_all(&downloads);
        kept
    }

    /// Starts buildkitd in the builder VM `name` as root, unless it runs,
    /// and waits for it to take builds.
    async fn start_buildkitd(&self, name: &str) -> Result<()> {
        let ready = format!("buildctl --addr {SOCKET} debug workers >/dev/null 2>&1");
        if self.builder_sh(name, &ready).await.is_ok() {
            return Ok(());
        }
        let vm = self.vm(name)?;
        let serve = format!(
            "mkdir -p {STATE} && exec buildkitd --root {STATE} --addr {SOCKET} --oci-worker-net host >>{STATE}/buildkitd.log 2>&1"
        );
        vsock::detach(
            &vm.vsock(),
            self.as_root(name, vec!["sh".into(), "-c".into(), serve]),
        )
        .await
        .context("starting buildkitd")?;
        let until = std::time::Instant::now() + Duration::from_secs(120);
        while self.builder_sh(name, &ready).await.is_err() {
            if std::time::Instant::now() > until {
                let said = self
                    .builder_sh(name, &format!("tail -n 20 {STATE}/buildkitd.log"))
                    .await
                    .unwrap_or_default();
                bail!(
                    "buildkitd didn't start in the image builder: {}",
                    said.trim()
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Ok(())
    }

    /// Unpacks the archive `archive` into `dir` in the builder VM `name`.
    async fn send_archive(&self, name: &str, archive: &Path, dir: &str) -> Result<()> {
        let vm = self.vm(name)?;
        let exec = self.as_root(
            name,
            vec![
                "tar".into(),
                "-x".into(),
                "-C".into(),
                dir.into(),
                "-f".into(),
                "-".into(),
            ],
        );
        let mut started = vsock::start(&vm.vsock(), exec).await?;
        let mut file = tokio::fs::File::open(archive).await?;
        tokio::io::copy(&mut file, &mut started.stdin).await?;
        started.stdin.shutdown().await?;
        let mut said = String::new();
        let _ = tokio::io::AsyncReadExt::read_to_string(&mut started.stderr, &mut said).await;
        match started.exit.await.ok().flatten() {
            Some(0) => Ok(()),
            code => bail!("tar exited with {code:?}: {}", said.trim()),
        }
    }

    /// Copies `from` in the builder VM `name` to `to` on the host.
    async fn receive_file(&self, name: &str, from: &str, to: &Path) -> Result<()> {
        let vm = self.vm(name)?;
        let exec = self.as_root(name, vec!["cat".into(), from.into()]);
        let mut started = vsock::start(&vm.vsock(), exec).await?;
        started.stdin.shutdown().await?;
        let mut file = tokio::fs::File::create(to).await?;
        tokio::io::copy(&mut started.stdout, &mut file).await?;
        file.flush().await?;
        match started.exit.await.ok().flatten() {
            Some(0) => Ok(()),
            code => bail!("cat {from} exited with {code:?}"),
        }
    }

    /// Runs the build, saying its steps in `log`.
    async fn run_buildctl(&self, name: &str, job: &BuildJob<'_>, log: Log<'_>) -> Result<()> {
        let vm = self.vm(name)?;
        let mut started = vsock::start(&vm.vsock(), self.as_root(name, buildctl_args(job))).await?;
        started.stdin.shutdown().await?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        crate::driver::container::forward_lines(started.stdout, tx.clone());
        crate::driver::container::forward_lines(started.stderr, tx);
        let mut tail: VecDeque<String> = VecDeque::new();
        while let Some(line) = rx.recv().await {
            if crate::driver::container::build_step(&line) {
                log(crate::driver::container::short(&line));
            }
            if tail.len() == 20 {
                tail.pop_front();
            }
            tail.push_back(line);
        }
        match started.exit.await.ok().flatten() {
            Some(0) => Ok(()),
            code => {
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
                bail!("Building the image failed (exit {code:?}){why}")
            }
        }
    }

    /// Keeps the image built, `image`, under `tag`: its layers unpacked in
    /// a VM, as a pull's are, onto a filesystem made in `dir`.
    async fn keep_built(
        &self,
        tag: &str,
        image: oci::Image,
        dir: &Path,
        log: Log<'_>,
    ) -> Result<()> {
        let key = images::canonical(tag).with_context(|| format!("{tag} isn't an image name"))?;
        if self.store.image(&image.resolved.id).is_some() {
            let _images = self.lock_images().await?;
            return self.store.tag(tag, &image.resolved.id);
        }
        let rootfs = dir.join("rootfs.ext4");
        let _ = std::fs::remove_file(&rootfs);
        super::make_fs(&rootfs, super::IMAGE_FS_GB, "fctl-image", false).await?;
        let layers: Vec<fctl_proto::vm::Layer> = image
            .resolved
            .layers
            .iter()
            .map(|l| fctl_proto::vm::Layer {
                media_type: l.media_type.clone(),
                size: l.size,
            })
            .collect();
        let (files, bytes) = self.unpack(&rootfs, &layers, &image.layers, log).await?;
        self.keep(tag, key, &image.resolved, &rootfs, files, bytes)
            .await
    }
}

/// Writes the directory tree `dir` as a tar archive to `to`, its symlinks
/// as symlinks: the build's context.
fn archive_tree(dir: &Path, to: &Path) -> Result<()> {
    let mut tar = tar::Builder::new(std::fs::File::create(to)?);
    tar.follow_symlinks(false);
    tar.append_dir_all(".", dir)
        .with_context(|| format!("archiving {}", dir.display()))?;
    tar.finish()?;
    Ok(())
}

/// Writes the Dockerfile `dockerfile`, and the ignore file beside it, as a
/// tar archive to `to`, as `Dockerfile` and `Dockerfile.dockerignore`.
fn archive_dockerfile(dockerfile: &Path, to: &Path) -> Result<()> {
    let mut tar = tar::Builder::new(std::fs::File::create(to)?);
    tar.append_path_with_name(dockerfile, "Dockerfile")?;
    let ignore = dockerfile.with_file_name("Dockerfile.dockerignore");
    if std::fs::symlink_metadata(&ignore).is_ok_and(|m| m.is_file()) {
        tar.append_path_with_name(&ignore, "Dockerfile.dockerignore")?;
    }
    tar.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_project_builds_in_a_vm_of_its_own() {
        assert_eq!(
            builder_name("fctl-build-prj_01m6s0dk:0123abc"),
            "fctl-builder-prj-01m6s0dk"
        );
        assert_ne!(
            builder_name("fctl-build-prj_a:1"),
            builder_name("fctl-build-prj_b:1")
        );
    }

    #[test]
    fn buildctl_gets_the_jobs_target_and_arguments_as_words() {
        let args = std::collections::BTreeMap::from([(
            "GREETING".to_owned(),
            "hi there; rm -rf /".to_owned(),
        )]);
        let job = BuildJob {
            tag: "fctl-build-prj_1:abc",
            dockerfile: Path::new("/d/Dockerfile"),
            context: Path::new("/d/src"),
            args: &args,
            target: Some("dev"),
            limit: Duration::from_secs(60),
        };
        let argv = buildctl_args(&job);
        assert_eq!(argv[..4], ["buildctl", "--addr", SOCKET, "build"]);
        assert!(argv.windows(2).any(|w| w == ["--opt", "target=dev"]));
        assert!(
            argv.windows(2)
                .any(|w| w == ["--opt", "build-arg:GREETING=hi there; rm -rf /"])
        );
        assert!(argv.contains(&format!("type=oci,dest={WORK}/image.tar")));
    }

    #[test]
    fn archives_the_context_with_its_symlinks_as_symlinks() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let context = dir.path().join("src");
        std::fs::create_dir_all(context.join("app"))?;
        std::fs::write(context.join("app/main.rs"), b"fn main() {}")?;
        std::os::unix::fs::symlink("/etc/passwd", context.join("leak"))?;
        let tar = dir.path().join("context.tar");
        archive_tree(&context, &tar)?;
        let mut names = Vec::new();
        for entry in tar::Archive::new(std::fs::File::open(&tar)?).entries()? {
            let entry = entry?;
            let kind = entry.header().entry_type();
            names.push((entry.path()?.display().to_string(), kind.is_symlink()));
        }
        assert!(names.contains(&("app/main.rs".into(), false)));
        assert!(names.contains(&("leak".into(), true)));
        Ok(())
    }
}
