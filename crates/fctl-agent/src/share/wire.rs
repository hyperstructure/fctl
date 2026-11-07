//! 9P2000.L's messages, as the Linux kernel's 9p client sends them and a
//! server answers: a size (4 bytes), a type (1), a tag (2) and the fields,
//! all little-endian; strings after their length (2 bytes).

use std::io;

/// The message types this server reads (`T…`) and writes (`R…`), as the
/// Linux kernel numbers them (`include/net/9p/9p.h`).
pub mod kind {
    pub const RLERROR: u8 = 7;
    pub const TSTATFS: u8 = 8;
    pub const TLOPEN: u8 = 12;
    pub const TLCREATE: u8 = 14;
    pub const TSYMLINK: u8 = 16;
    pub const TMKNOD: u8 = 18;
    pub const TRENAME: u8 = 20;
    pub const TREADLINK: u8 = 22;
    pub const TGETATTR: u8 = 24;
    pub const TSETATTR: u8 = 26;
    pub const TXATTRWALK: u8 = 30;
    pub const TXATTRCREATE: u8 = 32;
    pub const TREADDIR: u8 = 40;
    pub const TFSYNC: u8 = 50;
    pub const TLOCK: u8 = 52;
    pub const TGETLOCK: u8 = 54;
    pub const TLINK: u8 = 70;
    pub const TMKDIR: u8 = 72;
    pub const TRENAMEAT: u8 = 74;
    pub const TUNLINKAT: u8 = 76;
    pub const TVERSION: u8 = 100;
    pub const TAUTH: u8 = 102;
    pub const TATTACH: u8 = 104;
    pub const TFLUSH: u8 = 108;
    pub const TWALK: u8 = 110;
    pub const TREAD: u8 = 116;
    pub const TWRITE: u8 = 118;
    pub const TCLUNK: u8 = 120;
    pub const TREMOVE: u8 = 122;
}

/// The tag of `Tversion`, which has none.
pub const NOTAG: u16 = 0xffff;

/// A fid that names no file (`Tattach`'s `afid` without authentication).
pub const NOFID: u32 = u32::MAX;

/// What a message's header and a reply's take: size, type and tag.
pub const HEADER: usize = 7;

/// What `Rread` takes before its data: the header and the count.
pub const READ_OVERHEAD: u32 = HEADER as u32 + 4;

/// A file's identity: its type (directory, symlink, file), a version and
/// a number unique on the server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Qid {
    pub kind: u8,
    pub version: u32,
    pub path: u64,
}

pub const QTDIR: u8 = 0x80;
pub const QTSYMLINK: u8 = 0x02;
pub const QTFILE: u8 = 0x00;

/// The fields of a message, read in order.
pub struct Reader<'a> {
    buf: &'a [u8],
}

fn short() -> io::Error {
    io::Error::from_raw_os_error(libc::EPROTO)
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.buf.len() < n {
            return Err(short());
        }
        let (head, rest) = self.buf.split_at(n);
        self.buf = rest;
        Ok(head)
    }

    pub fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> io::Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> io::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> io::Result<u64> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    /// A string, as the bytes it is: file names are bytes on Linux.
    pub fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let n = usize::from(self.u16()?);
        self.take(n)
    }

    /// What follows, as `Twrite`'s data: its count, then that many bytes.
    pub fn data(&mut self) -> io::Result<&'a [u8]> {
        let n = usize::try_from(self.u32()?).map_err(|_| short())?;
        self.take(n)
    }
}

/// A reply: [`Writer::new`] leaves room for its size, which
/// [`Writer::finish`] fills in.
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new(kind: u8, tag: u16) -> Self {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&[0, 0, 0, 0, kind]);
        buf.extend_from_slice(&tag.to_le_bytes());
        Self { buf }
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// A string; one longer than a string can be is cut short.
    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        let v = &v[..v.len().min(usize::from(u16::MAX))];
        self.u16(u16::try_from(v.len()).unwrap_or(u16::MAX));
        self.buf.extend_from_slice(v);
        self
    }

    pub fn qid(&mut self, q: Qid) -> &mut Self {
        self.u8(q.kind).u32(q.version).u64(q.path)
    }

    /// Data after its count.
    pub fn data(&mut self, v: &[u8]) -> &mut Self {
        self.u32(u32::try_from(v.len()).unwrap_or(u32::MAX));
        self.buf.extend_from_slice(v);
        self
    }

    /// Raw bytes, already in the wire's format (`Rreaddir`'s entries).
    pub fn raw(&mut self, v: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(v);
        self
    }

    pub fn finish(mut self) -> Vec<u8> {
        let size = u32::try_from(self.buf.len()).unwrap_or(u32::MAX);
        self.buf[..4].copy_from_slice(&size.to_le_bytes());
        self.buf
    }
}

/// `Rlerror`: the request failed with `errno`.
pub fn error(tag: u16, errno: i32) -> Vec<u8> {
    let mut w = Writer::new(kind::RLERROR, tag);
    w.u32(u32::try_from(errno).unwrap_or(libc::EIO as u32));
    w.finish()
}

/// A directory entry as `Rreaddir` carries it: its qid, the offset of the
/// entry after it, its type (`DT_…`) and its name.
pub fn dirent(out: &mut Vec<u8>, qid: Qid, offset: u64, kind: u8, name: &[u8]) {
    out.push(qid.kind);
    out.extend_from_slice(&qid.version.to_le_bytes());
    out.extend_from_slice(&qid.path.to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
    out.push(kind);
    let name = &name[..name.len().min(usize::from(u16::MAX))];
    out.extend_from_slice(&u16::try_from(name.len()).unwrap_or(u16::MAX).to_le_bytes());
    out.extend_from_slice(name);
}

/// What a directory entry takes in `Rreaddir`.
pub fn dirent_len(name: &[u8]) -> usize {
    13 + 8 + 1 + 2 + name.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_carry_their_size_and_strings_their_length() -> io::Result<()> {
        let mut w = Writer::new(101, NOTAG);
        w.u32(8192).bytes(b"9P2000.L");
        let reply = w.finish();
        assert_eq!(
            &reply[..4],
            &u32::try_from(reply.len()).unwrap_or(0).to_le_bytes()
        );
        assert_eq!(reply[4], 101);
        let mut r = Reader::new(&reply[5..]);
        assert_eq!(r.u16()?, NOTAG);
        assert_eq!(r.u32()?, 8192);
        assert_eq!(r.bytes()?, b"9P2000.L");
        // A message cut short is refused, rather than read past its end.
        assert!(r.u8().is_err());
        assert!(Reader::new(&[5, 0, b'a']).bytes().is_err());
        Ok(())
    }

    #[test]
    fn directory_entries_are_as_long_as_they_say() {
        let mut out = Vec::new();
        let qid = Qid {
            kind: QTDIR,
            version: 0,
            path: 42,
        };
        dirent(&mut out, qid, 7, 4, b"npm");
        assert_eq!(out.len(), dirent_len(b"npm"));
        assert_eq!(out[0], QTDIR);
        assert_eq!(&out[out.len() - 3..], b"npm");
    }
}
