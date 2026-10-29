//! What the host needs besides images to run microVMs: Firecracker itself,
//! a kernel, and the initramfs with fctl-agent as its init.

use super::registry::{Registry, file_digest};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The Firecracker the host downloads, and the SHA-256 of each
/// architecture's release archive. Older ones lack what the VMs use
/// (discard on their disks, from 1.17).
pub const FIRECRACKER_VERSION: &str = "1.17.0";
const FIRECRACKER_ARCHIVES: [(&str, &str); 2] = [
    (
        "x86_64",
        "06094a1108ae9e82aa4c23a775aa92758f53f1175d422270d9d6162cb9ade558",
    ),
    (
        "aarch64",
        "e351ebe4f7a16b5873bbd51005d2e6767103cff4d5ebc829df2d3f95a93e2256",
    ),
];

/// The kernel VMs boot unless `host.toml` names another: Firecracker's
/// guest configuration for Linux 6.18, built and published by
/// hyperstructure/sandbox, as `/vmlinux` in an image.
pub const KERNEL_IMAGE: &str = "ghcr.io/hyperstructure/sandbox-kernel:6.18.59";

/// Where the release archive of Firecracker `version` is.
fn firecracker_url(version: &str, arch: &str) -> String {
    format!(
        "https://github.com/firecracker-microvm/firecracker/releases/download/v{version}/firecracker-v{version}-{arch}.tgz"
    )
}

/// The Firecracker binary the host downloads for this machine, in `bin`.
pub fn pinned_firecracker(bin: &Path) -> PathBuf {
    bin.join(format!("firecracker-v{FIRECRACKER_VERSION}"))
}

/// Downloads the pinned Firecracker into `bin`, checked against its
/// archive's SHA-256, unless it is there.
pub async fn download_firecracker(
    bin: &Path,
    log: &(dyn Fn(String) + Send + Sync),
) -> Result<PathBuf> {
    let path = pinned_firecracker(bin);
    if path.is_file() {
        return Ok(path);
    }
    let arch = std::env::consts::ARCH;
    let Some((_, sum)) = FIRECRACKER_ARCHIVES.iter().find(|(a, _)| *a == arch) else {
        bail!("Firecracker runs on x86_64 and aarch64, not {arch}");
    };
    std::fs::create_dir_all(bin)?;
    let url = firecracker_url(FIRECRACKER_VERSION, arch);
    log(format!("Downloading Firecracker {FIRECRACKER_VERSION}"));
    let archive = bin.join(format!(".firecracker-v{FIRECRACKER_VERSION}.tgz"));
    let http = fctl_client::HttpClient::new(std::time::Duration::from_secs(600));
    http.download_to(
        &url,
        None,
        &[],
        &archive,
        512 * 1024 * 1024,
        std::time::Duration::from_secs(600),
    )
    .await
    .with_context(|| format!("downloading {url}"))?;
    let result = (|| -> Result<()> {
        let digest = file_digest(&archive)?;
        if digest != format!("sha256:{sum}") {
            bail!("{url} doesn't match the checksum fctl expects");
        }
        let inside = format!(
            "release-v{FIRECRACKER_VERSION}-{arch}/firecracker-v{FIRECRACKER_VERSION}-{arch}"
        );
        let file = std::fs::File::open(&archive)?;
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        for entry in tar.entries()? {
            let mut entry = entry?;
            if entry.path()?.to_string_lossy() == inside {
                let tmp = path.with_extension("tmp");
                let mut out = std::fs::File::create(&tmp)?;
                std::io::copy(&mut entry, &mut out)?;
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
                std::fs::rename(&tmp, &path)?;
                return Ok(());
            }
        }
        bail!("{url} has no {inside}")
    })();
    let _ = std::fs::remove_file(&archive);
    result.map(|()| path)
}

