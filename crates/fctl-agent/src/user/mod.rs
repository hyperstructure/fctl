//! Who fctl-agent runs as, and who runs the programs it starts.
//!
//! In container and VM sandboxes the host starts `fctl-agent serve --user
//! <user>` as root (`docker exec -u 0`, `sudo` in Lima, root over the vsock
//! with Firecracker), and fctl-agent [`protect`]s itself: it keeps root as
//! its real and saved user, so that the commands Claude runs, as that user,
//! can't signal it (`kill` and `pkill` fail with EPERM), and otherwise works
//! as the user, so that what it writes in the user's home stays the user's.
//! Its own files, in a directory only root may enter, it reaches as root
//! ([`as_root`]). The programs it starts, Claude first, run as the user,
//! through `fctl-agent run-as` ([`command`]).
//!
//! Without `--user` (the process driver, a sandbox without root) it runs as
//! whoever started it, and so do its programs.

#[cfg(target_os = "linux")]
pub mod accounts;
#[cfg(target_os = "linux")]
pub mod run_as;

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use tokio::process::Command;

/// The user Claude runs as, when fctl-agent runs as root.
#[cfg(target_os = "linux")]
static PROTECTED: std::sync::OnceLock<accounts::User> = std::sync::OnceLock::new();

/// Takes `user`'s identity, keeping root as the real and saved user: the
/// user's processes can't signal this one then, and what it writes is the
/// user's. Call it as root, before other threads start. A root `user`
/// leaves things as they are.
#[cfg(target_os = "linux")]
pub fn protect(user: &accounts::User) -> anyhow::Result<()> {
    use anyhow::Context;
    use nix::unistd::{Gid, Uid, setgroups, setresgid, setresuid};
    if user.uid == 0 {
        return Ok(());
    }
    let groups: Vec<Gid> = user.groups.iter().copied().map(Gid::from_raw).collect();
    setgroups(&groups).context("taking the user's groups")?;
    let (uid, gid) = (Uid::from_raw(user.uid), Gid::from_raw(user.gid));
    // Root stays the saved group and the real and saved user: [`as_root`]
    // takes it back for files, and `run-as` for programs.
    setresgid(gid, gid, Gid::from_raw(0)).context("taking the user's group")?;
    setresuid(Uid::from_raw(0), uid, Uid::from_raw(0)).context("taking the user's identity")?;
    let _ = PROTECTED.set(user.clone());
    Ok(())
}

/// The user Claude runs as, when fctl-agent [`protect`]s itself from it.
#[cfg(target_os = "linux")]
pub fn protected() -> Option<&'static accounts::User> {
    PROTECTED.get()
}

/// Runs `f` with root's access to files: fctl-agent's own, which only root
/// may reach in a sandbox where it [`protect`]s itself. Elsewhere, as it is.
/// `f` runs on this thread alone, which is the only one that gains it.
pub fn as_root<T>(f: impl FnOnce() -> T) -> T {
    #[cfg(target_os = "linux")]
    if protected().is_some() {
        let _root = FilesAsRoot::take();
        return f();
    }
    f()
}

/// This thread's file access as root, until dropped (Linux keeps a
/// filesystem user and group per thread); then as it was, so that it nests.
#[cfg(target_os = "linux")]
struct FilesAsRoot {
    uid: nix::unistd::Uid,
    gid: nix::unistd::Gid,
}

#[cfg(target_os = "linux")]
impl FilesAsRoot {
    fn take() -> Self {
        use nix::unistd::{Gid, Uid, setfsgid, setfsuid};
        Self {
            uid: setfsuid(Uid::from_raw(0)),
            gid: setfsgid(Gid::from_raw(0)),
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for FilesAsRoot {
    fn drop(&mut self) {
        nix::unistd::setfsgid(self.gid);
        nix::unistd::setfsuid(self.uid);
    }
}

/// A command for `program` that runs as the session's user, in a session
/// (and process group) of its own where fctl-agent runs as root. Callers
/// add its arguments, environment and directory as for any command; one
/// that clears the environment sets `HOME`, `USER` and `LOGNAME` from
/// [`var`].
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[cfg(target_os = "linux")]
    if let Some(user) = protected() {
        let groups: Vec<String> = user.groups.iter().map(u32::to_string).collect();
        let mut cmd = Command::new("/proc/self/exe");
        cmd.arg("run-as")
            .arg(user.uid.to_string())
            .arg(user.gid.to_string())
            .arg(groups.join(","))
            .arg(".")
            .arg("--")
            .arg(program);
        for key in ["HOME", "USER", "LOGNAME"] {
            if let Some(v) = var(key) {
                cmd.env(key, v);
            }
        }
        return cmd;
    }
    Command::new(program)
}

/// [`command`], in a process group of its own also where fctl-agent runs
/// as its user, so that a supervisor after this one can stop it with
/// everything it started.
pub fn command_in_own_group(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = command(program);
    #[cfg(target_os = "linux")]
    let own = protected().is_some();
    #[cfg(not(target_os = "linux"))]
    let own = false;
    // `run-as` starts a session, which a group's leader can't.
    #[cfg(unix)]
    if !own {
        cmd.process_group(0);
    }
    cmd
}

/// The environment variable `key` as the session's user has it: its
/// `HOME`, `USER` and `LOGNAME` where fctl-agent runs as root, whatever
/// root's are; fctl-agent's own otherwise.
pub fn var(key: &str) -> Option<OsString> {
    #[cfg(target_os = "linux")]
    if let Some(user) = protected() {
        match key {
            "HOME" => return Some(user.home.clone().into()),
            "USER" | "LOGNAME" => return user.name.clone().map(OsString::from),
            _ => {}
        }
    }
    std::env::var_os(key)
}

/// The session's user's home, where it has one.
pub fn home() -> Option<PathBuf> {
    var("HOME").map(PathBuf::from)
}

/// The OOM killer's adjustment that keeps a process from it.
#[cfg(target_os = "linux")]
const OOM_NEVER: &str = "-1000";

/// Keeps fctl-agent from the OOM killer, so that a build that fills the
/// sandbox's memory takes the build rather than the supervisor. It takes
/// root with the capability to lower it (`CAP_SYS_RESOURCE`): VMs have it,
/// Docker's containers don't. Whether it is kept from it now.
#[cfg(target_os = "linux")]
pub fn shield_from_oom() -> std::io::Result<()> {
    std::fs::write("/proc/self/oom_score_adj", OOM_NEVER)
}

/// What [`shield_from_oom`] did, undone for a program fctl-agent starts:
/// the kernel's default, where its parent's was lower.
#[cfg(target_os = "linux")]
pub fn reset_oom_score() {
    let path = "/proc/self/oom_score_adj";
    let low = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .is_some_and(|adj| adj < 0);
    if low {
        let _ = std::fs::write(path, "0");
    }
}
