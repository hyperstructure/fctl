//! fctl-agent as the init of a Firecracker microVM, and what it runs there
//! (the `firecracker` driver: crates/fctl-host/src/driver/firecracker).
//!
//! The host boots each VM with fctl-agent as the initramfs's `/init`, and
//! the kernel's command line says what to boot to ([`fctl_proto::vm::Boot`]).
//!
//! - **A session's sandbox.** The init mounts the image's root filesystem
//!   (`/dev/vda`, read only, shared by the sandboxes made from that image)
//!   under the sandbox's own disk (`/dev/vdb`) as an overlay, copies itself
//!   to `/opt/fctl/bin/fctl-agent` there, and runs that copy as PID 1 in the
//!   new root ([`boot`]). That one sets the system up and keeps `fctl-agent
//!   vm-serve` running ([`serve`]), which runs what the host asks over the
//!   vsock, as `docker exec` would, and bridges the VM's network to the
//!   host's `passt` ([`net`]). PID 1 itself only reaps the programs whose
//!   parents exit before them.
//! - **Unpacking an image.** The init mounts `/dev/vda`, an empty
//!   filesystem, and unpacks the layers the host sends onto it, as root, so
//!   that their owners and modes stay theirs ([`unpack`]), then powers off.
//!   An image's files never touch the host's filesystem.

mod boot;
mod net;
mod netlink;
mod run_as;
mod serve;
mod unpack;
mod users;

pub use boot::init;
pub use run_as::run_as;
pub use serve::serve;

/// What the init reports to the host's `hello`.
const VERSION: &str = env!("CARGO_PKG_VERSION");
