//! Wire types shared by every fctl component.
//!
//! Every protocol is JSON. Enums are tagged (`"type"` for messages, `"kind"`
//! for event bodies) and use `snake_case`. Hosts and agents may lag the
//! control plane by a version, so new fields must be optional
//! (`#[serde(default)]`) and tags are never repurposed.
//!
//! | Module | Between |
//! |---|---|
//! | [`api`] | REST clients ⇄ control plane |
//! | [`autopilot`] | autopilots (experimental), through the REST API |
//! | [`event`] | the session event model used everywhere |
//! | [`hooks`] | webhooks: the REST API's, and what fctl posts to them |
//! | [`identity`] | identity tokens agents mint, and the clouds they reach with them |
//! | [`service_accounts`] | a workspace's own programs, through the REST API |
//! | [`viewer`] | SessionDO ⇄ browsers and the CLI |
//! | [`runner`] | FleetDO ⇄ hosts (control connection) |
//! | [`uplink`] | SessionDO ⇄ hosts (one connection per session) |
//! | [`agent`] | hosts ⇄ fctl-agent inside a sandbox, and `fctl` ⇄ fctl-agent within it |
//! | [`tickets`] | tickets in a project's tracker, whatever the tracker, and Linear's connections |
//! | [`slack`] | Slack's connections, people's Slack accounts, and what agents read and post there |
//! | [`vm`] | hosts ⇄ the init of a Firecracker microVM, over its vsock |

pub mod agent;
pub mod api;
pub mod autopilot;
pub mod error;
pub mod event;
pub mod hooks;
pub mod identity;
pub mod ids;
pub mod runner;
pub mod service_accounts;
pub mod session;
pub mod slack;
pub mod tickets;
pub mod uplink;
pub mod viewer;
pub mod vm;

#[cfg(feature = "ts")]
mod ts_export;
#[cfg(feature = "ts")]
pub use ts_export::{DEFAULTS_FILE, export_typescript};

pub use error::{ApiError, ErrorCode};
pub use ids::*;

/// Version of the runner, uplink and agent protocols. Bump it only for
/// changes that older peers cannot ignore.
pub const PROTOCOL_VERSION: u32 = 1;

/// Milliseconds since the Unix epoch.
pub type Millis = i64;
