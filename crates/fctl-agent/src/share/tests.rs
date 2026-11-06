use super::*;
use std::os::unix::fs::PermissionsExt;

/// A 9P client that speaks to a [`Session`] directly, as the VM's kernel
/// would over its connection.
struct Client {
    session: Session,
    tag: u16,
}

type Reply = std::result::Result<Vec<u8>, i32>;

fn root_of(dir: &Path) -> io::Result<OwnedFd> {
    Ok(File::options()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY)
        .open(dir)?
        .into())
}

/// `P9_DOTL_RDWR`, `P9_DOTL_RDONLY` and `P9_DOTL_DIRECTORY`.
const RDWR: u32 = 2;
const RDONLY: u32 = 0;
const DIRECTORY: u32 = 0o200000;

impl Client {
    /// A client that agreed on the protocol and attached as the VM's user
    /// 1000:1000, with the shared directory as fid 0.
    fn new(dir: &Path) -> io::Result<Self> {
        let mut c = Self {
            session: Session::new(root_of(dir)?),
            tag: 1,
        };
        let v = c.call(kind::TVERSION, |w| {
            w.u32(512 * 1024).bytes(b"9P2000.L");
        });
        assert_eq!(v.map(|b| b[4..].to_vec()), Ok(b"\x08\x009P2000.L".to_vec()));
        let attached = c.call(kind::TATTACH, |w| {
            w.u32(0)
                .u32(wire::NOFID)
                .bytes(b"")
                .bytes(b"1000:1000")
                .u32(1000);
        });
        assert!(attached.is_ok());
        Ok(c)
    }

    /// Sends a request and returns its reply's fields, or its errno.
    fn call(&mut self, t: u8, build: impl FnOnce(&mut Writer)) -> Reply {
        self.tag = self.tag.wrapping_add(1);
        let mut w = Writer::new(t, self.tag);
        build(&mut w);
        let message = w.finish();
        let reply = self.session.answer(&message[4..]);
        let size = u32::from_le_bytes([reply[0], reply[1], reply[2], reply[3]]);
        assert_eq!(size as usize, reply.len());
        assert_eq!(u16::from_le_bytes([reply[5], reply[6]]), self.tag);
        if reply[4] == kind::RLERROR {
            return Err(i32::from_le_bytes([
                reply[7], reply[8], reply[9], reply[10],
            ]));
        }
        assert_eq!(reply[4], t + 1);
        Ok(reply[7..].to_vec())
    }

    /// How many names the walk took.
    fn walk(&mut self, fid: u32, newfid: u32, names: &[&[u8]]) -> std::result::Result<usize, i32> {
        let body = self.call(kind::TWALK, |w| {
            w.u32(fid)
                .u32(newfid)
                .u16(u16::try_from(names.len()).unwrap_or(0));
            for n in names {
                w.bytes(n);
            }
        })?;
        Ok(usize::from(u16::from_le_bytes([body[0], body[1]])))
    }

    fn lopen(&mut self, fid: u32, flags: u32) -> Reply {
        self.call(kind::TLOPEN, |w| {
            w.u32(fid).u32(flags);
        })
    }

    fn lcreate(&mut self, fid: u32, name: &[u8], mode: u32) -> Reply {
        self.call(kind::TLCREATE, |w| {
            w.u32(fid).bytes(name).u32(RDWR).u32(mode).u32(0);
        })
    }

    fn mkdir(&mut self, fid: u32, name: &[u8], mode: u32) -> Reply {
        self.call(kind::TMKDIR, |w| {
            w.u32(fid).bytes(name).u32(mode).u32(0);
        })
    }

    fn write(&mut self, fid: u32, offset: u64, data: &[u8]) -> Reply {
        self.call(kind::TWRITE, |w| {
            w.u32(fid).u64(offset).data(data);
        })
    }

    fn read(&mut self, fid: u32, offset: u64, count: u32) -> std::result::Result<Vec<u8>, i32> {
        let body = self.call(kind::TREAD, |w| {
            w.u32(fid).u64(offset).u32(count);
        })?;
        Ok(body[4..].to_vec())
    }

    /// Mode, uid, gid and size.
    fn getattr(&mut self, fid: u32) -> std::result::Result<(u32, u32, u32, u64), i32> {
        let body = self.call(kind::TGETATTR, |w| {
            w.u32(fid).u64(GETATTR_BASIC);
        })?;
        let mut r = Reader::new(&body[8 + 13..]);
        let mode = r.u32().map_err(|_| libc::EPROTO)?;
        let uid = r.u32().map_err(|_| libc::EPROTO)?;
        let gid = r.u32().map_err(|_| libc::EPROTO)?;
        let _nlink = r.u64().map_err(|_| libc::EPROTO)?;
        let _rdev = r.u64().map_err(|_| libc::EPROTO)?;
        let size = r.u64().map_err(|_| libc::EPROTO)?;
        Ok((mode, uid, gid, size))
    }

