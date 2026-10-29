//! PID 1: from the initramfs to the sandbox's root, the system there, and
//! reaping.

use super::{serve, unpack};
use anyhow::{Context, Result, bail};
use fctl_proto::vm::{Boot, Mode};
use nix::errno::Errno;
use nix::mount::{MsFlags, mount};
use nix::sys::reboot::{RebootMode, reboot};
use nix::sys::wait::waitpid;
use nix::unistd::{Pid, chdir, chroot};
use std::ffi::CString;
use std::path::Path;
use std::time::{Duration, Instant};

/// A file only the initramfs has, which the host puts there.
pub const MARKER: &str = "/fctl-initramfs";

/// Where the sandbox's root comes together before the init switches to it.
const NEW_ROOT: &str = "/newroot";

/// Where the image's filesystem and the sandbox's disk stay mounted, in the
/// sandbox's root.
const IMAGE_MOUNT: &str = "/run/fctl/image";
const DISK_MOUNT: &str = "/run/fctl/disk";

/// Where fctl-agent lives in sandboxes (the host's `driver::AGENT_PATH`).
const AGENT_PATH: &str = "/opt/fctl/bin/fctl-agent";

/// fctl-agent as PID 1: in the initramfs it prepares the sandbox's root and
/// runs itself there, or unpacks an image; in the sandbox's root it sets the
/// system up and reaps. It never returns: when it can't go on, it powers the
/// VM off, which ends its Firecracker.
pub fn init() -> ! {
    let result = if Path::new(MARKER).exists() {
        first()
    } else {
        second()
    };
    if let Err(e) = result {
        say(&format!("{e:#}"));
    }
    power_off()
}

/// The init writes to the VM's console, which the host keeps in the
/// sandbox's `console.log`.
pub fn say(line: &str) {
    eprintln!("fctl-agent init: {line}");
}