/// The kernel at `kernel`: a file, or an image reference whose `/vmlinux`
/// it pulls into `dir` once.
pub async fn kernel(
    kernel: &str,
    dir: &Path,
    log: &(dyn Fn(String) + Send + Sync),
) -> Result<PathBuf> {
    if kernel.starts_with('/') {
        let path = PathBuf::from(kernel);
        if !path.is_file() {
            bail!("the kernel {kernel} (firecracker.kernel in host.toml) isn't there");
        }
        return Ok(path);
    }
    let key = hex::encode(&Sha256::digest(kernel.as_bytes())[..8]);
    let path = dir.join(key).join("vmlinux");
    if path.is_file() {
        return Ok(path);
    }
    log(format!("Pulling the VMs' kernel, {kernel}"));
    let registry = Registry::new();
    let image = registry.resolve(kernel, std::env::consts::ARCH).await?;
    let Some(parent) = path.parent() else {
        bail!("no directory for the kernel");
    };
    std::fs::create_dir_all(parent)?;
    // The kernel is the image's only file; its last layer has it.
    for layer in image.layers.iter().rev() {
        let blob = parent.join("layer.tmp");
        registry.blob(kernel, layer, &blob).await?;
        let found = extract(
            &blob,
            &layer.media_type,
            "vmlinux",
            &path.with_extension("tmp"),
        );
        let _ = std::fs::remove_file(&blob);
        if found? {
            std::fs::rename(path.with_extension("tmp"), &path)?;
            return Ok(path);
        }
    }
    bail!("{kernel} has no /vmlinux")
}

