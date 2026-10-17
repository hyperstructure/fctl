//! Host configuration: `host.toml` in the fctl config directory.
//!
//! ```toml
//! server = "https://app.fctl.sh"
//! name = "studio-mac"
//! driver = "auto"          # apple | docker | lima | process | auto
//! capacity = 4
//! image = "fctl-agent:latest"
//! cpus = 4                 # per session
//! memory_mb = 8192         # per session
//! ```
//!
//! The runner token lives next to it in `host-token` (mode 0600).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HostConfig {
    /// The instance URL, e.g. `https://app.fctl.sh`.
    pub server: String,
    /// Display name of this runner.
    pub name: String,
    pub driver: String,
    /// Maximum concurrent sessions.
    pub capacity: u32,
    /// Sandbox image for sessions whose project sets none.
    pub image: String,
    pub cpus: u32,
    pub memory_mb: u32,
    pub labels: Vec<String>,
    /// Where sandboxes and caches live (default: the platform data dir).
    pub data_dir: Option<PathBuf>,
    /// `claude` for the process driver.
    pub claude_bin: Option<String>,
    /// Extra arguments for `claude` (for tests: point at fake-claude).
    pub claude_args: Vec<String>,
    /// A Linux fctl-agent binary to put into sandboxes (default: found or downloaded).
    pub agent_bin: Option<PathBuf>,
    /// Where releases are downloaded from.
    pub download_base: String,
    /// Push new commits after each turn.
    pub push_on_commit: bool,
    /// Keep a stopped session's sandbox (to resume it) for this many hours.
    pub keep_stopped_hours: u32,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            name: hostname(),
            driver: "auto".into(),
            capacity: 4,
            image: "fctl-agent:latest".into(),
            cpus: 4,
            memory_mb: 8192,
            labels: vec![],
            data_dir: None,
            claude_bin: None,
            claude_args: vec![],
            agent_bin: None,
            download_base: "https://fctl.sh/releases".into(),
            push_on_commit: true,
            keep_stopped_hours: 72,
        }
    }
}

pub fn hostname() -> String {
    std::env::var("FCTL_HOSTNAME")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        })
        .filter(|h| !h.is_empty())
        .map(|h| h.trim_end_matches(".local").to_owned())
        .unwrap_or_else(|| "host".into())
}

/// `~/.config/fctl` (or `$FCTL_CONFIG_DIR`).
pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FCTL_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(x) => PathBuf::from(x).join("fctl"),
        None => home.join(".config/fctl"),
    }
}

/// Default data directory for sandboxes and caches.
pub fn default_data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FCTL_DATA_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/fctl/host")
    } else {
        match std::env::var_os("XDG_DATA_HOME") {
            Some(x) => PathBuf::from(x).join("fctl/host"),
            None => home.join(".local/share/fctl/host"),
        }
    }
}

impl HostConfig {
    pub fn path() -> PathBuf {
        config_dir().join("host.toml")
    }

    pub fn token_path() -> PathBuf {
        config_dir().join("host-token")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "{} not found: run `fctl host install` first",
                path.display()
            )
        })?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn load_token() -> Result<String> {
        if let Ok(t) = std::env::var("FCTL_RUNNER_TOKEN") {
            return Ok(t);
        }
        let path = Self::token_path();
        Ok(std::fs::read_to_string(&path)
            .with_context(|| {
                format!(
                    "{} not found: run `fctl host install` first",
                    path.display()
                )
            })?
            .trim()
            .to_owned())
    }

    pub fn save_token(token: &str) -> Result<()> {
        let path = Self::token_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_private(&path, token)
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.clone().unwrap_or_else(default_data_dir)
    }
}

pub fn write_private(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}
