//! A microVM's processes on the host: Firecracker, and `passt` for its
//! network. Both run in process groups of their own, so that restarting the
//! host (whose service stops only its main process) leaves them running.

use super::vsock;
use anyhow::{Context, Result, bail};
use fctl_proto::vm::{ADDRESS, Boot, DNS, GATEWAY, Mode, NET_PORT, PREFIX};
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
}

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
        Ok(Self { dir, run })
    }

    pub fn api(&self) -> PathBuf {
        self.run.join("api.sock")
    }

    pub fn vsock(&self) -> PathBuf {
        self.run.join("v.sock")
    }

    /// Where `passt` takes the VM's network: what Firecracker connects to
    /// when the VM connects to the host's [`NET_PORT`].
    pub fn net(&self) -> PathBuf {
        self.run.join(format!("v.sock_{NET_PORT}"))
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
    /// no command line yet, for a moment.
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
        let mine = cmdline.is_empty()
            || cmdline
                .split(|b| *b == 0)
                .any(|arg| String::from_utf8_lossy(arg).starts_with(&ours));
        (mine && !zombie).then_some(pid)
    }

    /// Whether the VM runs.
    pub fn running(&self) -> bool {
        self.pid("firecracker").is_some()
    }

    /// Starts `passt`, if the VM gets a network, and Firecracker.
    pub async fn start(&self, spec: &Spec, firecracker: &Path, passt: Option<&Path>) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let _ = std::fs::remove_dir_all(&self.run);
        std::fs::create_dir_all(&self.run)?;
        if spec.network {
            let passt = passt.context("the VM needs passt for its network")?;
            self.start_passt(passt, &spec.boot.name).await?;
        }
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

    /// `passt`, which takes the VM's frames at [`Vm::net`] and makes them
    /// connections of the host's: the VM gets [`ADDRESS`], the gateway and
    /// a DNS forwarder, and reaches neither the host's loopback nor other
    /// VMs. Nothing reaches the VM from outside.
    async fn start_passt(&self, passt: &Path, name: &str) -> Result<()> {
        let log = std::fs::File::create(self.dir.join("passt.log"))?;
        let mut cmd = std::process::Command::new(passt);
        cmd.args(passt_args(&self.net(), name))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        spawn_detached(&mut cmd, &self.pid_file("passt"))
            .with_context(|| format!("starting {}", passt.display()))?;
        let until = Instant::now() + Duration::from_secs(10);
        while !self.net().exists() {
            if Instant::now() > until || self.pid("passt").is_none() {
                let said = std::fs::read_to_string(self.dir.join("passt.log")).unwrap_or_default();
                bail!("passt did not start: {}", said.trim());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
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

    /// Asks the VM's init to stop it, and stops what is left.
    pub async fn stop(&self) {
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
            if let Some(pid) = self.pid(what) {
                let _ = crate::driver::run_cmd("kill", &["-KILL", &pid.to_string()]).await;
                let until = Instant::now() + Duration::from_secs(5);
                while self.pid(what).is_some() && Instant::now() < until {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
            let _ = std::fs::remove_file(self.pid_file(what));
        }
        let _ = std::fs::remove_dir_all(&self.run);
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

/// The arguments of `passt` for a VM named `name` whose frames come to
/// `socket`.
fn passt_args(socket: &Path, name: &str) -> Vec<String> {
    let mask = PREFIX.to_string();
    vec![
        "--foreground".into(),
        "--quiet".into(),
        "--socket".into(),
        socket.display().to_string(),
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
        "--hostname".into(),
        name.into(),
    ]
}

/// Starts `cmd` in a process group of its own, writes its id to
/// `pid_file`, and reaps it when it exits while this host runs.
fn spawn_detached(cmd: &mut std::process::Command, pid_file: &Path) -> Result<u32> {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
    let mut child = cmd.spawn()?;
    let pid = child.id();
    std::fs::write(pid_file, pid.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
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

    #[test]
    fn firecracker_gets_the_vms_disks_size_and_vsock() -> Result<()> {
        let vm = Vm {
            dir: PathBuf::from("/data/firecracker/sandboxes/fctl-ses-1"),
            run: PathBuf::from("/tmp/fctl-ada/vm-0123"),
        };
        let spec = Spec {
            kernel: "/k/vmlinux".into(),
            initrd: "/i/1.cpio".into(),
            boot: Boot {
                mode: Mode::Sandbox,
                name: "fctl-ses-1".into(),
                time_ms: Some(5),
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
                "{} fctl.mode=sandbox fctl.name=fctl-ses-1 fctl.time=5",
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
        assert_eq!(vm.net(), PathBuf::from("/tmp/fctl-ada/vm-0123/v.sock_1025"));
        Ok(())
    }

    #[test]
    fn passt_gives_the_vm_its_network_and_nothing_of_the_hosts() {
        let args = passt_args(Path::new("/tmp/fctl-ada/vm-0/v.sock_1025"), "fctl-ses-1").join(" ");
        assert_eq!(
            args,
            "--foreground --quiet --socket /tmp/fctl-ada/vm-0/v.sock_1025 --address 10.0.2.15 --netmask 24 --gateway 10.0.2.2 --dns-forward 10.0.2.3 --no-map-gw --hostname fctl-ses-1"
        );
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
        std::fs::write(vm.pid_file("firecracker"), child.id().to_string())?;
        assert!(vm.running());
        // Another process with that id isn't the VM's.
        std::fs::write(vm.pid_file("firecracker"), std::process::id().to_string())?;
        assert!(!vm.running());
        child.kill()?;
        child.wait()?;
        Ok(())
    }
}