/// Copies the file `name` from the layer at `blob` to `to`; whether it was
/// there.
fn extract(blob: &Path, media_type: &str, name: &str, to: &Path) -> Result<bool> {
    let file = std::fs::File::open(blob)?;
    let reader: Box<dyn Read> = if media_type.ends_with("gzip") {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut tar = tar::Archive::new(reader);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let path = path.strip_prefix("/").unwrap_or(&path);
        let path = path.strip_prefix("./").unwrap_or(path);
        if path == Path::new(name) && entry.header().entry_type().is_file() {
            let mut out = std::fs::File::create(to)?;
            std::io::copy(&mut entry, &mut out)?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether the ELF binary `bytes` loads without shared libraries: the
/// initramfs has none. `None` when it isn't an ELF binary.
pub fn is_static(bytes: &[u8]) -> Option<bool> {
    if bytes.get(..4)? != b"\x7fELF" || *bytes.get(4)? != 2 || *bytes.get(5)? != 1 {
        // Only 64-bit little-endian binaries run here.
        return None;
    }
    let u16_at = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let u64_at = |at: usize| Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?));
    let phoff = usize::try_from(u64_at(0x20)?).ok()?;
    let phentsize = usize::from(u16_at(0x36)?);
    let phnum = usize::from(u16_at(0x38)?);
    // PT_INTERP names the dynamic loader.
    let interp = (0..phnum).any(|i| u32_at(phoff + i * phentsize) == Some(3));
    Some(!interp)
}

/// An archive in the `newc` format the kernel unpacks as its initramfs.
#[derive(Default)]
pub struct Cpio {
    bytes: Vec<u8>,
    ino: u32,
}

impl Cpio {
    fn entry(&mut self, name: &str, mode: u32, rdev: (u32, u32), data: &[u8]) {
        self.ino += 1;
        let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
        let namesize = u32::try_from(name.len() + 1).unwrap_or(u32::MAX);
        let fields = [
            self.ino, mode, 0, 0, 1, 0, size, 0, 0, rdev.0, rdev.1, namesize, 0,
        ];
        self.bytes.extend_from_slice(b"070701");
        for f in fields {
            self.bytes.extend_from_slice(format!("{f:08x}").as_bytes());
        }
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes.push(0);
        self.pad();
        self.bytes.extend_from_slice(data);
        self.pad();
    }

    fn pad(&mut self) {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
    }

    pub fn dir(&mut self, name: &str) {
        self.entry(name, 0o040755, (0, 0), &[]);
    }

    pub fn file(&mut self, name: &str, mode: u32, data: &[u8]) {
        self.entry(name, 0o100000 | mode, (0, 0), data);
    }

    pub fn char_device(&mut self, name: &str, mode: u32, major: u32, minor: u32) {
        self.entry(name, 0o020000 | mode, (major, minor), &[]);
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.entry("TRAILER!!!", 0, (0, 0), &[]);
        self.bytes
    }
}

/// The initramfs for `agent`, a static fctl-agent: it is the init, with a
/// console to write to and the file that tells it it's in the initramfs
/// (`fctl-agent`'s `vm::boot::MARKER`). Made in `dir` once for each agent.
pub fn initrd(agent: &Path, dir: &Path) -> Result<PathBuf> {
    let bytes = std::fs::read(agent).with_context(|| format!("reading {}", agent.display()))?;
    match is_static(&bytes) {
        Some(true) => {}
        Some(false) => bail!(
            "{} loads shared libraries, which a microVM's initramfs doesn't have: the firecracker driver needs the static fctl-agent of fctl's release archives (fctl-agent-{}-linux), or one built for {}-unknown-linux-musl (agent_bin in host.toml)",
            agent.display(),
            std::env::consts::ARCH,
            std::env::consts::ARCH
        ),
        None => bail!("{} isn't a Linux binary for this machine", agent.display()),
    }
    let key = hex::encode(&Sha256::digest(&bytes)[..8]);
    let path = dir.join(format!("{key}.cpio"));
    if path.is_file() {
        return Ok(path);
    }
    let mut cpio = Cpio::default();
    cpio.dir("dev");
    cpio.char_device("dev/console", 0o600, 5, 1);
    cpio.dir("proc");
    cpio.dir("sys");
    cpio.file("fctl-initramfs", 0o644, b"");
    cpio.file("init", 0o755, &bytes);
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, cpio.finish())?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_firecracker_is_for_both_architectures() {
        assert_eq!(
            firecracker_url("1.17.0", "aarch64"),
            "https://github.com/firecracker-microvm/firecracker/releases/download/v1.17.0/firecracker-v1.17.0-aarch64.tgz"
        );
        for arch in ["x86_64", "aarch64"] {
            let sum = FIRECRACKER_ARCHIVES.iter().find(|(a, _)| *a == arch);
            assert!(sum.is_some_and(|(_, s)| s.len() == 64), "{arch}");
        }
    }

    #[test]
    fn tells_static_binaries_from_dynamic_ones() {
        // An ELF header with one program header at 64: PT_LOAD, or PT_INTERP.
        let elf = |kind: u32| {
            let mut b = vec![0u8; 64 + 56];
            b[..4].copy_from_slice(b"\x7fELF");
            b[4] = 2;
            b[5] = 1;
            b[0x20..0x28].copy_from_slice(&64u64.to_le_bytes());
            b[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
            b[0x38..0x3a].copy_from_slice(&1u16.to_le_bytes());
            b[64..68].copy_from_slice(&kind.to_le_bytes());
            b
        };
        assert_eq!(is_static(&elf(1)), Some(true));
        assert_eq!(is_static(&elf(3)), Some(false));
        assert_eq!(is_static(b"#!/bin/sh\n"), None);
        // This test's own binary, built against the system's libc.
        if cfg!(all(target_os = "linux", target_env = "gnu"))
            && let Ok(exe) = std::env::current_exe()
            && let Ok(bytes) = std::fs::read(exe)
        {
            assert_eq!(is_static(&bytes), Some(false));
        }
    }

    #[test]
    fn writes_newc_archives() {
        let mut cpio = Cpio::default();
        cpio.dir("dev");
        cpio.char_device("dev/console", 0o600, 5, 1);
        cpio.file("init", 0o755, b"hello");
        let bytes = cpio.finish();
        assert_eq!(bytes.len() % 4, 0);
        assert!(bytes.starts_with(b"070701"));
        // The console: a character device (020600), 5:1.
        let console = bytes
            .windows(11)
            .position(|w| w == b"dev/console")
            .unwrap_or_default();
        let header = &bytes[console - 110..console];
        assert_eq!(&header[14..22], b"00002180");
        assert_eq!(&header[78..86], b"00000005");
        assert_eq!(&header[86..94], b"00000001");
        assert!(bytes.windows(5).any(|w| w == b"hello"));
        assert!(bytes.windows(10).any(|w| w == b"TRAILER!!!"));
        // What the system's cpio makes of it, where there is one.
        if let Ok(mut child) = std::process::Command::new("cpio")
            .args(["-t", "--quiet"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
        {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(&bytes);
            }
            if let Ok(out) = child.wait_with_output() {
                let listed = String::from_utf8_lossy(&out.stdout).into_owned();
                assert_eq!(listed, "dev\ndev/console\ninit\n");
            }
        }
    }
}
