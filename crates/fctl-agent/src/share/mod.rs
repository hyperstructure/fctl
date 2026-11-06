//! `fctl-agent share`: a 9P2000.L file server for a project's package
//! caches, which the host runs beside each Firecracker VM, as it runs
//! `passt` (crates/fctl-host/src/driver/firecracker). Firecracker's VMs share
//! no filesystem with the host, so the VM's kernel mounts the caches over
//! its vsock: the host listens where Firecracker connects the VM's
//! [`fctl_proto::vm::SHARE_PORT`], and `fctl-agent vm-mount` in the VM
//! connects to it and mounts the connection (9p's `trans=fd`). Each VM has a
//! server of its own; the files are the host's, in the directory every
//! sandbox of the project shares, as Docker's volumes and Apple's
//! `container`'s mounts are.
//!
//! Anything in the VM can connect to the host's port, so the server takes
//! no request as more than the files under its directory:
//!
//! - Every file it hands out was reached from the directory one name at a
//!   time, without following symlinks; names are never `.`, `..` or have a
//!   `/`. The client resolves symlinks, against the VM's files.
//! - It opens only directories and regular files, and makes neither device
//!   files nor setuid or setgid ones.
//! - Files belong to the host's user. The VM sees them as its user's (the
//!   uid and gid the mount names), whose package managers and git check that
//!   their caches are theirs, and changes of owner do nothing.
//! - Locks are the host's, on open files (`F_OFD_SETLK`), so two VMs' cargo
//!   runs wait for each other as two containers' do.

pub mod wire;

#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use nix::errno::Errno;
use nix::fcntl::{AT_FDCWD, FcntlArg, OFlag, openat};
use nix::sys::stat::{
    FchmodatFlags, FileStat, Mode, SFlag, UtimensatFlags, fchmodat, fstatat, mkdirat, utimensat,
};
use nix::sys::time::TimeSpec;
use nix::unistd::{UnlinkatFlags, linkat, symlinkat, unlinkat};
use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirEntryExt, FileExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use wire::{Qid, Reader, Writer, kind};

/// The largest message either side sends: the client asks for less.
pub const MAX_MSIZE: u32 = 1024 * 1024;

/// The most fids one connection holds: each is an open file of the
/// server's.
const MAX_FIDS: usize = 1 << 16;

/// The most connections served at once: the VM's mount makes one.
const MAX_CONNECTIONS: usize = 16;

/// The protocol, the one the Linux client asks for first.
const VERSION: &[u8] = b"9P2000.L";

/// `Rgetattr`'s fields that this server fills in (`P9_GETATTR_BASIC`).
const GETATTR_BASIC: u64 = 0x7ff;

/// `Tsetattr`'s fields (`P9_ATTR_…`).
mod attr {
    pub const MODE: u32 = 1;
    pub const SIZE: u32 = 8;
    pub const ATIME: u32 = 16;
    pub const MTIME: u32 = 32;
    pub const ATIME_SET: u32 = 128;
    pub const MTIME_SET: u32 = 256;
}

/// The open flags of 9P2000.L (`P9_DOTL_…`), which are x86's whatever the
/// machine: the client translates its own.
mod open {
    pub const ACCESS: u32 = 3;
    pub const WRONLY: u32 = 1;
    pub const RDWR: u32 = 2;
    pub const EXCL: u32 = 0o200;
    pub const TRUNC: u32 = 0o1000;
    pub const APPEND: u32 = 0o2000;
    pub const DSYNC: u32 = 0o10000;
    pub const SYNC: u32 = 0o4000000;
}

/// `Tlock`'s types and statuses.
mod lock {
    pub const RDLCK: u8 = 0;
    pub const WRLCK: u8 = 1;
    pub const UNLCK: u8 = 2;
    pub const SUCCESS: u8 = 0;
    pub const BLOCKED: u8 = 1;
    pub const ERROR: u8 = 2;
}

/// `Tunlinkat`'s flag for a directory (`AT_REMOVEDIR`).
const REMOVEDIR: u32 = 0x200;

/// What the VM's `statfs` says the filesystem is (`V9FS_MAGIC`).
const V9FS_MAGIC: u32 = 0x0102_1997;