    fn setattr(&mut self, fid: u32, valid: u32, mode: u32, size: u64) -> Reply {
        self.call(kind::TSETATTR, |w| {
            w.u32(fid).u32(valid).u32(mode).u32(0).u32(0).u64(size);
            w.u64(0).u64(0).u64(0).u64(0);
        })
    }

    /// The names and next offsets of a directory's entries from `offset`.
    fn readdir(
        &mut self,
        fid: u32,
        offset: u64,
        count: u32,
    ) -> std::result::Result<Vec<(Vec<u8>, u64)>, i32> {
        let body = self.call(kind::TREADDIR, |w| {
            w.u32(fid).u64(offset).u32(count);
        })?;
        let mut r = Reader::new(&body[4..]);
        let mut out = Vec::new();
        while let Ok(_qid_kind) = r.u8() {
            let parsed = (|| -> io::Result<(Vec<u8>, u64)> {
                r.u32()?;
                r.u64()?;
                let next = r.u64()?;
                r.u8()?;
                Ok((r.bytes()?.to_vec(), next))
            })();
            out.push(parsed.map_err(|_| libc::EPROTO)?);
        }
        Ok(out)
    }

    fn clunk(&mut self, fid: u32) -> Reply {
        self.call(kind::TCLUNK, |w| {
            w.u32(fid);
        })
    }

    fn lock(&mut self, fid: u32, kind: u8) -> std::result::Result<u8, i32> {
        let body = self.call(kind::TLOCK, |w| {
            w.u32(fid).u8(kind).u32(0).u64(0).u64(0).u32(1).bytes(b"vm");
        })?;
        Ok(body[0])
    }
}

#[test]
fn serves_its_directory_as_the_vms_users_files() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut c = Client::new(dir.path())?;
    assert!(c.mkdir(0, b"npm", 0o755).is_ok());
    assert_eq!(c.walk(0, 1, &[b"npm"]), Ok(1));
    // Setuid and setgid bits are left out.
    assert!(c.lcreate(1, b"a.tgz", 0o6755).is_ok());
    assert!(c.write(1, 0, b"hello").is_ok());
    assert_eq!(c.read(1, 0, 100), Ok(b"hello".to_vec()));
    assert_eq!(c.read(1, 3, 100), Ok(b"lo".to_vec()));
    let (mode, uid, gid, size) = c.getattr(1).map_err(io::Error::from_raw_os_error)?;
    assert_eq!((mode & 0o7777, uid, gid, size), (0o755, 1000, 1000, 5));
    assert_eq!(mode & libc::S_IFMT, libc::S_IFREG);
    assert!(c.clunk(1).is_ok());
    // On the host: the host's user's file, without the bits.
    let host = dir.path().join("npm/a.tgz");
    assert_eq!(std::fs::read(&host)?, b"hello");
    assert_eq!(
        std::fs::metadata(&host)?.permissions().mode() & 0o7777,
        0o755
    );
    // Opened again, truncated, and read from the start.
    assert_eq!(c.walk(0, 2, &[b"npm", b"a.tgz"]), Ok(2));
    assert!(c.lopen(2, RDWR).is_ok());
    assert!(c.setattr(2, attr::SIZE | attr::MODE, 0o4600, 2).is_ok());
    assert_eq!(c.read(2, 0, 100), Ok(b"he".to_vec()));
    assert_eq!(
        std::fs::metadata(&host)?.permissions().mode() & 0o7777,
        0o600
    );
    // Files open once; a directory reads only through readdir.
    assert_eq!(c.lopen(2, RDONLY), Err(libc::EBADF));
    assert_eq!(c.walk(0, 3, &[b"npm"]), Ok(1));
    assert!(c.lopen(3, RDONLY | DIRECTORY).is_ok());
    assert_eq!(c.read(3, 0, 10), Err(libc::EISDIR));
    Ok(())
}