/// Resets the VM: Firecracker exits when its guest does (`reboot=k` on
/// x86_64, PSCI on aarch64).
pub fn power_off() -> ! {
    nix::unistd::sync();
    let _ = reboot(RebootMode::RB_AUTOBOOT);
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

/// In the initramfs.
fn first() -> Result<()> {
    mount_fs(
        "devtmpfs",
        "/dev",
        "devtmpfs",
        MsFlags::MS_NOSUID,
        Some("mode=0755"),
    )?;
    let hidden = MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC;
    mount_fs("proc", "/proc", "proc", hidden, None)?;
    mount_fs("sysfs", "/sys", "sysfs", hidden, None)?;
    let boot = boot_params()?;
    match boot.mode {
        Mode::Unpack => unpack_image(),
        Mode::Sandbox => switch_root(),
    }
}

/// The kernel's command line says what to boot to.
fn boot_params() -> Result<Boot> {
    let cmdline = std::fs::read_to_string("/proc/cmdline").context("reading /proc/cmdline")?;
    Boot::parse(&cmdline).context("the kernel's command line has no fctl.mode")
}

/// Mounts `source` at `target`, making `target` first.
fn mount_fs(
    source: &str,
    target: &str,
    fstype: &str,
    flags: MsFlags,
    data: Option<&str>,
) -> Result<()> {
    std::fs::create_dir_all(target).with_context(|| format!("making {target}"))?;
    mount(Some(source), target, Some(fstype), flags, data)
        .with_context(|| format!("mounting {source} ({fstype}) at {target}"))
}

/// Moves the mount at `from` to `to`.
fn move_mount(from: &str, to: &str) -> Result<()> {
    std::fs::create_dir_all(to).with_context(|| format!("making {to}"))?;
    mount(Some(from), to, None::<&str>, MsFlags::MS_MOVE, None::<&str>)
        .with_context(|| format!("moving {from} to {to}"))
}

/// Waits for the kernel to find a disk.
fn wait_for(device: &str) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(10);
    while !Path::new(device).exists() {
        if Instant::now() > until {
            bail!("{device} did not appear");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// Puts the sandbox's root together, copies fctl-agent into it, and runs it
/// there as PID 1. Returns only when something fails.
fn switch_root() -> Result<()> {
    wait_for("/dev/vda")?;
    wait_for("/dev/vdb")?;
    mount_fs("/dev/vda", "/image", "ext4", MsFlags::MS_RDONLY, None)?;
    // `discard` gives the host back what the session deletes;
    // `noinit_itable` keeps the kernel from zeroing the inode tables of a
    // new disk, which would make it take room on the host.
    mount_fs(
        "/dev/vdb",
        "/disk",
        "ext4",
        MsFlags::empty(),
        Some("discard,noinit_itable"),
    )?;
    for dir in ["/disk/root", "/disk/work"] {
        std::fs::create_dir_all(dir).with_context(|| format!("making {dir}"))?;
    }
    mount_fs(
        "overlay",
        NEW_ROOT,
        "overlay",
        MsFlags::empty(),
        Some("lowerdir=/image,upperdir=/disk/root,workdir=/disk/work"),
    )?;
    let run = format!("{NEW_ROOT}/run");
    mount_fs(
        "tmpfs",
        &run,
        "tmpfs",
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("mode=0755"),
    )?;
    move_mount("/image", &format!("{NEW_ROOT}{IMAGE_MOUNT}"))?;
    move_mount("/disk", &format!("{NEW_ROOT}{DISK_MOUNT}"))?;
    install_self(&format!("{NEW_ROOT}{AGENT_PATH}"))?;
    for dir in ["/dev", "/proc", "/sys"] {
        move_mount(dir, &format!("{NEW_ROOT}{dir}"))?;
    }
    // The initramfs stays in memory under the new root; this is most of it.
    let _ = std::fs::remove_file("/init");
    let _ = std::fs::remove_file(MARKER);
    chdir(NEW_ROOT).context("changing to the new root")?;
    mount(Some("."), "/", None::<&str>, MsFlags::MS_MOVE, None::<&str>)
        .context("moving the new root to /")?;
    chroot(".").context("changing the root")?;
    chdir("/").context("changing to /")?;
    let path = CString::new(AGENT_PATH)?;
    let argv = [CString::new("fctl-agent")?, CString::new("vm-init")?];
    let e = nix::unistd::execv(&path, &argv).err();
    bail!("running {AGENT_PATH}: {e:?}")
}

/// Copies the running fctl-agent (`/init`) to `to`, where the host's
/// commands find it.
fn install_self(to: &str) -> Result<()> {
    let to = Path::new(to);
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    let new = to.with_extension("new");
    std::fs::copy("/init", &new).with_context(|| format!("copying /init to {}", new.display()))?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&new, to).with_context(|| format!("installing {}", to.display()))
}

/// Unpacks an image onto `/dev/vda`, then returns for [`init`] to power
/// off.
fn unpack_image() -> Result<()> {
    wait_for("/dev/vda")?;
    mount_fs(
        "/dev/vda",
        "/target",
        "ext4",
        MsFlags::empty(),
        Some("noinit_itable"),
    )?;
    let result = unpack::serve(Path::new("/target"));
    nix::unistd::sync();
    let _ = nix::mount::umount("/target");
    result
}

/// PID 1 in the sandbox's root.
fn second() -> Result<()> {
    let boot = boot_params()?;
    system(&boot);
    supervise()
}

/// What a system needs that the image doesn't bring: the kernel's
/// filesystems, a hostname, names for the network. What fails is said on
/// the console and left out.
fn system(boot: &Boot) {
    let hidden = MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC;
    let mounts: [(&str, &str, &str, MsFlags, Option<&str>); 3] = [
        (
            "devpts",
            "/dev/pts",
            "devpts",
            hidden,
            Some("gid=5,mode=620,ptmxmode=666"),
        ),
        (
            "tmpfs",
            "/dev/shm",
            "tmpfs",
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
            Some("mode=1777"),
        ),
        (
            "cgroup2",
            "/sys/fs/cgroup",
            "cgroup2",
            hidden | MsFlags::MS_NODEV,
            None,
        ),
    ];
    for (source, target, fstype, flags, data) in mounts {
        if let Err(e) = mount_fs(source, target, fstype, flags, data) {
            say(&format!("{e:#}"));
        }
    }
    for (link, to) in [
        ("/dev/fd", "/proc/self/fd"),
        ("/dev/stdin", "/proc/self/fd/0"),
        ("/dev/stdout", "/proc/self/fd/1"),
        ("/dev/stderr", "/proc/self/fd/2"),
    ] {
        let _ = std::os::unix::fs::symlink(to, link);
    }
    if let Err(e) = nix::unistd::sethostname(&boot.name) {
        say(&format!("setting the hostname: {e}"));
    }
    let hosts = format!(
        "127.0.0.1\tlocalhost\n::1\tlocalhost ip6-localhost ip6-loopback\n127.0.1.1\t{}\n",
        boot.name
    );
    let dns = std::net::Ipv4Addr::from(fctl_proto::vm::DNS);
    for (file, text) in [
        ("/etc/hosts", hosts),
        ("/etc/resolv.conf", format!("nameserver {dns}\n")),
    ] {
        if let Err(e) = replace(file, &text) {
            say(&format!("writing {file}: {e}"));
        }
    }
    if let Some(t) = boot.time_ms {
        serve::follow_clock(t);
    }
    // What Docker gives its containers: Claude Code, Node.js and builds
    // open many files.
    let files = 1024 * 1024;
    if let Err(e) =
        nix::sys::resource::setrlimit(nix::sys::resource::Resource::RLIMIT_NOFILE, files, files)
    {
        say(&format!("raising the open files limit: {e}"));
    }
    if !Path::new("/tmp").is_dir() {
        let _ = std::fs::create_dir("/tmp");
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions("/tmp", std::fs::Permissions::from_mode(0o1777));
    }
}

/// Writes `text` to `file`, in place of a link there (to systemd's
/// resolver, say, which doesn't run here).
fn replace(file: &str, text: &str) -> std::io::Result<()> {
    if std::fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_symlink()) {
        std::fs::remove_file(file)?;
    }
    std::fs::write(file, text)
}

/// Keeps `fctl-agent vm-serve` running, and reaps every program whose
/// parent exited before it.
fn supervise() -> Result<()> {
    loop {
        let server = std::process::Command::new("/proc/self/exe")
            .arg("vm-serve")
            .spawn();
        match server {
            Ok(child) => {
                let pid = Pid::from_raw(i32::try_from(child.id()).unwrap_or(i32::MAX));
                reap_until(pid);
            }
            Err(e) => say(&format!("starting vm-serve: {e}")),
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Reaps children until `server` exits.
fn reap_until(server: Pid) {
    loop {
        match waitpid(Pid::from_raw(-1), None) {
            Ok(status) if status.pid() == Some(server) => {
                say(&format!("vm-serve stopped ({status:?}); starting it again"));
                return;
            }
            Ok(_) | Err(Errno::EINTR) => {}
            Err(Errno::ECHILD) => return,
            Err(e) => {
                say(&format!("waiting for programs: {e}"));
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}