/// Serves the directory `dir` on the unix socket `socket`, each connection
/// in a thread of its own, until the host stops it.
pub fn serve(dir: &Path, socket: &Path) -> Result<()> {
    raise_open_files();
    let root: OwnedFd = File::options()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY)
        .open(dir)
        .with_context(|| format!("opening {}", dir.display()))?
        .into();
    // What a server that stopped left.
    let _ = std::fs::remove_file(socket);
    let listener =
        UnixListener::bind(socket).with_context(|| format!("listening on {}", socket.display()))?;
    tracing::info!("serving {} on {}", dir.display(), socket.display());
    let serving = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // A failure here costs a connection, never the server: the VM's mount
    // would fail with it.
    let pause = || std::thread::sleep(std::time::Duration::from_millis(100));
    for conn in listener.incoming() {
        let conn = match conn {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!("taking a connection: {e}");
                pause();
                continue;
            }
        };
        if serving.load(std::sync::atomic::Ordering::Relaxed) >= MAX_CONNECTIONS {
            tracing::warn!("refused a connection: {MAX_CONNECTIONS} are served already");
            continue;
        }
        let root = match root.try_clone() {
            Ok(root) => root,
            Err(e) => {
                tracing::warn!("refused a connection: {e}");
                pause();
                continue;
            }
        };
        let slot = Slot::take(&serving);
        let spawned = std::thread::Builder::new()
            .name("share".into())
            .spawn(move || {
                // Given back however the session ends, a panic too.
                let _slot = slot;
                let mut session = Session::new(root);
                if let Err(e) = session.run(conn) {
                    tracing::debug!("a connection ended: {e}");
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("refused a connection: {e}");
            pause();
        }
    }
    Ok(())
}