#[test]
fn names_never_lead_out_of_the_directory() -> anyhow::Result<()> {
    let top = tempfile::tempdir()?;
    let dir = top.path().join("caches");
    std::fs::create_dir(&dir)?;
    std::fs::write(top.path().join("secret"), b"the runner's token")?;
    // A link the host's user left, and one the VM makes: both point out.
    std::os::unix::fs::symlink(top.path(), dir.join("up"))?;
    let mut c = Client::new(&dir)?;
    assert!(
        c.call(kind::TSYMLINK, |w| {
            w.u32(0).bytes(b"etc").bytes(b"/etc").u32(0);
        })
        .is_ok()
    );
    for bad in [&b".."[..], b".", b"up/secret", b"", b"a\0b"] {
        assert_eq!(c.walk(0, 1, &[bad]), Err(libc::EINVAL), "{bad:?}");
        assert_eq!(c.lcreate(0, bad, 0o644), Err(libc::EINVAL), "{bad:?}");
        assert_eq!(c.mkdir(0, bad, 0o755), Err(libc::EINVAL), "{bad:?}");
    }
    // A symlink is a file of its own: never followed, opened or walked
    // through. The VM resolves it, against its own files.
    assert_eq!(c.walk(0, 1, &[b"up"]), Ok(1));
    assert_eq!(c.lopen(1, RDONLY), Err(libc::ELOOP));
    let target = c.call(kind::TREADLINK, |w| {
        w.u32(1);
    });
    assert_eq!(
        target.map(|b| b[2..].to_vec()),
        Ok(top.path().as_os_str().as_bytes().to_vec())
    );
    assert_eq!(c.walk(0, 2, &[b"up", b"secret"]), Ok(1));
    assert_eq!(c.lopen(2, RDONLY), Err(libc::EBADF));
    assert_eq!(c.walk(0, 2, &[b"etc", b"passwd"]), Ok(1));
    // Neither links to it nor devices.
    assert_eq!(
        c.call(kind::TLINK, |w| {
            w.u32(0).u32(1).bytes(b"again");
        }),
        Err(libc::EPERM)
    );
    assert_eq!(
        c.call(kind::TMKNOD, |w| {
            w.u32(0).bytes(b"null").u32(0o20666).u32(1).u32(3).u32(0);
        }),
        Err(libc::EPERM)
    );
    assert_eq!(
        c.call(kind::TRENAMEAT, |w| {
            w.u32(0).bytes(b"etc").u32(0).bytes(b"../etc");
        }),
        Err(libc::EINVAL)
    );
    assert_eq!(
        std::fs::read(top.path().join("secret"))?,
        b"the runner's token"
    );
    Ok(())
}

#[test]
fn lists_directories_a_page_at_a_time() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut want = std::collections::BTreeSet::new();
    for i in 0..60 {
        let name = format!("package-{i:03}-{}", "x".repeat(40));
        std::fs::write(dir.path().join(&name), b"")?;
        want.insert(name.into_bytes());
    }
    want.insert(b".".to_vec());
    want.insert(b"..".to_vec());
    let mut c = Client::new(dir.path())?;
    assert_eq!(c.walk(0, 1, &[]), Ok(0));
    assert!(c.lopen(1, RDONLY | DIRECTORY).is_ok());
    let mut got = std::collections::BTreeSet::new();
    let mut offset = 0;
    let mut pages = 0;
    loop {
        let page = c
            .readdir(1, offset, 512)
            .map_err(io::Error::from_raw_os_error)?;
        let Some((_, next)) = page.last() else { break };
        offset = *next;
        pages += 1;
        got.extend(page.into_iter().map(|(n, _)| n));
    }
    assert!(pages > 3, "{pages} pages");
    assert_eq!(got, want);
    // Reading from the start again sees what changed.
    std::fs::write(dir.path().join("new"), b"")?;
    let first = c
        .readdir(1, 0, 64 * 1024)
        .map_err(io::Error::from_raw_os_error)?;
    assert_eq!(first.len(), want.len() + 1);
    Ok(())
}

#[test]
fn renames_and_removes_what_the_vm_asks() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut c = Client::new(dir.path())?;
    assert!(c.mkdir(0, b"cargo", 0o755).is_ok());
    assert_eq!(c.walk(0, 1, &[b"cargo"]), Ok(1));
    assert!(c.lcreate(1, b"index.tmp", 0o644).is_ok());
    assert!(c.write(1, 0, b"v2").is_ok());
    assert_eq!(c.walk(0, 2, &[b"cargo"]), Ok(1));
    let renamed = c.call(kind::TRENAMEAT, |w| {
        w.u32(2).bytes(b"index.tmp").u32(2).bytes(b"index");
    });
    assert!(renamed.is_ok());
    // The fid follows its file.
    assert_eq!(c.read(1, 0, 10), Ok(b"v2".to_vec()));
    assert_eq!(std::fs::read(dir.path().join("cargo/index"))?, b"v2");
    // A hard link, then both names go, then the directory.
    let linked = c.call(kind::TLINK, |w| {
        w.u32(2).u32(1).bytes(b"index.bak");
    });
    assert!(linked.is_ok());
    for name in [&b"index"[..], b"index.bak"] {
        let removed = c.call(kind::TUNLINKAT, |w| {
            w.u32(2).bytes(name).u32(0);
        });
        assert!(removed.is_ok());
    }
    let removed = c.call(kind::TUNLINKAT, |w| {
        w.u32(0).bytes(b"cargo").u32(REMOVEDIR);
    });
    assert!(removed.is_ok());
    assert!(!dir.path().join("cargo").exists());
    // What the VM's statfs gets.
    let st = c.call(kind::TSTATFS, |w| {
        w.u32(0);
    });
    assert_eq!(
        st.map(|b| b[..4].to_vec()),
        Ok(V9FS_MAGIC.to_le_bytes().to_vec())
    );
    Ok(())
}

