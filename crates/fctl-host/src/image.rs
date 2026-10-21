//! Building the default sandbox image (or the Lima base VM) on this host,
//! from its sources in hyperstructure/sandbox.

use crate::driver::{Driver, lima::BASE_VM, spawn_error};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// The default image's sources: an archive of the branch it is published
/// from. Its Dockerfile builds with the archive's root as the context, and
/// its provision.sh also prepares the Lima VM.
pub const SOURCE: &str = "https://github.com/hyperstructure/sandbox/archive/refs/heads/main.tar.gz";

/// Builds `tag` with the driver's tooling, streaming output to the terminal,
/// from `source`, a checkout of hyperstructure/sandbox, or else from
/// [`SOURCE`]. The image gets this host's version of fctl, from
/// `download_base`.
pub async fn build(
    driver: &Driver,
    tag: &str,
    download_base: &str,
    source: Option<&Path>,
) -> Result<()> {
    if driver.name() == "process" {
        println!(
            "The process driver runs agents directly on this machine; there is no image to build."
        );
        println!(
            "Install Claude Code here instead: curl -fsSL https://claude.ai/install.sh | bash"
        );
        return Ok(());
    }
    let info = driver.probe().await;
    // Lima only needs limactl here: its base VM is what this builds.
    if !info.available && !(driver.name() == "lima" && info.version.is_some()) {
        bail!(
            "The {} driver is not ready: {}",
            info.name,
            info.detail.unwrap_or_default()
        );
    }
    match source {
        Some(dir) => {
            let dir =
                std::fs::canonicalize(dir).with_context(|| format!("finding {}", dir.display()))?;
            build_from(driver, tag, download_base, &dir, info.version.as_deref()).await
        }
        None => {
            let tmp = private_dir("fctl-image")?;
            let result = async {
                let dir = download(&tmp).await?;
                build_from(driver, tag, download_base, &dir, info.version.as_deref()).await
            }
            .await;
            let _ = std::fs::remove_dir_all(&tmp);
            result
        }
    }
}

/// A new directory for the image's sources that only this user can use.
/// In the shared temporary directory, one of a name another user made
/// first would be theirs, with the sources they put in it.
fn private_dir(prefix: &str) -> Result<PathBuf> {
    let base = std::env::temp_dir();
    for _ in 0..16 {
        let name = format!(
            "{prefix}-{}-{}",
            std::process::id(),
            hex::encode(fctl_domain::random_bytes::<8>())
        );
        let path = base.join(name);
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("making {}", path.display())),
        }
    }
    bail!("couldn't make a directory in {}", base.display())
}

/// `tool` is the driver's version: what `limactl --version` says, with Lima.
async fn build_from(
    driver: &Driver,
    tag: &str,
    download_base: &str,
    dir: &Path,
    tool: Option<&str>,
) -> Result<()> {
    check_source(dir)?;
    let ctx = dir.display().to_string();
    // provision.sh installs the latest release when this version is not
    // published (a development build).
    let version = format!("FCTL_VERSION={}", crate::agentbin::VERSION);
    let download = format!("FCTL_DOWNLOAD={download_base}");
    let file = format!("{ctx}/Dockerfile");
    let (program, args): (&str, Vec<&str>) = match driver.name() {
        "apple" => (
            "container",
            vec![
                "build",
                "--tag",
                tag,
                "--build-arg",
                &version,
                "--build-arg",
                &download,
                "--file",
                &file,
                &ctx,
            ],
        ),
        "docker" => (
            "docker",
            vec![
                "build",
                "--tag",
                tag,
                "--build-arg",
                &version,
                "--build-arg",
                &download,
                &ctx,
            ],
        ),
        "lima" => return build_lima_base(dir, &[&version, &download], tool).await,
        other => bail!("the {other} driver has no image to build"),
    };
    // A published image that the build replaces goes once no sandbox uses it.
    crate::refresh::replacing(driver, tag, run_inherit(program, &args))
        .await
        .map(drop)
}

