//! The `firecracker` driver (Linux hosts with KVM): each sandbox is a
//! Firecracker microVM with a kernel of its own (ADR 0070).
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
//!   network over a vsock connection the host makes once the VM is up,
//!   without a TAP device or root on the host, run in process groups of
//!   their own, so that restarting the host leaves them running ([`vm`]).
//! - **The host runs programs** in the VM through its vsock, as `docker
//!   exec` would ([`vsock`]).

pub mod assets;
mod builds;
pub mod images;
pub mod oci;
pub mod registry;
pub mod vm;
pub mod vsock;

use super::{
    AGENT_DIR_SCRIPT, AGENT_PATH, AgentIo, DiskSpot, RAN_AS_ROOT_SCRIPT, Running, Sandbox,
    SandboxInfo, SandboxSpec, attach_as_root, attach_script, ran_as_root, serve_as_root,
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
use std::time::Duration;
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
    /// The VM's state, saved as it stopped (in `saved/`), which it resumes
    /// from.
    #[serde(default)]
    saved: Option<Saved>,
}

/// A VM's state that the host saved as the VM stopped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    /// The Firecracker release that saved it: only the same one loads it.
    firecracker: String,
    at_ms: i64,
}

/// What a VM's saved state may leave free, at least: below it, the VM stops
/// without saving it.
const SAVE_MARGIN_MB: u64 = 2048;

type Log<'a> = &'a (dyn Fn(String) + Send + Sync);

pub struct FirecrackerDriver {
    host: HostConfig,
    root: PathBuf,
    store: Store,
    /// One for each image being pulled and each sandbox being prepared.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// The jailer beside the Firecracker `firecracker`: `jailer`, or the
/// release's name for it, which the archive gives both (`jailer-v1.17.0`
/// beside `firecracker-v1.17.0`, `jailer-v1.17.0-x86_64` beside
/// `firecracker-v1.17.0-x86_64`).
fn jailer_beside(firecracker: &Path) -> Option<PathBuf> {
    let name = firecracker.file_name()?.to_string_lossy().into_owned();
    let released = name
        .strip_prefix("firecracker")
        .map(|rest| firecracker.with_file_name(format!("jailer{rest}")));
    [Some(firecracker.with_file_name("jailer")), released]
        .into_iter()
        .flatten()
        .find(|p| p.is_file())
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

/// Removes what is in `dir` for more than two days: what pulls left that
/// stopped halfway, when the host did.
fn remove_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(2 * 24 * 60 * 60));
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The layers' digests to keep for the image `resolved`: its config's
/// (`rootfs.diff_ids`), once those of the layers after the first `skip`
/// (which an image here has) are what the VM that unpacked them found,
/// `unpacked`. An image whose layers aren't what its config says isn't
/// kept, as Docker keeps none; one that a VM unpacked without saying is
/// kept with none, and no image lies over it.
fn checked_diff_ids(
    resolved: &registry::Resolved,
    skip: usize,
    unpacked: Option<&[String]>,
) -> Result<Vec<String>> {
    let Some(unpacked) = unpacked else {
        return Ok(Vec::new());
    };
    let claimed = resolved.diff_ids.get(skip..).unwrap_or_default();
    if claimed.len() != unpacked.len() {
        bail!(
            "its config lists {} layers, its manifest {}",
            resolved.diff_ids.len(),
            resolved.layers.len()
        );
    }
    for (i, (claimed, found)) in claimed.iter().zip(unpacked).enumerate() {
        if claimed != found {
            bail!(
                "its layer {} is {found} unpacked, where its config says {claimed}",
                skip + i + 1
            );
        }
    }
    Ok(resolved.diff_ids.clone())
}

/// Makes an ext4 filesystem in a new sparse file of `gb` GiB at `path`,
/// whose root is root's, labelled `label`, with a journal or not. It never
/// writes over a file there, a sandbox's disk say.
async fn make_fs(path: &Path, gb: u64, label: &str, journal: bool) -> Result<()> {
    let mkfs = find_program("mkfs.ext4").context("mkfs.ext4 is not installed (e2fsprogs)")?;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("making {}", path.display()))?;
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

/// The release of the Firecracker binary `firecracker` (`1.17.0`).
async fn binary_version(firecracker: &Path) -> Result<String> {
    let out = super::run_cmd(&firecracker.display().to_string(), &["--version"]).await?;
    out.lines()
        .next()
        .map(|l| {
            l.trim()
                .trim_start_matches("Firecracker ")
                .trim_start_matches('v')
                .to_owned()
        })
        .filter(|v| !v.is_empty())
        .with_context(|| format!("{} didn't say its version", firecracker.display()))
}

/// Gives back the disk that the pages of zeros in the file at `path` take,
/// which most of a VM's saved memory is.
fn punch_zeros(path: &Path) -> Result<()> {
    use nix::fcntl::{FallocateFlags, fallocate};
    use std::os::unix::fs::FileExt;
    const PAGE: usize = 4096;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let punch = |start: u64, end: u64| {
        let (Ok(start), Ok(len)) = (i64::try_from(start), i64::try_from(end - start)) else {
            return Ok(());
        };
        fallocate(
            &file,
            FallocateFlags::FALLOC_FL_PUNCH_HOLE | FallocateFlags::FALLOC_FL_KEEP_SIZE,
            start,
            len,
        )
    };
    let mut buf = vec![0u8; 1 << 20];
    let mut offset = 0u64;
    // Where the run of zero pages this is in started.
    let mut zeros: Option<u64> = None;
    loop {
        let n = file.read_at(&mut buf, offset)?;
        if n == 0 {
            break;
        }
        for (i, page) in buf[..n].chunks(PAGE).enumerate() {
            let at = offset + (i * PAGE) as u64;
            match (page.iter().all(|b| *b == 0), zeros) {
                (true, None) => zeros = Some(at),
                (false, Some(start)) => {
                    punch(start, at)?;
                    zeros = None;
                }
                _ => {}
            }
        }
        offset += n as u64;
    }
    if let Some(start) = zeros {
        punch(start, offset)?;
    }
    Ok(())
}

impl FirecrackerDriver {
    pub fn new(host: &HostConfig) -> Self {
        let root = host.data_dir().join("firecracker");
        Self {
            host: host.clone(),
            store: Store::new(root.join("images")),
            root,
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// The image store's lock ([`Store::lock`]), held while names move
    /// between images, images go, and a new sandbox records the image it is
    /// made from, so that none goes from under it.
    async fn lock_images(&self) -> Result<images::Lock> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.lock()).await?
    }

    /// Makes the driver's directory, which only this user may read: it
    /// holds the sandboxes' disks.
    fn make_root(&self) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("making {}", self.root.display()))?;
        let mode = std::fs::metadata(&self.root)?.permissions().mode();
        if mode & 0o077 != 0 {
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
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
        self.with_jails(Vm::new(self.sandboxes().join(name))?)
    }

    /// Where the VMs' jails go, when they run through Firecracker's jailer:
    /// on a host that runs as root, unless `host.toml` says otherwise.
    fn jails(&self) -> Option<PathBuf> {
        (self.cfg().jailer && nix::unistd::geteuid().is_root()).then(|| self.root.join("jail"))
    }

    fn with_jails(&self, vm: Vm) -> Result<Vm> {
        Ok(match self.jails() {
            Some(jails) => vm.with_jails(jails),
            None => vm,
        })
    }

    /// How `vm` runs: through Firecracker's jailer, on a host that runs as
    /// root ([`jailer`](Self::jailer)); or without it, where the host
    /// doesn't run as root.
    fn jail_for(&self, vm: &Vm, firecracker: &Path) -> Result<Option<vm::Jail>> {
        if vm.jails.is_none() {
            return Ok(None);
        }
        let jailer = self.jailer(firecracker).with_context(|| {
            format!(
                "Firecracker's jailer isn't beside {} (as jailer, or named as the release names it) or on the PATH: install it there, or set jailer = false in host.toml's [firecracker]",
                firecracker.display()
            )
        })?;
        Ok(Some(vm::Jail { jailer }))
    }

    /// Firecracker's jailer for `firecracker`: the pinned release's, which
    /// comes with it; else beside it, as `jailer` or as the release names it
    /// (`jailer-v1.17.0-x86_64` beside `firecracker-v1.17.0-x86_64`); else
    /// on the PATH.
    fn jailer(&self, firecracker: &Path) -> Option<PathBuf> {
        let pinned = assets::pinned_jailer(&self.root.join("bin"));
        if self.cfg().firecracker.is_none() {
            return Some(pinned);
        }
        jailer_beside(firecracker).or_else(|| find_program("jailer"))
    }

