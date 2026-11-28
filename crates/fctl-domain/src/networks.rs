//! The networks an instance takes sign-ins and requests from
//! (`InstanceSettings::networks`, #618): IP ranges such as an office's or
//! a VPN's, in CIDR notation. None, the default, takes them from
//! anywhere.
//!
//! People's browsers, CLIs and access tokens, admin tokens and service
//! accounts' secrets are held to them; runners' connections, webhooks
//! (GitHub's, Linear's), published links and the identity tokens' keys
//! aren't, as they never sign anyone in. The primary instance owner signs
//! in from anywhere, in a browser or with the CLI: the instance's way in
//! when its network changes. Their access tokens and admin tokens are held
//! to them as everyone's are.

use fctl_proto::ApiError;
use std::net::IpAddr;

/// The most ranges an instance names.
pub const MAX_NETWORKS: usize = 100;

/// A range of addresses: an address and how many of its leading bits a
/// member shares with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Network {
    addr: IpAddr,
    prefix: u8,
}

impl Network {
    /// A range as an admin types it: `10.0.0.0/8`, `2001:db8::/32`, or an
    /// address alone, which is a range of one. Its host bits must be zero,
    /// so that what's saved says what it takes.
    pub fn parse(input: &str) -> Result<Self, String> {
        let s = input.trim();
        let bad = || format!("{s} isn't a network: give one like 203.0.113.0/24 or 2001:db8::/32");
        let (addr, prefix) = match s.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (s, None),
        };
        let addr: IpAddr = addr.trim().parse().map_err(|_| bad())?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p.trim().parse::<u8>().map_err(|_| bad())?,
            None => max,
        };
        if prefix > max {
            return Err(bad());
        }
        let net = Self { addr, prefix };
        if net.masked(addr) != addr {
            return Err(format!(
                "{s} has bits past its prefix: did you mean {}?",
                Self {
                    addr: net.masked(addr),
                    prefix
                }
            ));
        }
        Ok(net)
    }

    /// `ip` with the bits past this range's prefix cleared.
    fn masked(&self, ip: IpAddr) -> IpAddr {
        match ip {
            IpAddr::V4(v4) => {
                let bits = u32::from(v4);
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                IpAddr::V4((bits & mask).into())
            }
            IpAddr::V6(v6) => {
                let bits = u128::from(v6);
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                IpAddr::V6((bits & mask).into())
            }
        }
    }

    /// Whether `ip` is in it. An IPv4 address mapped into IPv6
    /// (`::ffff:203.0.113.7`) counts as the IPv4 one.
    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
            v4 => v4,
        };
        ip.is_ipv4() == self.addr.is_ipv4() && self.masked(ip) == self.addr
    }
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

/// `networks` as the instance keeps them, from what an admin saves: each
/// checked and written as `address/prefix`, once each, [`MAX_NETWORKS`]
/// at most.
pub fn check(networks: &[String]) -> Result<Vec<String>, ApiError> {
    let mut out: Vec<String> = Vec::new();
    for typed in networks.iter().filter(|n| !n.trim().is_empty()) {
        let net = Network::parse(typed).map_err(|m| ApiError::validation("networks", m))?;
        let s = net.to_string();
        if !out.contains(&s) {
            out.push(s);
        }
    }
    if out.len() > MAX_NETWORKS {
        return Err(ApiError::validation(
            "networks",
            format!("Name {MAX_NETWORKS} networks at most: a wider one takes in several"),
        ));
    }
    Ok(out)
}

/// Whether `networks` (as kept) take a request from `ip`: they name none,
/// or one holds it. An address that isn't one, or none at all, is in none
/// of them.
pub fn admits(networks: &[String], ip: Option<&str>) -> bool {
    if networks.is_empty() {
        return true;
    }
    let Some(ip) = ip.and_then(|i| i.trim().parse::<IpAddr>().ok()) else {
        return false;
    };
    networks
        .iter()
        .filter_map(|n| Network::parse(n).ok())
        .any(|n| n.contains(ip))
}

/// Whether saving `next` in place of `current` takes requests from
/// anywhere `current` didn't: from anywhere at all, or from a network it
/// didn't name. Such a change takes confirming it's you.
pub fn widens(current: &[String], next: &[String]) -> bool {
    if current.is_empty() {
        return false;
    }
    let read = |n: &String| Network::parse(n).map_or_else(|_| n.clone(), |n| n.to_string());
    let was: Vec<String> = current.iter().map(read).collect();
    let next: Vec<String> = next
        .iter()
        .filter(|n| !n.trim().is_empty())
        .map(read)
        .collect();
    next.is_empty() || next.iter().any(|n| !was.contains(n))
}

/// What a request from outside `networks` answers.
pub fn refusal(ip: Option<&str>) -> String {
    format!(
        "This instance takes sign-ins and requests only from its organization's networks, and {} isn't in one: connect to one (a VPN, say), then try again.",
        ip.map_or_else(|| "this request's address".to_owned(), |i| i.to_owned())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nets(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn ranges_read_as_admins_type_them() {
        assert_eq!(
            check(&nets(&[
                " 10.0.0.0/8",
                "203.0.113.7",
                "2001:DB8::/32",
                "10.0.0.0/8",
                ""
            ]))
            .ok(),
            Some(nets(&["10.0.0.0/8", "203.0.113.7/32", "2001:db8::/32"]))
        );
        for bad in [
            "10.0.0/8",
            "10.0.0.0/33",
            "acme.com",
            "2001:db8::/129",
            "10.0.0.1/8",
        ] {
            assert!(check(&nets(&[bad])).is_err(), "{bad}");
        }
        // What a mistyped one meant.
        assert!(
            check(&nets(&["10.1.2.3/8"]))
                .err()
                .is_some_and(|e| e.message.contains("did you mean 10.0.0.0/8"))
        );
        let many: Vec<String> = (0..=MAX_NETWORKS)
            .map(|i| format!("10.0.{i}.0/24"))
            .collect();
        assert!(check(&many).is_err());
        assert_eq!(check(&[]).ok(), Some(vec![]));
    }

    #[test]
    fn requests_from_inside_a_range_are_taken() {
        let ranges = nets(&["10.0.0.0/8", "203.0.113.7/32", "2001:db8::/32"]);
        for inside in ["10.1.2.3", "203.0.113.7", "2001:db8::1", "::ffff:10.9.9.9"] {
            assert!(admits(&ranges, Some(inside)), "{inside}");
        }
        for outside in ["11.0.0.1", "203.0.113.8", "2001:db9::1", "not an address"] {
            assert!(!admits(&ranges, Some(outside)), "{outside}");
        }
        // Without an address, only an instance that names none takes it.
        assert!(!admits(&ranges, None));
        assert!(admits(&[], None));
        assert!(admits(&[], Some("198.51.100.1")));
        // Everything, as wide as ranges go.
        assert!(admits(&nets(&["0.0.0.0/0"]), Some("198.51.100.1")));
        assert!(!admits(&nets(&["0.0.0.0/0"]), Some("2001:db8::1")));
    }
}
