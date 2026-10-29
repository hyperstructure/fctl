//! Unpacking an image's layers onto a disk, in a VM booted for it: the host
//! sends the layers as its registry gave them, and this applies them one
//! after the other, as Docker does, with their whiteouts.

use super::boot::say;
use anyhow::{Context, Result, bail};
use fctl_proto::vm::{Layer, MAX_LINE, PORT, Reply, Request};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use vsock::{VMADDR_CID_ANY, VsockAddr, VsockListener};

/// What unpacking wrote.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Unpacked {
    pub files: u64,
    pub bytes: u64,
}

/// Answers the host until one `unpack` request is done.
pub fn serve(root: &Path) -> Result<()> {
    let listener = VsockListener::bind(&VsockAddr::new(VMADDR_CID_ANY, PORT))
        .context("listening on the vsock")?;
    loop {
        let (conn, _) = listener.accept().context("taking a connection")?;
        let mut reader = BufReader::new(conn.try_clone()?);
        let mut writer = conn;
        let mut line = Vec::new();
        (&mut reader)
            .take(MAX_LINE as u64)
            .read_until(b'\n', &mut line)?;
        let request = serde_json::from_slice::<Request>(&line);
        let reply = |w: &mut vsock::VsockStream, r: &Reply| -> Result<()> {
            let mut line = serde_json::to_vec(r)?;
            line.push(b'\n');
            w.write_all(&line)?;
            Ok(())
        };
        match request {
            Ok(Request::Hello { .. }) => reply(
                &mut writer,
                &Reply {
                    version: Some(super::VERSION.into()),
                    ..Reply::default()
                },
            )?,
            Ok(Request::Unpack { layers }) => {
                reply(&mut writer, &Reply::default())?;
                let result = layers_onto(root, &layers, &mut reader);
                let answer = match &result {
                    Ok(u) => Reply {
                        files: Some(u.files),
                        bytes: Some(u.bytes),
                        ..Reply::default()
                    },
                    Err(e) => Reply::failed(format!("{e:#}")),
                };
                nix::unistd::sync();
                reply(&mut writer, &answer)?;
                return result.map(drop);
            }
            Ok(_) => reply(
                &mut writer,
                &Reply::failed("this VM unpacks an image; it runs nothing"),
            )?,
            Err(e) => reply(&mut writer, &Reply::failed(format!("unknown request: {e}")))?,
        }
    }
}

/// Reads each of `layers` from `from` and applies it onto `root`.
fn layers_onto(root: &Path, layers: &[Layer], from: &mut impl Read) -> Result<Unpacked> {
    let mut done = Unpacked::default();
    for (i, layer) in layers.iter().enumerate() {
        let mut bytes = from.take(layer.size);
        let n = i + 1;
        match compression(&layer.media_type)? {
            Compression::Gzip => apply(
                root,
                flate2::read::MultiGzDecoder::new(&mut bytes),
                &mut done,
            ),
            Compression::None => apply(root, &mut bytes, &mut done),
        }
        .with_context(|| format!("unpacking layer {n} of {}", layers.len()))?;
        // What the decompressor left, padding say.
        std::io::copy(&mut bytes, &mut std::io::sink())?;
        say(&format!(
            "unpacked layer {n} of {}: {} files so far",
            layers.len(),
            done.files
        ));
    }
    Ok(done)
}

enum Compression {
    Gzip,
    None,
}

/// How a layer of `media_type` is compressed.
fn compression(media_type: &str) -> Result<Compression> {
    match media_type {
        "application/vnd.oci.image.layer.v1.tar+gzip"
        | "application/vnd.oci.image.layer.nondistributable.v1.tar+gzip"
        | "application/vnd.docker.image.rootfs.diff.tar.gzip"
        | "application/vnd.docker.image.rootfs.foreign.diff.tar.gzip" => Ok(Compression::Gzip),
        "application/vnd.oci.image.layer.v1.tar"
        | "application/vnd.oci.image.layer.nondistributable.v1.tar"
        | "application/vnd.docker.image.rootfs.diff.tar" => Ok(Compression::None),
        other => bail!("layers of type {other} can't be unpacked here yet (only tar and tar+gzip)"),
    }
}