/// One of the connections [`serve`] serves at once, given back as it goes.
struct Slot(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Slot {
    fn take(serving: &std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Self {
        serving.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(serving.clone())
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The client may hold a fid for each file it has in use, each an open
/// file here: as many as the system lets this process have.
fn raise_open_files() {
    use nix::sys::resource::{Resource, getrlimit, setrlimit};
    if let Ok((_, hard)) = getrlimit(Resource::RLIMIT_NOFILE) {
        let _ = setrlimit(Resource::RLIMIT_NOFILE, hard, hard);
    }
}

/// What a fid refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Dir,
    File,
    Symlink,
    /// A device, a FIFO or a socket, which the server never opens.
    Other,
}

impl Kind {
    fn of(st: &FileStat) -> Self {
        match SFlag::from_bits_truncate(st.st_mode & SFlag::S_IFMT.bits()) {
            SFlag::S_IFDIR => Self::Dir,
            SFlag::S_IFREG => Self::File,
            SFlag::S_IFLNK => Self::Symlink,
            _ => Self::Other,
        }
    }
}

/// A file the client holds: `fd` refers to it (opened with `O_PATH`, or
/// for reading and writing), `open` once the client opened it.
struct Fid {
    fd: OwnedFd,
    kind: Kind,
    open: Option<File>,
    /// The file opened for its locks, which `fcntl` takes only on a file
    /// open for writing (write locks) or reading (read locks), whatever
    /// the client opened it for.
    locks: Option<File>,
    /// A directory's entries, read when the client reads it from the start.
    entries: Vec<(Vec<u8>, Qid, u8)>,
    /// An extended attribute's: it has none, so it reads as nothing.
    xattr: bool,
}

impl Fid {
    fn new(fd: OwnedFd, kind: Kind) -> Self {
        Self {
            fd,
            kind,
            open: None,
            locks: None,
            entries: Vec::new(),
            xattr: false,
        }
    }
}

/// One client's connection: its fids, and what it agreed to.
pub struct Session {
    root: OwnedFd,
    fids: HashMap<u32, Fid>,
    msize: u32,
    /// Who the files belong to in the VM: the uid and gid the mount names.
    owner: (u32, u32),
}

fn errno(e: Errno) -> io::Error {
    io::Error::from_raw_os_error(e as i32)
}

fn fail<T>(e: i32) -> io::Result<T> {
    Err(io::Error::from_raw_os_error(e))
}

/// A name in a directory, as the client sends it: one component, never
/// `.` or `..`.
fn name(n: &[u8]) -> io::Result<CString> {
    if n.is_empty() || n == b"." || n == b".." || n.contains(&b'/') || n.contains(&0) {
        return fail(libc::EINVAL);
    }
    if n.len() > 255 {
        return fail(libc::ENAMETOOLONG);
    }
    CString::new(n).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
}

fn stat_of(fd: impl AsFd) -> io::Result<FileStat> {
    fstatat(
        fd,
        c"",
        nix::fcntl::AtFlags::AT_EMPTY_PATH | nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW,
    )
    .map_err(errno)
}

fn stat_at(dir: impl AsFd, n: &CString) -> io::Result<FileStat> {
    fstatat(dir, n.as_c_str(), nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW).map_err(errno)
}

fn qid(st: &FileStat) -> Qid {
    let kind = match Kind::of(st) {
        Kind::Dir => wire::QTDIR,
        Kind::Symlink => wire::QTSYMLINK,
        Kind::File | Kind::Other => wire::QTFILE,
    };
    Qid {
        kind,
        // Changes when the file does, for the client's caches.
        version: (st.st_mtime as u32) ^ (st.st_mtime_nsec as u32),
        path: st.st_ino,
    }
}

/// The path that opens `fd`'s file again, with other flags: the file
/// itself, wherever it has moved since.
fn reopen_path(fd: &OwnedFd) -> String {
    format!("/proc/self/fd/{}", fd.as_raw_fd())
}

/// Never more than this: the mode a client may give a file, without the
/// setuid and setgid bits.
const MODE_BITS: u32 = 0o1777;

impl Session {
    pub fn new(root: OwnedFd) -> Self {
        Self {
            root,
            fids: HashMap::new(),
            msize: 8192,
            owner: (0, 0),
        }
    }

    /// Answers the client's requests, one after the other, until it goes.
    pub fn run(&mut self, conn: UnixStream) -> io::Result<()> {
        let mut reader = BufReader::with_capacity(256 * 1024, conn.try_clone()?);
        let mut writer = conn;
        let mut message = Vec::new();
        loop {
            let mut size = [0u8; 4];
            match reader.read_exact(&mut size) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(e) => return Err(e),
            }
            let size = u32::from_le_bytes(size);
            if !(wire::HEADER as u32..=MAX_MSIZE + 64).contains(&size) {
                return Err(io::Error::other(format!("a message of {size} bytes")));
            }
            message.resize(size as usize - 4, 0);
            reader.read_exact(&mut message)?;
            let reply = self.answer(&message);
            writer.write_all(&reply)?;
        }
    }

    /// The reply to one message (its type, tag and fields).
    pub fn answer(&mut self, message: &[u8]) -> Vec<u8> {
        let mut r = Reader::new(message);
        let (Ok(kind), Ok(tag)) = (r.u8(), r.u16()) else {
            return wire::error(wire::NOTAG, libc::EPROTO);
        };
        match self.handle(kind, tag, &mut r) {
            Ok(reply) => reply,
            Err(e) => wire::error(tag, e.raw_os_error().unwrap_or(libc::EIO)),
        }
    }

    fn fid(&self, fid: u32) -> io::Result<&Fid> {
        self.fids
            .get(&fid)
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EBADF))
    }

    fn fid_mut(&mut self, fid: u32) -> io::Result<&mut Fid> {
        self.fids
            .get_mut(&fid)
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EBADF))
    }

    /// A directory the client holds, to make or find names in.
    fn dir(&self, fid: u32) -> io::Result<&OwnedFd> {
        let f = self.fid(fid)?;
        if f.kind != Kind::Dir {
            return fail(libc::ENOTDIR);
        }
        Ok(&f.fd)
    }

    /// The file `fid`'s locks are taken on: the file the client opened,
    /// opened again for reading and writing, where the host's user may, so
    /// that a write lock on a file the client opened to read holds, as
    /// `flock` on one does.
    fn locks(&mut self, fid: u32) -> io::Result<&File> {
        let f = self.fid_mut(fid)?;
        if f.kind != Kind::File {
            return fail(libc::EBADF);
        }
        if f.locks.is_none() {
            let Some(open) = &f.open else {
                return fail(libc::EBADF);
            };
            let locks = File::options()
                .read(true)
                .write(true)
                .open(reopen_path(&f.fd))
                .or_else(|_| open.try_clone())?;
            f.locks = Some(locks);
        }
        f.locks
            .as_ref()
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EBADF))
    }

    fn insert(&mut self, fid: u32, f: Fid) -> io::Result<()> {
        if self.fids.len() >= MAX_FIDS && !self.fids.contains_key(&fid) {
            return fail(libc::EMFILE);
        }
        self.fids.insert(fid, f);
        Ok(())
    }

    fn handle(&mut self, t: u8, tag: u16, r: &mut Reader) -> io::Result<Vec<u8>> {
        let reply = |t: u8| Writer::new(t + 1, tag);
        match t {
            kind::TVERSION => {
                let msize = r.u32()?;
                let version = r.bytes()?;
                self.fids.clear();
                self.msize = msize.clamp(4096, MAX_MSIZE);
                let mut w = reply(t);
                w.u32(self.msize);
                w.bytes(if version.starts_with(VERSION) {
                    VERSION
                } else {
                    b"unknown"
                });
                Ok(w.finish())
            }
            kind::TATTACH => {
                let fid = r.u32()?;
                // Nothing to authenticate: the VM reaches no other server.
                if r.u32()? != wire::NOFID {
                    return fail(libc::EOPNOTSUPP);
                }
                let _uname = r.bytes()?;
                let aname = r.bytes()?;
                let n_uname = r.u32()?;
                if self.fids.contains_key(&fid) {
                    return fail(libc::EBADF);
                }
                self.owner = owner(aname, n_uname);
                let root = self.root.try_clone()?;
                let st = stat_of(&root)?;
                self.insert(fid, Fid::new(root, Kind::Dir))?;
                let mut w = reply(t);
                w.qid(qid(&st));
                Ok(w.finish())
            }
            kind::TWALK => self.walk(tag, r),
            kind::TLOPEN => {
                let fid = r.u32()?;
                let flags = r.u32()?;
                let f = self.fid_mut(fid)?;
                if f.open.is_some() {
                    return fail(libc::EBADF);
                }
                let file = match f.kind {
                    Kind::Dir => File::from(
                        openat(
                            &f.fd,
                            c".",
                            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
                            Mode::empty(),
                        )
                        .map_err(errno)?,
                    ),
                    Kind::File => open_options(flags).open(reopen_path(&f.fd))?,
                    Kind::Symlink => return fail(libc::ELOOP),
                    Kind::Other => return fail(libc::EPERM),
                };
                let st = stat_of(&file)?;
                f.open = Some(file);
                let mut w = reply(t);
                w.qid(qid(&st)).u32(0);
                Ok(w.finish())
            }
            kind::TLCREATE => {
                let fid = r.u32()?;
                let n = name(r.bytes()?)?;
                let flags = r.u32()?;
                let mode = r.u32()?;
                let _gid = r.u32()?;
                let dir = self.dir(fid)?;
                if self.fid(fid)?.open.is_some() {
                    return fail(libc::EBADF);
                }
                let mut oflags = open_flags(flags) | OFlag::O_CREAT | OFlag::O_NOFOLLOW;
                if flags & open::EXCL != 0 {
                    oflags |= OFlag::O_EXCL;
                }
                let fd = openat(dir, n.as_c_str(), oflags, file_mode(mode)).map_err(errno)?;
                let file = File::from(fd);
                let st = stat_of(&file)?;
                let path = OwnedFd::from(file.try_clone()?);
                let f = self.fid_mut(fid)?;
                *f = Fid::new(path, Kind::File);
                f.open = Some(file);
                let mut w = reply(t);
                w.qid(qid(&st)).u32(0);
                Ok(w.finish())
            }
            kind::TSYMLINK => {
                let fid = r.u32()?;
                let n = name(r.bytes()?)?;
                let target = r.bytes()?;
                let _gid = r.u32()?;
                if target.is_empty() || target.len() > 4095 || target.contains(&0) {
                    return fail(libc::EINVAL);
                }
                let dir = self.dir(fid)?;
                symlinkat(OsStr::from_bytes(target), dir, n.as_c_str()).map_err(errno)?;
                let st = stat_at(dir, &n)?;
                let mut w = reply(t);
                w.qid(qid(&st));
                Ok(w.finish())
            }
            kind::TMKDIR => {
                let fid = r.u32()?;
                let n = name(r.bytes()?)?;
                let mode = r.u32()?;
                let _gid = r.u32()?;
                let dir = self.dir(fid)?;
                mkdirat(dir, n.as_c_str(), file_mode(mode)).map_err(errno)?;
                let st = stat_at(dir, &n)?;
                let mut w = reply(t);
                w.qid(qid(&st));
                Ok(w.finish())
            }
            kind::TREADLINK => {
                let f = self.fid(r.u32()?)?;
                if f.kind != Kind::Symlink {
                    return fail(libc::EINVAL);
                }
                let target = nix::fcntl::readlinkat(&f.fd, c"").map_err(errno)?;
                let mut w = reply(t);
                w.bytes(target.as_bytes());
                Ok(w.finish())
            }
            kind::TGETATTR => {
                let f = self.fid(r.u32()?)?;
                let _mask = r.u64()?;
                let st = stat_of(&f.fd)?;
                // `st_nlink` is a u32 on aarch64 and a u64 on x86_64.
                #[allow(clippy::useless_conversion)]
                let nlink = u64::from(st.st_nlink);
                let mut w = reply(t);
                w.u64(GETATTR_BASIC)
                    .qid(qid(&st))
                    .u32(st.st_mode)
                    .u32(self.owner.0)
                    .u32(self.owner.1)
                    .u64(nlink)
                    .u64(st.st_rdev)
                    .u64(u64::try_from(st.st_size).unwrap_or(0))
                    .u64(u64::try_from(st.st_blksize).unwrap_or(4096))
                    .u64(u64::try_from(st.st_blocks).unwrap_or(0));
                for (sec, nsec) in [
                    (st.st_atime, st.st_atime_nsec),
                    (st.st_mtime, st.st_mtime_nsec),
                    (st.st_ctime, st.st_ctime_nsec),
                    (0, 0),
                ] {
                    w.u64(sec as u64).u64(nsec as u64);
                }
                // gen and data_version.
                w.u64(0).u64(0);
                Ok(w.finish())
            }
            kind::TSETATTR => self.setattr(tag, r),
            kind::TXATTRWALK => {
                let fid = r.u32()?;
                let newfid = r.u32()?;
                let n = r.bytes()?;
                // No file has any: a list of them is empty, and each is missing.
                if !n.is_empty() {
                    return fail(libc::ENODATA);
                }
                let fd = self.fid(fid)?.fd.try_clone()?;
                let mut f = Fid::new(fd, Kind::Other);
                f.xattr = true;
                self.insert(newfid, f)?;
                let mut w = reply(t);
                w.u64(0);
                Ok(w.finish())
            }
            kind::TREADDIR => self.readdir(tag, r),
            kind::TFSYNC => {
                let f = self.fid(r.u32()?)?;
                let datasync = r.u32()? != 0;
                if let Some(file) = &f.open {
                    if datasync {
                        file.sync_data()?;
                    } else {
                        file.sync_all()?;
                    }
                }
                Ok(reply(t).finish())
            }
            kind::TLOCK => {
                let fid = r.u32()?;
                let kind = r.u8()?;
                let _flags = r.u32()?;
                let start = r.u64()?;
                let length = r.u64()?;
                let fl = flock(kind, start, length)?;
                let status = match nix::fcntl::fcntl(self.locks(fid)?, FcntlArg::F_OFD_SETLK(&fl)) {
                    Ok(_) => lock::SUCCESS,
                    Err(Errno::EAGAIN | Errno::EACCES) => lock::BLOCKED,
                    Err(_) => lock::ERROR,
                };
                let mut w = reply(t);
                w.u8(status);
                Ok(w.finish())
            }
            kind::TGETLOCK => {
                let fid = r.u32()?;
                let kind = r.u8()?;
                let start = r.u64()?;
                let length = r.u64()?;
                let proc_id = r.u32()?;
                let client = r.bytes()?;
                let mut fl = flock(kind, start, length)?;
                nix::fcntl::fcntl(self.locks(fid)?, FcntlArg::F_OFD_GETLK(&mut fl))
                    .map_err(errno)?;
                let mut w = reply(t);
                if i32::from(fl.l_type) == libc::F_UNLCK {
                    w.u8(lock::UNLCK)
                        .u64(start)
                        .u64(length)
                        .u32(proc_id)
                        .bytes(client);
                } else {
                    let kind = if i32::from(fl.l_type) == libc::F_WRLCK {
                        lock::WRLCK
                    } else {
                        lock::RDLCK
                    };
                    // Another VM's: no process here holds it.
                    w.u8(kind)
                        .u64(u64::try_from(fl.l_start).unwrap_or(0))
                        .u64(u64::try_from(fl.l_len).unwrap_or(0))
                        .u32(u32::MAX)
                        .bytes(b"");
                }
                Ok(w.finish())
            }
            kind::TLINK => {
                let dfid = r.u32()?;
                let fid = r.u32()?;
                let n = name(r.bytes()?)?;
                let f = self.fid(fid)?;
                // A link to a symlink would follow it.
                if f.kind != Kind::File {
                    return fail(libc::EPERM);
                }
                let dir = self.dir(dfid)?;
                linkat(
                    AT_FDCWD,
                    reopen_path(&f.fd).as_str(),
                    dir,
                    n.as_c_str(),
                    nix::fcntl::AtFlags::AT_SYMLINK_FOLLOW,
                )
                .map_err(errno)?;
                Ok(reply(t).finish())
            }
            kind::TRENAMEAT => {
                let old_dir = r.u32()?;
                let old = name(r.bytes()?)?;
                let new_dir = r.u32()?;
                let new = name(r.bytes()?)?;
                nix::fcntl::renameat(
                    self.dir(old_dir)?,
                    old.as_c_str(),
                    self.dir(new_dir)?,
                    new.as_c_str(),
                )
                .map_err(errno)?;
                Ok(reply(t).finish())
            }
            kind::TUNLINKAT => {
                let dir = r.u32()?;
                let n = name(r.bytes()?)?;
                let flags = r.u32()?;
                let how = if flags & REMOVEDIR != 0 {
                    UnlinkatFlags::RemoveDir
                } else {
                    UnlinkatFlags::NoRemoveDir
                };
                unlinkat(self.dir(dir)?, n.as_c_str(), how).map_err(errno)?;
                Ok(reply(t).finish())
            }
            kind::TFLUSH => {
                // Requests are answered in order: the one to flush is done.
                let _oldtag = r.u16()?;
                Ok(reply(t).finish())
            }
            kind::TREAD => {
                let f = self.fid(r.u32()?)?;
                let offset = r.u64()?;
                let count = r.u32()?.min(self.msize - wire::READ_OVERHEAD);
                let mut w = reply(t);
                if f.xattr {
                    w.data(&[]);
                    return Ok(w.finish());
                }
                let file = match (&f.open, f.kind) {
                    (Some(file), Kind::File) => file,
                    (Some(_), _) => return fail(libc::EISDIR),
                    (None, _) => return fail(libc::EBADF),
                };
                let mut buf = vec![0u8; count as usize];
                let n = file.read_at(&mut buf, offset)?;
                w.data(&buf[..n]);
                Ok(w.finish())
            }
            kind::TWRITE => {
                let f = self.fid(r.u32()?)?;
                let offset = r.u64()?;
                let data = r.data()?;
                let file = match (&f.open, f.kind) {
                    (Some(file), Kind::File) => file,
                    (Some(_), _) => return fail(libc::EISDIR),
                    (None, _) => return fail(libc::EBADF),
                };
                let n = file.write_at(data, offset)?;
                let mut w = reply(t);
                w.u32(u32::try_from(n).unwrap_or(0));
                Ok(w.finish())
            }
            kind::TCLUNK => {
                let fid = r.u32()?;
                self.fids
                    .remove(&fid)
                    .ok_or_else(|| io::Error::from_raw_os_error(libc::EBADF))?;
                Ok(reply(t).finish())
            }
            kind::TREMOVE => {
                // The Linux client removes names with Tunlinkat; this one
                // only lets the fid go, as a remove that fails must.
                self.fids.remove(&r.u32()?);
                fail(libc::EOPNOTSUPP)
            }
            kind::TSTATFS => {
                let f = self.fid(r.u32()?)?;
                let st = nix::sys::statfs::fstatfs(&f.fd).map_err(errno)?;
                let mut w = reply(t);
                w.u32(V9FS_MAGIC)
                    .u32(u32::try_from(st.block_size()).unwrap_or(4096))
                    .u64(st.blocks())
                    .u64(st.blocks_free())
                    .u64(st.blocks_available())
                    .u64(st.files())
                    .u64(st.files_free())
                    .u64(0)
                    .u32(u32::try_from(st.maximum_name_length()).unwrap_or(255));
                Ok(w.finish())
            }
            // No device files, and no attributes to set; the old 9P2000's
            // requests, authentication and renames by fid, which the Linux
            // client asks only of servers that lack the 9P2000.L ones.
            kind::TMKNOD => fail(libc::EPERM),
            kind::TXATTRCREATE | kind::TAUTH | kind::TRENAME => fail(libc::EOPNOTSUPP),
            _ => fail(libc::EOPNOTSUPP),
        }
    }

    fn walk(&mut self, tag: u16, r: &mut Reader) -> io::Result<Vec<u8>> {
        let fid = r.u32()?;
        let newfid = r.u32()?;
        let count = r.u16()?;
        if count > 16 {
            return fail(libc::EINVAL);
        }
        if fid != newfid && self.fids.contains_key(&newfid) {
            return fail(libc::EBADF);
        }
        let start = self.fid(fid)?;
        let mut current = start.fd.try_clone()?;
        let mut kind_now = start.kind;
        let mut qids = Vec::new();
        for i in 0..count {
            let step = r.bytes().and_then(name).and_then(|n| {
                let fd = openat(
                    &current,
                    n.as_c_str(),
                    OFlag::O_PATH | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                    Mode::empty(),
                )
                .map_err(errno)?;
                let st = stat_of(&fd)?;
                Ok((fd, st))
            });
            match step {
                Ok((fd, st)) => {
                    current = fd;
                    kind_now = Kind::of(&st);
                    qids.push(qid(&st));
                }
                // The first name must be there; past it, the reply says how
                // far the walk went.
                Err(e) if i == 0 => return Err(e),
                Err(_) => break,
            }
        }
        if qids.len() == usize::from(count) {
            self.insert(newfid, Fid::new(current, kind_now))?;
        }
        let mut w = Writer::new(kind::TWALK + 1, tag);
        w.u16(u16::try_from(qids.len()).unwrap_or(0));
        for q in qids {
            w.qid(q);
        }
        Ok(w.finish())
    }

    fn setattr(&mut self, tag: u16, r: &mut Reader) -> io::Result<Vec<u8>> {
        let fid = r.u32()?;
        let valid = r.u32()?;
        let mode = r.u32()?;
        let _uid = r.u32()?;
        let _gid = r.u32()?;
        let size = r.u64()?;
        let atime = (r.u64()?, r.u64()?);
        let mtime = (r.u64()?, r.u64()?);
        let f = self.fid(fid)?;
        // The client changes no symlink: chmod and truncate follow them.
        if f.kind == Kind::Symlink {
            return Ok(Writer::new(kind::TSETATTR + 1, tag).finish());
        }
        let path = reopen_path(&f.fd);
        if valid & attr::MODE != 0 {
            fchmodat(
                AT_FDCWD,
                path.as_str(),
                Mode::from_bits_truncate(mode & MODE_BITS),
                FchmodatFlags::FollowSymlink,
            )
            .map_err(errno)?;
        }
        if valid & attr::SIZE != 0 {
            match (&f.open, f.kind) {
                (_, Kind::Dir) => return fail(libc::EISDIR),
                (_, Kind::Other) => return fail(libc::EPERM),
                (Some(file), _) if file.set_len(size).is_ok() => {}
                _ => File::options().write(true).open(&path)?.set_len(size)?,
            }
        }
        if valid & (attr::ATIME | attr::MTIME) != 0 {
            let time = |set: u32, given: u32, (sec, nsec): (u64, u64)| {
                if valid & set == 0 {
                    TimeSpec::UTIME_OMIT
                } else if valid & given == 0 {
                    TimeSpec::UTIME_NOW
                } else {
                    TimeSpec::new(sec as i64, nsec as i64)
                }
            };
            utimensat(
                AT_FDCWD,
                path.as_str(),
                &time(attr::ATIME, attr::ATIME_SET, atime),
                &time(attr::MTIME, attr::MTIME_SET, mtime),
                UtimensatFlags::FollowSymlink,
            )
            .map_err(errno)?;
        }
        // Owners stay the host's user's, and ctime the kernel's.
        Ok(Writer::new(kind::TSETATTR + 1, tag).finish())
    }

    /// A directory's entries from `offset`, the index after the last one
    /// the client has: the directory is read anew when it starts over.
    fn readdir(&mut self, tag: u16, r: &mut Reader) -> io::Result<Vec<u8>> {
        let fid = r.u32()?;
        let offset = r.u64()?;
        let count = r.u32()?.min(self.msize - wire::READ_OVERHEAD) as usize;
        let f = self.fid_mut(fid)?;
        let Some(dir) = &f.open else {
            return fail(libc::EBADF);
        };
        if f.kind != Kind::Dir {
            return fail(libc::ENOTDIR);
        }
        if offset == 0 || f.entries.is_empty() {
            let here = qid(&stat_of(dir)?);
            let mut entries = vec![
                (b".".to_vec(), here, libc::DT_DIR),
                (b"..".to_vec(), here, libc::DT_DIR),
            ];
            let read = std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))?;
            for entry in read.flatten() {
                let (kind, dt) = match entry.file_type() {
                    Ok(t) if t.is_dir() => (wire::QTDIR, libc::DT_DIR),
                    Ok(t) if t.is_symlink() => (wire::QTSYMLINK, libc::DT_LNK),
                    Ok(t) if t.is_file() => (wire::QTFILE, libc::DT_REG),
                    _ => (wire::QTFILE, libc::DT_UNKNOWN),
                };
                let q = Qid {
                    kind,
                    version: 0,
                    path: entry.ino(),
                };
                entries.push((entry.file_name().as_bytes().to_vec(), q, dt));
            }
            f.entries = entries;
        }
        let mut out = Vec::new();
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        for (i, (n, q, dt)) in f.entries.iter().enumerate().skip(start) {
            if out.len() + wire::dirent_len(n) > count {
                if out.is_empty() {
                    return fail(libc::EINVAL);
                }
                break;
            }
            wire::dirent(&mut out, *q, i as u64 + 1, *dt, n);
        }
        let mut w = Writer::new(kind::TREADDIR + 1, tag);
        w.u32(u32::try_from(out.len()).unwrap_or(0)).raw(&out);
        Ok(w.finish())
    }
}

