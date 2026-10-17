//! Finding the fctl-agent binary that goes into sandboxes.
//!
//! VM and container sandboxes run Linux, so a macOS host needs a Linux build
//! of fctl-agent of its own version. In order: `agent_bin` from the config;
//! `fctl-agent-linux-<arch>` next to the fctl executable; a cached download;
//! a fresh download from `download_base` (checked against its `.sha256`).
//! A Linux host with the same architecture uses its own `fctl-agent`.

use crate::config::HostConfig;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn arch() -> &'static str {
    std::env::consts::ARCH
}

pub fn asset_name(arch: &str) -> String {
    format!("fctl-agent-{arch}-linux")
}

pub async fn linux_agent(cfg: &HostConfig) -> Result<PathBuf> {
    if let Some(p) = &cfg.agent_bin {
        if p.is_file() {
            return Ok(p.clone());
        }
        bail!("agent_bin {} does not exist", p.display());
    }
    let exe = std::env::current_exe()?;
    let bundled = exe.with_file_name(asset_name(arch()));
    if bundled.is_file() {
        return Ok(bundled);
    }
    if cfg!(target_os = "linux") {
        let native = exe.with_file_name("fctl-agent");
        if native.is_file() {
            return Ok(native);
        }
    }
    let cache_dir = cfg.data_dir().join("bin");
    let cached = cache_dir.join(format!("fctl-agent-{VERSION}-{}-linux", arch()));
    if cached.is_file() {
        return Ok(cached);
    }
    std::fs::create_dir_all(&cache_dir)?;
    let base = cfg.download_base.trim_end_matches('/');
    let url = format!("{base}/v{VERSION}/{}", asset_name(arch()));
    tracing::info!("downloading {url}");
    let http = fctl_client::HttpClient::new(std::time::Duration::from_secs(300));
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
    if expected != actual {
        bail!("checksum mismatch for {url}: expected {expected}, got {actual}");
    }
    let tmp = cached.with_extension("tmp");
    std::fs::write(&tmp, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, &cached)?;
    Ok(cached)
}
