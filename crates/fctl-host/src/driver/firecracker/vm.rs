//! A microVM's processes on the host: Firecracker, and `passt` for its
//! network. Both run in process groups of their own, so that restarting the
//! host (whose service stops only its main process) leaves them running.

use super::vsock;
use anyhow::{Context, Result, anyhow, bail};
use fctl_proto::vm::{ADDRESS, Boot, DNS, GATEWAY, Mode, NET_PORT, PREFIX};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

/// How long a VM may take to boot to its init. Natively it takes a second
/// or two; nested in another VM without hardware help, much longer.
pub const BOOT_TIMEOUT: Duration = Duration::from_secs(180);

/// How long a VM may take to stop once its init is asked to.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// How long Firecracker has to answer a request on its API socket.
const API_TIMEOUT: Duration = Duration::from_secs(10);

/// The kernel's command line, before what fctl-agent reads: the console, a
/// reset that ends Firecracker, and no modules to look for. On x86_64 also
/// what Firecracker suggests to boot faster: no PS/2 probing, and no PCI
/// (its devices are MMIO); aarch64 has neither.
fn kernel_args() -> String {
    let common = "console=ttyS0 panic=1 nomodule quiet rdinit=/init";
    if cfg!(target_arch = "x86_64") {
        format!("{common} reboot=k i8042.noaux i8042.nomux i8042.dumbkbd swiotlb=noforce pci=off")
    } else {
        common.to_owned()
    }
}

/// A disk for the VM: its file, and whether the VM may write to it.
#[derive(Clone, Debug)]
pub struct Drive {
    pub path: PathBuf,
    pub writable: bool,
}

/// What to boot.
#[derive(Clone, Debug)]
pub struct Spec {
    pub kernel: PathBuf,
    pub initrd: PathBuf,
    pub boot: Boot,
    /// `/dev/vda`, `/dev/vdb`, … in this order.
    pub drives: Vec<Drive>,
    pub cpus: u32,
    pub memory_mb: u32,
    /// Whether the VM gets a network (`passt`).
    pub network: bool,
}

/// A microVM's files: `dir` keeps its configuration, its console's log and
/// the processes' ids; `run`, a short path (unix sockets' paths are
/// limited to about 100 bytes), its sockets.
#[derive(Clone, Debug)]
pub struct Vm {
    pub dir: PathBuf,
    pub run: PathBuf,
    /// What tells the VM's processes and jail from others': the start of
    /// the SHA-256 of `dir`, which names `run` too.
    pub key: String,
    /// Where the VMs' jails are, when they run through Firecracker's jailer
    /// ([`Jail`]): `run` is then a link to this VM's.
    pub jails: Option<PathBuf>,
}

/// How a VM runs through Firecracker's jailer (`jailer`), which a host that
/// runs as root can: as a uid and gid that no other running VM's jail has,
/// in a chroot of its own (`<jails>/<Firecracker's file name>/<key>/root`)
/// that holds only its files, and in a cgroup of its own (`fctl/<key>`).
#[derive(Clone, Debug)]
pub struct Jail {
    pub jailer: PathBuf,
}

/// What a jail has of the host's, which the host gives back as the VM goes:
/// `jail.json` beside the chroot, where the jail's user can't change it.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Lent {
    /// The jail's uid and gid.
    uid: u32,
    /// The VM's writable disks, which the jail's user owns while it runs.
    writable: Vec<PathBuf>,
}

/// The first uid and gid of the VMs' jails: far from those people, systems
/// and user namespaces' ranges take.
pub const JAIL_ID_BASE: u32 = 1_900_000_000;

/// How many uids the jails take, one for each VM that runs at once.
const JAIL_IDS: u32 = 1 << 16;

/// Where VMs' sockets go: a directory of this user's in /tmp, private.
/// One that another user made first is theirs, and refused.
fn socket_base() -> Result<PathBuf> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let user = std::env::var("USER").unwrap_or_else(|_| "fctl".into());
    let user: String = user
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(16)
        .collect();
    let dir = PathBuf::from(format!("/tmp/fctl-{user}"));
    std::fs::create_dir_all(&dir)?;
    let meta = std::fs::symlink_metadata(&dir)?;
    let me = std::fs::metadata("/proc/self")
        .map(|m| m.uid())
        .context("finding this process's user")?;
    if !meta.is_dir() || meta.uid() != me {
        bail!("{} isn't a directory of this user's", dir.display());
    }
    if meta.mode() & 0o077 != 0 {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("making {} private", dir.display()))?;
    }
    Ok(dir)
}

impl Vm {
    pub fn new(dir: PathBuf) -> Result<Self> {
        let key = hex::encode(&Sha256::digest(dir.display().to_string().as_bytes())[..8]);
        let run = socket_base()?.join(format!("vm-{key}"));
        Ok(Self {
            dir,
            run,
            key,
            jails: None,
        })
    }

    /// The VM, its jail among the jails in `jails`, should it run in one.
    pub fn with_jails(self, jails: PathBuf) -> Self {
        Self {
            jails: Some(jails),
            ..self
        }
    }

    /// The directories of this VM's jails: one for each Firecracker binary
    /// that ran it (one, but for an update of the host's Firecracker).
    fn jail_dirs(&self) -> Vec<PathBuf> {
        let Some(jails) = &self.jails else {
            return Vec::new();
        };
        let Ok(execs) = std::fs::read_dir(jails) else {
            return Vec::new();
        };
        execs
            .flatten()
            .map(|e| e.path().join(&self.key))
            .filter(|d| d.is_dir())
            .collect()
    }

    /// The uids of other VMs' jails here.
    fn taken_jail_ids(&self, jails: &Path) -> std::collections::HashSet<u32> {
        use std::os::unix::fs::MetadataExt;
        std::fs::read_dir(jails)
            .into_iter()
            .flat_map(|execs| execs.flatten())
            .filter_map(|exec| std::fs::read_dir(exec.path()).ok())
            .flat_map(|vms| vms.flatten())
            .filter(|vm| vm.file_name().to_string_lossy() != self.key)
            .filter_map(|vm| std::fs::metadata(vm.path().join("root")).ok())
            .map(|m| m.uid())
            .collect()
    }

    pub fn api(&self) -> PathBuf {
        self.run.join("api.sock")
    }

    pub fn vsock(&self) -> PathBuf {
        self.run.join("v.sock")
    }

    pub fn console(&self) -> PathBuf {
        self.dir.join("console.log")
    }

    fn pid_file(&self, what: &str) -> PathBuf {
        self.dir.join(format!("{what}.pid"))
    }

    /// The process `what` started for this VM, while it runs: its id file
    /// names it, and its command line has this VM's sockets, which a process
    /// that took the id later wouldn't. One that is still being started has
    /// no command line yet, for a moment, but an executable; its id file is
    /// written once it no longer shows this process's ([`wait_for_exec`]).
    fn pid(&self, what: &str) -> Option<u32> {
        let pid: u32 = std::fs::read_to_string(self.pid_file(what))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let zombie = stat
            .rsplit_once(')')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'));
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let ours = self.run.display().to_string();
        // A kernel thread, which may take the id too, has no executable.
        let starting = cmdline.is_empty() && std::fs::read_link(format!("/proc/{pid}/exe")).is_ok();
        // passt has its pid's file named by this VM's key, and a jailed
        // Firecracker its jail's paths and this key as its `--id`.
        let mine = starting
            || cmdline.split(|b| *b == 0).any(|arg| {
                let arg = String::from_utf8_lossy(arg);
                arg.starts_with(&ours) || arg.contains(&self.key)
            });
        (mine && !zombie).then_some(pid)
    }

    /// Whether the VM runs.
    pub fn running(&self) -> bool {
        self.pid("firecracker").is_some()
    }

    /// Whether the VM's `passt` runs, which takes its network.
    pub fn linked(&self) -> bool {
        self.pid("passt").is_some()
    }