/// Who the files belong to in the VM: `aname`'s `uid:gid`, which
/// `fctl-agent vm-mount` puts there, else the attaching uid.
fn owner(aname: &[u8], n_uname: u32) -> (u32, u32) {
    let parsed = std::str::from_utf8(aname).ok().and_then(|a| {
        let (u, g) = a.split_once(':')?;
        Some((u.parse().ok()?, g.parse().ok()?))
    });
    let uid = if n_uname == u32::MAX { 0 } else { n_uname };
    parsed.unwrap_or((uid, uid))
}

/// A file's mode, as the client may give it.
fn file_mode(mode: u32) -> Mode {
    Mode::from_bits_truncate(mode & MODE_BITS)
}

/// How `Tlopen`'s and `Tlcreate`'s flags open a file here.
fn open_flags(flags: u32) -> OFlag {
    let mut o = match flags & open::ACCESS {
        open::WRONLY => OFlag::O_WRONLY,
        open::RDWR => OFlag::O_RDWR,
        _ => OFlag::O_RDONLY,
    };
    for (p9, here) in [
        (open::TRUNC, OFlag::O_TRUNC),
        (open::APPEND, OFlag::O_APPEND),
        (open::DSYNC, OFlag::O_DSYNC),
        (open::SYNC, OFlag::O_SYNC),
    ] {
        if flags & p9 != 0 {
            o |= here;
        }
    }
    o | OFlag::O_CLOEXEC
}

/// [`open_flags`], to open a file again through `/proc/self/fd`.
fn open_options(flags: u32) -> std::fs::OpenOptions {
    let o = open_flags(flags);
    let mut options = File::options();
    match flags & open::ACCESS {
        open::WRONLY => options.write(true),
        open::RDWR => options.read(true).write(true),
        _ => options.read(true),
    };
    options
        .truncate(o.contains(OFlag::O_TRUNC))
        .append(o.contains(OFlag::O_APPEND))
        .custom_flags((o & (OFlag::O_DSYNC | OFlag::O_SYNC)).bits());
    options
}

/// A lock as `fcntl` takes it, from `Tlock`'s or `Tgetlock`'s.
fn flock(kind: u8, start: u64, length: u64) -> io::Result<libc::flock> {
    let l_type = match kind {
        lock::RDLCK => libc::F_RDLCK,
        lock::WRLCK => libc::F_WRLCK,
        lock::UNLCK => libc::F_UNLCK,
        _ => return fail(libc::EINVAL),
    };
    Ok(libc::flock {
        l_type: l_type as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: i64::try_from(start).unwrap_or(i64::MAX),
        l_len: i64::try_from(length).unwrap_or(0),
        l_pid: 0,
    })
}