/// Downloads [`SOURCE`] into `dir`; returns the directory with its files.
async fn download(dir: &Path) -> Result<PathBuf> {
    println!("Downloading the image's sources from {SOURCE}");
    let http = fctl_client::HttpClient::new(std::time::Duration::from_secs(120));
    let archive = http
        .download(SOURCE)
        .await
        .with_context(|| format!("downloading {SOURCE}"))?;
    unpack(&archive, dir)
}

/// Unpacks a `.tar.gz` of a repository into `dir`, and returns the one
/// directory GitHub's archives hold (`sandbox-main`).
fn unpack(archive: &[u8], dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    tar::Archive::new(flate2::read::GzDecoder::new(archive))
        .unpack(dir)
        .context("unpacking the image's sources")?;
    let mut dirs = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir());
    match (dirs.next(), dirs.next()) {
        (Some(top), None) => Ok(top),
        _ => bail!("the archive of the image's sources should hold one directory"),
    }
}

/// What a build needs from a checkout of hyperstructure/sandbox.
fn check_source(dir: &Path) -> Result<()> {
    for file in ["Dockerfile", "provision.sh"] {
        if !dir.join(file).is_file() {
            bail!(
                "{} has no {file}: build from a checkout of https://github.com/hyperstructure/sandbox",
                dir.display()
            );
        }
    }
    Ok(())
}

/// Prepares the base VM with `provision.sh`, on Lima's Debian template for
/// `lima` (what `limactl --version` says).
async fn build_lima_base(dir: &Path, env: &[&str], lima: Option<&str>) -> Result<()> {
    let template = lima_template(lima);
    if template == DEBIAN_12 {
        println!(
            "This Lima has no Debian 13 template ({}; Lima 2.0 added it), so {BASE_VM} gets \
             Debian 12, not the default image's Debian 13. Upgrade Lima (brew upgrade lima) \
             to prepare it on Debian 13.",
            lima.unwrap_or_default()
        );
    }
    run_inherit(
        "limactl",
        &[
            "create",
            "--name",
            BASE_VM,
            "--tty=false",
            "--set",
            crate::driver::lima::ISOLATED,
            template,
        ],
    )
    .await?;
    run_inherit("limactl", &["start", "--tty=false", BASE_VM]).await?;
    let script = dir.join("provision.sh").display().to_string();
    run_inherit(
        "limactl",
        &["copy", &script, &format!("{BASE_VM}:/tmp/provision.sh")],
    )
    .await?;
    // Lima's default user becomes the agent user.
    let env: Vec<String> = env.iter().map(|v| shell_quote(v)).collect();
    let provision = format!(
        "sudo AGENT_USER=$(id -un) {} bash /tmp/provision.sh",
        env.join(" ")
    );
    run_inherit("limactl", &["shell", BASE_VM, "--", "sh", "-c", &provision]).await?;
    run_inherit("limactl", &["stop", BASE_VM]).await?;
    println!("The {BASE_VM} VM is ready; sessions are cloned from it.");
    Ok(())
}

const DEBIAN_12: &str = "template://debian-12";
const DEBIAN_13: &str = "template://debian-13";

/// The base VM's template: Debian 13, as the default image, with Lima 2.0
/// and newer; older Limas have only Debian 12's. A version that can't be
/// read gets Debian 13, and limactl's error if it lacks the template.
fn lima_template(lima: Option<&str>) -> &'static str {
    match lima.and_then(major_version) {
        Some(major) if major < 2 => DEBIAN_12,
        _ => DEBIAN_13,
    }
}

