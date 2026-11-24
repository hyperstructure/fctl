//! Whether a sandbox reaches the network: what `fctl-agent reach` answers
//! the host, which asks sandboxes of Apple's `container` before fctl-agent
//! starts in them, and while they run (crates/fctl-host/src/driver/network.rs).

use std::net::SocketAddr;

/// Where the host asks sandboxes to reach: GitHub, which sessions clone
/// their repositories from, over HTTPS.
pub const TARGET: &str = "github.com:443";

/// What a sandbox found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// The name resolved, and a connection to it opened.
    Reached,
    /// It didn't, and why: `github.com didn't resolve: …`.
    Unreachable(String),
}

const REACHED: &str = "reached ";
const UNREACHABLE: &str = "unreachable: ";

impl Reach {
    /// The line `fctl-agent reach` prints for `target`.
    pub fn line(&self, target: &str) -> String {
        match self {
            Self::Reached => format!("{REACHED}{target}"),
            Self::Unreachable(why) => format!("{UNREACHABLE}{}", one_line(why)),
        }
    }

    /// What `fctl-agent reach` printed. `None` for anything else: what an
    /// fctl-agent without `reach` says, or a sandbox that ran nothing.
    pub fn parse(out: &str) -> Option<Self> {
        let line = out.lines().map(str::trim).find(|l| !l.is_empty())?;
        if line.starts_with(REACHED) {
            return Some(Self::Reached);
        }
        let why = line.strip_prefix(UNREACHABLE.trim_end())?.trim();
        Some(Self::Unreachable(if why.is_empty() {
            "no reason given".into()
        } else {
            why.to_owned()
        }))
    }
}

/// `why` on one line, as `fctl-agent reach` prints it.
fn one_line(why: &str) -> String {
    why.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The order to try `addrs` in: IPv4 first, as a sandbox may have no
/// route for IPv6, where a connection fails or waits for its time limit.
pub fn order(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let mut ordered: Vec<SocketAddr> = Vec::with_capacity(addrs.len());
    for addr in addrs {
        if !ordered.contains(&addr) {
            ordered.push(addr);
        }
    }
    ordered.sort_by_key(SocketAddr::is_ipv6);
    ordered
}

/// `github.com` of `github.com:443`.
pub fn host_of(target: &str) -> &str {
    target
        .rsplit_once(':')
        .map_or(target, |(host, _)| host)
        .trim_start_matches('[')
        .trim_end_matches(']')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_reads_what_fctl_agent_says() {
        for reach in [
            Reach::Reached,
            Reach::Unreachable("github.com didn't resolve: Try again".into()),
        ] {
            assert_eq!(Reach::parse(&reach.line(TARGET)), Some(reach));
        }
        // Its reasons stay on one line.
        let r = Reach::Unreachable("connecting failed:\n  timed out".into());
        assert_eq!(r.line(TARGET), "unreachable: connecting failed: timed out");
        assert_eq!(
            Reach::parse("\n  reached github.com:443\n"),
            Some(Reach::Reached)
        );
        assert_eq!(
            Reach::parse("unreachable: "),
            Some(Reach::Unreachable("no reason given".into()))
        );
        // An fctl-agent without `reach`, or a sandbox that ran nothing,
        // says neither.
        for other in [
            "",
            "error: unrecognized subcommand 'reach'",
            "Usage: fctl-agent <COMMAND>",
        ] {
            assert_eq!(Reach::parse(other), None, "{other}");
        }
    }

    #[test]
    fn ipv4_comes_first() -> Result<(), std::net::AddrParseError> {
        let v6: SocketAddr = "[2606:50c0:8000::153]:443".parse()?;
        let v4: SocketAddr = "140.82.121.4:443".parse()?;
        let other: SocketAddr = "140.82.121.3:443".parse()?;
        assert_eq!(order(vec![v6, v4, v6, other, v4]), [v4, other, v6]);
        Ok(())
    }

    #[test]
    fn names_the_host_of_a_target() {
        assert_eq!(host_of(TARGET), "github.com");
        assert_eq!(host_of("[::1]:443"), "::1");
        assert_eq!(host_of("github.com"), "github.com");
    }
}
