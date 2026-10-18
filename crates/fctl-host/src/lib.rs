//! The fctl host: runs agent sessions in sandboxes on this machine.
//!
//! `fctl host run` starts [`daemon::Daemon`]; `fctl host install` writes the
//! config and a launchd/systemd service. See docs/architecture.md.

pub mod agentbin;
pub mod build;
pub mod config;
pub mod daemon;
pub mod driver;
pub mod expiry;
pub mod files;
pub mod image;
pub mod liveness;
pub mod login;
pub mod refresh;
pub mod service;
pub mod session;
pub mod sysinfo;
pub mod uplink;

pub use config::HostConfig;
pub use daemon::Daemon;
