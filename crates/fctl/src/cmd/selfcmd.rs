//! `fctl self update`: replace this fctl, and the Linux agent next to it,
//! with a release from fctl.sh, the way the installer does: one archive for
//! this platform, checked against its SHA-256. Afterwards it restarts the
//! host service (agents keep running) and refreshes installed completions.

use crate::style;
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use fctl_client::HttpClient;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Subcommand)]
pub enum SelfCmd {
    /// Update fctl to the latest release; a host service on this machine
    /// restarts on the new version (its agents keep running).
    Update {
        /// Install this version instead of the latest (e.g. 0.1.2).
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
        /// Only check whether an update is available.
        #[arg(long, conflicts_with = "version")]
        check: bool,
    },
}

fn download_base() -> String {
    std::env::var("FCTL_DOWNLOAD")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://fctl.sh/releases".into())
        .trim_end_matches('/')
        .to_owned()
}

/// `0.1.2` (a leading `v` is fine) as comparable numbers.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim().trim_start_matches('v').splitn(3, '.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// The release target for an OS and architecture, as the installer names them.
pub fn target(os: &str, arch: &str, translated: bool) -> Result<String> {
    let os = match os {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-musl",
        other => bail!("fctl releases are built for macOS and Linux, not {other}"),
    };
    let arch = match arch {
        // An x86_64 build under Rosetta on Apple silicon updates to the native one.
        "x86_64" if translated => "aarch64",
        "aarch64" | "x86_64" => arch,
        other => bail!("fctl releases are built for aarch64 and x86_64, not {other}"),
    };
    Ok(format!("{arch}-{os}"))
}

fn under_rosetta() -> bool {
    cfg!(target_os = "macos")
        && std::process::Command::new("sysctl")
            .args(["-n", "sysctl.proc_translated"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
}

/// The named files from a `.tar.gz` release archive.
pub fn unpack(archive: &[u8], names: &[&str]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let mut out = Vec::new();
    for entry in tar.entries().context("reading the archive")? {
        let mut entry = entry.context("reading the archive")?;
        let path = entry.path()?.to_string_lossy().into_owned();
        let name = path.trim_start_matches("./");
        if names.contains(&name) {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            out.push((name.to_owned(), bytes));
        }
    }
    for n in names {
        if !out.iter().any(|(name, _)| name == n) {
            bail!("{n} is missing from the archive");
        }
    }
    Ok(out)
}

/// Replaces `dir/name` atomically (a running binary keeps its old copy).
fn replace(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dest = dir.join(name);
    let tmp = dir.join(format!(".{name}.update-{}", std::process::id()));
    let write = || -> std::io::Result<()> {
        std::fs::write(&tmp, bytes)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp, &dest)
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            bail!(
                "cannot write to {}: run the installer with the rights to it instead:\n  curl -fsSL https://fctl.sh/install | FCTL_INSTALL_DIR={} sh",
                dir.display(),
                dir.display()
            );
        }
        return Err(e).with_context(|| format!("writing {}", dest.display()));
    }
    Ok(dest)
}

