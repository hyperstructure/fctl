//! CLI configuration and credentials.
//!
//! `~/.config/fctl/config.toml` holds the default server; `credentials.toml`
//! (mode 0600) holds a token per server. `FCTL_SERVER` and `FCTL_TOKEN`
//! override both, which is how agents and CI use fctl.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Default, Serialize, Deserialize)]
pub struct CliConfig {
    pub server: Option<String>,
    pub project: Option<String>,
    /// The workspace commands use without `--workspace`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub tokens: BTreeMap<String, String>,
}

pub fn dir() -> PathBuf {
    fctl_host::config::config_dir()
}

impl CliConfig {
    pub fn load() -> Self {
        let mut config: Self = std::fs::read_to_string(dir().join("config.toml"))
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        // In an autopilot's own session, its project is the default.
        if config.project.is_none() {
            config.project = std::env::var(fctl_proto::agent::AUTOPILOT_ENV)
                .ok()
                .filter(|p| !p.is_empty());
        }
        config
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(dir())?;
        std::fs::write(dir().join("config.toml"), toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

impl Credentials {
    pub fn load() -> Self {
        std::fs::read_to_string(dir().join("credentials.toml"))
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(dir())?;
        fctl_host::config::write_private(
            &dir().join("credentials.toml"),
            &toml::to_string_pretty(self)?,
        )
    }
}

/// Normalizes `app.fctl.sh` → `https://app.fctl.sh`.
pub fn normalize_server(s: &str) -> String {
    let s = s.trim().trim_end_matches('/');
    if s.starts_with("http://") || s.starts_with("https://") {
        s.to_owned()
    } else {
        format!("https://{s}")
    }
}

/// The server to talk to: flag, environment, config.
pub fn server(flag: Option<&str>) -> Result<String> {
    if let Some(s) = flag
        .map(str::to_owned)
        .or_else(|| std::env::var("FCTL_SERVER").ok())
        .or_else(|| CliConfig::load().server)
    {
        return Ok(normalize_server(&s));
    }
    anyhow::bail!(
        "Which fctl instance? Sign in with `fctl login https://app.example.com` (or set FCTL_SERVER)."
    )
}

/// The token for a server, if signed in.
pub fn token(server: &str) -> Option<String> {
    std::env::var("FCTL_TOKEN")
        .ok()
        .or_else(|| Credentials::load().tokens.get(server).cloned())
}

/// An API client, signed in. Where a sensitive action needs you to
/// confirm it's you first, it asks in a terminal (`crate::verify`). In an
/// autopilot's own session's sandbox, where nobody signed in, its requests
/// go through fctl-agent, as the autopilot.
pub fn api(flag: Option<&str>) -> Result<fctl_client::Api> {
    if flag.is_none()
        && std::env::var("FCTL_TOKEN").is_err()
        && let Some(api) = crate::cmd::sessions::autopilot_api()
    {
        return Ok(api);
    }
    let server = server(flag)?;
    let token =
        token(&server).with_context(|| format!("Not signed in to {server}. Run `fctl login`."))?;
    Ok(fctl_client::Api::new(&server, Some(token)).with_verifier(crate::verify::verifier()))
}
