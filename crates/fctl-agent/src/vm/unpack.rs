//! Unpacking an image's layers onto a disk, in a VM booted for it: the host
//! sends the layers as its registry gave them, and this applies them one
//! after the other, as Docker does, with their whiteouts.

use super::boot::say;
use anyhow::{Context, Result, bail};
use fctl_proto::vm::{Layer, MAX_LINE, PORT, Reply, Request};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use vsock::{VMADDR_CID_ANY, VMADDR_CID_HOST, VsockAddr, VsockListener};

/// What unpacking wrote.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Unpacked {
    pub files: u64,
    pub bytes: u64,
    /// Each layer's digest, uncompressed (`sha256:<hex>`): what the image's
    /// config says they are (`rootfs.diff_ids`), when it tells the truth.
    pub diff_ids: Vec<String>,
}

/// Answers the host until one `unpack` request is done.
pub fn serve(root: &Path) -> Result<()> {
    let listener = VsockListener::bind(&VsockAddr::new(VMADDR_CID_ANY, PORT))
        .context("listening on the vsock")?;
    loop {
        let (conn, peer) = listener.accept().context("taking a connection")?;
        if peer.cid() != VMADDR_CID_HOST {
            continue;
        }
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
            Ok(Request::Unpack { layers, overlay }) => {
                reply(&mut writer, &Reply::default())?;
                let whiteouts = if overlay {
                    Whiteouts::Overlay
                } else {
                    Whiteouts::Apply
                };
                let result = layers_onto(root, &layers, &mut reader, whiteouts);
                let answer = match &result {
                    Ok(u) => Reply {
                        files: Some(u.files),
                        bytes: Some(u.bytes),
                        diff_ids: Some(u.diff_ids.clone()),
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

/// How a layer's whiteouts land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whiteouts {
    /// Applied: what they hide goes, as the layers stack into one tree.
    Apply,
    /// Kept as overlayfs reads them, for an image whose filesystem lies
    /// over another image's: `.wh.<name>` as a character device 0/0 named
    /// `<name>`, `.wh..wh..opq` as its directory's `trusted.overlay.opaque`,
    /// which hide what the image below has there. A directory in place of
    /// something else is opaque too: what the image below has there went.
    Overlay,
}

/// Reads each of `layers` from `from` and applies it onto `root`.
fn layers_onto(
    root: &Path,
    layers: &[Layer],
    from: &mut impl Read,
    whiteouts: Whiteouts,
) -> Result<Unpacked> {
    let mut done = Unpacked::default();
    for (i, layer) in layers.iter().enumerate() {
        let mut bytes = from.take(layer.size);
        let n = i + 1;
        let digest = match compression(&layer.media_type)? {
            Compression::Gzip => apply_hashed(
                root,
                flate2::read::MultiGzDecoder::new(&mut bytes),
                &mut done,
                whiteouts,
            ),
            Compression::None => apply_hashed(root, &mut bytes, &mut done, whiteouts),
        }
        .with_context(|| format!("unpacking layer {n} of {}", layers.len()))?;
        done.diff_ids.push(digest);
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

/// Applies one layer, a tar stream, as [`apply`] does; its digest, of all
/// of the stream, the end the tar archive doesn't need too.
fn apply_hashed(
    root: &Path,
    layer: impl Read,
    done: &mut Unpacked,
    whiteouts: Whiteouts,
) -> Result<String> {
    let mut hashed = Hashed {
        inner: layer,
        hasher: Sha256::new(),
    };
    apply(root, &mut hashed, done, whiteouts)?;
    std::io::copy(&mut hashed, &mut std::io::sink())?;
    Ok(format!("sha256:{}", hex::encode(hashed.hasher.finalize())))
}

/// A stream that hashes what is read from it.
struct Hashed<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Read for Hashed<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
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

/// Makes the directories of `dir` (relative to `root`, which is canonical)
/// that aren't there, each only once the one it goes in resolves inside
/// `root`, as the tar crate does for an entry's: none through a link that
/// leads out of it, which is `Ok(false)`. Each is root's, 0755, as Docker
/// makes those a layer doesn't list.
fn make_dirs(root: &Path, dir: &Path) -> Result<bool> {
    let mut missing = Vec::new();
    let mut at = dir;
    while std::fs::symlink_metadata(root.join(at)).is_err() {
        missing.push(at);
        match at.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => at = parent,
            _ => break,
        }
    }
    for dir in missing.into_iter().rev() {
        let Some(made) = within(root, dir) else {
            return Ok(false);
        };
        match std::fs::create_dir(&made) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).with_context(|| format!("making {}", dir.display())),
        }
    }
    Ok(true)
}

/// Applies one layer, a tar stream, onto `root`: `.wh.<name>` removes
/// `<name>` from the layers below, `.wh..wh..opq` everything they have in
/// its directory, and an entry replaces what was at its path unless both
/// are directories; with [`Whiteouts::Overlay`], they also stay as
/// overlayfs reads them. Owners, modes, times and extended attributes stay
/// as the layer has them (the VM's init runs as root). Nothing it does
/// reaches out of `root`, whatever links the layers make.
pub fn apply(
    root: &Path,
    layer: impl Read,
    done: &mut Unpacked,
    whiteouts: Whiteouts,
) -> Result<()> {
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
            // Kept, it needs its directory, as a whiteout does.
            if whiteouts == Whiteouts::Overlay
                && let Some(dir) = path.parent()
                && !dir.as_os_str().is_empty()
                && !make_dirs(root, dir)?
            {
                continue;
            }
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
            if whiteouts == Whiteouts::Overlay {
                opaque(&dir)?;
            }
            continue;
        }
        if let Some(hidden) = name.strip_prefix(".wh.") {
            let hidden = path.with_file_name(hidden);
            // Kept, the whiteout needs its directory, which a layer lists
            // (as BuildKit's and Docker's do) or the image below has.
            if whiteouts == Whiteouts::Overlay
                && let Some(dir) = hidden.parent()
                && !dir.as_os_str().is_empty()
                && !make_dirs(root, dir)?
            {
                continue;
            }
            if let Some(target) = within(root, &hidden) {
                remove(&target)?;
                if whiteouts == Whiteouts::Overlay {
                    whiteout(&target)?;
                }
            }
            continue;
        }
        let kind = entry.header().entry_type();
        // In place of something else, a directory hides what the image
        // below has there.
        let mut replaced = false;
        if let Some(target) = within(root, &path)
            && let Ok(meta) = std::fs::symlink_metadata(&target)
            && !(meta.is_dir() && kind.is_dir())
        {
            remove(&target)?;
            replaced = true;
        }
        if kind.is_character_special() || kind.is_block_special() || kind.is_fifo() {
            special(root, &path, entry.header())?;
        } else {
            entry.unpack_in(root)?;
        }
        if replaced
            && kind.is_dir()
            && whiteouts == Whiteouts::Overlay
            && let Some(target) = within(root, &path)
        {
            opaque(&target)?;
        }
        done.files += 1;
        done.bytes += entry.header().size().unwrap_or(0);
        ours.insert(path);
    }
    Ok(())
}

/// Hides what the image below has at `path`, as overlayfs reads it: a
/// character device 0/0, root's.
fn whiteout(path: &Path) -> Result<()> {
    use nix::sys::stat::{Mode, SFlag, makedev, mknod};
    mknod(path, SFlag::S_IFCHR, Mode::empty(), makedev(0, 0))
        .with_context(|| format!("making the whiteout {}", path.display()))
}

/// Hides what the image below has in the directory `dir`, as overlayfs
/// reads it.
fn opaque(dir: &Path) -> Result<()> {
    xattr::set(dir, "trusted.overlay.opaque", b"y")
        .with_context(|| format!("making {} opaque", dir.display()))
}

/// A device node or a FIFO, which the tar crate would make a plain file.
fn special(root: &Path, path: &Path, header: &tar::Header) -> Result<()> {
    use nix::sys::stat::{Mode, SFlag, makedev, mknod};
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
        && !make_dirs(root, dir)?
    {
        bail!("{} would be outside the image", path.display());
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
            // Root's, which a root that keeps owners reads.
            h.set_uid(0);
            h.set_gid(0);
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
            Whiteouts::Apply,
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
            Whiteouts::Apply,
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
        h.set_uid(0);
        h.set_gid(0);
        h.set_size(0);
        b.append_link(&mut h, "escape", dir.path())?;
        let mut h = tar::Header::new_gnu();
        h.set_uid(0);
        h.set_gid(0);
        h.set_size(1);
        h.set_mode(0o644);
        b.append_data(&mut h, "escape/outside", &b"x"[..])?;
        let bytes = b.into_inner()?;
        let r = apply(&root, &bytes[..], &mut done, Whiteouts::Apply);
        assert!(r.is_err(), "{r:?}");
        assert!(!dir.path().join("outside").exists());
        // Whiteouts and replaced entries reach nothing through the link.
        std::fs::write(dir.path().join("precious"), "x")?;
        apply(
            &root,
            &layer(&[("escape/.wh.precious", ""), ("escape/.wh..wh..opq", "")])[..],
            &mut done,
            Whiteouts::Apply,
        )?;
        assert!(dir.path().join("precious").exists());
        let r = apply(
            &root,
            &layer(&[("escape/precious", "replaced")])[..],
            &mut done,
            Whiteouts::Apply,
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

    /// Each layer's digest is that of all of its tar stream, uncompressed,
    /// the blocks after the archive's end too, as Docker's are.
    #[test]
    fn layers_tell_their_digests() -> Result<()> {
        use std::io::Write;
        let dir = tempfile::tempdir()?;
        let mut plain = layer(&[("etc/", ""), ("etc/hostname", "base")]);
        // Padded to a whole record, as GNU tar does.
        plain.resize(10240, 0);
        let other = layer(&[("etc/hostname", "top")]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&other)?;
        let gz = gz.finish()?;
        let layers = [
            Layer {
                media_type: "application/vnd.oci.image.layer.v1.tar".into(),
                size: plain.len() as u64,
            },
            Layer {
                media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
                size: gz.len() as u64,
            },
        ];
        let stream = [plain.clone(), gz].concat();
        let done = layers_onto(dir.path(), &layers, &mut &stream[..], Whiteouts::Apply)?;
        let digest = |b: &[u8]| format!("sha256:{}", hex::encode(Sha256::digest(b)));
        assert_eq!(done.diff_ids, [digest(&plain), digest(&other)]);
        assert_eq!(read(dir.path(), "etc/hostname").as_deref(), Some("top"));
        Ok(())
    }

    /// Kept for overlayfs, whiteouts make the directories they need in the
    /// layer, but none through a link out of the root.
    #[test]
    fn kept_whiteouts_make_no_directory_outside_the_root() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("root");
        std::fs::create_dir(&root)?;
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside)?;
        let mut done = Unpacked::default();
        apply(
            &root,
            &layer(&[("escape", &format!("->{}", outside.display()))])[..],
            &mut done,
            Whiteouts::Overlay,
        )?;
        apply(
            &root,
            &layer(&[("escape/sub/.wh.y", ""), ("escape/deeper/.wh..wh..opq", "")])[..],
            &mut done,
            Whiteouts::Overlay,
        )?;
        assert_eq!(std::fs::read_dir(&outside)?.count(), 0);
        // Nor does a device node's.
        let mut b = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Fifo);
        h.set_mode(0o644);
        h.set_uid(0);
        h.set_gid(0);
        h.set_size(0);
        b.append_data(&mut h, "escape/pipes/fifo", std::io::empty())?;
        let bytes = b.into_inner()?;
        let r = apply(&root, &bytes[..], &mut done, Whiteouts::Apply);
        assert!(r.is_err(), "{r:?}");
        assert_eq!(std::fs::read_dir(&outside)?.count(), 0);
        let canonical = root.canonicalize()?;
        assert!(!make_dirs(&canonical, Path::new("escape/x"))?);
        assert!(make_dirs(&canonical, Path::new("a/b/c"))?);
        assert!(root.join("a/b/c").is_dir());
        // Anything else that keeps a directory from being made fails the
        // layer, rather than lose what it hides.
        std::fs::write(root.join("file"), "x")?;
        assert!(make_dirs(&canonical, Path::new("file/sub")).is_err());
        let r = apply(
            &root,
            &layer(&[("file/sub/.wh.y", "")])[..],
            &mut done,
            Whiteouts::Overlay,
        );
        assert!(r.is_err(), "{r:?}");
        Ok(())
    }

    /// Whether this process can make whiteouts as overlayfs reads them: a
    /// character device 0/0 and `trusted.` attributes, which need root
    /// with `CAP_MKNOD` and `CAP_SYS_ADMIN` (a container's root may lack
    /// them).
    fn keeps_whiteouts(dir: &Path) -> bool {
        nix::unistd::geteuid().is_root()
            && whiteout(&dir.join("probe")).is_ok()
            && opaque(dir).is_ok()
    }

    /// Kept for overlayfs: a whiteout is a character device 0/0, an opaque
    /// directory has `trusted.overlay.opaque`, and so does a directory in
    /// place of something else; an entry in place of a whiteout replaces
    /// it. Only as root, which the VM's init is.
    #[test]
    fn whiteouts_stay_as_overlayfs_reads_them() -> Result<()> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let dir = tempfile::tempdir()?;
        if !keeps_whiteouts(dir.path()) {
            // CI runs it as root, and sets this so that it can't skip.
            assert!(
                std::env::var_os("FCTL_TEST_WHITEOUTS").is_none(),
                "FCTL_TEST_WHITEOUTS is set, but this process can't make whiteouts"
            );
            eprintln!("skipped: only root with CAP_MKNOD and CAP_SYS_ADMIN makes whiteouts");
            return Ok(());
        }
        let root = dir.path().join("root");
        std::fs::create_dir(&root)?;
        let mut done = Unpacked::default();
        apply(
            &root,
            &layer(&[
                ("etc/", ""),
                ("etc/.wh.motd", ""),
                ("etc/hostname", "top"),
                ("opt/app/", ""),
                ("opt/app/new", "3"),
                ("opt/app/.wh..wh..opq", ""),
                ("var/thing", "a file"),
                ("srv/.wh.data", ""),
                // In a directory the layer doesn't list.
                ("usr/share/.wh.games", ""),
            ])[..],
            &mut done,
            Whiteouts::Overlay,
        )?;
        let is_whiteout = |p: &str| {
            std::fs::symlink_metadata(root.join(p))
                .is_ok_and(|m| m.file_type().is_char_device() && m.rdev() == 0)
        };
        let is_opaque = |p: &str| {
            xattr::get(root.join(p), "trusted.overlay.opaque")
                .ok()
                .flatten()
                .is_some_and(|v| v == b"y")
        };
        assert!(is_whiteout("etc/motd"));
        assert!(!root.join("etc/.wh.motd").exists());
        assert!(is_opaque("opt/app") && !is_opaque("etc"));
        assert_eq!(read(&root, "opt/app/new").as_deref(), Some("3"));
        assert!(!root.join("opt/app/.wh..wh..opq").exists());
        assert!(is_whiteout("srv/data") && is_whiteout("usr/share/games"));
        let share = std::fs::metadata(root.join("usr/share"))?;
        assert_eq!((share.uid(), share.mode() & 0o7777), (0, 0o755));
        apply(
            &root,
            &layer(&[
                // A file and a directory in place of whiteouts.
                ("etc/motd", "back"),
                ("srv/data/", ""),
                ("srv/data/new", "1"),
                // A directory in place of a file.
                ("var/thing/", ""),
                ("var/thing/inside", "x"),
            ])[..],
            &mut done,
            Whiteouts::Overlay,
        )?;
        assert_eq!(read(&root, "etc/motd").as_deref(), Some("back"));
        assert!(is_opaque("srv/data") && is_opaque("var/thing"));
        assert_eq!(read(&root, "var/thing/inside").as_deref(), Some("x"));
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