    /// Starts Firecracker, after stopping what an earlier boot left: the
    /// `passt` of a VM that powered off, say. What it started goes again
    /// when it fails. A VM that gets a network gets it once it is up
    /// ([`link_network`](Self::link_network)).
    pub async fn start(&self, spec: &Spec, firecracker: &Path, jail: Option<&Jail>) -> Result<()> {
        self.kill().await;
        let started = match jail {
            Some(jail) => self.start_jailed(spec, firecracker, jail).await,
            None => self.start_processes(spec, firecracker).await,
        };
        if started.is_err() {
            self.kill().await;
        }
        started
    }

    /// Firecracker through its jailer ([`Jail`]), in a chroot that holds the
    /// VM's kernel, initramfs and disks ([`make_jail`](Self::make_jail)),
    /// its configuration and its sockets. The jail's user owns the chroot,
    /// so the host never goes through it to reach the sockets: it links
    /// them into a directory of its own beside it, checked to be the
    /// sockets Firecracker made ([`link_sockets`](Self::link_sockets)), and
    /// `run` leads there.
    async fn start_jailed(&self, spec: &Spec, firecracker: &Path, jail: &Jail) -> Result<()> {
        let jails = self
            .jails
            .as_ref()
            .context("no directory for the VM's jail")?;
        let exec = firecracker
            .file_name()
            .context("Firecracker's path has no file name")?;
        std::fs::create_dir_all(jails)?;
        if let Some(why) = jails_problem(jails) {
            bail!("{why}");
        }
        let (uid, inside, root) = self.make_jail(jails, exec, spec)?;
        let mut config = self.config(&inside);
        config["vsock"]["uds_path"] = json!("/v.sock");
        std::fs::write(
            root.join("firecracker.json"),
            serde_json::to_vec_pretty(&config)?,
        )?;
        // The host's links to the sockets, beside the chroot (on its
        // filesystem, as links must be), where only root may change them;
        // `run`, short as sockets' paths must be, leads there.
        let sockets = root.with_file_name("run");
        std::fs::create_dir(&sockets)?;
        let _ = std::fs::remove_file(&self.run);
        let _ = std::fs::remove_dir_all(&self.run);
        std::os::unix::fs::symlink(&sockets, &self.run)
            .with_context(|| format!("linking {} to the VM's sockets", self.run.display()))?;
        std::fs::create_dir_all(&self.dir)?;
        let console = std::fs::File::create(self.console())?;
        let mut cmd = std::process::Command::new(&jail.jailer);
        cmd.args(jailer_args(&self.key, firecracker, uid, jails))
            .stdin(Stdio::null())
            .stdout(console.try_clone()?)
            .stderr(console);
        spawn_detached(&mut cmd, &self.pid_file("firecracker"))
            .with_context(|| format!("starting {}", jail.jailer.display()))?;
        self.link_sockets(&root, &sockets, Duration::from_secs(10))
            .await
    }