    /// Removes the jails of VMs that don't run, which a host that stopped
    /// or crashed left ([`vm::remove_stale_jails`]).
    fn remove_stale_jails(&self) {
        let Some(jails) = self.jails() else {
            return;
        };
        let running: std::collections::HashSet<String> =
            [self.sandboxes(), self.root.join("unpacking")]
                .iter()
                .filter_map(|d| std::fs::read_dir(d).ok())
                .flat_map(|entries| entries.flatten())
                .filter_map(|e| Vm::new(e.path()).ok())
                .filter(Vm::running)
                .map(|vm| vm.key)
                .collect();
        vm::remove_stale_jails(&jails, &running, Duration::from_secs(10 * 60));
    }

    fn record(&self, name: &str) -> Option<Record> {
        let bytes = std::fs::read(self.sandboxes().join(name).join("sandbox.json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Writes the record whole or not at all: a sandbox whose record can't
    /// be read is one the host can't start.
    fn save_record(&self, name: &str, record: &Record) -> Result<()> {
        let path = self.sandboxes().join(name).join("sandbox.json");
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec_pretty(record)?)?;
        std::fs::rename(&tmp, &path)?;
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
                self.make_root()?;
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
        self.make_root()?;
        let lock = self.lock("kernel");
        let _held = lock.lock().await;
        assets::kernel(&kernel, &self.root.join("kernels"), log).await
    }

    /// The initramfs, and the start of the SHA-256 of the fctl-agent in it.
    async fn initrd(&self) -> Result<(PathBuf, String)> {
        let agent = crate::agentbin::linux_agent(&self.host)
            .await
            .context("getting fctl-agent for the VMs")?;
        self.make_root()?;
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
        if let Some(jails) = self.jails() {
            if let Some(why) = vm::jails_problem(&jails) {
                return info(false, why, version);
            }
            if let Some(b) = &binary
                && self.cfg().firecracker.is_some()
                && self.jailer(b).is_none()
            {
                return info(
                    false,
                    format!(
                        "Firecracker's jailer isn't beside {} (as jailer, or named as the release names it) or on the PATH: install it there, or set jailer = false in host.toml's [firecracker]",
                        b.display()
                    ),
                    version,
                );
            }
        }
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

    /// What `image` takes on the disk, the images it lies over included.
    pub async fn image_size(&self, image: &str) -> Option<u64> {
        let image = self.store.lookup(image)?;
        use std::os::unix::fs::MetadataExt;
        let mut size = 0;
        for i in self.store.chain(&image.id)? {
            size += std::fs::metadata(self.store.rootfs(&i.id)).ok()?.blocks() * 512;
        }
        Some(size)
    }

    pub async fn tag_image(&self, image: &str, name: &str) -> Result<()> {
        let _images = self.lock_images().await?;
        let here = self
            .store
            .lookup(image)
            .with_context(|| format!("{image} is not on this host"))?;
        self.store.tag(name, &here.id)
    }

    pub async fn images(&self, repository: &str) -> Result<Vec<String>> {
        Ok(self.store.names(repository))
    }

    /// The images that stay: those a name refers to, those a sandbox here
    /// was made from, and those any of them lies over.
    fn needed(&self) -> std::collections::HashSet<String> {
        let mut needed = std::collections::HashSet::new();
        let roots = self
            .store
            .refs()
            .into_values()
            .chain(self.images_in_use())
            .chain(self.store.held());
        for id in roots {
            match self.store.chain(&id) {
                Some(chain) => needed.extend(chain.into_iter().map(|i| i.id)),
                None => {
                    needed.insert(id);
                }
            }
        }
        needed
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
        let _images = self.lock_images().await?;
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
        // Its filesystem stays while an image here lies over it, and those
        // it lay over go when nothing else needs them.
        if !self.needed().contains(&here.id) {
            self.store.remove(&here.id)?;
        }
        self.remove_unused();
        Ok(())
    }

    /// Removes the images no name refers to, no sandbox uses and no other
    /// image that stays lies over: what a newer pull of their name left.
    fn remove_unused(&self) {
        let needed = self.needed();
        for id in self.store.ids() {
            if !needed.contains(&id)
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
        self.make_root()?;
        let lock = self.lock(&format!("pull {key}"));
        let _held = lock.lock().await;
        let registry = registry::Registry::new();
        let resolved = registry.resolve(image, std::env::consts::ARCH).await?;
        if self.store.image(&resolved.id).is_some() {
            // One an earlier fctl pulled has no layers' digests, so no
            // image lies over it: the config's, which no VM checked, won't
            // do, and this only names it.
            let _images = self.lock_images().await?;
            self.store.tag(image, &resolved.id)?;
            self.remove_unused();
            return Ok(());
        }
        // An image here whose layers it starts with: only the others come.
        let (base, _held) = self.base_for(&resolved).await?.unzip();
        let skip = base.as_ref().map_or(0, |b| b.diff_ids.len());
        let total: u64 = resolved.layers[skip..].iter().map(|l| l.size).sum();
        match &base {
            Some(b) => log(format!(
                "Pulling {image}: {} layers over {}'s {skip}, {} MB",
                resolved.layers.len() - skip,
                b.reference,
                total.div_ceil(1_000_000)
            )),
            None => log(format!(
                "Pulling {image}: {} layers, {} MB",
                resolved.layers.len(),
                total.div_ceil(1_000_000)
            )),
        }
        // A directory of this pull's own: another pull of the same image, by
        // another name or in another process, has another.
        remove_stale(&self.root.join("downloads"));
        let downloads = self.root.join("downloads").join(format!(
            "{}-{}",
            hex::encode(&Sha256::digest(resolved.id.as_bytes())[..8]),
            hex::encode(fctl_domain::random_bytes::<6>())
        ));
        let result = async {
            let (rootfs, unpacked) = self
                .download_and_unpack(image, &registry, &resolved, skip, &downloads, log)
                .await?;
            let diff_ids = checked_diff_ids(&resolved, skip, unpacked.diff_ids.as_deref())
                .with_context(|| format!("{image} isn't what its config says"))?;
            let (files, bytes) = (unpacked.files, unpacked.bytes);
            let base = base.map(|b| b.id);
            self.keep(image, key, &resolved, base, diff_ids, &rootfs, files, bytes)
                .await?;
            Ok::<_, anyhow::Error>((files, bytes))
        }
        .await;
        let _ = std::fs::remove_dir_all(&downloads);
        let (files, bytes) = result?;
        log(format!(
            "Pulled {image}: {files} files, {} MB",
            bytes.div_ceil(1_000_000)
        ));
        Ok(())
    }

    /// The image here that `resolved` can lie over ([`Store::base_for`]),
    /// when its config says its layers' digests, held until `resolved` is
    /// kept: nothing removes it meanwhile.
    async fn base_for(
        &self,
        resolved: &registry::Resolved,
    ) -> Result<Option<(Image, images::Held)>> {
        if resolved.diff_ids.len() != resolved.layers.len() {
            return Ok(None);
        }
        let _images = self.lock_images().await?;
        let Some(base) = self.store.base_for(&resolved.diff_ids) else {
            return Ok(None);
        };
        let held = self.store.hold(&base.id)?;
        Ok(Some((base, held)))
    }

    /// Keeps the image `resolved`, whose layers unpacked to `rootfs`
    /// (`files` and `bytes`; over `base`'s, those `base` doesn't have),
    /// named `name` (`key` as the store keeps it), with its layers'
    /// digests as a VM found them ([`checked_diff_ids`]).
    #[allow(clippy::too_many_arguments)]
    async fn keep(
        &self,
        name: &str,
        key: String,
        resolved: &registry::Resolved,
        base: Option<String>,
        diff_ids: Vec<String>,
        rootfs: &Path,
        files: u64,
        bytes: u64,
    ) -> Result<()> {
        let config = resolved.config.clone();
        let image = Image {
            id: resolved.id.clone(),
            reference: key,
            user: config.user.filter(|u| !u.is_empty()),
            env: config.env.unwrap_or_default(),
            workdir: config.working_dir.filter(|w| !w.is_empty()),
            labels: config.labels.unwrap_or_default(),
            files,
            bytes,
            pulled_ms: vsock::now_ms(),
            diff_ids,
            base,
        };
        let _images = self.lock_images().await?;
        // The image it lies over went meanwhile.
        if let Some(base) = &image.base
            && self.store.image(base).is_none()
        {
            bail!("the image {name} was pulled over went as it was pulled");
        }
        // Another pull may have brought the same image meanwhile, which
        // sandboxes may use already.
        if self.store.image(&image.id).is_some() {
            self.store.tag(name, &image.id)?;
        } else {
            self.store.add(&image, rootfs)?;
        }
        self.remove_unused();
        Ok(())
    }

    /// Downloads `resolved`'s layers but the first `skip`, which an image
    /// here has, and unpacks them in a VM: as overlayfs reads them, to lie
    /// over that image's, when it skips any.
    async fn download_and_unpack(
        &self,
        image: &str,
        registry: &registry::Registry,
        resolved: &registry::Resolved,
        skip: usize,
        downloads: &Path,
        log: Log<'_>,
    ) -> Result<(PathBuf, vsock::Unpacked)> {
        std::fs::create_dir_all(downloads)?;
        let mut files = Vec::new();
        for (i, layer) in resolved.layers.iter().enumerate().skip(skip) {
            let path = downloads.join(format!("layer-{i}"));
            registry.blob(image, layer, &path).await?;
            files.push(path);
        }
        log(format!("Unpacking {image} in a VM"));
        let rootfs = downloads.join("rootfs.ext4");
        // What a pull that stopped halfway left.
        let _ = std::fs::remove_file(&rootfs);
        make_fs(&rootfs, IMAGE_FS_GB, "fctl-image", false).await?;
        let layers: Vec<fctl_proto::vm::Layer> = resolved.layers[skip..]
            .iter()
            .map(|l| fctl_proto::vm::Layer {
                media_type: l.media_type.clone(),
                size: l.size,
            })
            .collect();
        let unpacked = self.unpack(&rootfs, &layers, &files, skip > 0, log).await?;
        Ok((rootfs, unpacked))
    }

    /// Unpacks `layers`, the files `blobs`, onto the filesystem `rootfs`
    /// in a VM booted for it; what it wrote. None, for an image that only
    /// sets its config over another, needs no VM.
    async fn unpack(
        &self,
        rootfs: &Path,
        layers: &[fctl_proto::vm::Layer],
        blobs: &[PathBuf],
        overlay: bool,
        log: Log<'_>,
    ) -> Result<vsock::Unpacked> {
        if layers.is_empty() {
            return Ok(vsock::Unpacked {
                diff_ids: Some(Vec::new()),
                ..vsock::Unpacked::default()
            });
        }
        let firecracker = self.firecracker(log).await?;
        let kernel = self.kernel(log).await?;
        let (initrd, _) = self.initrd().await?;
        let name = format!(
            "fctl-unpack-{}",
            hex::encode(fctl_domain::random_bytes::<6>())
        );
        remove_stale(&self.root.join("unpacking"));
        self.remove_stale_jails();
        let vm = self.with_jails(Vm::new(self.root.join("unpacking").join(&name))?)?;
        let spec = vm::unpack_spec(kernel, initrd, &name, rootfs);
        self.remove_stale_jails();
        let jail = self.jail_for(&vm, &firecracker)?;
        vm.start(&spec, &firecracker, jail.as_ref()).await?;
        let result = async {
            vm.wait_ready().await?;
            let within =
                registry::LAYER_TIME * u32::try_from(blobs.len().max(1)).unwrap_or(u32::MAX);
            vsock::unpack(&vm.vsock(), layers, blobs, overlay, within).await
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
        self.make_root()?;
        let lock = self.lock(&format!("sandbox {}", spec.name));
        let _held = lock.lock().await;
        let vm = self.vm(&spec.name)?;
        // Paused while the disk was nearly full: the session that starts
        // in it now wants it to run.
        if vm.running() && vm.paused().await {
            log(format!("Resuming the paused sandbox {}", spec.name));
            vm.resume().await?;
        }
        if vm.running() {
            let socket = vm.vsock();
            let answers = tokio::time::timeout(Duration::from_secs(10), vsock::hello(&socket));
            if matches!(answers.await, Ok(Ok(_))) {
                // Running, its disk is past any state saved before.
                self.drop_saved(&vm, &spec.name)?;
                // Its passt exited, or was stopped: the VM gets another.
                if !vm.linked() {
                    log(format!("Linking the network of {} again", spec.name));
                    let passt = self
                        .passt()
                        .context("passt is not installed: it gives the VMs their network")?;
                    vm.link_network(&passt).await?;
                }
                return Ok(sb);
            }
            log(format!(
                "Starting the sandbox {} again: its VM doesn't answer",
                spec.name
            ));
            vm.kill().await;
        }
        // A host that keeps no states (`snapshots = false`) resumes none it
        // kept before, nor does a jailed VM ([`stop`](Self::stop)) one saved
        // before the host ran as root: they go.
        if let Some(saved) = self.record(&spec.name).and_then(|r| r.saved)
            && self.cfg().snapshots
            && vm.jails.is_none()
            && !self.resized(&vm, spec, log)
        {
            match self.resume_saved(&vm, &spec.name, &saved, log).await {
                Ok(()) => return Ok(sb),
                Err(e) => log(format!(
                    "{} couldn't resume where it stopped ({e:#}), so it starts again from its disk",
                    spec.name
                )),
            }
        }
        // A state saved as the host stopped, before it could record it.
        self.drop_saved(&vm, &spec.name)?;
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
        // A sandbox whose making stopped after its record has no disk yet.
        if !vm.dir.join("disk.ext4").exists() {
            self.make_disk(&vm).await?;
        }
        let image = self.store.image(&record.image).with_context(|| {
            format!(
                "the image {} that {} was made from is gone",
                record.reference, spec.name
            )
        })?;
        let chain = self.store.chain(&image.id).with_context(|| {
            format!(
                "an image that {}, which {} was made from, lies over is gone",
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
                dns: vm::vm_dns(),
                lowers: u32::try_from(chain.len()).unwrap_or(1),
            },
            // The images' filesystems, the one at the bottom first, then
            // the sandbox's own disk.
            drives: chain
                .iter()
                .map(|i| Drive {
                    path: self.store.rootfs(&i.id),
                    writable: false,
                })
                .chain(std::iter::once(Drive {
                    path: vm.dir.join("disk.ext4"),
                    writable: true,
                }))
                .collect(),
            cpus: spec.cpus,
            memory_mb: spec.memory_mb,
            network: true,
        };
        let jail = self.jail_for(&vm, &firecracker)?;
        vm.start(&boot, &firecracker, jail.as_ref()).await?;
        vm.wait_ready().await?;
        if let Err(e) = vm.link_network(&passt).await {
            vm.kill().await;
            return Err(e);
        }
        record.agent = Some(agent);
        self.save_record(&spec.name, &record)?;
        Ok(sb)
    }

    /// A new sandbox's directory and record, which come before its disk
    /// ([`make_disk`](Self::make_disk)), so that a disk is never without a
    /// record: a disk without one is a sandbox whose record something else
    /// removed, which the host leaves alone.
    async fn create(&self, spec: &SandboxSpec, vm: &Vm, log: Log<'_>) -> Result<Record> {
        if vm.dir.join("disk.ext4").exists() {
            bail!(
                "{} has a disk but no record of the image it was made from ({} is missing), so the host leaves it as it is",
                spec.name,
                vm.dir.join("sandbox.json").display()
            );
        }
        let pulled = self.image_for(spec, log).await?;
        // Until the record says which image the sandbox has, nothing removes it.
        let _images = self.lock_images().await?;
        let image = self
            .store
            .image(&pulled.id)
            .with_context(|| format!("{} went as the sandbox was made", pulled.reference))?;
        self.make_root()?;
        std::fs::create_dir_all(&vm.dir)?;
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&vm.dir, std::fs::Permissions::from_mode(0o700));
        let record = Record {
            image: image.id.clone(),
            reference: image.reference.clone(),
            created_ms: vsock::now_ms(),
            agent: None,
            saved: None,
        };
        self.save_record(&spec.name, &record)?;
        Ok(record)
    }

    /// The sandbox's own disk, made under another name and then put in
    /// place, so that one that stopped halfway is never booted.
    async fn make_disk(&self, vm: &Vm) -> Result<()> {
        let disk = vm.dir.join("disk.ext4");
        let new = vm.dir.join("disk.ext4.new");
        let _ = std::fs::remove_file(&new);
        make_fs(
            &new,
            u64::from(self.cfg().disk_gb.max(1)),
            "fctl-disk",
            true,
        )
        .await?;
        std::fs::rename(&new, &disk)?;
        Ok(())
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
                    "{AGENT_DIR_SCRIPT} && cat > {AGENT_PATH}.new && chmod 755 {AGENT_PATH}.new && mv -f {AGENT_PATH}.new {AGENT_PATH}"
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

    /// Starts fctl-agent as root, and Claude and its commands as the
    /// image's user, who can't stop it then; as the image's user too where
    /// the host doesn't run it as root.
    pub async fn start_agent(&self, sb: &Sandbox) -> Result<()> {
        let vm = self.vm(&sb.name)?;
        let exec = if self.agent_root(sb).await? {
            self.serve(&sb.name)
        } else {
            tracing::info!(
                "fctl-agent runs as {}'s user (agent_as_root = false), so the agent's commands can stop it",
                sb.name
            );
            self.sh(&sb.name, &serve_script(AGENT_PATH))
        };
        vsock::detach(&vm.vsock(), exec).await
    }

    /// Whether fctl-agent runs as root in `sb`: where the host runs it so,
    /// and where it ran so before ([`AGENT_DIR`] is there), which keeps it
    /// so, rather than start another beside one that may run.
    /// An error when the VM doesn't say.
    async fn agent_root(&self, sb: &Sandbox) -> Result<bool> {
        if self.host.agent_as_root {
            return Ok(true);
        }
        let said = self
            .exec(sb, RAN_AS_ROOT_SCRIPT)
            .await
            .context("asking whether fctl-agent ran as root in the sandbox")?;
        ran_as_root(&said)
    }

    /// `fctl-agent serve` as root, for the image's user.
    fn serve(&self, name: &str) -> Exec {
        let exec = self.exec_for(name, Vec::new());
        let user = exec.user.clone().unwrap_or_else(|| "root".into());
        Exec {
            argv: serve_as_root(&user),
            user: Some("root".into()),
            cwd: Some("/".into()),
            ..exec
        }
    }

    pub async fn attach(&self, sb: &Sandbox) -> Result<AgentIo> {
        let vm = self.vm(&sb.name)?;
        let exec = if self.agent_root(sb).await? {
            Exec {
                argv: attach_as_root(),
                user: Some("root".into()),
                cwd: Some("/".into()),
                ..self.exec_for(&sb.name, Vec::new())
            }
        } else {
            self.sh(&sb.name, &attach_script(AGENT_PATH))
        };
        let started = vsock::start(&vm.vsock(), exec).await?;
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
            let said = String::from_utf8_lossy(&out.stderr);
            match said.trim() {
                "" => bail!("the command exited with {} in {}", out.code, sb.name),
                said => bail!("{said}"),
            }
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub async fn spawn(&self, sb: &Sandbox, script: &str) -> Result<Running> {
        let vm = self.vm(&sb.name)?;
        let started = vsock::start(&vm.vsock(), self.sh(&sb.name, script)).await?;
        Ok(Running::vm(started))
    }

    /// Pauses the sandbox's VM, which holds what it has yet to write in its
    /// memory until it resumes.
    pub async fn pause(&self, sb: &Sandbox) -> Result<()> {
        let vm = self.vm(&sb.name)?;
        if !vm.running() {
            bail!("the VM of {} doesn't run", sb.name);
        }
        vm.pause().await
    }

    pub async fn resume(&self, sb: &Sandbox) -> Result<()> {
        self.vm(&sb.name)?.resume().await
    }

    /// Whether the sandbox's VM is paused.
    pub async fn paused(&self, sb: &Sandbox) -> bool {
        match self.vm(&sb.name) {
            Ok(vm) => vm.running() && vm.paused().await,
            Err(_) => false,
        }
    }

    /// Stops the sandbox's VM, saving its state first where the host
    /// keeps them (`snapshots`) and the disk has room for its memory, so
    /// that it resumes where it was. A VM paused while the disk was nearly
    /// full stops without, as does one in the jailer's chroot, which
    /// Firecracker would write its state into and which goes as it stops,
    /// and an image builder, which boots afresh for each build.
    pub async fn stop(&self, sb: &Sandbox) -> Result<()> {
        let lock = self.lock(&format!("sandbox {}", sb.name));
        let _held = lock.lock().await;
        let vm = self.vm(&sb.name)?;
        if self.cfg().snapshots
            && vm.jails.is_none()
            && !crate::driver::is_builder(&sb.name)
            && vm.running()
            && !vm.paused().await
        {
            match self.save(&vm, &sb.name).await {
                Ok(true) => {
                    vm.kill().await;
                    return Ok(());
                }
                Ok(false) => {}
                Err(e) => tracing::warn!(
                    "saving the state of {} failed, so it stops without it: {e:#}",
                    sb.name
                ),
            }
        }
        vm.stop().await;
        Ok(())
    }

    /// Saves the state of `vm`, which runs, in its `saved/`, unless the disk
    /// lacks room for its memory; whether it did. What the VM has yet to
    /// write goes to its disk first, so that one whose state doesn't come
    /// back boots from a disk as whole as a clean stop leaves it.
    async fn save(&self, vm: &Vm, name: &str) -> Result<bool> {
        // One at a time: sandboxes that stop together (a host that drains)
        // would each find room for their memory, and all write it at once.
        let lock = self.lock("saving");
        let _held = lock.lock().await;
        let memory_mb = vm.memory_mb().unwrap_or(u64::MAX / 2);
        let floor = self
            .host
            .min_free_disk_mb()
            .unwrap_or(0)
            .max(SAVE_MARGIN_MB);
        let free = crate::sysinfo::disk_available_mb(&vm.dir).await;
        if free.is_none_or(|free| free < memory_mb.saturating_add(floor)) {
            tracing::info!(
                "{name} stops without saving its state: the disk lacks room for its {memory_mb} MiB of memory"
            );
            return Ok(false);
        }
        let firecracker = vm.firecracker_version().await?;
        let sync = Exec {
            argv: vec!["sync".into()],
            user: Some("root".into()),
            cwd: Some("/".into()),
            ..Exec::default()
        };
        let synced =
            tokio::time::timeout(Duration::from_secs(60), vsock::run(&vm.vsock(), sync, None))
                .await;
        if !matches!(synced, Ok(Ok(vsock::Output { code: 0, .. }))) {
            tracing::warn!("{name}'s disk wasn't written out before its state was saved");
        }
        let (dir, new) = (vm.dir.join("saved"), vm.dir.join("saved.new"));
        let _ = std::fs::remove_dir_all(&new);
        let _ = std::fs::remove_dir_all(&dir);
        let saved = async {
            vm.snapshot(&new).await?;
            // The memory the VM never touched, or gave back, takes no disk.
            let memory = new.join("memory");
            if let Err(e) = tokio::task::spawn_blocking(move || punch_zeros(&memory)).await? {
                tracing::warn!("giving back the disk {name}'s memory doesn't use: {e:#}");
            }
            std::fs::rename(&new, &dir)?;
            let Some(mut record) = self.record(name) else {
                bail!("{name} has no record to keep its saved state in");
            };
            record.saved = Some(Saved {
                firecracker,
                at_ms: vsock::now_ms(),
            });
            self.save_record(name, &record)
        }
        .await;
        // What a save that failed left takes as much disk as the VM's memory.
        if saved.is_err() {
            let _ = std::fs::remove_dir_all(&new);
            let _ = std::fs::remove_dir_all(&dir);
        }
        saved.map(|()| true)
    }

    /// Starts the VM of the sandbox `name` from the state it saved as it
    /// stopped, with its programs running on, then links its network and
    /// lets the state go: the VM's disk is past it once the VM runs.
    /// Fails, letting it go too, when the state can't come back: another
    /// Firecracker saved it, say.
    async fn resume_saved(&self, vm: &Vm, name: &str, saved: &Saved, log: Log<'_>) -> Result<()> {
        let dir = vm.dir.join("saved");
        let restored = async {
            if !dir.join("vmstate").is_file() || !dir.join("memory").is_file() {
                bail!("its saved state is incomplete");
            }
            let firecracker = self.firecracker(log).await?;
            let version = binary_version(&firecracker).await?;
            if version != saved.firecracker {
                bail!(
                    "Firecracker {} saved its state, and this host runs {version}",
                    saved.firecracker
                );
            }
            log(format!("Resuming the sandbox {name} where it stopped"));
            vm.restore(&dir, &firecracker, name).await
        }
        .await;
        // Firecracker maps the memory it loaded, which stays while it runs.
        self.drop_saved(vm, name)?;
        restored?;
        vm.wait_ready().await?;
        let passt = self
            .passt()
            .context("passt is not installed: it gives the VMs their network")?;
        if let Err(e) = vm.link_network(&passt).await {
            vm.kill().await;
            return Err(e);
        }
        Ok(())
    }

    /// Whether the session asks `vm`, stopped, for another size than it
    /// had: its saved state, which has the old one, then won't do.
    fn resized(&self, vm: &Vm, spec: &SandboxSpec, log: Log<'_>) -> bool {
        let (cpus, memory_mb) = vm::size(spec.cpus, spec.memory_mb);
        let had = (vm.cpus(), vm.memory_mb());
        if had == (Some(u64::from(cpus)), Some(u64::from(memory_mb))) {
            return false;
        }
        log(format!(
            "{} asks for {cpus} CPUs and {memory_mb} MiB now, so it starts again from its disk rather than where it stopped",
            spec.name
        ));
        true
    }

    /// Lets the state `name` saved go, files and record.
    fn drop_saved(&self, vm: &Vm, name: &str) -> Result<()> {
        // Saved, or half saved as a save failed or the host stopped.
        for dir in [vm.dir.join("saved"), vm.dir.join("saved.new")] {
            if dir.exists() {
                std::fs::remove_dir_all(&dir)
                    .with_context(|| format!("removing {}", dir.display()))?;
            }
        }
        if let Some(mut record) = self.record(name)
            && record.saved.take().is_some()
        {
            self.save_record(name, &record)?;
        }
        Ok(())
    }

    /// Lets the states that stopped sandboxes saved go, but those of `keep`,
    /// to make room on the disk: their sandboxes start again from their
    /// disks. What that freed, in MiB.
    pub async fn drop_saved_states(&self, keep: &std::collections::HashSet<String>) -> u64 {
        let Ok(entries) = std::fs::read_dir(self.sandboxes()) else {
            return 0;
        };
        let mut freed = 0;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let saved = entry.path().join("saved");
            let half = entry.path().join("saved.new");
            if keep.contains(&name) || !(saved.is_dir() || half.is_dir()) {
                continue;
            }
            let lock = self.lock(&format!("sandbox {name}"));
            let _held = lock.lock().await;
            let Ok(vm) = self.vm(&name) else { continue };
            if vm.running() {
                continue;
            }
            let size = crate::caches::dir_size_mb(&saved).unwrap_or(0)
                + crate::caches::dir_size_mb(&half).unwrap_or(0);
            match self.drop_saved(&vm, &name) {
                Ok(()) => freed += size,
                Err(e) => tracing::warn!("letting {name}'s saved state go: {e:#}"),
            }
        }
        freed
    }

    pub async fn remove(&self, sb: &Sandbox) -> Result<()> {
        let lock = self.lock(&format!("sandbox {}", sb.name));
        let _held = lock.lock().await;
        let vm = self.vm(&sb.name)?;
        // A builder runs only while it builds, and starts under this lock:
        // one that the sweep found stopped, and that builds now, stays.
        if crate::driver::is_builder(&sb.name) && vm.running() {
            bail!("it is building an image");
        }
        vm.remove().await?;
        let _images = self.lock_images().await?;
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

    /// An image is kept with its config's layer digests only once the VM
    /// that unpacked its layers found them so; one whose layers aren't
    /// what its config says isn't kept.
    #[test]
    fn layers_are_what_their_config_says() {
        let digest = |n: u8| format!("sha256:{}", n.to_string().repeat(64));
        let resolved = registry::Resolved {
            id: digest(9),
            config: registry::Config::default(),
            layers: (0..3)
                .map(|_| registry::Descriptor {
                    media_type: String::new(),
                    digest: digest(8),
                    size: 1,
                })
                .collect(),
            diff_ids: vec![digest(1), digest(2), digest(3)],
        };
        let found = |d: &[u8]| d.iter().map(|n| digest(*n)).collect::<Vec<_>>();
        let checked = |skip, unpacked: Option<&[String]>| {
            checked_diff_ids(&resolved, skip, unpacked).map_err(|e| e.to_string())
        };
        assert_eq!(
            checked(0, Some(&found(&[1, 2, 3]))),
            Ok(resolved.diff_ids.clone())
        );
        // Over an image here that has the first two.
        assert_eq!(
            checked(2, Some(&found(&[3]))),
            Ok(resolved.diff_ids.clone())
        );
        // Over one that has all of them: nothing to unpack.
        assert_eq!(checked(3, Some(&[])), Ok(resolved.diff_ids.clone()));
        assert_eq!(
            checked(1, Some(&found(&[2, 4]))),
            Err(format!(
                "its layer 3 is {} unpacked, where its config says {}",
                digest(4),
                digest(3)
            ))
        );
        assert_eq!(
            checked(0, Some(&found(&[1, 2]))),
            Err("its config lists 3 layers, its manifest 3".to_owned())
        );
        // An fctl-agent that doesn't say: kept with none.
        assert_eq!(checked(0, None), Ok(Vec::new()));
    }

    /// What a pull or a build lies a new image over stays, though nothing
    /// else needs it, until it is done.
    #[tokio::test]
    async fn an_image_being_lain_over_stays() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let d = driver(dir.path());
        let base = Image {
            id: format!("sha256:{}", "3".repeat(64)),
            reference: "docker.io/library/debian:trixie".into(),
            diff_ids: vec![format!("sha256:{}", "4".repeat(64))],
            ..Image::default()
        };
        let rootfs = dir.path().join("rootfs");
        std::fs::write(&rootfs, b"ext4")?;
        d.store.add(&base, &rootfs)?;
        let resolved = registry::Resolved {
            id: format!("sha256:{}", "5".repeat(64)),
            config: registry::Config::default(),
            layers: (0..2)
                .map(|_| registry::Descriptor {
                    media_type: String::new(),
                    digest: String::new(),
                    size: 1,
                })
                .collect(),
            diff_ids: vec![
                base.diff_ids[0].clone(),
                format!("sha256:{}", "6".repeat(64)),
            ],
        };
        let (chosen, held) = d.base_for(&resolved).await?.context("no base")?;
        assert_eq!(chosen.id, base.id);
        // Its name goes meanwhile, and no sandbox uses it.
        d.remove_image("docker.io/library/debian:trixie").await?;
        assert!(d.store.image(&base.id).is_some());
        drop(held);
        d.remove_unused();
        assert!(d.store.image(&base.id).is_none());
        Ok(())
    }

    /// Beside a Firecracker that `host.toml` names, the jailer is `jailer`
    /// or named as the release names it.
    #[test]
    fn finds_the_jailer_beside_firecracker() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let firecracker = dir.path().join("firecracker-v1.17.0-x86_64");
        std::fs::write(&firecracker, b"")?;
        assert_eq!(jailer_beside(&firecracker), None);
        let released = dir.path().join("jailer-v1.17.0-x86_64");
        std::fs::write(&released, b"")?;
        assert_eq!(jailer_beside(&firecracker), Some(released));
        let plain = dir.path().join("jailer");
        std::fs::write(&plain, b"")?;
        assert_eq!(jailer_beside(&firecracker), Some(plain.clone()));
        assert_eq!(jailer_beside(&dir.path().join("firecracker")), Some(plain));
        Ok(())
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
                saved: None,
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

    /// A builder that the sweep found stopped, and that builds by the time
    /// its removal comes up, stays: it runs only while it builds.
    #[tokio::test]
    async fn a_builder_that_builds_stays() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let d = driver(dir.path());
        let name = "fctl-builder-prj-01abc";
        let sb = Sandbox {
            name: name.into(),
            root: None,
        };
        let vm = d.vm(name)?;
        std::fs::create_dir_all(d.sandboxes().join(name))?;
        // A process whose command line has the VM's socket directory.
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 30; true", &vm.api().display().to_string()])
            .spawn()?;
        let pid_file = d.sandboxes().join(name).join("firecracker.pid");
        std::fs::write(&pid_file, child.id().to_string())?;
        // Until its exec, other tests' threads may show it as this process.
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while !vm.running() && std::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(vm.running());
        let removed = d.remove(&sb).await;
        assert_eq!(
            removed.map_err(|e| e.to_string()),
            Err("it is building an image".to_owned())
        );
        assert!(d.sandboxes().join(name).exists());
        child.kill()?;
        child.wait()?;
        d.remove(&sb).await?;
        assert!(!d.sandboxes().join(name).exists());
        Ok(())
    }

    #[tokio::test]
    async fn never_makes_a_sandbox_over_a_disk_it_has_no_record_of() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let d = driver(dir.path());
        let vm = d.vm("fctl-ses-01abc")?;
        std::fs::create_dir_all(&vm.dir)?;
        let disk = vm.dir.join("disk.ext4");
        std::fs::write(&disk, b"the session's files")?;
        std::fs::write(vm.dir.join("sandbox.json"), b"{ half written")?;
        let spec = SandboxSpec {
            name: "fctl-ses-01abc".into(),
            image: "docker.io/library/debian:trixie-slim".into(),
            fallback_image: None,
            cpus: 1,
            memory_mb: 512,
            cache: None,
        };
        let log = |_: String| {};
        let e = d
            .create(&spec, &vm, &log)
            .await
            .err()
            .map(|e| e.to_string());
        assert!(e.is_some_and(|e| e.contains("has a disk but no record")));
        assert_eq!(std::fs::read(&disk)?, b"the session's files");
        // Records are written whole.
        let record = Record {
            image: "sha256:1".into(),
            reference: "x".into(),
            created_ms: 1,
            agent: None,
            saved: None,
        };
        d.save_record("fctl-ses-01abc", &record)?;
        assert_eq!(d.record("fctl-ses-01abc"), Some(record));
        // A sandbox whose disk was being made when the host stopped gets a
        // new one, never the half-made one.
        if find_program("mkfs.ext4").is_some() {
            std::fs::remove_file(&disk)?;
            std::fs::write(vm.dir.join("disk.ext4.new"), b"half made")?;
            d.make_disk(&vm).await?;
            assert!(!vm.dir.join("disk.ext4.new").exists());
            assert!(std::fs::metadata(&disk)?.len() > 1024 * 1024);
        }
        Ok(())
    }

    /// An image that another lies over stays when its last name goes,
    /// until nothing lies over it.
    #[tokio::test]
    async fn an_image_others_lie_over_stays_without_a_name() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let d = driver(dir.path());
        let add = |id: char, reference: &str, base: Option<&str>| -> Result<Image> {
            let image = Image {
                id: format!("sha256:{}", id.to_string().repeat(64)),
                reference: reference.into(),
                diff_ids: vec![format!("sha256:{}", id.to_string().repeat(64))],
                base: base.map(str::to_owned),
                ..Image::default()
            };
            let rootfs = dir.path().join(format!("{id}.ext4"));
            std::fs::write(&rootfs, b"ext4")?;
            d.store.add(&image, &rootfs)?;
            Ok(image)
        };
        let debian = add('a', "docker.io/library/debian:trixie-slim", None)?;
        let project = add('b', "fctl-build-prj_1:abc", Some(&debian.id))?;
        d.remove_image("docker.io/library/debian:trixie-slim")
            .await?;
        assert!(!d.has_image("docker.io/library/debian:trixie-slim").await);
        assert!(d.store.image(&debian.id).is_some());
        assert!(d.image_size("fctl-build-prj_1:abc").await.is_some());
        d.remove_image("fctl-build-prj_1:abc").await?;
        assert!(d.store.image(&project.id).is_none());
        assert!(d.store.image(&debian.id).is_none());
        Ok(())
    }

    #[test]
    fn saved_memory_takes_no_disk_for_its_zero_pages() -> Result<()> {
        use std::os::unix::fs::{FileExt, MetadataExt};
        let dir = tempfile::tempdir()?;
        let memory = dir.path().join("memory");
        // 8 MiB of written zeros, but for two pages of something else.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&memory)?;
        file.write_all_at(&vec![0u8; 8 << 20], 0)?;
        file.write_all_at(b"the agent's heap", 4096)?;
        file.write_all_at(b"its stack", (6 << 20) + 17)?;
        file.sync_all()?;
        let before = std::fs::metadata(&memory)?.blocks() * 512;
        punch_zeros(&memory)?;
        let after = std::fs::metadata(&memory)?;
        assert_eq!(after.len(), 8 << 20);
        let mut page = [0u8; 16];
        file.read_exact_at(&mut page, 4096)?;
        assert_eq!(&page, b"the agent's heap");
        let mut tail = [0u8; 9];
        file.read_exact_at(&mut tail, (6 << 20) + 17)?;
        assert_eq!(&tail, b"its stack");
        // Where the filesystem gives back what holes don't take.
        if before >= 8 << 20 {
            assert!(
                after.blocks() * 512 <= 64 * 1024,
                "{} bytes",
                after.blocks() * 512
            );
        }
        Ok(())
    }

    /// A sandbox with a saved state, as a stop leaves it.
    fn with_saved(d: &FirecrackerDriver, name: &str, firecracker: &str) -> Result<Vm> {
        let vm = d.vm(name)?;
        std::fs::create_dir_all(vm.dir.join("saved"))?;
        std::fs::write(vm.dir.join("saved/vmstate"), b"state")?;
        std::fs::write(vm.dir.join("saved/memory"), vec![1u8; 64 * 1024])?;
        d.save_record(
            name,
            &Record {
                image: "sha256:1".into(),
                reference: "x".into(),
                created_ms: 1,
                agent: None,
                saved: Some(Saved {
                    firecracker: firecracker.into(),
                    at_ms: 2,
                }),
            },
        )?;
        Ok(vm)
    }

    /// A state that another Firecracker saved goes, and the sandbox boots
    /// from its disk instead; one that never came about goes too.
    #[tokio::test]
    async fn lets_go_of_saved_states_it_cant_resume() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let bin = tempfile::tempdir()?;
        let firecracker = bin.path().join("firecracker");
        crate::driver::fake::write_script(
            &firecracker,
            "#!/bin/sh\necho 'Firecracker v1.18.0'\necho\necho 'Supported snapshot data format versions: v8.0.0'\n",
        )?;
        let mut host = HostConfig {
            data_dir: Some(dir.path().to_owned()),
            ..HostConfig::default()
        };
        host.firecracker.firecracker = Some(firecracker);
        let d = FirecrackerDriver::new(&host);
        let vm = with_saved(&d, "fctl-ses-01old", "1.17.0")?;
        let log = |_: String| {};
        let saved = d
            .record("fctl-ses-01old")
            .and_then(|r| r.saved)
            .context("saved")?;
        let e = d
            .resume_saved(&vm, "fctl-ses-01old", &saved, &log)
            .await
            .err()
            .map(|e| e.to_string());
        assert_eq!(
            e.as_deref(),
            Some("Firecracker 1.17.0 saved its state, and this host runs 1.18.0")
        );
        assert!(!vm.dir.join("saved").exists());
        assert_eq!(d.record("fctl-ses-01old").and_then(|r| r.saved), None);
        // Its memory never written out.
        let vm = with_saved(&d, "fctl-ses-01half", "1.18.0")?;
        std::fs::remove_file(vm.dir.join("saved/memory"))?;
        let saved = d
            .record("fctl-ses-01half")
            .and_then(|r| r.saved)
            .context("saved")?;
        let e = d.resume_saved(&vm, "fctl-ses-01half", &saved, &log).await;
        assert!(e.is_err_and(|e| e.to_string() == "its saved state is incomplete"));
        assert!(!vm.dir.join("saved").exists());
        Ok(())
    }

    /// A VM asked for another size boots from its disk rather than resume
    /// as it was; what a save that stopped halfway left goes with the rest.
    #[tokio::test]
    async fn saved_states_go_when_the_size_changes_or_they_were_never_done() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let d = driver(dir.path());
        let vm = with_saved(&d, "fctl-ses-01size", "1.17.0")?;
        let spec = |cpus, memory_mb| SandboxSpec {
            name: "fctl-ses-01size".into(),
            image: "x".into(),
            fallback_image: None,
            cpus,
            memory_mb,
            cache: None,
        };
        let boot = vm::unpack_spec("/k".into(), "/i".into(), "fctl-ses-01size", Path::new("/r"));
        std::fs::write(
            vm.dir.join("firecracker.json"),
            serde_json::to_vec(&vm.config(&vm::Spec {
                cpus: 2,
                memory_mb: 4096,
                ..boot
            }))?,
        )?;
        let log = |_: String| {};
        assert!(!d.resized(&vm, &spec(2, 4096), &log));
        assert!(d.resized(&vm, &spec(4, 4096), &log));
        assert!(d.resized(&vm, &spec(2, 8192), &log));
        // Half saved, as the host stopped.
        std::fs::create_dir_all(vm.dir.join("saved.new"))?;
        std::fs::write(vm.dir.join("saved.new/memory"), vec![1u8; 4096])?;
        d.drop_saved(&vm, "fctl-ses-01size")?;
        assert!(!vm.dir.join("saved").exists() && !vm.dir.join("saved.new").exists());
        assert_eq!(d.record("fctl-ses-01size").and_then(|r| r.saved), None);
        Ok(())
    }

    /// For room on the disk, the states of stopped sandboxes go, but those
    /// of the sessions here.
    #[tokio::test]
    async fn lets_saved_states_go_for_room_on_the_disk() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let d = driver(dir.path());
        let stopped = with_saved(&d, "fctl-ses-01stopped", "1.17.0")?;
        let starting = with_saved(&d, "fctl-ses-01starting", "1.17.0")?;
        let keep = std::collections::HashSet::from(["fctl-ses-01starting".to_owned()]);
        assert!(d.drop_saved_states(&keep).await >= 1);
        assert!(!stopped.dir.join("saved").exists());
        assert_eq!(d.record("fctl-ses-01stopped").and_then(|r| r.saved), None);
        assert!(starting.dir.join("saved").is_dir());
        assert!(
            d.record("fctl-ses-01starting")
                .and_then(|r| r.saved)
                .is_some()
        );
        Ok(())
    }

