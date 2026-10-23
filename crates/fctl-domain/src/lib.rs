//! Pure fctl logic, shared by the control plane (wasm) and native binaries.
//!
//! Nothing here does I/O or reads the clock: callers pass `now` (milliseconds
//! since the epoch) so the same code runs in a Worker and in tests.

pub mod account;
pub mod allowlist;
pub mod branches;
pub mod cbor;
pub mod claude;
pub mod cookies;
pub mod cron;
pub mod disk;
pub mod events;
pub mod files;
pub mod follow_ups;
pub mod github;
pub mod ids;
pub mod image;
pub mod issues;
pub mod journal;
pub mod limits;
pub mod linear;
pub mod memory;
pub mod naming;
pub mod notify;
pub mod order;
pub mod perm;
pub mod published;
pub mod pulls;
pub mod qr;
pub mod reconcile;
pub mod remote_control;
pub mod repos;
pub mod retrospectives;
pub mod reviews;
pub mod schedule;
pub mod schedules;
pub mod settings;
pub mod size;
pub mod threads;
pub mod throttle;
pub mod tickets;
pub mod tokens;
pub mod totp;
pub mod two_factor;
pub mod usage;
pub mod validate;
pub mod verify;
pub mod webauthn;
pub mod webhooks;
pub mod webpush;
pub mod workspace;

/// Fills `buf` with cryptographically secure random bytes.
///
/// # Panics
/// Never in practice: the OS (or `crypto.getRandomValues` in Workers) does not
/// fail. If it ever does, continuing with predictable tokens would be worse.
#[allow(clippy::expect_used)]
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("secure random number generator unavailable");
    buf
}