    /// Makes this VM's jail under `jails` for Firecracker `exec`: a chroot,
    /// owned by a uid and gid that no other jail here has (picked under the
    /// jails' lock, which other processes' VMs take too), holding the VM's
    /// kernel and initramfs and its disks, the writable ones lent to the
    /// jail's user ([`Lent`]), what an earlier jail of it lent given back
    /// first. Its uid, `spec` as the chroot has it, and the chroot.
    fn make_jail(
        &self,
        jails: &Path,
        exec: &std::ffi::OsStr,
        spec: &Spec,
    ) -> Result<(u32, Spec, PathBuf)> {
        use std::os::unix::fs::PermissionsExt;
        let dir = jails.join(exec).join(&self.key);
        let root = dir.join("root");
        let uid = {
            let _lock = lock_jails(jails)?;
            self.give_back();
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)?;
            let uid = pick_jail_id(&self.key, &self.taken_jail_ids(jails));
            std::fs::create_dir(&root)?;
            std::os::unix::fs::chown(&root, Some(uid), Some(uid))?;
            uid
        };
        let mut lent = Lent {
            uid,
            writable: Vec::new(),
        };
        let mut inside = spec.clone();
        inside.kernel = put_in_jail(&root, &spec.kernel, "vmlinux")?;
        inside.initrd = put_in_jail(&root, &spec.initrd, "initrd.cpio")?;
        for (i, drive) in inside.drives.iter_mut().enumerate() {
            let host = drive
                .path
                .canonicalize()
                .with_context(|| format!("finding {}", drive.path.display()))?;
            drive.path = link_into(&root, &host, &format!("drive{i}.ext4"))?;
            if drive.writable {
                // Noted before it is lent, for a host that stops halfway.
                lent.writable.push(host.clone());
                std::fs::write(dir.join("jail.json"), serde_json::to_vec(&lent)?)?;
                std::os::unix::fs::chown(&host, Some(uid), Some(uid))?;
            } else {
                let mode = std::fs::metadata(&host)?.permissions().mode();
                if mode & 0o004 == 0 {
                    std::fs::set_permissions(&host, std::fs::Permissions::from_mode(mode | 0o444))?;
                }
            }
        }
        std::fs::write(dir.join("jail.json"), serde_json::to_vec(&lent)?)?;
        Ok((uid, inside, root))
    }

    /// Links the sockets Firecracker makes in the chroot `root`, its API's
    /// and its vsock's, into `sockets`, within `within`: each opened without
    /// following a link and checked to be a socket, then linked by what
    /// was opened, so that the jail's user, who may change the chroot,
    /// can't send the host to another VM's sockets.
    async fn link_sockets(&self, root: &Path, sockets: &Path, within: Duration) -> Result<()> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
        let until = Instant::now() + within;
        for name in ["api.sock", "v.sock"] {
            loop {
                let opened = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(nix::libc::O_PATH | nix::libc::O_NOFOLLOW)
                    .open(root.join(name));
                match opened {
                    Ok(file) => {
                        if !file.metadata()?.file_type().is_socket() {
                            bail!("{name} in the VM's jail isn't a socket");
                        }
                        let to = sockets.join(name);
                        let _ = std::fs::remove_file(&to);
                        nix::unistd::linkat(
                            nix::fcntl::AT_FDCWD,
                            format!("/proc/self/fd/{}", file.as_raw_fd()).as_str(),
                            nix::fcntl::AT_FDCWD,
                            &to,
                            nix::fcntl::AtFlags::AT_SYMLINK_FOLLOW,
                        )
                        .with_context(|| format!("linking the VM's {name}"))?;
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        if Instant::now() > until || !self.running() {
                            let said = self.console_tail();
                            bail!("Firecracker made no {name} in its jail: {}", said.trim());
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Err(e) => return Err(e).with_context(|| format!("opening the VM's {name}")),
                }
            }
        }
        Ok(())
    }

    /// Gives back what this VM's jails lent of the host's ([`Lent`]): its
    /// writable disks go back to the host's user, so that a later jail
    /// with the same uid owns none of them.
    fn give_back(&self) {
        for jail in self.jail_dirs() {
            give_back(&jail);
        }
    }

    async fn start_processes(&self, spec: &Spec, firecracker: &Path) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::create_dir_all(&self.run)?;
        let config = self.dir.join("firecracker.json");
        std::fs::write(&config, serde_json::to_vec_pretty(&self.config(spec))?)?;
        let console = std::fs::File::create(self.console())?;
        let mut cmd = std::process::Command::new(firecracker);
        cmd.arg("--api-sock")
            .arg(self.api())
            .arg("--config-file")
            .arg(&config)
            .args(["--id", &spec.boot.name, "--level", "Warning"])
            .stdin(Stdio::null())
            .stdout(console.try_clone()?)
            .stderr(console);
        spawn_detached(&mut cmd, &self.pid_file("firecracker"))
            .with_context(|| format!("starting {}", firecracker.display()))?;
        Ok(())
    }

    /// Links the VM's network to a `passt` of its own, once the VM is up:
    /// connects to the VM's [`NET_PORT`] and hands passt the connection
    /// (`--fd`), which makes the VM's frames connections of the host's. The
    /// VM gets [`ADDRESS`] and the gateway, and reaches neither the host's
    /// loopback nor other VMs; nothing reaches it from outside. passt exits
    /// when the VM closes the connection, as it stops. passt could take the
    /// VM's connection on a socket of its own, but Ubuntu's AppArmor profile
    /// for it refuses one from Firecracker (`accept` fails, EACCES). A link
    /// made before goes: its passt took a connection that a restored
    /// snapshot, say, has lost.
    pub async fn link_network(&self, passt: &Path) -> Result<()> {
        self.kill_one("passt").await;
        let conn = self.connect_port(NET_PORT, LINK_TIMEOUT).await?;
        let ready = self.new_passt_pid()?;
        let log = std::fs::File::create(self.dir.join("passt.log"))?;
        let mut cmd = std::process::Command::new(passt);
        cmd.args(passt_args(&ready))
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(conn)))
            .stdout(log.try_clone()?)
            .stderr(log);
        let spawned = spawn_detached(&mut cmd, &self.pid_file("passt"))
            .with_context(|| format!("starting {}", passt.display()))?;
        // passt makes its pid's file as it reads its options, writes its
        // pid there once it has set itself up and is about to take frames,
        // and exits when it can't.
        let has_pid = || {
            std::fs::read_to_string(&ready).is_ok_and(|p| p.trim().parse::<u32>() == Ok(spawned))
        };
        let until = Instant::now() + Duration::from_secs(10);
        while !has_pid() {
            if Instant::now() > until || self.pid("passt").is_none() {
                let said = std::fs::read_to_string(self.dir.join("passt.log")).unwrap_or_default();
                self.kill_one("passt").await;
                bail!("passt did not start: {}", said.trim());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }

    /// A connection to `port` in the VM, through Firecracker's vsock
    /// socket, once something there takes it, within `within`.
    async fn connect_port(
        &self,
        port: u32,
        within: Duration,
    ) -> Result<std::os::unix::net::UnixStream> {
        let until = Instant::now() + within;
        loop {
            let socket = self.vsock();
            let tried = tokio::task::spawn_blocking(move || connect_raw(&socket, port)).await?;
            match tried {
                Ok(conn) => return Ok(conn),
                Err(e) if Instant::now() > until || !self.running() => {
                    return Err(e.context(format!("linking the VM's network (vsock port {port})")));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }

    /// Firecracker's configuration for `spec`.
    pub fn config(&self, spec: &Spec) -> Value {
        let drives: Vec<Value> = spec
            .drives
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let mut drive = json!({
                    "drive_id": format!("vd{}", char::from(b'a' + u8::try_from(i).unwrap_or(25).min(25))),
                    "path_on_host": d.path,
                    "is_root_device": false,
                    "is_read_only": !d.writable,
                });
                if d.writable {
                    // The session's writes reach the host's disk when the
                    // VM flushes them, and what it deletes goes from it.
                    drive["cache_type"] = json!("Writeback");
                    drive["discard"] = json!(true);
                }
                drive
            })
            .collect();
        json!({
            "boot-source": {
                "kernel_image_path": spec.kernel,
                "initrd_path": spec.initrd,
                "boot_args": format!("{} {}", kernel_args(), spec.boot.params()),
            },
            "drives": drives,
            "machine-config": {
                "vcpu_count": spec.cpus.clamp(1, 32),
                "mem_size_mib": spec.memory_mb.max(256),
                "smt": false,
            },
            "vsock": { "guest_cid": 3, "uds_path": self.vsock() },
            // Memory the VM frees goes back to the host.
            "balloon": { "amount_mib": 0, "deflate_on_oom": true, "free_page_reporting": true },
            "entropy": {},
        })
    }

    /// What the VM's console said last, for an error.
    pub fn console_tail(&self) -> String {
        let text = std::fs::read_to_string(self.console()).unwrap_or_default();
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(12)..].join("\n")
    }

    /// Waits for the VM's init to answer.
    pub async fn wait_ready(&self) -> Result<String> {
        let result = vsock::wait_ready(&self.vsock(), BOOT_TIMEOUT, || self.running()).await;
        if let Err(e) = &result {
            let tail = self.console_tail();
            self.kill().await;
            if !tail.is_empty() {
                bail!("{e:#}; its console said:\n{tail}");
            }
        }
        result
    }

    /// What Firecracker says its VM does: `Running`, `Paused` or `Not
    /// started`.
    pub async fn state(&self) -> Result<String> {
        let body = api(&self.api(), "GET", "/", None).await?;
        let info: Value = serde_json::from_slice(&body).context("reading Firecracker's answer")?;
        info["state"]
            .as_str()
            .map(str::to_owned)
            .context("Firecracker didn't say what its VM does")
    }

    /// Whether the VM is paused. One that doesn't run, or whose Firecracker
    /// doesn't answer, isn't.
    pub async fn paused(&self) -> bool {
        self.state().await.is_ok_and(|s| s == "Paused")
    }

    /// Pauses the VM: its CPUs stop, and its memory keeps what it holds,
    /// what it has yet to write to its disks too, until it resumes.
    pub async fn pause(&self) -> Result<()> {
        api(
            &self.api(),
            "PATCH",
            "/vm",
            Some(&json!({ "state": "Paused" })),
        )
        .await
        .map(drop)
    }

    /// Resumes a paused VM.
    pub async fn resume(&self) -> Result<()> {
        api(
            &self.api(),
            "PATCH",
            "/vm",
            Some(&json!({ "state": "Resumed" })),
        )
        .await
        .map(drop)
    }

    /// Asks the VM's init to stop it, and stops what is left. A paused VM
    /// resumes first, so that its init can write out its disks.
    pub async fn stop(&self) {
        if self.running()
            && self.paused().await
            && let Err(e) = self.resume().await
        {
            tracing::warn!("resuming {} to stop it: {e:#}", self.dir.display());
        }
        if self.running() {
            let asked =
                tokio::time::timeout(Duration::from_secs(10), vsock::shutdown(&self.vsock())).await;
            if !matches!(asked, Ok(Ok(()))) {
                tracing::debug!("{} didn't take the request to stop", self.dir.display());
            }
            let until = Instant::now() + STOP_TIMEOUT;
            while self.running() && Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        self.kill().await;
    }

    /// Kills the VM's processes.
    pub async fn kill(&self) {
        for what in ["firecracker", "passt"] {
            self.kill_one(what).await;
        }
        // A link to its jail, as an earlier fctl made, or its own directory.
        let _ = std::fs::remove_file(&self.run);
        let _ = std::fs::remove_dir_all(&self.run);
        for jail in self.jail_dirs() {
            remove_jail(&jail);
        }
    }

    /// Where passt writes its pid, the path naming this VM's key, which
    /// tells passt from other processes; recorded in the VM's directory.
    /// Beside `run`, in the host's user's directory in /tmp, where passt's
    /// AppArmor profile lets it write. On a host that runs as root, passt
    /// opens it as nobody (passt of 2024) or as root without its
    /// capabilities (later ones), which only /tmp itself lets write: there
    /// its name has a random part no one else could make first.
    fn new_passt_pid(&self) -> Result<PathBuf> {
        if let Some(old) = self.passt_pid() {
            let _ = std::fs::remove_file(old);
        }
        let pid = if nix::unistd::geteuid().is_root() {
            PathBuf::from(format!(
                "/tmp/fctl-passt-{}-{}.pid",
                self.key,
                hex::encode(fctl_domain::random_bytes::<8>())
            ))
        } else {
            self.run
                .with_file_name(format!("vm-{}.passt.pid", self.key))
        };
        // One an earlier passt left, whose record went with the VM's
        // directory, say.
        let _ = std::fs::remove_file(&pid);
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(
            self.dir.join("passt.pid-file"),
            pid.as_os_str().as_encoded_bytes(),
        )?;
        Ok(pid)
    }

    /// Where passt writes its pid ([`new_passt_pid`](Self::new_passt_pid)).
    fn passt_pid(&self) -> Option<PathBuf> {
        let path = std::fs::read_to_string(self.dir.join("passt.pid-file")).ok()?;
        let path = PathBuf::from(path.trim());
        let name = path.file_name()?.to_string_lossy().into_owned();
        // Only what this VM named so: nothing else of the host's goes.
        (name.contains(&self.key) && name.ends_with(".pid")).then_some(path)
    }

    /// Kills the VM's process `what`, if it runs.
    async fn kill_one(&self, what: &str) {
        if let Some(pid) = self.pid(what) {
            let _ = crate::driver::run_cmd("kill", &["-KILL", &pid.to_string()]).await;
            let until = Instant::now() + Duration::from_secs(5);
            while self.pid(what).is_some() && Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        let _ = std::fs::remove_file(self.pid_file(what));
        if what == "passt"
            && let Some(pid) = self.passt_pid()
        {
            let _ = std::fs::remove_file(pid);
            let _ = std::fs::remove_file(self.dir.join("passt.pid-file"));
        }
    }

    /// Stops the VM and removes its files.
    pub async fn remove(&self) -> Result<()> {
        self.stop().await;
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| format!("removing {}", self.dir.display()))
            }
            _ => Ok(()),
        }
    }
}