/// The path of an entry, relative and without `.` or `..`; `None` for one
/// that would leave the root, or names it.
fn relative(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(p) => out.push(p),
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Removes whatever is at `path`, a directory with all it holds too.
fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// `path` in `root` through no link that leads out of it: its directory as
/// the filesystem resolves it, and its name. `None` when that is outside
/// `root` (`root` is canonical), or isn't there.
fn within(root: &Path, path: &Path) -> Option<PathBuf> {
    let dir = root.join(path.parent()?).canonicalize().ok()?;
    let name = path.file_name()?;
    dir.starts_with(root).then(|| dir.join(name))
}

/// Applies one layer, a tar stream, onto `root`: `.wh.<name>` removes
/// `<name>` from the layers below, `.wh..wh..opq` everything they have in
/// its directory, and an entry replaces what was at its path unless both
/// are directories. Owners, modes, times and extended attributes stay as
/// the layer has them (the VM's init runs as root). Nothing it does
/// reaches out of `root`, whatever links the layers make.
pub fn apply(root: &Path, layer: impl Read, done: &mut Unpacked) -> Result<()> {
    let root = root.canonicalize()?;
    let root = root.as_path();
    let mut archive = tar::Archive::new(layer);
    archive.set_preserve_permissions(true);
    archive.set_preserve_ownerships(nix::unistd::geteuid().is_root());
    archive.set_preserve_mtime(true);
    archive.set_unpack_xattrs(true);
    archive.set_overwrite(true);
    // What this layer put in place, which its opaque whiteouts keep.
    let mut ours: HashSet<PathBuf> = HashSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let Some(path) = relative(&entry.path()?) else {
            continue;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name == ".wh..wh..opq" {
            let Some(dir) = within(root, &path)
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
            else {
                continue;
            };
            if let Ok(children) = std::fs::read_dir(&dir) {
                for child in children.flatten() {
                    let full = child.path();
                    let rel = full.strip_prefix(root).map(Path::to_path_buf);
                    if rel.is_ok_and(|r| !ours.contains(&r)) {
                        remove(&full)?;
                    }
                }
            }
            continue;
        }
        if let Some(hidden) = name.strip_prefix(".wh.") {
            if let Some(target) = within(root, &path.with_file_name(hidden)) {
                remove(&target)?;
            }
            continue;
        }
        let kind = entry.header().entry_type();
        if let Some(target) = within(root, &path)
            && let Ok(meta) = std::fs::symlink_metadata(&target)
            && !(meta.is_dir() && kind.is_dir())
        {
            remove(&target)?;
        }
        if kind.is_character_special() || kind.is_block_special() || kind.is_fifo() {
            special(root, &path, entry.header())?;
        } else {
            entry.unpack_in(root)?;
        }
        done.files += 1;
        done.bytes += entry.header().size().unwrap_or(0);
        ours.insert(path);
    }
    Ok(())
}

/// A device node or a FIFO, which the tar crate would make a plain file.
fn special(root: &Path, path: &Path, header: &tar::Header) -> Result<()> {
    use nix::sys::stat::{Mode, SFlag, makedev, mknod};
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(root.join(dir))?;
    }
    let Some(target) = within(root, path) else {
        bail!("{} would be outside the image", path.display());
    };
    let kind = header.entry_type();
    let flag = if kind.is_character_special() {
        SFlag::S_IFCHR
    } else if kind.is_block_special() {
        SFlag::S_IFBLK
    } else {
        SFlag::S_IFIFO
    };
    let mode = Mode::from_bits_truncate(header.mode()? & 0o7777);
    let dev = makedev(
        u64::from(header.device_major()?.unwrap_or(0)),
        u64::from(header.device_minor()?.unwrap_or(0)),
    );
    mknod(&target, flag, mode, dev).with_context(|| format!("making {}", path.display()))?;
    if nix::unistd::geteuid().is_root() {
        let uid = u32::try_from(header.uid()?).ok();
        let gid = u32::try_from(header.gid()?).ok();
        nix::unistd::chown(
            &target,
            uid.map(nix::unistd::Uid::from_raw),
            gid.map(nix::unistd::Gid::from_raw),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layer of `(path, contents)`: a path ending in `/` is a directory,
    /// contents starting with `->` a symlink.
    fn layer(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (path, contents) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_mtime(1_700_000_000);
            if let Some(dir) = path.strip_suffix('/') {
                h.set_entry_type(tar::EntryType::Directory);
                h.set_mode(0o755);
                h.set_size(0);
                b.append_data(&mut h, dir, std::io::empty()).ok();
            } else if let Some(to) = contents.strip_prefix("->") {
                h.set_entry_type(tar::EntryType::Symlink);
                h.set_size(0);
                b.append_link(&mut h, path, to).ok();
            } else {
                h.set_mode(0o644);
                h.set_size(contents.len() as u64);
                b.append_data(&mut h, path, contents.as_bytes()).ok();
            }
        }
        b.into_inner().unwrap_or_default()
    }

    fn read(root: &Path, path: &str) -> Option<String> {
        std::fs::read_to_string(root.join(path)).ok()
    }

    #[test]
    fn layers_apply_with_their_whiteouts() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        let mut done = Unpacked::default();
        apply(
            root,
            &layer(&[
                ("etc/", ""),
                ("etc/hostname", "base"),
                ("etc/motd", "hello"),
                ("opt/app/", ""),
                ("opt/app/old", "1"),
                ("opt/app/keep", "2"),
                ("bin/", ""),
                ("bin/sh", "->busybox"),
                ("var/thing", "a file"),
            ])[..],
            &mut done,
        )?;
        assert_eq!(done.files, 9);
        apply(
            root,
            &layer(&[
                ("etc/.wh.motd", ""),
                // Opaque: what the layers below have in opt/app goes, but
                // not what this layer puts there, before or after.
                ("opt/app/new", "3"),
                ("opt/app/.wh..wh..opq", ""),
                ("opt/app/later", "4"),
                ("etc/hostname", "top"),
                // A directory where a file was.
                ("var/thing/", ""),
                ("var/thing/inside", "x"),
            ])[..],
            &mut done,
        )?;
        assert_eq!(read(root, "etc/hostname").as_deref(), Some("top"));
        assert!(!root.join("etc/motd").exists());
        assert!(!root.join("etc/.wh.motd").exists());
        assert_eq!(read(root, "opt/app/new").as_deref(), Some("3"));
        assert_eq!(read(root, "opt/app/later").as_deref(), Some("4"));
        assert!(!root.join("opt/app/old").exists());
        assert!(!root.join("opt/app/keep").exists());
        assert!(!root.join("opt/app/.wh..wh..opq").exists());
        assert_eq!(read(root, "var/thing/inside").as_deref(), Some("x"));
        assert_eq!(
            std::fs::read_link(root.join("bin/sh"))?,
            PathBuf::from("busybox")
        );
        Ok(())
    }

    #[test]
    fn entries_stay_inside_the_root() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("root");
        std::fs::create_dir(&root)?;
        let mut done = Unpacked::default();
        // A link out of the root, then a file through it, and a whiteout
        // that climbs out.
        let mut b = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        b.append_link(&mut h, "escape", dir.path())?;
        let mut h = tar::Header::new_gnu();
        h.set_size(1);
        h.set_mode(0o644);
        b.append_data(&mut h, "escape/outside", &b"x"[..])?;
        let bytes = b.into_inner()?;
        let r = apply(&root, &bytes[..], &mut done);
        assert!(r.is_err(), "{r:?}");
        assert!(!dir.path().join("outside").exists());
        // Whiteouts and replaced entries reach nothing through the link.
        std::fs::write(dir.path().join("precious"), "x")?;
        apply(
            &root,
            &layer(&[("escape/.wh.precious", ""), ("escape/.wh..wh..opq", "")])[..],
            &mut done,
        )?;
        assert!(dir.path().join("precious").exists());
        let r = apply(
            &root,
            &layer(&[("escape/precious", "replaced")])[..],
            &mut done,
        );
        assert!(r.is_err(), "{r:?}");
        assert_eq!(read(dir.path(), "precious").as_deref(), Some("x"));
        assert_eq!(relative(Path::new("../../etc/passwd")), None);
        assert_eq!(relative(Path::new("./")), None);
        assert_eq!(
            relative(Path::new("/./usr//bin/")),
            Some(PathBuf::from("usr/bin"))
        );
        Ok(())
    }

    #[test]
    fn layers_are_gzip_or_plain_tar() {
        assert!(matches!(
            compression("application/vnd.oci.image.layer.v1.tar+gzip"),
            Ok(Compression::Gzip)
        ));
        assert!(matches!(
            compression("application/vnd.docker.image.rootfs.diff.tar.gzip"),
            Ok(Compression::Gzip)
        ));
        assert!(matches!(
            compression("application/vnd.oci.image.layer.v1.tar"),
            Ok(Compression::None)
        ));
        assert!(compression("application/vnd.oci.image.layer.v1.tar+zstd").is_err());
    }
}