/// 2 for `limactl version 2.2.1`.
fn major_version(version: &str) -> Option<u64> {
    let number = version.split_whitespace().last()?;
    number
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// `'…'` for sh.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

async fn run_inherit(program: &str, args: &[&str]) -> Result<()> {
    println!("$ {program} {}", args.join(" "));
    let status = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .status()
        .await
        .map_err(|e| spawn_error(program, e))?;
    if !status.success() {
        bail!(
            "{program} {} failed ({status})",
            args.first().copied().unwrap_or_default()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        DEBIAN_12, DEBIAN_13, check_source, lima_template, private_dir, shell_quote, unpack,
    };

    #[cfg(unix)]
    #[test]
    fn sources_download_into_a_new_directory_of_this_users() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let a = private_dir("fctl-image-test")?;
        let b = private_dir("fctl-image-test")?;
        assert_ne!(a, b);
        assert_eq!(std::fs::metadata(&a)?.permissions().mode() & 0o777, 0o700);
        std::fs::remove_dir(&a)?;
        std::fs::remove_dir(&b)?;
        Ok(())
    }

    #[test]
    fn quotes_values_for_sh() {
        assert_eq!(
            shell_quote("FCTL_DOWNLOAD=https://fctl.sh/releases"),
            "'FCTL_DOWNLOAD=https://fctl.sh/releases'"
        );
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn prepares_lima_vms_on_debian_13_from_lima_2() {
        assert_eq!(lima_template(Some("limactl version 2.2.1")), DEBIAN_13);
        assert_eq!(lima_template(Some("limactl version 2.0.0")), DEBIAN_13);
        assert_eq!(lima_template(Some("limactl version 1.2.2")), DEBIAN_12);
        assert_eq!(lima_template(Some("limactl version v1.0.7")), DEBIAN_12);
        assert_eq!(lima_template(Some("limactl version HEAD")), DEBIAN_13);
        assert_eq!(lima_template(None), DEBIAN_13);
    }

    /// An archive like GitHub's: a pax header with the commit, then one
    /// directory named after the repository and the branch.
    fn github_archive(files: &[(&str, &str)]) -> anyhow::Result<Vec<u8>> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let comment = b"52 comment=688d0ddc129fb274e9d97e73de7fa70803f7a0f7\n";
        let mut pax = tar::Header::new_ustar();
        pax.set_entry_type(tar::EntryType::XGlobalHeader);
        pax.set_size(comment.len() as u64);
        tar.append_data(&mut pax, "pax_global_header", &comment[..])?;
        let mut dir = tar::Header::new_ustar();
        dir.set_entry_type(tar::EntryType::Directory);
        dir.set_mode(0o755);
        dir.set_size(0);
        tar.append_data(&mut dir, "sandbox-main/", std::io::empty())?;
        for (name, text) in files {
            let mut file = tar::Header::new_ustar();
            file.set_mode(0o644);
            file.set_size(text.len() as u64);
            tar.append_data(&mut file, format!("sandbox-main/{name}"), text.as_bytes())?;
        }
        Ok(tar.into_inner()?.finish()?)
    }

    #[test]
    fn unpacks_the_sources_from_githubs_archive() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let archive = github_archive(&[
            ("Dockerfile", "FROM debian:trixie-slim\n"),
            ("provision.sh", "#!/usr/bin/env bash\n"),
            ("README.md", "# Sandbox\n"),
        ])?;
        let dir = unpack(&archive, tmp.path())?;
        assert_eq!(dir, tmp.path().join("sandbox-main"));
        assert_eq!(
            std::fs::read_to_string(dir.join("Dockerfile"))?,
            "FROM debian:trixie-slim\n"
        );
        check_source(&dir)?;
        Ok(())
    }

    #[test]
    fn needs_a_dockerfile_and_provision_sh() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let archive = github_archive(&[("Dockerfile", "FROM debian:trixie-slim\n")])?;
        let dir = unpack(&archive, tmp.path())?;
        let err = check_source(&dir).err().map(|e| e.to_string());
        assert!(
            err.as_deref()
                .is_some_and(|e| e.contains("has no provision.sh")),
            "{err:?}"
        );
        Ok(())
    }
}
