//! The `firecracker` driver (Linux hosts with KVM): each sandbox is a
//! Firecracker microVM with a kernel of its own (ADR 0069).
//!
//! - **Images** are pulled from their registries as Docker pulls them
//!   ([`registry`]), and their layers unpacked onto an ext4 filesystem in a
//!   short-lived VM ([`fctl_proto::vm::Mode::Unpack`]), as root there, so
//!   that their owners and modes stay theirs and none of their files touch
//!   the host's filesystem. The sandboxes made from an image share its
//!   filesystem, read only ([`images`]).
//! - **A sandbox** is a directory with the VM's own disk, a sparse ext4 file
//!   that takes what the session writes (`disk_gb` at most), which the
//!   VM's init lays over the image's filesystem. Stopping the VM keeps it,
//!   and starting it again boots from it.
//! - **The VM** boots the kernel with fctl-agent as its init
//!   ([`assets::initrd`]); Firecracker and `passt`, which gives the VM its
//!   network over the vsock without a TAP device or root on the host, run
//!   in process groups of their own, so that restarting the host leaves
//!   them running ([`vm`]).
//! - **The host runs programs** in the VM through its vsock, as `docker
//!   exec` would ([`vsock`]).

pub mod assets;
pub mod images;
pub mod registry;
pub mod vm;
pub mod vsock;

use super::{
    AGENT_PATH, AgentIo, DiskSpot, Running, Sandbox, SandboxInfo, SandboxSpec, attach_script,
    serve_script,
};
use crate::config::{FirecrackerConfig, HostConfig};
use crate::driver::ImageInfo;
use anyhow::{Context, Result, bail};
use fctl_proto::runner::DriverInfo;
use fctl_proto::vm::{Boot, Exec, Mode};
use images::{Image, Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use vm::{Drive, Spec, Vm};

/// The size of an image's filesystem: a sparse file, which takes only what
/// the image holds.
const IMAGE_FS_GB: u64 = 64;

/// What a sandbox was made from, in `sandbox.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    /// The image's ID.
    image: String,
    /// The name it was made from.
    reference: String,
    created_ms: i64,
    /// The fctl-agent its VM last booted with (the start of its SHA-256).
    #[serde(default)]
    agent: Option<String>,
}

type Log<'a> = &'a (dyn Fn(String) + Send + Sync);

pub struct FirecrackerDriver {
    host: HostConfig,
    root: PathBuf,
    store: Store,
    /// One for each image being pulled and each sandbox being prepared.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// Finds `program` on the PATH, or in the directories where Linux keeps
/// what administrators run, which users' PATHs may lack.
pub fn find_program(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return p.is_file().then_some(p);
    }
    super::process::which(program).or_else(|| {
        ["/usr/sbin", "/sbin", "/usr/local/sbin", "/usr/bin", "/bin"]
            .iter()
            .map(|d| Path::new(d).join(program))
            .find(|p| p.is_file())
    })
}