/// Two VMs' cargo runs wait for each other, as on one machine.
#[test]
fn locks_hold_between_vms() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join(".package-cache"), b"")?;
    let mut a = Client::new(dir.path())?;
    let mut b = Client::new(dir.path())?;
    for c in [&mut a, &mut b] {
        assert_eq!(c.walk(0, 1, &[b".package-cache"]), Ok(1));
        assert!(c.lopen(1, RDWR).is_ok());
    }
    assert_eq!(a.lock(1, lock::WRLCK), Ok(lock::SUCCESS));
    assert_eq!(b.lock(1, lock::WRLCK), Ok(lock::BLOCKED));
    assert_eq!(b.lock(1, lock::RDLCK), Ok(lock::BLOCKED));
    let held = b.call(kind::TGETLOCK, |w| {
        w.u32(1).u8(lock::WRLCK).u64(0).u64(0).u32(7).bytes(b"vm-b");
    });
    assert_eq!(held.map(|body| body[0]), Ok(lock::WRLCK));
    assert_eq!(a.lock(1, lock::UNLCK), Ok(lock::SUCCESS));
    assert_eq!(b.lock(1, lock::WRLCK), Ok(lock::SUCCESS));
    // A lock goes with the file that held it.
    assert!(b.clunk(1).is_ok());
    assert_eq!(a.lock(1, lock::WRLCK), Ok(lock::SUCCESS));
    Ok(())
}

#[test]
fn has_no_extended_attributes_or_old_requests() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut c = Client::new(dir.path())?;
    let listed = c.call(kind::TXATTRWALK, |w| {
        w.u32(0).u32(5).bytes(b"");
    });
    assert_eq!(listed, Ok(0u64.to_le_bytes().to_vec()));
    assert_eq!(c.read(5, 0, 100), Ok(Vec::new()));
    let missing = c.call(kind::TXATTRWALK, |w| {
        w.u32(0).u32(6).bytes(b"security.capability");
    });
    assert_eq!(missing, Err(libc::ENODATA));
    // 9P2000.u's Topen, and a version this server doesn't speak.
    assert_eq!(
        c.call(112, |w| {
            w.u32(0).u8(0);
        }),
        Err(libc::EOPNOTSUPP)
    );
    let v = c.call(kind::TVERSION, |w| {
        w.u32(8192).bytes(b"9P2000.u");
    });
    assert_eq!(v.map(|b| b[4..].to_vec()), Ok(b"\x07\x00unknown".to_vec()));
    // A message cut short.
    assert_eq!(
        c.session.answer(&[kind::TWALK, 1, 0, 9]),
        wire::error(1, libc::EPROTO)
    );
    Ok(())
}

/// The server on its socket, as Firecracker connects the VM to it.
#[test]
fn serves_on_a_unix_socket() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let shared = dir.path().join("caches");
    std::fs::create_dir(&shared)?;
    let socket = dir.path().join("v.sock_1026");
    let (s, d) = (socket.clone(), shared.clone());
    std::thread::spawn(move || serve(&d, &s));
    let mut conn = loop {
        match UnixStream::connect(&socket) {
            Ok(conn) => break conn,
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    };
    let mut w = Writer::new(kind::TVERSION, wire::NOTAG);
    w.u32(MAX_MSIZE * 4).bytes(b"9P2000.L");
    conn.write_all(&w.finish())?;
    let mut reply = vec![0u8; 4 + 1 + 2 + 4 + 2 + 8];
    conn.read_exact(&mut reply)?;
    assert_eq!(reply[4], kind::TVERSION + 1);
    assert_eq!(&reply[7..11], &MAX_MSIZE.to_le_bytes());
    assert_eq!(&reply[13..], b"9P2000.L");
    // A message larger than any the server takes ends the connection.
    conn.write_all(&(MAX_MSIZE * 2).to_le_bytes())?;
    let mut rest = Vec::new();
    assert_eq!(conn.read_to_end(&mut rest)?, 0);
    Ok(())
}
