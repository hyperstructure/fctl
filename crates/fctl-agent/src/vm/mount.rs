//! `fctl-agent vm-mount`: mounts what the host shares on a vsock port (the
//! project's package caches, which `fctl-agent share` serves there) with
//! the kernel's 9p client, over the vsock connection itself (`trans=fd`),
//! as one user's files. The host runs it as root once the VM is up, and
//! again after a snapshot's restore, whose connections are gone.

use crate::user::accounts as users;
use anyhow::{Context, Result};
use nix::errno::Errno;
use nix::mount::{MntFlags, MsFlags, umount2};
use std::os::fd::AsRawFd;
use std::path::Path;

/// The largest message the VM's 9p client sends or asks for: a read or a
/// write of up to this much in one trip over the vsock.
const MSIZE: u32 = 512 * 1024;

/// Mounts the host's share on `port` at `dir`, whose files are `user`'s
/// there (a name or a uid; unset, root's), in place of a mount an earlier
/// connection left.
pub fn mount(port: u32, dir: &Path, user: Option<&str>) -> Result<()> {
    let user = users::find(user).map_err(anyhow::Error::msg)?;
    if mounted(dir) {
        umount2(dir, MntFlags::MNT_DETACH)
            .with_context(|| format!("unmounting what {} had", dir.display()))?;
    }
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    let conn = vsock::VsockStream::connect_with_cid_port(vsock::VMADDR_CID_HOST, port)
        .with_context(|| format!("connecting to the host's vsock port {port}"))?;
    let fd = conn.as_raw_fd();
    // The kernel holds the connection once mounted; this process's copy of
    // it goes as it exits.
    let options = options(fd, user.uid, user.gid);
    match nix::mount::mount(
        Some("fctl-share"),
        dir,
        Some("9p"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some(options.as_str()),
    ) {
        Ok(()) => Ok(()),
        Err(Errno::ENODEV) => anyhow::bail!(
            "the VM's kernel has no 9p filesystem (CONFIG_9P_FS and CONFIG_NET_9P_FD), so it can't mount what the host shares at {}",
            dir.display()
        ),
        Err(e) => {
            Err(e).with_context(|| format!("mounting what the host shares at {}", dir.display()))
        }
    }
}

/// 9p's options: over the connection `fd`, one attach for every user of the
/// VM (`access=any`), whose files the server says are `uid:gid`'s (its
/// `aname`), and the page cache only for files mapped into memory.
fn options(fd: i32, uid: u32, gid: u32) -> String {
    format!(
        "trans=fd,rfdno={fd},wfdno={fd},version=9p2000.L,msize={MSIZE},access=any,cache=mmap,aname={uid}:{gid}"
    )
}

/// Whether something is mounted at `dir`.
fn mounted(dir: &Path) -> bool {
    let Ok(info) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let want = dir.to_string_lossy();
    info.lines()
        .any(|line| line.split(' ').nth(4) == Some(want.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounts_over_the_connection_as_the_users_files() {
        assert_eq!(
            options(5, 1000, 1001),
            "trans=fd,rfdno=5,wfdno=5,version=9p2000.L,msize=524288,access=any,cache=mmap,aname=1000:1001"
        );
        // The root of this machine is mounted; a directory made now isn't.
        assert!(mounted(Path::new("/")));
        assert!(!mounted(Path::new("/proc/self/fdinfo")));
    }
}