/// Why this host can't run KVM VMs, if it can't.
fn kvm_problem() -> Option<String> {
    match std::fs::OpenOptions::new().read(true).write(true).open("/dev/kvm") {
        Ok(_) => None,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(
            "this machine has no /dev/kvm: Firecracker needs a CPU with virtualization turned on, or nested virtualization in a VM".into(),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Some(
            "this user may not use /dev/kvm: `sudo usermod -aG kvm $USER`, then log in again".into(),
        ),
        Err(e) => Some(format!("opening /dev/kvm: {e}")),
    }
}

/// Makes an ext4 filesystem in a new sparse file of `gb` GiB at `path`,
/// whose root is root's, labelled `label`, with a journal or not.
async fn make_fs(path: &Path, gb: u64, label: &str, journal: bool) -> Result<()> {
    let mkfs = find_program("mkfs.ext4").context("mkfs.ext4 is not installed (e2fsprogs)")?;
    let file = std::fs::File::create(path).with_context(|| format!("making {}", path.display()))?;
    file.set_len(gb * 1024 * 1024 * 1024)?;
    drop(file);
    let features = if journal {
        "has_journal"
    } else {
        "^has_journal"
    };
    let path_s = path.display().to_string();
    super::run_cmd(
        &mkfs.display().to_string(),
        &[
            "-q",
            "-F",
            "-L",
            label,
            "-m",
            "0",
            "-O",
            features,
            "-E",
            "root_owner=0:0,lazy_itable_init=1,lazy_journal_init=1",
            &path_s,
        ],
    )
    .await
    .with_context(|| format!("making a filesystem in {}", path.display()))?;
    Ok(())
}

impl FirecrackerDriver {
    pub fn new(host: &HostConfig) -> Self {
        let root = host.data_dir().join("firecracker");
        Self {
            host: host.clone(),
            store: Store {
                dir: root.join("images"),
            },
            root,
            locks: Mutex::new(HashMap::new()),
        }
    }

    fn cfg(&self) -> &FirecrackerConfig {
        &self.host.firecracker
    }

    fn lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        locks.retain(|_, l| Arc::strong_count(l) > 1);
        locks.entry(key.to_owned()).or_default().clone()
    }

    fn sandboxes(&self) -> PathBuf {
        self.root.join("sandboxes")
    }

    fn vm(&self, name: &str) -> Result<Vm> {
        Vm::new(self.sandboxes().join(name))
    }

    fn record(&self, name: &str) -> Option<Record> {
        let bytes = std::fs::read(self.sandboxes().join(name).join("sandbox.json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn save_record(&self, name: &str, record: &Record) -> Result<()> {
        let path = self.sandboxes().join(name).join("sandbox.json");
        std::fs::write(&path, serde_json::to_vec_pretty(record)?)?;
        Ok(())
    }

    /// `passt`: the configured one, or the one installed.
    fn passt(&self) -> Option<PathBuf> {
        match &self.cfg().passt {
            Some(p) => p.is_file().then(|| p.clone()),
            None => find_program("passt"),
        }
    }

    /// Firecracker: the configured one, or the pinned release, downloaded
    /// once.
    async fn firecracker(&self, log: Log<'_>) -> Result<PathBuf> {
        match &self.cfg().firecracker {
            Some(p) if p.is_file() => Ok(p.clone()),
            Some(p) => bail!(
                "{} (firecracker.firecracker in host.toml) isn't there",
                p.display()
            ),
            None => {
                let lock = self.lock("firecracker");
                let _held = lock.lock().await;
                assets::download_firecracker(&self.root.join("bin"), log).await
            }
        }
    }

    async fn kernel(&self, log: Log<'_>) -> Result<PathBuf> {
        let kernel = self
            .cfg()
            .kernel
            .clone()
            .unwrap_or_else(|| assets::KERNEL_IMAGE.into());
        let lock = self.lock("kernel");
        let _held = lock.lock().await;
        assets::kernel(&kernel, &self.root.join("kernels"), log).await
    }

    /// The initramfs, and the start of the SHA-256 of the fctl-agent in it.
    async fn initrd(&self) -> Result<(PathBuf, String)> {
        let agent = crate::agentbin::linux_agent(&self.host)
            .await
            .context("getting fctl-agent for the VMs")?;
        let dir = self.root.join("initrd");
        let lock = self.lock("initrd");
        let _held = lock.lock().await;
        tokio::task::spawn_blocking(move || {
            let path = assets::initrd(&agent, &dir)?;
            let key = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok((path, key))
        })
        .await?
    }

    pub async fn probe(&self) -> DriverInfo {
        let info = |available: bool, detail: String, version: Option<String>| DriverInfo {
            name: "firecracker".into(),
            available,
            isolation: "vm".into(),
            version,
            detail: Some(detail),
        };
        if !cfg!(target_os = "linux") {
            return info(
                false,
                "Firecracker needs a Linux host with KVM".into(),
                None,
            );
        }
        if !matches!(std::env::consts::ARCH, "x86_64" | "aarch64") {
            return info(
                false,
                format!(
                    "Firecracker runs on x86_64 and aarch64, not {}",
                    std::env::consts::ARCH
                ),
                None,
            );
        }
        if let Some(why) = kvm_problem() {
            return info(false, why, None);
        }
        if self.passt().is_none() {
            return info(
                false,
                "passt is not installed (`sudo apt install passt`, or `sudo dnf install passt`): it gives the VMs their network".into(),
                None,
            );
        }
        if find_program("mkfs.ext4").is_none() {
            return info(false, "mkfs.ext4 is not installed (e2fsprogs)".into(), None);
        }
        let binary = match &self.cfg().firecracker {
            Some(p) if !p.is_file() => {
                return info(
                    false,
                    format!(
                        "{} (firecracker.firecracker in host.toml) isn't there",
                        p.display()
                    ),
                    None,
                );
            }
            Some(p) => Some(p.clone()),
            None => {
                Some(assets::pinned_firecracker(&self.root.join("bin"))).filter(|p| p.is_file())
            }
        };
        let version = match &binary {
            Some(b) => super::run_cmd(&b.display().to_string(), &["--version"])
                .await
                .ok()
                .and_then(|out| {
                    out.lines().next().map(|l| {
                        l.trim()
                            .trim_start_matches("Firecracker ")
                            .trim_start_matches('v')
                            .to_owned()
                    })
                }),
            None => None,
        };
        let mut later = Vec::new();
        if binary.is_none() {
            later.push(format!("Firecracker {}", assets::FIRECRACKER_VERSION));
        }
        match self.cfg().kernel.as_deref() {
            Some(k) if k.starts_with('/') && !Path::new(k).is_file() => {
                return info(
                    false,
                    format!("the kernel {k} (firecracker.kernel in host.toml) isn't there"),
                    version,
                );
            }
            Some(k) if k.starts_with('/') => {}
            _ if self.root.join("kernels").is_dir() => {}
            _ => later.push("the VMs' kernel".into()),
        }
        let detail = match later.as_slice() {
            [] => "ready".into(),
            [one] => format!("ready ({one} downloads with the first sandbox)"),
            [a, b] => format!("ready ({a} and {b} download with the first sandbox)"),
            _ => "ready".into(),
        };
        info(
            true,
            detail,
            version.or(Some(assets::FIRECRACKER_VERSION.into())),
        )
    }

    /// Where the VMs' files are: in the host's data directory.
    pub fn disk_spot(&self, data_dir: &Path) -> DiskSpot {
        DiskSpot::Dir(if self.root.is_dir() {
            self.root.clone()
        } else {
            data_dir.to_owned()
        })
    }

    // Images.

    pub async fn has_image(&self, image: &str) -> bool {
        self.store.lookup(image).is_some()
    }

    pub async fn image(&self, image: &str) -> Result<Option<ImageInfo>> {
        Ok(self.store.lookup(image).map(|i| ImageInfo {
            id: i.id,
            labels: i.labels,
        }))
    }

    pub async fn image_size(&self, image: &str) -> Option<u64> {
        let image = self.store.lookup(image)?;
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(self.store.rootfs(&image.id)).ok()?;
        Some(meta.blocks() * 512)
    }

    pub async fn tag_image(&self, image: &str, name: &str) -> Result<()> {
        let here = self
            .store
            .lookup(image)
            .with_context(|| format!("{image} is not on this host"))?;
        self.store.tag(name, &here.id)
    }

    pub async fn images(&self, repository: &str) -> Result<Vec<String>> {
        Ok(self.store.names(repository))
    }

    /// The IDs of the images the sandboxes here were made from.
    fn images_in_use(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.sandboxes()) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|e| self.record(&e.file_name().to_string_lossy()))
            .map(|r| r.image)
            .collect()
    }

    /// Removes the name `image`, and the image once no name refers to it,
    /// without force: as Docker, it keeps the last name of an image that a
    /// sandbox uses, and fails.
    pub async fn remove_image(&self, image: &str) -> Result<()> {
        let here = self
            .store
            .lookup(image)
            .with_context(|| format!("{image} is not on this host"))?;
        let names = self
            .store
            .refs()
            .values()
            .filter(|id| **id == here.id)
            .count();
        if names <= 1 && self.images_in_use().contains(&here.id) {
            bail!("a sandbox uses {image}");
        }
        self.store.untag(image)?;
        if !self.store.named(&here.id) {
            self.store.remove(&here.id)?;
        }
        Ok(())
    }

    /// Removes the images no name refers to and no sandbox uses: what a
    /// newer pull of their name left.
    fn remove_unused(&self) {
        let used = self.images_in_use();
        for id in self.store.ids() {
            if !self.store.named(&id)
                && !used.contains(&id)
                && let Err(e) = self.store.remove(&id)
            {
                tracing::warn!("removing the image {id}: {e:#}");
            }
        }
    }

    /// Pulls `image`; `progress` prints what it does.
    pub async fn pull(&self, image: &str, progress: bool) -> Result<()> {
        let say = |line: String| {
            if progress {
                println!("{line}");
            } else {
                tracing::info!("{line}");
            }
        };
        self.pull_with(image, &say).await
    }

    /// Pulls `image`, or `fallback` when `image` can't be pulled and isn't
    /// here either; returns the image it pulled.
    pub async fn pull_or_fallback(
        &self,
        image: &str,
        fallback: Option<&str>,
        progress: bool,
        log: Log<'_>,
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
                "{image} can't be pulled, and neither can {fallback}"
            ))),
        }
    }

    /// Pulls `image` from its registry: only its manifest when the image it
    /// names is here, else its layers too, which a VM unpacks.
    async fn pull_with(&self, image: &str, log: Log<'_>) -> Result<()> {
        let key = images::canonical(image)
            .with_context(|| format!("{image} isn't an image reference"))?;
        let lock = self.lock(&format!("pull {key}"));
        let _held = lock.lock().await;
        let registry = registry::Registry::new();
        let resolved = registry.resolve(image, std::env::consts::ARCH).await?;
        if self.store.image(&resolved.id).is_some() {
            self.store.tag(image, &resolved.id)?;
            self.remove_unused();
            return Ok(());
        }
        let total: u64 = resolved.layers.iter().map(|l| l.size).sum();
        log(format!(
            "Pulling {image}: {} layers, {} MB",
            resolved.layers.len(),
            total.div_ceil(1_000_000)
        ));
        let downloads = self
            .root
            .join("downloads")
            .join(hex::encode(&Sha256::digest(resolved.id.as_bytes())[..8]));
        let result = async {
            let (rootfs, files, bytes) = self
                .download_and_unpack(image, &registry, &resolved, &downloads, log)
                .await?;
            let config = resolved.config.clone();
            let pulled = Image {
                id: resolved.id.clone(),
                reference: key,
                user: config.user.filter(|u| !u.is_empty()),
                env: config.env.unwrap_or_default(),
                workdir: config.working_dir.filter(|w| !w.is_empty()),
                labels: config.labels.unwrap_or_default(),
                files,
                bytes,
                pulled_ms: vsock::now_ms(),
            };
            self.store.add(&pulled, &rootfs)?;
            Ok::<_, anyhow::Error>((files, bytes))
        }
        .await;
        let _ = std::fs::remove_dir_all(&downloads);
        let (files, bytes) = result?;
        self.remove_unused();
        log(format!(
            "Pulled {image}: {files} files, {} MB",
            bytes.div_ceil(1_000_000)
        ));
        Ok(())
    }

    async fn download_and_unpack(
        &self,
        image: &str,
        registry: &registry::Registry,
        resolved: &registry::Resolved,
        downloads: &Path,
        log: Log<'_>,
    ) -> Result<(PathBuf, u64, u64)> {
        std::fs::create_dir_all(downloads)?;
        let mut files = Vec::new();
        for (i, layer) in resolved.layers.iter().enumerate() {
            let path = downloads.join(format!("layer-{i}"));
            registry.blob(image, layer, &path).await?;
            files.push(path);
        }
        log(format!("Unpacking {image} in a VM"));
        let rootfs = downloads.join("rootfs.ext4");
        make_fs(&rootfs, IMAGE_FS_GB, "fctl-image", false).await?;
        let layers: Vec<fctl_proto::vm::Layer> = resolved
            .layers
            .iter()
            .map(|l| fctl_proto::vm::Layer {
                media_type: l.media_type.clone(),
                size: l.size,
            })
            .collect();
        let (files_n, bytes) = self.unpack(&rootfs, &layers, &files, log).await?;
        Ok((rootfs, files_n, bytes))
    }

    /// Unpacks `layers`, the files `blobs`, onto the filesystem `rootfs`
    /// in a VM booted for it; what it wrote: files and bytes.
    async fn unpack(
        &self,
        rootfs: &Path,
        layers: &[fctl_proto::vm::Layer],
        blobs: &[PathBuf],
        log: Log<'_>,
    ) -> Result<(u64, u64)> {
        let firecracker = self.firecracker(log).await?;
        let kernel = self.kernel(log).await?;
        let (initrd, _) = self.initrd().await?;
        let name = format!(
            "fctl-unpack-{}",
            hex::encode(fctl_domain::random_bytes::<6>())
        );
        let vm = Vm::new(self.root.join("unpacking").join(&name))?;
        let spec = vm::unpack_spec(kernel, initrd, &name, rootfs);
        vm.start(&spec, &firecracker, None).await?;
        let result = async {
            vm.wait_ready().await?;
            let request = fctl_proto::vm::Request::Unpack {
                layers: layers.to_vec(),
            };
            let (mut r, mut w) = vsock::request(&vm.vsock(), &request).await?;
            for blob in blobs {
                let mut file = tokio::fs::File::open(blob).await?;
                tokio::io::copy(&mut file, &mut w)
                    .await
                    .context("sending a layer to the VM")?;
            }
            let reply = tokio::time::timeout(registry::LAYER_TIME, vsock::read_reply(&mut r))
                .await
                .context("unpacking took too long")??;
            if let Some(e) = reply.error {
                bail!("{e}");
            }
            Ok((reply.files.unwrap_or(0), reply.bytes.unwrap_or(0)))
        }
        .await;
        let result = result.map_err(|e| {
            let tail = vm.console_tail();
            if tail.is_empty() {
                e
            } else {
                e.context(format!("the VM's console said:\n{tail}"))
            }
        });
        // It powers off once done.
        vm.stop().await;
        let _ = std::fs::remove_dir_all(&vm.dir);
        result
    }

    // Sandboxes.

    pub async fn exists(&self, name: &str) -> Result<bool> {
        Ok(self.record(name).is_some())
    }

    /// The image a new sandbox starts from: `spec.image`, pulled when it
    /// isn't here, else `spec.fallback_image`.
    async fn image_for(&self, spec: &SandboxSpec, log: Log<'_>) -> Result<Image> {
        if let Some(image) = self.store.lookup(&spec.image) {
            return Ok(image);
        }
        let error = match self.pull_with(&spec.image, log).await {
            Ok(()) => {
                return self
                    .store
                    .lookup(&spec.image)
                    .with_context(|| format!("{} was pulled but isn't here", spec.image));
            }
            Err(e) => e,
        };
        let missing = format!("{} is not on this host and can't be pulled", spec.image);
        let Some(fallback) = &spec.fallback_image else {
            return Err(error.context(missing));
        };
        log(format!("{missing}; starting from {fallback} instead"));
        if let Some(image) = self.store.lookup(fallback) {
            return Ok(image);
        }
        self.pull_with(fallback, log)
            .await
            .map_err(|_| error.context(missing.clone()))?;
        self.store.lookup(fallback).context(missing)
    }

    /// Creates the sandbox, or starts the one there, and waits for its VM.
    pub async fn ensure(&self, spec: &SandboxSpec, log: Log<'_>) -> Result<Sandbox> {
        let sb = Sandbox {
            name: spec.name.clone(),
            root: None,
        };
        let lock = self.lock(&format!("sandbox {}", spec.name));
        let _held = lock.lock().await;
        let vm = self.vm(&spec.name)?;
        if vm.running() {
            if vsock::hello(&vm.vsock()).await.is_ok() {
                return Ok(sb);
            }
            log(format!(
                "Starting the sandbox {} again: its VM doesn't answer",
                spec.name
            ));
            vm.kill().await;
        }
        let mut record = match self.record(&spec.name) {
            Some(record) => {
                log(format!("Starting the existing sandbox {}", spec.name));
                record
            }
            None => {
                log(format!(
                    "Creating sandbox {} from {}",
                    spec.name, spec.image
                ));
                self.create(spec, &vm, log).await?
            }
        };
        let image = self.store.image(&record.image).with_context(|| {
            format!(
                "the image {} that {} was made from is gone",
                record.reference, spec.name
            )
        })?;
        let firecracker = self.firecracker(log).await?;
        let kernel = self.kernel(log).await?;
        let (initrd, agent) = self.initrd().await?;
        let passt = self
            .passt()
            .context("passt is not installed: it gives the VMs their network")?;
        let boot = Spec {
            kernel,
            initrd,
            boot: Boot {
                mode: Mode::Sandbox,
                name: spec.name.clone(),
                time_ms: Some(vsock::now_ms()),
            },
            drives: vec![
                Drive {
                    path: self.store.rootfs(&image.id),
                    writable: false,
                },
                Drive {
                    path: vm.dir.join("disk.ext4"),
                    writable: true,
                },
            ],
            cpus: spec.cpus,
            memory_mb: spec.memory_mb,
            network: true,
        };
        vm.start(&boot, &firecracker, Some(&passt)).await?;
        vm.wait_ready().await?;
        record.agent = Some(agent);
        self.save_record(&spec.name, &record)?;
        Ok(sb)
    }

    /// A new sandbox's directory, disk and record.
    async fn create(&self, spec: &SandboxSpec, vm: &Vm, log: Log<'_>) -> Result<Record> {
        let image = self.image_for(spec, log).await?;
        std::fs::create_dir_all(&vm.dir)?;
        use std::os::unix::fs::PermissionsExt;
        for dir in [&self.root, &vm.dir] {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let made = async {
            make_fs(
                &vm.dir.join("disk.ext4"),
                u64::from(self.cfg().disk_gb.max(1)),
                "fctl-disk",
                true,
            )
            .await?;
            let record = Record {
                image: image.id.clone(),
                reference: image.reference.clone(),
                created_ms: vsock::now_ms(),
                agent: None,
            };
            self.save_record(&spec.name, &record)?;
            Ok::<_, anyhow::Error>(record)
        }
        .await;
        if made.is_err() {
            let _ = std::fs::remove_dir_all(&vm.dir);
        }
        made
    }

    /// How programs run in the sandbox `name`: as its image says.
    fn exec_for(&self, name: &str, argv: Vec<String>) -> Exec {
        let image = self
            .record(name)
            .and_then(|r| self.store.image(&r.image))
            .unwrap_or_default();
        Exec {
            argv,
            user: image.user,
            env: image.env,
            cwd: image.workdir,
            ..Exec::default()
        }
    }

    fn sh(&self, name: &str, script: &str) -> Exec {
        self.exec_for(name, vec!["sh".into(), "-c".into(), script.into()])
    }

    pub async fn install_agent(&self, sb: &Sandbox, binary: &Path) -> Result<()> {
        // The VM's init put its own fctl-agent in place as it booted: the
        // host's, unless the host was updated since.
        let bytes = tokio::fs::read(binary).await?;
        let key = hex::encode(&Sha256::digest(&bytes)[..8]);
        if self.record(&sb.name).and_then(|r| r.agent) == Some(key.clone()) {
            return Ok(());
        }
        let vm = self.vm(&sb.name)?;
        let exec = Exec {
            argv: vec![
                "sh".into(),
                "-c".into(),
                format!(
                    "mkdir -p /opt/fctl/bin && cat > {AGENT_PATH}.new && chmod 755 {AGENT_PATH}.new && mv -f {AGENT_PATH}.new {AGENT_PATH}"
                ),
            ],
            user: Some("root".into()),
            cwd: Some("/".into()),
            ..self.sh(&sb.name, "")
        };
        let out = vsock::run(&vm.vsock(), exec, Some(&bytes)).await?;
        if out.code != 0 {
            bail!(
                "copying fctl-agent into the sandbox failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        if let Some(mut record) = self.record(&sb.name) {
            record.agent = Some(key);
            self.save_record(&sb.name, &record)?;
        }
        Ok(())
    }

    pub async fn start_agent(&self, sb: &Sandbox) -> Result<()> {
        let vm = self.vm(&sb.name)?;
        vsock::detach(&vm.vsock(), self.sh(&sb.name, &serve_script(AGENT_PATH))).await
    }

    pub async fn attach(&self, sb: &Sandbox) -> Result<AgentIo> {
        let vm = self.vm(&sb.name)?;
        let started =
            vsock::start(&vm.vsock(), self.sh(&sb.name, &attach_script(AGENT_PATH))).await?;
        // Why an attach failed, in the host's log.
        let name = sb.name.clone();
        let mut stderr = started.stderr;
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(&mut stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    tracing::warn!("attaching to {name}: {}", line.trim());
                }
            }
        });
        Ok(AgentIo {
            reader: Box::new(started.stdout),
            writer: Box::new(started.stdin),
            child: None,
        })
    }

    pub async fn exec(&self, sb: &Sandbox, script: &str) -> Result<String> {
        let vm = self.vm(&sb.name)?;
        let out = tokio::time::timeout(
            super::COMMAND_TIMEOUT,
            vsock::run(&vm.vsock(), self.sh(&sb.name, script), None),
        )
        .await
        .map_err(|_| super::TimedOut {
            command: format!("sh -c {script} in {}", sb.name),
            after: super::COMMAND_TIMEOUT,
        })??;
        if out.code != 0 {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub async fn spawn(&self, sb: &Sandbox, script: &str) -> Result<Running> {
        let vm = self.vm(&sb.name)?;
        let started = vsock::start(&vm.vsock(), self.sh(&sb.name, script)).await?;
        Ok(Running::vm(started))
    }

    pub async fn stop(&self, sb: &Sandbox) -> Result<()> {
        let lock = self.lock(&format!("sandbox {}", sb.name));
        let _held = lock.lock().await;
        self.vm(&sb.name)?.stop().await;
        Ok(())
    }

    pub async fn remove(&self, sb: &Sandbox) -> Result<()> {
        let lock = self.lock(&format!("sandbox {}", sb.name));
        let _held = lock.lock().await;
        self.vm(&sb.name)?.remove().await?;
        self.remove_unused();
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<SandboxInfo>> {
        let Ok(entries) = std::fs::read_dir(self.sandboxes()) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("fctl-") || self.record(&name).is_none() {
                continue;
            }
            let running = self.vm(&name).is_ok_and(|vm| vm.running());
            out.push(SandboxInfo { name, running });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver(dir: &Path) -> FirecrackerDriver {
        let host = HostConfig {
            data_dir: Some(dir.to_owned()),
            ..HostConfig::default()
        };
        FirecrackerDriver::new(&host)
    }

    #[tokio::test]
    async fn names_and_sandboxes_keep_their_images() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let d = driver(dir.path());
        let image = Image {
            id: format!("sha256:{}", "2".repeat(64)),
            reference: "ghcr.io/hyperstructure/sandbox:0.26.0".into(),
            labels: [(
                "org.opencontainers.image.revision".to_owned(),
                "f392b5c".to_owned(),
            )]
            .into(),
            ..Image::default()
        };
        let rootfs = dir.path().join("rootfs");
        std::fs::write(&rootfs, b"ext4")?;
        d.store.add(&image, &rootfs)?;
        let info = d.image("ghcr.io/hyperstructure/sandbox:0.26.0").await?;
        assert_eq!(info.map(|i| i.id), Some(image.id.clone()));
        d.tag_image(
            "ghcr.io/hyperstructure/sandbox:0.26.0",
            "ghcr.io/hyperstructure/sandbox:0.26.0-replaced-222222222222",
        )
        .await?;
        assert_eq!(d.images("ghcr.io/hyperstructure/sandbox").await?.len(), 2);
        // A sandbox made from it keeps its last name, as Docker does.
        let sandbox = d.sandboxes().join("fctl-ses-01abc");
        std::fs::create_dir_all(&sandbox)?;
        d.save_record(
            "fctl-ses-01abc",
            &Record {
                image: image.id.clone(),
                reference: image.reference.clone(),
                created_ms: 1,
                agent: None,
            },
        )?;
        d.remove_image("ghcr.io/hyperstructure/sandbox:0.26.0-replaced-222222222222")
            .await?;
        assert!(
            d.remove_image("ghcr.io/hyperstructure/sandbox:0.26.0")
                .await
                .is_err()
        );
        assert!(d.has_image("ghcr.io/hyperstructure/sandbox:0.26.0").await);
        // The sandbox, stopped, is listed, and runs programs as its image says.
        let listed = d.list().await?;
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].running);
        let exec = d.sh("fctl-ses-01abc", "true");
        assert_eq!(exec.argv, ["sh", "-c", "true"]);
        Ok(())
    }

    #[tokio::test]
    async fn probes_what_the_host_lacks() {
        let dir = tempfile::tempdir().ok();
        let d = driver(dir.as_ref().map_or(Path::new("/nonexistent"), |d| d.path()));
        let info = d.probe().await;
        assert_eq!(info.name, "firecracker");
        assert_eq!(info.isolation, "vm");
        // This sandbox has no KVM; elsewhere the detail says what is missing or that it's ready.
        if !Path::new("/dev/kvm").exists() {
            assert!(!info.available);
            assert!(info.detail.unwrap_or_default().contains("/dev/kvm"));
        }
    }

    /// Boots a sandbox from an image of Docker Hub (or
    /// `FCTL_TEST_FIRECRACKER_IMAGE`), on a host with KVM, Firecracker's
    /// needs and a static fctl-agent:
    ///
    /// ```sh
    /// cargo build --release -p fctl-agent --target "$(uname -m)-unknown-linux-musl"
    /// FCTL_TEST_FIRECRACKER_AGENT=$PWD/target/$(uname -m)-unknown-linux-musl/release/fctl-agent \
    ///   FCTL_TEST_FIRECRACKER_KERNEL=/path/to/vmlinux \
    ///   cargo test -p fctl-host boots_a_sandbox -- --ignored --nocapture
    /// ```
    ///
    /// `FCTL_TEST_FIRECRACKER_DATA` keeps the host's files (the image, and
    /// Firecracker) between runs.
    #[tokio::test]
    #[ignore = "needs KVM, passt, a kernel and a static fctl-agent"]
    async fn boots_a_sandbox() -> Result<()> {
        use super::super::Driver;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let var = |k: &str| std::env::var(k).with_context(|| format!("{k} isn't set"));
        let temp = tempfile::tempdir()?;
        let data = std::env::var("FCTL_TEST_FIRECRACKER_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| temp.path().to_owned());
        let mut cfg = HostConfig {
            data_dir: Some(data),
            agent_bin: Some(var("FCTL_TEST_FIRECRACKER_AGENT")?.into()),
            ..HostConfig::default()
        };
        cfg.firecracker.kernel = Some(var("FCTL_TEST_FIRECRACKER_KERNEL")?);
        cfg.firecracker.disk_gb = 4;
        let d = Driver::select("firecracker", &cfg).await?;
        let probed = d.probe().await;
        assert!(probed.available, "{probed:?}");
        let log = |line: String| eprintln!("{line}");
        let name = format!(
            "fctl-test-{}",
            hex::encode(fctl_domain::random_bytes::<4>())
        );
        const DEBIAN: &str = "docker.io/library/debian:trixie-slim";
        let image = std::env::var("FCTL_TEST_FIRECRACKER_IMAGE").unwrap_or_else(|_| DEBIAN.into());
        let spec = SandboxSpec {
            name: name.clone(),
            image: image.clone(),
            fallback_image: None,
            cpus: 2,
            memory_mb: 1024,
            cache: None,
        };
        let started = std::time::Instant::now();
        let sb = d.ensure(&spec, &log).await?;
        eprintln!("booted in {:?}", started.elapsed());
        let result = async {
            // As the image says, with the VM's name, and its network.
            let out = d.exec(&sb, "id -u; hostname; echo $HOME; pwd").await?;
            eprintln!("as {:?}", out.lines().collect::<Vec<_>>());
            if image == DEBIAN {
                assert_eq!(out, format!("0\n{name}\n/root\n/\n"));
            }
            assert_eq!(out.lines().nth(1), Some(name.as_str()));
            let resolved = d.exec(&sb, "getent hosts example.com").await?;
            assert!(!resolved.trim().is_empty());
            let fetched = d
                .exec(
                    &sb,
                    r#"exec bash -c 'exec 3<>/dev/tcp/example.com/80; printf "HEAD / HTTP/1.0\r\nHost: example.com\r\n\r\n" >&3; head -1 <&3'"#,
                )
                .await?;
            assert!(fetched.starts_with("HTTP/1."), "{fetched}");
            // A failing command says why; a program's input and output.
            let failed = d.exec(&sb, "echo oops >&2; exit 3").await;
            assert_eq!(failed.err().map(|e| e.to_string()).as_deref(), Some("oops"));
            let mut cat = d.spawn(&sb, "cat").await?;
            let mut stdin = cat.stdin.take().context("stdin")?;
            stdin.write_all(b"through the vsock").await?;
            stdin.shutdown().await?;
            drop(stdin);
            let mut echoed = String::new();
            cat.stdout.take().context("stdout")?.read_to_string(&mut echoed).await?;
            assert_eq!(echoed, "through the vsock");
            assert_eq!(cat.wait().await?, Some(0));
            // fctl-agent, as sessions start it.
            let agent = PathBuf::from(var("FCTL_TEST_FIRECRACKER_AGENT")?);
            d.install_agent(&sb, &agent).await?;
            d.start_agent(&sb).await?;
            let mut io = d.attach(&sb).await?;
            let mut first = [0u8; 1];
            let n = tokio::time::timeout(std::time::Duration::from_secs(60), io.reader.read(&mut first)).await??;
            assert_eq!(n, 1, "fctl-agent said nothing");
            drop(io);
            // A stopped sandbox keeps its disk, and starts again from it.
            d.exec(&sb, "echo kept > \"$HOME/note\"").await?;
            d.stop(&sb).await?;
            let listed = d.list().await?;
            assert!(listed.iter().any(|s| s.name == name && !s.running), "{listed:?}");
            d.ensure(&spec, &log).await?;
            assert_eq!(d.exec(&sb, "cat \"$HOME/note\"").await?, "kept\n");
            Ok::<_, anyhow::Error>(())
        }
        .await;
        d.remove(&sb).await?;
        assert!(!d.list().await?.iter().any(|s| s.name == name));
        result
    }
}