/// A request to Firecracker's API on `socket`: its answer's body.
async fn api(socket: &Path, method: &str, path: &str, body: Option<&Value>) -> Result<Vec<u8>> {
    tokio::time::timeout(API_TIMEOUT, request(socket, method, path, body))
        .await
        .map_err(|_| anyhow!("Firecracker didn't answer {method} {path} within 10 seconds"))?
}

async fn request(socket: &Path, method: &str, path: &str, body: Option<&Value>) -> Result<Vec<u8>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting to {}", socket.display()))?;
    let body = body.map(Value::to_string).unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    let mut answer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some((status, body)) = response(&answer)? {
            if !(200..300).contains(&status) {
                let fault = serde_json::from_slice::<Value>(body)
                    .ok()
                    .and_then(|v| v["fault_message"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_owned());
                bail!("Firecracker answered {method} {path} with {status}: {fault}");
            }
            return Ok(body.to_vec());
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            bail!("Firecracker closed its API socket before it answered {method} {path}");
        }
        answer.extend_from_slice(&chunk[..n]);
    }
}

/// The HTTP answer in `buf`, once all of it is there: its status and body.
fn response(buf: &[u8]) -> Result<Option<(u16, &[u8])>> {
    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return Ok(None);
    };
    let head = String::from_utf8_lossy(&buf[..end]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .context("Firecracker's answer has no status")?;
    let len = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let body = &buf[end + 4..];
    Ok((body.len() >= len).then(|| (status, &body[..len])))
}

/// The arguments of `passt` for a VM whose link is its input, and which
/// writes its pid to `pid` (whose path tells it from other processes): the
/// options of passt's releases since 2022, which Ubuntu 24.04 and Debian 12
/// have. The VM names itself, so passt's DHCP needs no `--hostname`, which
/// came later. On a host that runs as root, passt runs as nobody.
fn passt_args(pid: &Path) -> Vec<String> {
    let mask = PREFIX.to_string();
    vec![
        "--foreground".into(),
        "--quiet".into(),
        "--fd".into(),
        "0".into(),
        "--pid".into(),
        pid.display().to_string(),
        "--address".into(),
        Ipv4Addr::from(ADDRESS).to_string(),
        "--netmask".into(),
        mask,
        "--gateway".into(),
        Ipv4Addr::from(GATEWAY).to_string(),
        "--dns-forward".into(),
        Ipv4Addr::from(DNS).to_string(),
        // The gateway's address doesn't lead to the host's loopback.
        "--no-map-gw".into(),
    ]
}

/// A uid and gid for the jail of the VM `key` that isn't `taken`: the
/// same for the same key, unless another VM's jail took it.
fn pick_jail_id(key: &str, taken: &std::collections::HashSet<u32>) -> u32 {
    let start = key
        .get(..8)
        .and_then(|k| u32::from_str_radix(k, 16).ok())
        .unwrap_or(0)
        % JAIL_IDS;
    (0..JAIL_IDS)
        .map(|i| JAIL_ID_BASE + (start + i) % JAIL_IDS)
        .find(|id| !taken.contains(id))
        .unwrap_or(JAIL_ID_BASE + start)
}

/// The jailer's arguments for the VM `key`: Firecracker `firecracker` as
/// `uid` (and the gid of the same number), chrooted under `jails`, in a
/// cgroup of its own under `fctl` (Firecracker runs a thread for each vCPU
/// and a few of its own), with its configuration and API socket at the
/// chroot's root.
fn jailer_args(key: &str, firecracker: &Path, uid: u32, jails: &Path) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec!["--id".into(), key.into()];
    args.extend(["--exec-file".into(), firecracker.as_os_str().to_owned()]);
    args.extend([
        "--uid".into(),
        uid.to_string().into(),
        "--gid".into(),
        uid.to_string().into(),
    ]);
    args.extend(["--chroot-base-dir".into(), jails.as_os_str().to_owned()]);
    args.extend(
        [
            "--cgroup-version",
            "2",
            "--parent-cgroup",
            "fctl",
            "--cgroup",
            "pids.max=256",
            "--",
            "--api-sock",
            "/api.sock",
            "--config-file",
            "/firecracker.json",
            "--level",
            "Warning",
        ]
        .map(std::ffi::OsString::from),
    );
    args
}

/// Why VMs can't run in jails under `jails`, if they can't: the jailer
/// needs the unified cgroup hierarchy (v2) for their cgroups, and makes
/// `/dev/kvm` in each chroot, which a filesystem mounted `nodev` won't
/// open.
pub fn jails_problem(jails: &Path) -> Option<String> {
    if !Path::new("/sys/fs/cgroup/cgroup.controllers").is_file() {
        return Some(
            "Firecracker's jailer needs cgroup v2 (the unified hierarchy at /sys/fs/cgroup), which this machine doesn't mount: boot it with systemd.unified_cgroup_hierarchy=1, or set jailer = false in host.toml's [firecracker] to run VMs as root without it".into(),
        );
    }
    let mut at = jails;
    while !at.exists() {
        at = at.parent()?;
    }
    if nix::sys::statvfs::statvfs(at)
        .is_ok_and(|v| v.flags().contains(nix::sys::statvfs::FsFlags::ST_NODEV))
    {
        return Some(format!(
            "the VMs' jails need a filesystem that allows device files, and {} is on one mounted nodev: move data_dir in host.toml, or set jailer = false in its [firecracker]",
            jails.display()
        ));
    }
    None
}

/// The jails' lock, under which a jail's uid is picked and taken; other
/// processes' VMs (`fctl host image pull`'s) take it too.
fn lock_jails(jails: &Path) -> Result<JailsLock> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(jails.join(".lock"))?;
    file.lock()
        .with_context(|| format!("locking {}", jails.display()))?;
    Ok(JailsLock(file))
}

struct JailsLock(std::fs::File);

impl Drop for JailsLock {
    fn drop(&mut self) {
        // A process forked meanwhile shares the file until it execs.
        let _ = self.0.unlock();
    }
}

/// Gives back what the jail `dir` lent of the host's ([`Lent`]): each
/// writable disk it noted goes back to the host's user, not following a
/// link. Its note is the host's, beside the chroot.
fn give_back(dir: &Path) {
    let Ok(bytes) = std::fs::read(dir.join("jail.json")) else {
        return;
    };
    let Ok(lent) = serde_json::from_slice::<Lent>(&bytes) else {
        return;
    };
    let me = nix::unistd::geteuid().as_raw();
    let group = nix::unistd::getegid().as_raw();
    for disk in &lent.writable {
        if let Err(e) = std::os::unix::fs::lchown(disk, Some(me), Some(group))
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("giving {} back from its VM's jail: {e}", disk.display());
        }
    }
}

/// Removes the jail `dir` (`<jails>/<exec>/<key>`) and its cgroup, once
/// what it lent is given back.
fn remove_jail(dir: &Path) {
    give_back(dir);
    if let Err(e) = std::fs::remove_dir_all(dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("removing the jail {}: {e}", dir.display());
    }
    if let Some(key) = dir.file_name() {
        let _ = std::fs::remove_dir(Path::new("/sys/fs/cgroup/fctl").join(key));
    }
}

