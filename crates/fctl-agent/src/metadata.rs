//! `fctl-agent keep-off-metadata`: makes the cloud metadata services
//! (`fctl_domain::metadata::ADDRESSES`) unreachable from the network
//! namespace it runs in, with routes of the kind `ip route add
//! unreachable` makes.
//!
//! The Docker driver runs it in a helper container that shares a sandbox's
//! network namespace and alone has `NET_ADMIN` there, from the host's own
//! fctl-agent (crates/fctl-host/src/driver/container.rs,
//! `keep_off_metadata`). The sandbox has neither `NET_ADMIN` nor
//! `NET_RAW`, so not even its root takes the routes out or sends around
//! them.

use crate::netlink;
use anyhow::{Context, Result};
use fctl_domain::metadata::ADDRESSES;

pub fn keep_off() -> Result<()> {
    let mut unreachable = Vec::new();
    for ip in ADDRESSES {
        match netlink::ask(&netlink::unreachable_route_message(ip)) {
            Ok(()) => unreachable.push(ip.to_string()),
            // Without IPv6 in the namespace, nothing is reached over it.
            Err(_) if ip.is_ipv6() && !ipv6_on() => {}
            Err(e) => return Err(e).with_context(|| format!("making {ip} unreachable")),
        }
    }
    println!(
        "Cloud metadata services are unreachable from this network: {}",
        unreachable.join(", ")
    );
    Ok(())
}

/// Whether IPv6 is on in this network namespace.
fn ipv6_on() -> bool {
    std::fs::read_to_string("/proc/sys/net/ipv6/conf/all/disable_ipv6")
        .is_ok_and(|off| off.trim() == "0")
}
