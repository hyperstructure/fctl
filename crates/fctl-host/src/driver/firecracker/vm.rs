//! A microVM's processes on the host: Firecracker, and `passt` for its
//! network. Both run in process groups of their own, so that restarting the
//! host (whose service stops only its main process) leaves them running.

use super::vsock;
use anyhow::{Context, Result, anyhow, bail};
use fctl_proto::vm::{ADDRESS, Boot, DNS, GATEWAY, Mode, NET_PORT, PREFIX};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv4Addr};
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
        let mine = starting
            || cmdline
                .split(|b| *b == 0)
                .any(|arg| String::from_utf8_lossy(arg).starts_with(&ours));
        (mine && !zombie).then_some(pid)
    }

    /// Whether the VM runs.
    pub fn running(&self) -> bool {
        self.pid("firecracker").is_some()
    }

    /// Starts `passt`, if the VM gets a network, and Firecracker, after
    /// stopping what an earlier boot left: the `passt` of a VM that powered
    /// off, say. What it started goes again when it fails.
    pub async fn start(&self, spec: &Spec, firecracker: &Path, passt: Option<&Path>) -> Result<()> {
        self.kill().await;
        let started = self.start_processes(spec, firecracker, passt).await;
        if started.is_err() {
            self.kill().await;
        }
        started
    }

    async fn start_processes(
        &self,
        spec: &Spec,
        firecracker: &Path,
        passt: Option<&Path>,
    ) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::create_dir_all(&self.run)?;
        if spec.network {
            let passt = passt.context("the VM needs passt for its network")?;
            self.start_passt(passt).await?;
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
    async fn start_passt(&self, passt: &Path) -> Result<()> {
        let log = std::fs::File::create(self.dir.join("passt.log"))?;
        let mut cmd = std::process::Command::new(passt);
        cmd.args(passt_args(&self.net(), &host_dns()))
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

/// The arguments of `passt` for a VM whose frames come to `socket`, whose
/// DNS queries go to `dns` (when empty, to the host's resolver): the
/// options of passt's releases since 2022, which Ubuntu 24.04 and Debian 12
/// have. The VM names itself, so passt's DHCP needs no `--hostname`, which
/// came later.
fn passt_args(socket: &Path, dns: &[IpAddr]) -> Vec<String> {
    let mask = PREFIX.to_string();
    let mut args: Vec<String> = vec![
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
    ];
    for addr in dns {
        args.extend(["--dns".into(), addr.to_string()]);
    }
    args
}

/// Where the VMs' DNS queries go, when passt can't find it alone: as Docker
/// does, the servers systemd-resolved asks, where the host's
/// `/etc/resolv.conf` names only a resolver on its loopback (systemd-resolved's
/// 127.0.0.53). passt before 2024's summer forwards no queries to a resolver
/// on the loopback with `--no-map-gw` (Ubuntu 24.04's says "Couldn't get any
/// nameserver address"). Empty when the host names another resolver, or
/// nothing else is known: passt finds it then, or newer passt forwards to the
/// loopback's.
fn host_dns() -> Vec<IpAddr> {
    let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    dns_for_vms(
        &read("/etc/resolv.conf"),
        &read("/run/systemd/resolve/resolv.conf"),
    )
}

/// [`host_dns`], from the files' text: the IPv4 servers first, and IPv6
/// ones only without any, at most three.
fn dns_for_vms(resolv_conf: &str, resolved_upstream: &str) -> Vec<IpAddr> {
    let servers = |text: &str| -> Vec<IpAddr> {
        text.lines()
            .filter_map(|l| l.trim().strip_prefix("nameserver"))
            .filter_map(|rest| rest.split_whitespace().next())
            // A link-local IPv6 server carries its interface: `fe80::1%eth0`.
            .filter_map(|a| a.split('%').next().and_then(|a| a.parse().ok()))
            .collect()
    };
    let host = servers(resolv_conf);
    if host.is_empty() || host.iter().any(|a| !a.is_loopback()) {
        return Vec::new();
    }
    let upstream: Vec<IpAddr> = servers(resolved_upstream)
        .into_iter()
        .filter(|a| !a.is_loopback())
        .collect();
    let v4: Vec<IpAddr> = upstream.iter().copied().filter(IpAddr::is_ipv4).collect();
    let pick = if v4.is_empty() { upstream } else { v4 };
    pick.into_iter().take(3).collect()
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
        let args = passt_args(Path::new("/tmp/fctl-ada/vm-0/v.sock_1025"), &[]).join(" ");
        assert_eq!(
            args,
            "--foreground --quiet --socket /tmp/fctl-ada/vm-0/v.sock_1025 --address 10.0.2.15 --netmask 24 --gateway 10.0.2.2 --dns-forward 10.0.2.3 --no-map-gw"
        );
        let dns = [IpAddr::from([168, 63, 129, 16])];
        let args = passt_args(Path::new("/s"), &dns).join(" ");
        assert!(args.ends_with("--no-map-gw --dns 168.63.129.16"), "{args}");
    }

    /// systemd-resolved's stub on the loopback, as Ubuntu has it, and its
    /// upstream servers, which passt gets; any other resolver passt finds
    /// itself.
    #[test]
    fn dns_goes_past_a_resolver_on_the_hosts_loopback() {
        let stub = "# This is /run/systemd/resolve/stub-resolv.conf\nnameserver 127.0.0.53\noptions edns0 trust-ad\nsearch .\n";
        let upstream = "nameserver 168.63.129.16\nnameserver fe80::1%eth0\nnameserver 10.0.0.2\nnameserver 1.1.1.1\nnameserver 8.8.8.8\nsearch example.internal\n";
        let v4 = |a: [u8; 4]| IpAddr::from(a);
        assert_eq!(
            dns_for_vms(stub, upstream),
            [v4([168, 63, 129, 16]), v4([10, 0, 0, 2]), v4([1, 1, 1, 1])]
        );
        // IPv6 servers only where there is no IPv4 one.
        let only_v6 = "nameserver 2001:4860:4860::8888\nnameserver ::1\n";
        assert_eq!(
            dns_for_vms(stub, only_v6),
            [IpAddr::from([0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888])]
        );
        // A resolver passt can reach, or nothing to go past to.
        assert!(dns_for_vms("nameserver 10.0.2.3\n", upstream).is_empty());
        assert!(
            dns_for_vms("nameserver 127.0.0.53\nnameserver 192.168.1.1\n", upstream).is_empty()
        );
        assert!(dns_for_vms(stub, "").is_empty());
        assert!(dns_for_vms("", upstream).is_empty());
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

    /// Stand-ins: a `passt` that makes its socket's path and waits, and a
    /// Firecracker that waits.
    #[tokio::test]
    async fn starting_again_stops_what_the_last_boot_left() -> Result<()> {
        use crate::driver::fake::write_script;
        let dir = tempfile::tempdir()?;
        let passt = dir.path().join("passt");
        // Without `exec`, so that their command lines keep the sockets.
        write_script(
            &passt,
            "#!/bin/sh\nfor a; do [ \"$prev\" = --socket ] && touch \"$a\"; prev=$a; done\nsleep 60\n",
        )?;
        let firecracker = dir.path().join("firecracker");
        write_script(&firecracker, "#!/bin/sh\nsleep 60\n")?;
        let vm = Vm::new(dir.path().join("fctl-ses-x"))?;
        let spec = unpack_spec("/k".into(), "/i".into(), "fctl-ses-x", Path::new("/r"));
        let spec = Spec {
            network: true,
            ..spec
        };
        vm.start(&spec, &firecracker, Some(&passt)).await?;
        let first_passt = vm.pid("passt");
        assert!(first_passt.is_some() && vm.running());
        // Firecracker ends by itself (its VM powered off), and passt stays.
        if let Some(pid) = vm.pid("firecracker") {
            crate::driver::run_cmd("kill", &["-KILL", &pid.to_string()]).await?;
        }
        while vm.running() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(vm.pid("passt"), first_passt);
        vm.start(&spec, &firecracker, Some(&passt)).await?;
        let alive = |pid: u32| {
            std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| {
                !s.rsplit_once(')')
                    .is_some_and(|(_, r)| r.trim_start().starts_with('Z'))
            })
        };
        assert!(
            first_passt.is_some_and(|p| !alive(p)),
            "the first passt still runs"
        );
        assert!(vm.pid("passt").is_some() && vm.running());
        // A start that fails leaves nothing running.
        let missing = dir.path().join("no-firecracker");
        assert!(vm.start(&spec, &missing, Some(&passt)).await.is_err());
        assert!(vm.pid("passt").is_none() && !vm.running());
        Ok(())
    }
}