/// Removes the jails under `jails` that no running VM has (`running`, by
/// their keys) and that are older than `age`, which a VM that is starting
/// isn't: those of VMs that stopped while no host ran, or that a host that
/// crashed left.
pub fn remove_stale_jails(
    jails: &Path,
    running: &std::collections::HashSet<String>,
    age: Duration,
) {
    let Ok(execs) = std::fs::read_dir(jails) else {
        return;
    };
    for exec in execs.flatten().filter(|e| e.path().is_dir()) {
        let Ok(vms) = std::fs::read_dir(exec.path()) else {
            continue;
        };
        for vm in vms.flatten() {
            let key = vm.file_name().to_string_lossy().into_owned();
            let old = vm
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|e| e > age);
            if old && !running.contains(&key) {
                remove_jail(&vm.path());
            }
        }
    }
}

/// Puts the file `from` in a jail's chroot `root` as `name`, a hard link of
/// the file it names (through any links); its path in the chroot. The
/// file must be on the chroot's filesystem: a VM's disks are.
fn link_into(root: &Path, from: &Path, name: &str) -> Result<PathBuf> {
    let from = from
        .canonicalize()
        .with_context(|| format!("finding {}", from.display()))?;
    std::fs::hard_link(&from, root.join(name)).with_context(|| {
        format!(
            "linking {} into the VM's jail, which must be on its filesystem",
            from.display()
        )
    })?;
    Ok(Path::new("/").join(name))
}

/// Puts a file the VM boots from (its kernel or initramfs) in a jail's
/// chroot `root` as `name`: a hard link where everyone may read it and it
/// is on the chroot's filesystem, else a copy that the jail's user may read
/// (a kernel `host.toml` names elsewhere, or keeps private).
fn put_in_jail(root: &Path, from: &Path, name: &str) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let from = from
        .canonicalize()
        .with_context(|| format!("finding {}", from.display()))?;
    let readable = std::fs::metadata(&from)?.permissions().mode() & 0o004 != 0;
    if readable && std::fs::hard_link(&from, root.join(name)).is_ok() {
        return Ok(Path::new("/").join(name));
    }
    let to = root.join(name);
    let _ = std::fs::remove_file(&to);
    std::fs::copy(&from, &to)
        .with_context(|| format!("copying {} into the VM's jail", from.display()))?;
    std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o444))?;
    Ok(Path::new("/").join(name))
}

/// How long a VM that is up may take to take its network's link: its
/// init listens as it starts.
const LINK_TIMEOUT: Duration = Duration::from_secs(10);

/// Connects to `port` in the VM whose vsock socket is `socket` (`CONNECT`,
/// then Firecracker's `OK`), reading no byte past Firecracker's answer:
/// what follows is the VM's, for whoever takes the connection.
fn connect_raw(socket: &Path, port: u32) -> Result<std::os::unix::net::UnixStream> {
    use std::io::{Read, Write};
    let mut conn = std::os::unix::net::UnixStream::connect(socket)
        .with_context(|| format!("connecting to {}", socket.display()))?;
    conn.set_read_timeout(Some(Duration::from_secs(5)))?;
    // Firecracker closes the connection when nothing in the VM takes it,
    // which may come as a reset.
    let refused = || anyhow::anyhow!("nothing in the VM takes connections on port {port} yet");
    let gone = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe => refused(),
        _ => e.into(),
    };
    conn.write_all(format!("CONNECT {port}\n").as_bytes())
        .map_err(gone)?;
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.len() < 64 {
        if conn.read(&mut byte).map_err(gone)? == 0 {
            return Err(refused());
        }
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
    }
    if !line.starts_with(b"OK ") {
        bail!(
            "Firecracker answered {:?} to a connection to port {port}",
            String::from_utf8_lossy(&line)
        );
    }
    conn.set_read_timeout(None)?;
    Ok(conn)
}

/// The nameservers a VM asks, which passt reaches as the host does: as
/// Docker gives its containers, the host's own, but for those on its
/// loopback, which the VM can't reach, and where only those are left
/// (systemd-resolved's 127.0.0.53, as Ubuntu has it) the servers
/// systemd-resolved asks. Empty when there are none: the VM asks passt's
/// forwarder then, which passes its queries to the host's resolver, but in
/// passt's releases before the summer of 2024 (Ubuntu 24.04's) not to one on
/// the loopback.
pub fn vm_dns() -> Vec<Ipv4Addr> {
    let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    dns_for_vm(
        &read("/etc/resolv.conf"),
        &read("/run/systemd/resolve/resolv.conf"),
    )
}

/// [`vm_dns`], from the files' text: at most three IPv4 servers (the VM
/// has no IPv6), and none of the addresses that mean something else in the
/// VM's network, where the gateway and passt's forwarder are.
fn dns_for_vm(resolv_conf: &str, resolved_upstream: &str) -> Vec<Ipv4Addr> {
    let usable = |text: &str| -> Vec<Ipv4Addr> {
        text.lines()
            .filter_map(|l| l.trim().strip_prefix("nameserver"))
            .filter_map(|rest| rest.split_whitespace().next())
            .filter_map(|a| a.parse::<Ipv4Addr>().ok())
            .filter(|a| {
                !a.is_loopback()
                    && !a.is_unspecified()
                    && ![ADDRESS, GATEWAY, DNS].contains(&a.octets())
            })
            .take(3)
            .collect()
    };
    match usable(resolv_conf) {
        none if none.is_empty() => usable(resolved_upstream),
        some => some,
    }
}