    /// A host that keeps no states resumes none it kept before: a start
    /// lets the state go without trying it.
    #[tokio::test]
    async fn keeping_no_states_resumes_none() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let bin = tempfile::tempdir()?;
        // Another release than the one that saved: a resume would fail at
        // once, and say so.
        let firecracker = bin.path().join("firecracker");
        crate::driver::fake::write_script(
            &firecracker,
            "#!/bin/sh\necho 'Firecracker v1.18.0'\necho\necho 'Supported snapshot data format versions: v8.0.0'\n",
        )?;
        let mut host = HostConfig {
            data_dir: Some(dir.path().to_owned()),
            ..HostConfig::default()
        };
        host.firecracker.firecracker = Some(firecracker);
        host.firecracker.snapshots = false;
        let d = FirecrackerDriver::new(&host);
        let vm = with_saved(&d, "fctl-ses-01cold", "1.17.0")?;
        // The size it saved with, which the start asks for again.
        std::fs::write(
            vm.dir.join("firecracker.json"),
            br#"{"machine-config": {"vcpu_count": 1, "mem_size_mib": 512}}"#,
        )?;
        let spec = SandboxSpec {
            name: "fctl-ses-01cold".into(),
            image: "127.0.0.1:9/fctl-test/none:1".into(),
            fallback_image: None,
            cpus: 1,
            memory_mb: 512,
            cache: None,
        };
        let said = std::sync::Mutex::new(Vec::new());
        let log = |line: String| {
            said.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line)
        };
        // No KVM here, nor that image: it can't boot, but its state goes first.
        let _ = d.ensure(&spec, &log).await;
        assert!(!vm.dir.join("saved").exists());
        assert_eq!(d.record("fctl-ses-01cold").and_then(|r| r.saved), None);
        let said = said.into_inner().unwrap_or_else(PoisonError::into_inner);
        assert!(!said.iter().any(|l| l.contains("resume")), "{said:?}");
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

    /// What passt, the VM's console and the VM's network say, for a test
    /// whose sandbox can't reach the network.
    async fn network_report(d: &super::super::Driver, sb: &Sandbox, data: &Path) -> String {
        let dir = data.join("firecracker/sandboxes").join(&sb.name);
        let read = |file: &str| std::fs::read_to_string(dir.join(file)).unwrap_or_default();
        let passt = read("passt.pid")
            .trim()
            .parse::<u32>()
            .ok()
            .and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/cmdline")).ok())
            .map(|cmdline| cmdline.replace('\0', " "));
        let vm = d
            .exec(
                sb,
                "cat /etc/resolv.conf /proc/net/route /proc/net/arp /proc/net/dev 2>&1; timeout 5 bash -c 'exec 3<>/dev/tcp/1.1.1.1/80' 2>&1 && echo 'TCP to 1.1.1.1:80 works'; true",
            )
            .await;
        format!(
            "passt runs: {passt:?}\npasst said:\n{}\nthe VM's console:\n{}\nthe VM's network:\n{}",
            read("passt.log").trim(),
            read("console.log").trim(),
            vm.unwrap_or_else(|e| format!("{e:#}"))
        )
    }

    /// Boots a sandbox from an image of Docker Hub (or
    /// `FCTL_TEST_FIRECRACKER_IMAGE`), on a host with KVM, Firecracker's
    /// needs and a static fctl-agent, with the kernel hosts pull (or the
    /// file `FCTL_TEST_FIRECRACKER_KERNEL`). CI runs it on GitHub's x86_64
    /// runners, which have KVM (the `firecracker` job of ci.yml):
    ///
    /// ```sh
    /// cargo build --release -p fctl-agent --target "$(uname -m)-unknown-linux-musl"
    /// FCTL_TEST_FIRECRACKER_AGENT=$PWD/target/$(uname -m)-unknown-linux-musl/release/fctl-agent \
    ///   cargo test -p fctl-host boots_a_sandbox -- --ignored --nocapture
    /// ```
    ///
    /// `FCTL_TEST_FIRECRACKER_DATA` keeps the host's files (the image, the
    /// kernel and Firecracker) between runs, and `FCTL_TEST_FIRECRACKER_RUN`
    /// is a command whose output it shows (`claude --version`, say).
    #[tokio::test]
    #[ignore = "needs KVM, passt and a static fctl-agent"]
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
        cfg.firecracker.kernel = std::env::var("FCTL_TEST_FIRECRACKER_KERNEL").ok();
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
            // As root, the VM runs in Firecracker's jailer, as a user of
            // its own.
            if nix::unistd::geteuid().is_root() {
                let pid = std::fs::read_to_string(
                    cfg.data_dir()
                        .join("firecracker/sandboxes")
                        .join(&name)
                        .join("firecracker.pid"),
                )?;
                let status = std::fs::read_to_string(format!("/proc/{}/status", pid.trim()))?;
                let uid: u32 = status
                    .lines()
                    .find_map(|l| l.strip_prefix("Uid:"))
                    .and_then(|l| l.split_whitespace().next())
                    .and_then(|u| u.parse().ok())
                    .context("Firecracker's uid")?;
                assert!(uid >= vm::JAIL_ID_BASE, "Firecracker runs as {uid}");
                eprintln!("jailed as {uid}");
            }
            if let Ok(command) = std::env::var("FCTL_TEST_FIRECRACKER_RUN") {
                eprintln!("{command}: {}", d.exec(&sb, &command).await?.trim());
            }
            // The network, through passt: what it and the VM say when it
            // doesn't work.
            let resolved = d.exec(&sb, "getent hosts example.com").await;
            if resolved.as_ref().is_ok_and(|r| !r.trim().is_empty()) {
                eprintln!("example.com is {}", resolved.unwrap_or_default().trim());
            } else {
                let report = network_report(&d, &sb, &cfg.data_dir()).await;
                bail!("no address for example.com in the sandbox ({resolved:?})\n{report}");
            }
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
            // Its disk, which its jail had, is the host's again, as are the
            // images' filesystems, which their unpacking VMs' jails had.
            if nix::unistd::geteuid().is_root() {
                use std::os::unix::fs::MetadataExt;
                let firecracker = cfg.data_dir().join("firecracker");
                let images = std::fs::read_dir(firecracker.join("images"))?
                    .flatten()
                    .map(|e| e.path().join("rootfs.ext4"))
                    .filter(|p| p.is_file());
                let disks = std::fs::read_dir(firecracker.join("sandboxes").join(&name))?
                    .flatten()
                    .map(|e| e.path());
                for path in images.chain(disks) {
                    let owner = std::fs::symlink_metadata(&path)?.uid();
                    assert!(owner < vm::JAIL_ID_BASE, "{path:?} is {owner}'s");
                }
                // A jailed VM saves no state yet.
                let saved = firecracker.join("sandboxes").join(&name).join("saved");
                assert!(!saved.exists(), "the jailed VM saved its state");
            }
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

    /// Builds an image from a Dockerfile with BuildKit in a VM, and boots a
    /// sandbox from it: its files, modes, user, environment and working
    /// directory as the Dockerfile says. As [`boots_a_sandbox`] says.
    #[tokio::test]
    #[ignore = "needs KVM, passt and a static fctl-agent"]
    async fn builds_an_image_in_a_vm() -> Result<()> {
        use super::super::{BuildJob, Driver};
        let temp = tempfile::tempdir()?;
        let data = std::env::var("FCTL_TEST_FIRECRACKER_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| temp.path().to_owned());
        let mut cfg = HostConfig {
            data_dir: Some(data),
            agent_bin: Some(
                std::env::var("FCTL_TEST_FIRECRACKER_AGENT")
                    .context("FCTL_TEST_FIRECRACKER_AGENT isn't set")?
                    .into(),
            ),
            cpus: 2,
            memory_mb: 2048,
            ..HostConfig::default()
        };
        cfg.firecracker.kernel = std::env::var("FCTL_TEST_FIRECRACKER_KERNEL").ok();
        cfg.firecracker.disk_gb = 8;
        let d = Driver::select("firecracker", &cfg).await?;
        assert!(d.builds_images());
        let log = |line: String| eprintln!("{line}");
        let build = temp.path().join("build");
        std::fs::create_dir_all(build.join("src"))?;
        std::fs::write(build.join("src/hello.txt"), b"hello\n")?;
        // What a whiteout and an opaque directory hide in the image it lies
        // over, Debian's, which the host has.
        std::fs::write(
            build.join("Dockerfile"),
            "FROM docker.io/library/debian:trixie-slim\nCOPY hello.txt /hello.txt\nARG GREETING\nRUN echo \"$GREETING\" > /greeting && cp /bin/true /built && chmod 4755 /built\nRUN rm /etc/issue.net && rm -rf /usr/share/doc && mkdir /usr/share/doc && echo only > /usr/share/doc/only\nENV BUILT=yes\nWORKDIR /tmp\nUSER nobody\n",
        )?;
        d.pull("docker.io/library/debian:trixie-slim", false)
            .await?;
        let project = hex::encode(fctl_domain::random_bytes::<4>());
        let tag = format!("fctl-build-prj_{project}:abc");
        let args = std::collections::BTreeMap::from([("GREETING".to_owned(), "hi".to_owned())]);
        let job = BuildJob {
            tag: &tag,
            dockerfile: &build.join("Dockerfile"),
            context: &build.join("src"),
            args: &args,
            target: None,
            limit: Duration::from_secs(30 * 60),
        };
        let started = std::time::Instant::now();
        d.build(&job, &log).await?;
        eprintln!("built in {:?}", started.elapsed());
        assert!(d.has_image(&tag).await);
        // The builder stopped, and the sweep counts its time from now. It
        // saved no state: it boots afresh for each build.
        let builder = super::builds::builder_name(&tag);
        let stopped = crate::expiry::load(&crate::expiry::path(&cfg.data_dir()));
        assert!(stopped.contains_key(&builder), "{stopped:?}");
        let saved = cfg
            .data_dir()
            .join("firecracker/sandboxes")
            .join(&builder)
            .join("saved");
        assert!(!saved.exists(), "the builder saved its state");
        // It keeps only its own layers, over Debian's.
        let Driver::Firecracker(fd) = &d else {
            bail!("not the firecracker driver");
        };
        let built = fd.store.lookup(&tag).context("the image built")?;
        let debian = fd
            .store
            .lookup("docker.io/library/debian:trixie-slim")
            .context("Debian's image")?;
        assert_eq!(built.base.as_deref(), Some(debian.id.as_str()));
        let name = format!("fctl-test-{project}");
        let spec = SandboxSpec {
            name: name.clone(),
            image: tag.clone(),
            fallback_image: None,
            cpus: 1,
            memory_mb: 512,
            cache: None,
        };
        let sb = d.ensure(&spec, &log).await?;
        let result = async {
            let out = d
                .exec(
                    &sb,
                    "cat /hello.txt /greeting; id -un; pwd; echo $BUILT; stat -c %a /built; ls /usr/share/doc; test -e /etc/issue.net && echo kept; cat /etc/debian_version >/dev/null && echo debian",
                )
                .await?;
            assert_eq!(
                out,
                "hello\nhi\nnobody\n/tmp\nyes\n4755\nonly\ndebian\n"
            );
            Ok::<_, anyhow::Error>(())
        }
        .await;
        d.remove(&sb).await?;
        d.remove(&d.handle(&super::builds::builder_name(&tag)))
            .await?;
        d.remove_image(&tag).await?;
        result
    }

    /// A sandbox stopped with its state saved resumes with its programs
    /// running and its network linked again; without it, it boots from its
    /// disk. As [`boots_a_sandbox`] says.
    #[tokio::test]
    #[ignore = "needs KVM, passt and a static fctl-agent"]
    async fn stopped_sandboxes_resume_where_they_were() -> Result<()> {
        use super::super::Driver;
        let temp = tempfile::tempdir()?;
        let data = std::env::var("FCTL_TEST_FIRECRACKER_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| temp.path().to_owned());
        let mut cfg = HostConfig {
            data_dir: Some(data.clone()),
            agent_bin: Some(
                std::env::var("FCTL_TEST_FIRECRACKER_AGENT")
                    .context("FCTL_TEST_FIRECRACKER_AGENT isn't set")?
                    .into(),
            ),
            ..HostConfig::default()
        };
        cfg.firecracker.kernel = std::env::var("FCTL_TEST_FIRECRACKER_KERNEL").ok();
        cfg.firecracker.disk_gb = 4;
        // Jailed VMs save no state yet: as root, this tests saving without
        // the jailer.
        cfg.firecracker.jailer = false;
        let d = Driver::select("firecracker", &cfg).await?;
        let log = |line: String| eprintln!("{line}");
        let name = format!(
            "fctl-test-{}",
            hex::encode(fctl_domain::random_bytes::<4>())
        );
        let spec = SandboxSpec {
            name: name.clone(),
            image: std::env::var("FCTL_TEST_FIRECRACKER_IMAGE")
                .unwrap_or_else(|_| "docker.io/library/debian:trixie-slim".into()),
            fallback_image: None,
            cpus: 1,
            memory_mb: 512,
            cache: None,
        };
        let sb = d.ensure(&spec, &log).await?;
        let dir = data.join("firecracker/sandboxes").join(&name);
        let result = async {
            // A program of its own, and a file in memory (/run is a tmpfs).
            d.exec(
                &sb,
                "setsid sh -c 'while :; do sleep 1; done' </dev/null >/dev/null 2>&1 & echo $! > /run/held",
            )
            .await?;
            let held = d.exec(&sb, "cat /run/held").await?;
            let started = std::time::Instant::now();
            d.stop(&sb).await?;
            eprintln!(
                "saved in {:?}: {} MiB of memory on disk",
                started.elapsed(),
                crate::caches::dir_size_mb(&dir.join("saved"))?
            );
            assert!(dir.join("saved/memory").is_file());
            assert!(!d.list().await?.iter().any(|s| s.name == name && s.running));
            let started = std::time::Instant::now();
            d.ensure(&spec, &log).await?;
            eprintln!("resumed in {:?}", started.elapsed());
            assert!(!dir.join("saved").exists());
            assert_eq!(d.exec(&sb, "cat /run/held").await?, held);
            let alive = d
                .exec(&sb, "kill -0 \"$(cat /run/held)\" && echo alive")
                .await?;
            assert_eq!(alive.trim(), "alive");
            let resolved = d.exec(&sb, "getent hosts example.com").await?;
            assert!(!resolved.trim().is_empty());
            // Without saving its state, it boots again from its disk.
            d.exec(&sb, "echo kept > /root/note").await?;
            let mut cold = cfg.clone();
            cold.firecracker.snapshots = false;
            let d = Driver::select("firecracker", &cold).await?;
            d.stop(&sb).await?;
            assert!(!dir.join("saved").exists());
            d.ensure(&spec, &log).await?;
            assert_eq!(d.exec(&sb, "cat /root/note").await?, "kept\n");
            assert!(d.exec(&sb, "cat /run/held").await.is_err());
            Ok::<_, anyhow::Error>(())
        }
        .await;
        d.remove(&sb).await?;
        result
    }
}