pub async fn run(cmd: SelfCmd, json: bool) -> Result<()> {
    let SelfCmd::Update { version, check } = cmd;
    let base = download_base();
    let http = HttpClient::new(Duration::from_secs(300));
    let latest = match version.as_deref() {
        Some(v) => v.trim().trim_start_matches('v').to_owned(),
        None => {
            let body = http
                .download(&format!("{base}/latest"))
                .await
                .context("looking up the latest release")?;
            String::from_utf8_lossy(&body).trim().to_owned()
        }
    };
    let (Some(want), Some(have)) = (parse_version(&latest), parse_version(VERSION)) else {
        bail!("unexpected version: {latest}");
    };
    if check {
        if json {
            return style::print_json(&serde_json::json!({
                "current": VERSION,
                "latest": latest,
                "update_available": want > have,
            }));
        }
        if want > have {
            println!("fctl {latest} is available (you have {VERSION}). Run `fctl self update`.");
        } else {
            println!("fctl {VERSION} is up to date.");
        }
        return Ok(());
    }
    if version.is_none() && want <= have {
        println!("fctl {VERSION} is up to date.");
        return Ok(());
    }

    let target = target(
        std::env::consts::OS,
        std::env::consts::ARCH,
        under_rosetta(),
    )?;
    let arch = target.split('-').next().unwrap_or("aarch64");
    let archive = format!("fctl-{target}.tar.gz");
    let url = format!("{base}/v{latest}/{archive}");
    if !json {
        println!("Downloading fctl {latest} for {target}");
    }
    let bytes = http
        .download(&url)
        .await
        .with_context(|| format!("downloading {url}"))?;
    let sums = http
        .download(&format!("{url}.sha256"))
        .await
        .with_context(|| format!("downloading {url}.sha256"))?;
    let expected = String::from_utf8_lossy(&sums)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let actual = hex::encode(Sha256::digest(&bytes));
    if expected.is_empty() || expected != actual {
        bail!("checksum mismatch for {archive}; nothing was changed");
    }
    let agent = format!("fctl-agent-{arch}-linux");
    let files = unpack(&bytes, &["fctl", &agent])?;

    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .context("finding this fctl")?;
    let dir = exe.parent().context("finding fctl's directory")?.to_owned();
    // The new binary has to run here before it replaces this one.
    let Some((_, new_fctl)) = files.iter().find(|(n, _)| n == "fctl") else {
        bail!("fctl is missing from the archive");
    };
    let probe = replace(&dir, ".fctl-probe", new_fctl)?;
    let out = std::process::Command::new(&probe).arg("--version").output();
    let _ = std::fs::remove_file(&probe);
    match out {
        Ok(o) if o.status.success() && String::from_utf8_lossy(&o.stdout).contains(&latest) => {}
        Ok(o) => bail!(
            "the new fctl did not run here ({}); nothing was changed",
            String::from_utf8_lossy(&o.stderr).trim()
        ),
        Err(e) => bail!("the new fctl did not run here ({e}); nothing was changed"),
    }
    for (name, data) in &files {
        replace(&dir, name, data)?;
    }

    // A host service runs the binary at this path: restart it on the new one.
    let (installed, running) = fctl_host::service::status();
    let restarted = installed && running && fctl_host::service::restart().is_ok();
    // Completion scripts list the commands; the new fctl rewrites its own.
    let refreshed = std::process::Command::new(dir.join("fctl"))
        .args(["completions", "--refresh"])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty());
    if json {
        return style::print_json(&serde_json::json!({
            "previous": VERSION,
            "version": latest,
            "path": dir.join("fctl"),
            "host_restarted": restarted,
            "completions_refreshed": refreshed,
        }));
    }
    println!(
        "Updated fctl {VERSION} → {} in {}",
        style::bold(&latest),
        dir.display()
    );
    if restarted {
        println!("Restarted the host service; its sessions keep running.");
    } else if installed {
        println!("Restart the host service to use it: fctl host restart");
    }
    if refreshed {
        println!("Refreshed your shell completions.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert_eq!(parse_version("0.1.2"), Some((0, 1, 2)));
        assert_eq!(parse_version("v1.10.0\n"), Some((1, 10, 0)));
        assert!(parse_version("1.10.0") > parse_version("1.9.9"));
        assert_eq!(parse_version("latest"), None);
        assert_eq!(parse_version("1.2"), None);
    }

    #[test]
    fn targets_match_the_installer() -> Result<()> {
        assert_eq!(target("macos", "aarch64", false)?, "aarch64-apple-darwin");
        assert_eq!(target("macos", "x86_64", true)?, "aarch64-apple-darwin");
        assert_eq!(target("macos", "x86_64", false)?, "x86_64-apple-darwin");
        assert_eq!(
            target("linux", "x86_64", false)?,
            "x86_64-unknown-linux-musl"
        );
        assert!(target("windows", "x86_64", false).is_err());
        Ok(())
    }

    #[test]
    fn unpacks_release_archives() -> Result<()> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (name, data) in [
            ("fctl", b"new fctl".as_slice()),
            ("fctl-agent-aarch64-linux", b"agent"),
        ] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            builder.append_data(&mut h, name, data)?;
        }
        let archive = builder.into_inner()?.finish()?;
        let files = unpack(&archive, &["fctl", "fctl-agent-aarch64-linux"])?;
        assert_eq!(files[0], ("fctl".into(), b"new fctl".to_vec()));
        assert!(unpack(&archive, &["fctl", "fctl-agent-x86_64-linux"]).is_err());
        Ok(())
    }

    #[test]
    fn replaces_files_atomically() -> Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("fctl"), "old")?;
        let p = replace(dir.path(), "fctl", b"new")?;
        assert_eq!(std::fs::read_to_string(&p)?, "new");
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
        Ok(())
    }
}
