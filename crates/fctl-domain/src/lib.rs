//! Pure fctl logic, shared by the control plane (wasm) and native binaries.
//!
//! Nothing here does I/O or reads the clock: callers pass `now` (milliseconds
//! since the epoch) so the same code runs in a Worker and in tests.

pub mod account;
pub mod allowlist;
pub mod events;
pub mod files;
pub mod ids;
pub mod naming;
pub mod perm;
pub mod schedule;
pub mod size;
pub mod tokens;
pub mod validate;

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
