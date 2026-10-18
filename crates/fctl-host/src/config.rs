//! Host configuration: `host.toml` in the fctl config directory.
//!
//! ```toml
//! server = "https://app.fctl.sh"
//! name = "studio-mac"
//! driver = "auto"          # apple | docker | lima | process | auto
//! capacity = 4
//! image = "ghcr.io/acme/agent:1"  # unset: the published image of this version
//! cpus = 4                 # per sandbox, when its project sets none
//! memory_mb = 8192         # per sandbox, when its project sets none
//! max_cpus = 8             # per sandbox (default: what the machine has)
//! max_memory_mb = 16384    # per sandbox (default: what the machine has)
//! ```
//!
//! The runner token lives next to it in `host-token` (mode 0600).

use anyhow::{Context, Result};
use fctl_proto::runner::{SandboxLimits, SandboxSize};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the default sandbox image is published, by the Image workflow of
/// https://github.com/hyperstructure/sandbox: a tag for every release.
pub const IMAGE_REPOSITORY: &str = "ghcr.io/hyperstructure/sandbox";

/// The default image before it was published, which hosts installed until
/// then have in host.toml.
const OLD_DEFAULT_IMAGE: &str = "fctl-agent:latest";

/// The published sandbox image of this version of fctl.
pub fn default_image() -> String {
    format!("{IMAGE_REPOSITORY}:{}", crate::agentbin::VERSION)
}

/// Whether an image setting, the host's or a project's, means the host's
/// default: empty, or the name the default had before it was published.
pub fn means_default_image(image: &str) -> bool {
    let image = image.trim();
    image.is_empty() || image == OLD_DEFAULT_IMAGE
}

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
    /// Sandbox image for sessions whose project sets none. Unset: the
    /// published image of this host's version ([`default_image`]).
    pub image: Option<String>,
    /// CPUs and memory (MiB) for sandboxes whose project sets none.
    pub cpus: u32,
    pub memory_mb: u32,
    /// The most CPUs and memory (MiB) one sandbox may have. Unset: all that
    /// the machine has, or the Docker VM with Docker on macOS.
    pub max_cpus: Option<u32>,
    pub max_memory_mb: Option<u32>,
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
            image: None,
            cpus: 4,
            memory_mb: 8192,
            max_cpus: None,
            max_memory_mb: None,
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
        Self::from_toml(&text).with_context(|| format!("parsing {}", path.display()))
    }

    fn from_toml(text: &str) -> Result<Self> {
        let mut cfg: Self = toml::from_str(text)?;
        cfg.image = cfg.image.filter(|i| !means_default_image(i));
        Ok(cfg)
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

    /// The sandbox image for sessions whose project sets none.
    pub fn image(&self) -> String {
        self.image.clone().unwrap_or_else(default_image)
    }

    /// What sandboxes use when [`image`](Self::image) can't be pulled: on
    /// the default image, the latest published one. A release's own image is
    /// published a few minutes after the release, and development builds may
    /// have a version with none.
    pub fn fallback_image(&self) -> Option<String> {
        self.image
            .is_none()
            .then(|| format!("{IMAGE_REPOSITORY}:latest"))
    }

    /// What this host gives each sandbox: the configured defaults and
    /// maximums, within `runtime`, what the sandboxes run on has (0: unknown).
    pub fn sandbox_limits(&self, runtime: SandboxSize) -> SandboxLimits {
        let within = |configured: Option<u32>, has: u32| match (configured, has) {
            (Some(c), 0) => c,
            (Some(c), has) => c.min(has),
            (None, 0) => u32::MAX,
            (None, has) => has,
        };
        let max = SandboxSize {
            cpus: within(self.max_cpus, runtime.cpus),
            memory_mb: within(self.max_memory_mb, runtime.memory_mb),
        };
        SandboxLimits {
            default: SandboxSize {
                cpus: self.cpus.max(1).min(max.cpus),
                memory_mb: self.memory_mb.max(512).min(max.memory_mb),
            },
            max,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn size(cpus: u32, memory_mb: u32) -> SandboxSize {
        SandboxSize { cpus, memory_mb }
    }

    #[test]
    fn hosts_on_the_default_image_follow_their_version() -> Result<()> {
        let published = format!(
            "ghcr.io/hyperstructure/sandbox:{}",
            crate::agentbin::VERSION
        );
        // Unset, empty, or the default's name from before it was published.
        for text in ["", "image = \"\"", "image = \"fctl-agent:latest\""] {
            let cfg = HostConfig::from_toml(text)?;
            assert_eq!(cfg.image, None, "{text}");
            assert_eq!(cfg.image(), published);
            assert_eq!(
                cfg.fallback_image().as_deref(),
                Some("ghcr.io/hyperstructure/sandbox:latest")
            );
        }
        let cfg = HostConfig::from_toml("image = \"ghcr.io/acme/agent:1\"")?;
        assert_eq!(cfg.image(), "ghcr.io/acme/agent:1");
        assert_eq!(cfg.fallback_image(), None);
        // host.toml leaves the default out, so the next version brings its own.
        let saved = toml::to_string_pretty(&HostConfig::default())?;
        assert!(!saved.lines().any(|l| l.starts_with("image")), "{saved}");
        Ok(())
    }

    #[test]
    fn sandboxes_get_at_most_what_the_runtime_has() {
        let docker_vm = size(10, 15_990);
        let cfg = HostConfig::default();
        let limits = cfg.sandbox_limits(docker_vm);
        assert_eq!(limits.default, size(4, 8192));
        assert_eq!(limits.max, docker_vm);

        let cfg = HostConfig {
            max_cpus: Some(8),
            max_memory_mb: Some(65_536),
            ..HostConfig::default()
        };
        assert_eq!(cfg.sandbox_limits(docker_vm).max, size(8, 15_990));

        // A default bigger than the maximum is lowered to it.
        let small_vm = size(2, 2048);
        let limits = HostConfig::default().sandbox_limits(small_vm);
        assert_eq!(limits.default, small_vm);
        assert_eq!(limits.max, small_vm);

        // An unknown runtime leaves only what is configured.
        let cfg = HostConfig {
            max_memory_mb: Some(16_384),
            ..HostConfig::default()
        };
        let limits = cfg.sandbox_limits(size(0, 0));
        assert_eq!(limits.max, size(u32::MAX, 16_384));
        assert_eq!(limits.default, size(4, 8192));
    }
}
