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
    let tmp = std::env::temp_dir().join(format!("fctl-image-{}", std::process::id()));
    let result = async {
        let dir = match source {
            Some(dir) => {
                std::fs::canonicalize(dir).with_context(|| format!("finding {}", dir.display()))?
            }
            None => download(&tmp).await?,
        };
        build_from(driver, tag, download_base, &dir).await
    }
    .await;
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

async fn build_from(driver: &Driver, tag: &str, download_base: &str, dir: &Path) -> Result<()> {
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
        "lima" => return build_lima_base(dir, &[&version, &download]).await,
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

async fn build_lima_base(dir: &Path, env: &[&str]) -> Result<()> {
    run_inherit(
        "limactl",
        &[
            "create",
            "--name",
            BASE_VM,
            "--tty=false",
            "template://debian-12",
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
    use super::{check_source, shell_quote, unpack};

    #[test]
    fn quotes_values_for_sh() {
        assert_eq!(
            shell_quote("FCTL_DOWNLOAD=https://fctl.sh/releases"),
            "'FCTL_DOWNLOAD=https://fctl.sh/releases'"
        );
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
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
            ("Dockerfile", "FROM debian:bookworm-slim\n"),
            ("provision.sh", "#!/usr/bin/env bash\n"),
            ("README.md", "# Sandbox\n"),
        ])?;
        let dir = unpack(&archive, tmp.path())?;
        assert_eq!(dir, tmp.path().join("sandbox-main"));
        assert_eq!(
            std::fs::read_to_string(dir.join("Dockerfile"))?,
            "FROM debian:bookworm-slim\n"
        );
        check_source(&dir)?;
        Ok(())
    }

    #[test]
    fn needs_a_dockerfile_and_provision_sh() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let archive = github_archive(&[("Dockerfile", "FROM debian:bookworm-slim\n")])?;
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