/// Starts `cmd` in a process group of its own, writes its id to
/// `pid_file` once it runs its own program, and reaps it when it exits
/// while this host runs.
fn spawn_detached(cmd: &mut std::process::Command, pid_file: &Path) -> Result<u32> {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
    let mut child = cmd.spawn()?;
    let pid = child.id();
    wait_for_exec(pid);
    std::fs::write(pid_file, pid.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

/// Waits, for at most a second, until the child `pid` no longer runs on
/// this process's memory. `spawn` returns as soon as the child's exec lets
/// go of it, but the child keeps it until the exec takes its memory map's
/// lock, which waits while another thread maps or unmaps memory. Until
/// then `/proc` shows this process's executable and command line for it,
/// and [`Vm::pid`] would take it for another process (fctl#427). The
/// programs started here (Firecracker, `passt`) are never this one.
fn wait_for_exec(pid: u32) {
    let Ok(ours) = std::fs::read_link("/proc/self/exe") else {
        return;
    };
    let until = Instant::now() + Duration::from_secs(1);
    while std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|exe| exe == ours)
        && Instant::now() < until
    {
        std::thread::sleep(Duration::from_micros(100));
    }
}

/// What a VM to unpack an image boots.
pub fn unpack_spec(kernel: PathBuf, initrd: PathBuf, name: &str, rootfs: &Path) -> Spec {
    Spec {
        kernel,
        initrd,
        boot: Boot {
            mode: Mode::Unpack,
            name: name.into(),
            time_ms: Some(vsock::now_ms()),
            dns: Vec::new(),
        },
        drives: vec![Drive {
            path: rootfs.to_owned(),
            writable: true,
        }],
        cpus: 2,
        memory_mb: 1024,
        network: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::FileTypeExt;

    #[test]
    fn firecracker_gets_the_vms_disks_size_and_vsock() -> Result<()> {
        let vm = Vm {
            dir: PathBuf::from("/data/firecracker/sandboxes/fctl-ses-1"),
            run: PathBuf::from("/tmp/fctl-ada/vm-0123"),
            key: "0123".into(),
            jails: None,
        };
        let spec = Spec {
            kernel: "/k/vmlinux".into(),
            initrd: "/i/1.cpio".into(),
            boot: Boot {
                mode: Mode::Sandbox,
                name: "fctl-ses-1".into(),
                time_ms: Some(5),
                dns: vec![[168, 63, 129, 16].into()],
            },
            drives: vec![
                Drive {
                    path: "/img/rootfs.ext4".into(),
                    writable: false,
                },
                Drive {
                    path: "/sb/disk.ext4".into(),
                    writable: true,
                },
            ],
            cpus: 64,
            memory_mb: 4096,
            network: true,
        };
        let c = vm.config(&spec);
        assert_eq!(
            c["boot-source"]["boot_args"],
            format!(
                "{} fctl.mode=sandbox fctl.name=fctl-ses-1 fctl.time=5 fctl.dns=168.63.129.16",
                kernel_args()
            )
        );
        assert_eq!(c["drives"][0]["drive_id"], "vda");
        assert_eq!(c["drives"][0]["is_read_only"], true);
        assert!(c["drives"][0].get("discard").is_none());
        assert_eq!(c["drives"][1]["drive_id"], "vdb");
        assert_eq!(c["drives"][1]["discard"], true);
        assert_eq!(c["machine-config"]["vcpu_count"], 32);
        assert_eq!(c["machine-config"]["mem_size_mib"], 4096);
        assert_eq!(c["vsock"]["uds_path"], "/tmp/fctl-ada/vm-0123/v.sock");
        Ok(())
    }

    #[test]
    fn passt_gives_the_vm_its_network_and_nothing_of_the_hosts() {
        let args = passt_args(Path::new("/tmp/fctl-ada/vm-0.passt.pid")).join(" ");
        assert_eq!(
            args,
            "--foreground --quiet --fd 0 --pid /tmp/fctl-ada/vm-0.passt.pid --address 10.0.2.15 --netmask 24 --gateway 10.0.2.2 --dns-forward 10.0.2.3 --no-map-gw"
        );
    }

    /// The host's nameservers, past systemd-resolved's stub on its loopback
    /// as Ubuntu has it, to the servers it asks.
    #[test]
    fn vms_ask_the_hosts_nameservers_and_none_on_its_loopback() {
        let stub = "# This is /run/systemd/resolve/stub-resolv.conf\nnameserver 127.0.0.53\noptions edns0 trust-ad\nsearch .\n";
        let upstream = "nameserver 168.63.129.16\nnameserver fe80::1%eth0\nnameserver 10.0.0.2\nnameserver 1.1.1.1\nnameserver 8.8.8.8\nsearch example.internal\n";
        let v4 = |a: [u8; 4]| Ipv4Addr::from(a);
        assert_eq!(
            dns_for_vm(stub, upstream),
            [v4([168, 63, 129, 16]), v4([10, 0, 0, 2]), v4([1, 1, 1, 1])]
        );
        // The host's own, but for those on its loopback.
        let router = "nameserver 127.0.0.1\nnameserver 192.168.1.1\n";
        assert_eq!(dns_for_vm(router, upstream), [v4([192, 168, 1, 1])]);
        assert_eq!(dns_for_vm("", upstream)[0], v4([168, 63, 129, 16]));
        // Nothing the VM could ask: passt's forwarder. QEMU's user network
        // has its resolver where the VM's forwarder is, 10.0.2.3, and IPv6
        // doesn't reach the VM.
        assert!(dns_for_vm(stub, "").is_empty());
        assert!(dns_for_vm(stub, "nameserver 2001:4860:4860::8888\n").is_empty());
        assert!(dns_for_vm("nameserver 10.0.2.3\n", "").is_empty());
    }

    /// Each VM's jail runs as a user of its own: the same for the same VM,
    /// another when another VM's jail has it.
    #[test]
    fn jails_take_users_of_their_own() {
        let none = std::collections::HashSet::new();
        let id = pick_jail_id("00000005abcdef01", &none);
        assert_eq!(id, JAIL_ID_BASE + 5);
        assert_eq!(pick_jail_id("00000005abcdef01", &none), id);
        let taken = std::collections::HashSet::from([id, id + 1]);
        assert_eq!(pick_jail_id("00000005abcdef01", &taken), id + 2);
        // The last one wraps around to the first.
        let last = format!("{:08x}", JAIL_IDS - 1);
        let taken = std::collections::HashSet::from([JAIL_ID_BASE + JAIL_IDS - 1]);
        assert_eq!(pick_jail_id(&last, &taken), JAIL_ID_BASE);
        assert!((JAIL_ID_BASE..JAIL_ID_BASE + JAIL_IDS).contains(&pick_jail_id("zz", &none)));
    }

    #[test]
    fn the_jailer_runs_firecracker_as_the_jails_user() {
        let args = jailer_args(
            "0123456789abcdef",
            Path::new("/var/lib/fctl/firecracker/bin/firecracker-v1.17.0"),
            1_900_000_007,
            Path::new("/var/lib/fctl/firecracker/jail"),
        );
        let line: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            line.join(" "),
            "--id 0123456789abcdef --exec-file /var/lib/fctl/firecracker/bin/firecracker-v1.17.0 --uid 1900000007 --gid 1900000007 --chroot-base-dir /var/lib/fctl/firecracker/jail --cgroup-version 2 --parent-cgroup fctl --cgroup pids.max=256 -- --api-sock /api.sock --config-file /firecracker.json --level Warning"
        );
    }

    /// A jail gets the VM's disks as hard links of the files they name, and
    /// its kernel as one where everyone may read it, else as a copy the
    /// jail's user may read.
    #[test]
    fn jails_get_the_vms_files() -> Result<()> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("root");
        std::fs::create_dir(&root)?;
        let disk = dir.path().join("disk.ext4");
        std::fs::write(&disk, b"ext4")?;
        let named = dir.path().join("named.ext4");
        std::os::unix::fs::symlink(&disk, &named)?;
        assert_eq!(
            link_into(&root, &named, "drive0.ext4")?,
            PathBuf::from("/drive0.ext4")
        );
        let linked = std::fs::symlink_metadata(root.join("drive0.ext4"))?;
        assert!(linked.is_file());
        assert_eq!(linked.ino(), std::fs::metadata(&disk)?.ino());
        let e = link_into(&root, &dir.path().join("gone"), "drive1.ext4");
        assert!(e.is_err_and(|e| format!("{e:#}").contains("gone")));
        // A kernel everyone may read, through a link: the file itself.
        let kernel = dir.path().join("vmlinux-6.18");
        std::fs::write(&kernel, b"kernel")?;
        std::fs::set_permissions(&kernel, std::fs::Permissions::from_mode(0o644))?;
        let link = dir.path().join("vmlinux");
        std::os::unix::fs::symlink(&kernel, &link)?;
        put_in_jail(&root, &link, "vmlinux")?;
        assert_eq!(
            std::fs::symlink_metadata(root.join("vmlinux"))?.ino(),
            std::fs::metadata(&kernel)?.ino()
        );
        // A private one: a copy the jail's user may read, the file kept as
        // it was.
        let private = dir.path().join("initrd.cpio");
        std::fs::write(&private, b"initrd")?;
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600))?;
        put_in_jail(&root, &private, "initrd.cpio")?;
        let copy = std::fs::metadata(root.join("initrd.cpio"))?;
        assert_ne!(copy.ino(), std::fs::metadata(&private)?.ino());
        assert_eq!(copy.mode() & 0o777, 0o444);
        assert_eq!(std::fs::read(root.join("initrd.cpio"))?, b"initrd");
        assert_eq!(std::fs::metadata(&private)?.mode() & 0o777, 0o600);
        Ok(())
    }

    /// The host reaches a jailed VM's sockets through links of its own, to
    /// the sockets Firecracker made: never through one the jail's user put
    /// in their place, which could lead to another VM's.
    #[tokio::test]
    async fn the_host_links_only_a_jails_own_sockets() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let vm = Vm::new(dir.path().join("fctl-ses-jailed"))?;
        let root = dir.path().join("root");
        std::fs::create_dir(&root)?;
        // As `start_jailed` makes them: `run` leads to the host's links.
        let sockets = dir.path().join("run");
        std::fs::create_dir(&sockets)?;
        std::fs::create_dir_all(vm.run.parent().context("no parent")?)?;
        let _ = std::fs::remove_file(&vm.run);
        std::os::unix::fs::symlink(&sockets, &vm.run)?;
        let _api = std::os::unix::net::UnixListener::bind(root.join("api.sock"))?;
        // Another VM's socket, and a link to it where this one's goes.
        let other = std::os::unix::net::UnixListener::bind(dir.path().join("other.sock"))?;
        std::os::unix::fs::symlink(dir.path().join("other.sock"), root.join("v.sock"))?;
        let e = vm
            .link_sockets(&root, &sockets, Duration::from_millis(100))
            .await;
        assert_eq!(
            e.map_err(|e| e.to_string()),
            Err("v.sock in the VM's jail isn't a socket".to_owned())
        );
        assert!(!vm.vsock().exists());
        drop(other);
        // Its own: the host's links reach it, whatever the jail does after.
        std::fs::remove_file(root.join("v.sock"))?;
        let own = std::os::unix::net::UnixListener::bind(root.join("v.sock"))?;
        vm.link_sockets(&root, &sockets, Duration::from_millis(100))
            .await?;
        std::fs::remove_file(root.join("v.sock"))?;
        std::os::unix::fs::symlink(dir.path().join("other.sock"), root.join("v.sock"))?;
        let _conn = std::os::unix::net::UnixStream::connect(vm.vsock())?;
        own.set_nonblocking(true)?;
        assert!(own.accept().is_ok());
        assert!(std::fs::symlink_metadata(vm.api())?.file_type().is_socket());
        std::fs::remove_file(&vm.run)?;
        Ok(())
    }

    /// Jails that no running VM has go once they are older than a VM takes
    /// to start, with what they lent given back.
    #[test]
    fn stale_jails_go() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let jails = dir.path().join("jail");
        let disk = dir.path().join("disk.ext4");
        std::fs::write(&disk, b"ext4")?;
        let lent = Lent {
            uid: JAIL_ID_BASE,
            writable: vec![disk.clone()],
        };
        let jail = |key: &str, age: Duration| -> Result<PathBuf> {
            let d = jails.join("firecracker-v1.17.0").join(key);
            std::fs::create_dir_all(d.join("root"))?;
            std::fs::write(d.join("jail.json"), serde_json::to_vec(&lent)?)?;
            std::fs::File::open(&d)?.set_modified(std::time::SystemTime::now() - age)?;
            Ok(d)
        };
        let hour = Duration::from_secs(3600);
        let stale = jail("aaaaaaaaaaaaaaaa", hour)?;
        let running = jail("bbbbbbbbbbbbbbbb", hour)?;
        let starting = jail("cccccccccccccccc", Duration::ZERO)?;
        // Lent to the jail's user, which only root can (CI runs this as root).
        let root = nix::unistd::geteuid().is_root();
        if root {
            std::os::unix::fs::chown(&disk, Some(JAIL_ID_BASE), Some(JAIL_ID_BASE))?;
        }
        let keys = std::collections::HashSet::from(["bbbbbbbbbbbbbbbb".to_owned()]);
        remove_stale_jails(&jails, &keys, Duration::from_secs(600));
        assert!(!stale.exists() && running.exists() && starting.exists());
        // The disk it lent is the host's user's again, and still there.
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(&disk)?.uid(),
            nix::unistd::geteuid().as_raw()
        );
        if !root {
            eprintln!("only root lends disks: this checked that the jail went");
        }
        Ok(())
    }

    #[test]
    fn a_vm_runs_only_while_its_process_does() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let vm = Vm::new(dir.path().join("fctl-ses-x"))?;
        assert!(vm.run.starts_with("/tmp"));
        std::fs::create_dir_all(&vm.dir)?;
        assert!(!vm.running());
        // A process whose command line has the VM's socket directory.
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 30; true", &vm.api().display().to_string()])
            .spawn()?;
        // As spawn_detached does: other tests' threads may hold its exec up.
        wait_for_exec(child.id());
        std::fs::write(vm.pid_file("firecracker"), child.id().to_string())?;
        assert!(vm.running());
        // Another process with that id isn't the VM's.
        std::fs::write(vm.pid_file("firecracker"), std::process::id().to_string())?;
        assert!(!vm.running());
        child.kill()?;
        child.wait()?;
        Ok(())
    }

    /// A process is the VM's as soon as `spawn_detached` returns, even
    /// while other threads map and unmap memory: that holds the child's
    /// exec up while it still runs on this process's memory, whose command
    /// line `/proc` then shows for it (fctl#427).
    #[test]
    fn a_process_is_the_vms_as_soon_as_it_starts() -> Result<()> {
        use crate::driver::fake::write_script;
        use nix::sys::signal::{Signal, killpg};
        use nix::unistd::Pid;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir()?;
        let firecracker = dir.path().join("firecracker");
        write_script(&firecracker, "#!/bin/sh\nsleep 60\n")?;
        let vm = Vm::new(dir.path().join("fctl-ses-x"))?;
        std::fs::create_dir_all(&vm.dir)?;
        let stop = Arc::new(AtomicBool::new(false));
        // Allocations this large are mapped and unmapped each time.
        let churn: Vec<_> = (0..3)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let mut v: Vec<u8> = Vec::with_capacity(64 << 20);
                        v.push(1);
                        std::hint::black_box(&v);
                    }
                })
            })
            .collect();
        let mut missed = 0;
        let tries = 300;
        let result = (|| -> Result<()> {
            for _ in 0..tries {
                let mut cmd = std::process::Command::new(&firecracker);
                cmd.arg("--api-sock")
                    .arg(vm.api())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                let pid = spawn_detached(&mut cmd, &vm.pid_file("firecracker"))?;
                if !vm.running() {
                    missed += 1;
                }
                let group = Pid::from_raw(i32::try_from(pid)?);
                killpg(group, Signal::SIGKILL)?;
            }
            Ok(())
        })();
        stop.store(true, Ordering::Relaxed);
        for thread in churn {
            let _ = thread.join();
        }
        result?;
        assert_eq!(
            missed, 0,
            "{missed} of {tries} just-started processes weren't the VM's"
        );
        Ok(())
    }

    /// What a stand-in Firecracker was asked, in order.
    type Requests = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    /// A stand-in for Firecracker's API on `socket`: it says what its VM
    /// does, pauses and resumes it as Firecracker does, and refuses to
    /// resume one that runs. It answers each request on a connection of its
    /// own, and keeps them in `requests`.
    fn firecracker_api(socket: &Path) -> Result<(tokio::task::JoinHandle<()>, Requests)> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        std::fs::create_dir_all(socket.parent().context("no parent")?)?;
        let listener = tokio::net::UnixListener::bind(socket)?;
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = requests.clone();
        let task = tokio::spawn(async move {
            let mut state = "Running";
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                let request = loop {
                    let Ok(n) = stream.read(&mut chunk).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    let Some((head, body)) = text.split_once("\r\n\r\n") else {
                        continue;
                    };
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: "))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if body.len() >= len {
                        let line = head.lines().next().unwrap_or_default();
                        break format!("{line} {body}");
                    }
                };
                if let Ok(mut r) = seen.lock() {
                    r.push(request.clone());
                }
                let answer = if request.starts_with("GET / ") {
                    let body = format!(
                        r#"{{"id":"fctl-ses-x","state":"{state}","vmm_version":"1.17.0"}}"#
                    );
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                } else if request.contains(r#""state":"Paused""#) {
                    state = "Paused";
                    "HTTP/1.1 204 No Content\r\n\r\n".to_owned()
                } else if request.contains(r#""state":"Resumed""#) && state == "Paused" {
                    state = "Running";
                    "HTTP/1.1 204 No Content\r\n\r\n".to_owned()
                } else {
                    let body = r#"{"fault_message":"The requested operation is not supported in the current VM state."}"#;
                    format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = stream.write_all(answer.as_bytes()).await;
            }
        });
        Ok((task, requests))
    }

    #[tokio::test]
    async fn pauses_and_resumes_the_vm_through_firecrackers_api() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let vm = Vm::new(dir.path().join("fctl-ses-x"))?;
        // No Firecracker: nothing is paused.
        assert!(!vm.paused().await);
        assert!(vm.pause().await.is_err());
        let (api, requests) = firecracker_api(&vm.api())?;
        assert_eq!(vm.state().await?, "Running");
        vm.pause().await?;
        assert!(vm.paused().await);
        vm.resume().await?;
        assert_eq!(vm.state().await?, "Running");
        // What Firecracker refuses comes back with its reason.
        let refused = vm.resume().await.map_err(|e| e.to_string());
        assert_eq!(
            refused,
            Err("Firecracker answered PATCH /vm with 400: The requested operation is not supported in the current VM state.".into())
        );
        let requests = requests.lock().map(|r| r.clone()).unwrap_or_default();
        let (get, pause, resume) = (
            "GET / HTTP/1.1 ",
            r#"PATCH /vm HTTP/1.1 {"state":"Paused"}"#,
            r#"PATCH /vm HTTP/1.1 {"state":"Resumed"}"#,
        );
        assert_eq!(requests, [get, pause, get, resume, get, resume]);
        api.abort();
        let _ = std::fs::remove_dir_all(&vm.run);
        Ok(())
    }

    #[test]
    fn reads_an_answer_once_all_of_it_is_there() -> Result<()> {
        let whole =
            b"HTTP/1.1 200 OK\r\nServer: Firecracker API\r\ncontent-length: 4\r\n\r\n{\"a\"";
        assert_eq!(response(&whole[..20])?, None);
        assert_eq!(response(&whole[..whole.len() - 1])?, None);
        assert_eq!(response(whole)?, Some((200, &b"{\"a\""[..])));
        assert_eq!(
            response(b"HTTP/1.1 204 No Content\r\n\r\n")?,
            Some((204, &b""[..]))
        );
        assert!(response(b"nonsense\r\n\r\n").is_err());
        Ok(())
    }

    /// Says a stand-in's connections are to close.
    type Closer = std::sync::Arc<std::sync::atomic::AtomicBool>;
    /// What a stand-in's connections read.
    type Seen = std::sync::Arc<std::sync::Mutex<Vec<u8>>>;

    /// A stand-in for Firecracker's vsock socket at `socket`: each
    /// connection asking for `CONNECT 1025` gets `OK` and stays open until
    /// `close` says otherwise; the VM's side keeps what it reads in `got`.
    /// Anything else is closed at once, as when nothing in the VM listens.
    fn vsock_stand_in(socket: &Path) -> Result<(Closer, Seen)> {
        use std::io::{BufRead, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        std::fs::create_dir_all(socket.parent().context("no parent")?)?;
        let listener = std::os::unix::net::UnixListener::bind(socket)?;
        let close = Arc::new(AtomicBool::new(false));
        let got = Arc::new(Mutex::new(Vec::new()));
        let (closing, seen) = (close.clone(), got.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let mut reader = std::io::BufReader::new(&conn);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line != format!("CONNECT {NET_PORT}\n") {
                    continue;
                }
                let mut w = &conn;
                // The VM's first frame comes with Firecracker's answer.
                let _ = w.write_all(b"OK 1073741824\n\0\0\0\x02hi");
                let (closing, seen) = (closing.clone(), seen.clone());
                std::thread::spawn(move || {
                    let _ = conn.set_read_timeout(Some(Duration::from_millis(50)));
                    let mut buf = [0u8; 64];
                    while !closing.load(Ordering::Relaxed) {
                        if let Ok(n) = std::io::Read::read(&mut &conn, &mut buf)
                            && let Ok(mut g) = seen.lock()
                        {
                            g.extend_from_slice(&buf[..n]);
                        }
                    }
                });
            }
        });
        Ok((close, got))
    }

    /// The host links a VM's network once it is up: passt gets the
    /// connection, with nothing of it read, a link made again takes the
    /// first one's place, passt exits as the VM goes, and a start that
    /// fails leaves nothing running. Stand-ins: a Firecracker that waits,
    /// and a passt that writes its pid where it's asked and echoes its
    /// input.
    #[tokio::test]
    async fn links_the_vms_network_once_it_is_up() -> Result<()> {
        use crate::driver::fake::write_script;
        use std::sync::atomic::Ordering;
        let dir = tempfile::tempdir()?;
        let passt = dir.path().join("passt");
        // As passt: its pid's file is made at once and written once it is
        // set up. Without `exec`, so that its command line keeps the VM's
        // path.
        let stand_in = |then: &str| {
            format!(
                "#!/bin/sh\nfor a; do [ \"$prev\" = --pid ] && pid=$a; prev=$a; done\n: > \"$pid\"\nsleep 0.3\n{then}\n"
            )
        };
        write_script(&passt, &stand_in("echo $$ > \"$pid\"\ncat >&0"))?;
        // One that fails as it sets itself up, and tells where its pid's
        // file was.
        let failing = dir.path().join("failing-passt");
        let told = dir.path().join("failing-passt.told");
        write_script(
            &failing,
            &stand_in(&format!(
                "echo \"$pid\" > '{}'\necho 'No routable interface' >&2; exit 1",
                told.display()
            )),
        )?;
        let firecracker = dir.path().join("firecracker");
        write_script(&firecracker, "#!/bin/sh\nsleep 60\n")?;
        let vm = Vm::new(dir.path().join("fctl-ses-x"))?;
        let spec = unpack_spec("/k".into(), "/i".into(), "fctl-ses-x", Path::new("/r"));
        vm.start(&spec, &firecracker, None).await?;
        assert!(vm.running() && !vm.linked());
        let (close, got) = vsock_stand_in(&vm.vsock())?;
        // A passt that fails once it has made its pid's file: no link,
        // though an earlier passt's pid is where this one's goes.
        if !nix::unistd::geteuid().is_root() {
            std::fs::write(
                vm.run.with_file_name(format!("vm-{}.passt.pid", vm.key)),
                std::process::id().to_string(),
            )?;
        }
        let e = vm.link_network(&failing).await.err().map(|e| e.to_string());
        assert_eq!(
            e.as_deref(),
            Some("passt did not start: No routable interface")
        );
        let made = PathBuf::from(std::fs::read_to_string(&told)?.trim());
        assert!(made.is_absolute(), "{made:?}");
        assert!(!vm.linked() && !made.exists() && vm.passt_pid().is_none());
        vm.link_network(&passt).await?;
        let first = vm.pid("passt");
        assert!(vm.linked());
        // passt got the VM's first frame, which came with Firecracker's
        // answer, and sent it back.
        let until = Instant::now() + Duration::from_secs(5);
        while got.lock().map(|g| g.clone()).unwrap_or_default() != b"\0\0\0\x02hi" {
            assert!(Instant::now() < until, "passt didn't get the VM's frame");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // Linked again: the first passt goes.
        vm.link_network(&passt).await?;
        let alive = |pid: u32| {
            std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| {
                !s.rsplit_once(')')
                    .is_some_and(|(_, r)| r.trim_start().starts_with('Z'))
            })
        };
        assert!(
            first.is_some_and(|p| !alive(p)),
            "the first passt still runs"
        );
        assert!(vm.linked() && vm.pid("passt") != first);
        // The VM goes, and its passt with it.
        close.store(true, Ordering::Relaxed);
        let until = Instant::now() + Duration::from_secs(5);
        while vm.linked() {
            assert!(Instant::now() < until, "passt outlived its link");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // Nothing in the VM takes the link: it fails, and says why.
        let other = Vm::new(dir.path().join("fctl-ses-y"))?;
        std::fs::create_dir_all(&other.run)?;
        let refusing = std::os::unix::net::UnixListener::bind(other.vsock())?;
        std::thread::spawn(move || {
            for conn in refusing.incoming().flatten() {
                drop(conn);
            }
        });
        let e = other
            .connect_port(NET_PORT, Duration::from_millis(300))
            .await
            .err()
            .map(|e| format!("{e:#}"));
        assert!(
            e.as_deref()
                .is_some_and(|e| e.contains("nothing in the VM takes connections")),
            "{e:?}"
        );
        // A start that fails leaves nothing running.
        let missing = dir.path().join("no-firecracker");
        assert!(vm.start(&spec, &missing, None).await.is_err());
        assert!(!vm.linked() && !vm.running());
        let _ = std::fs::remove_dir_all(&other.run);
        Ok(())
    }
}
