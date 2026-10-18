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
//! | [`event`] | the session event model used everywhere |
//! | [`viewer`] | SessionDO ⇄ browsers and the CLI |
//! | [`runner`] | FleetDO ⇄ hosts (control connection) |
//! | [`uplink`] | SessionDO ⇄ hosts (one connection per session) |
//! | [`agent`] | hosts ⇄ fctl-agent inside a sandbox, and `fctl` ⇄ fctl-agent within it |

pub mod agent;
pub mod api;
pub mod error;
pub mod event;
pub mod ids;
pub mod runner;
pub mod session;
pub mod uplink;
pub mod viewer;

#[cfg(feature = "ts")]
mod ts_export;
#[cfg(feature = "ts")]
pub use ts_export::export_typescript;

pub use error::{ApiError, ErrorCode};
pub use ids::*;

/// Version of the runner, uplink and agent protocols. Bump it only for
/// changes that older peers cannot ignore.
pub const PROTOCOL_VERSION: u32 = 1;

/// Milliseconds since the Unix epoch.
pub type Millis = i64;
