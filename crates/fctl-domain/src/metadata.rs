//! Cloud metadata services, which sandboxes must not reach.
//!
//! A cloud machine reaches its cloud's instance metadata service at a
//! link-local address, and so does whatever runs on it, sandboxes too: the
//! service hands out the machine's own credentials (an instance role's, a
//! service account's) and its user data, where boot scripts keep secrets
//! such as a runner's enrollment token. Hosts keep their sandboxes off these
//! addresses unless their owner lets them reach them
//! (`allow_cloud_metadata` in `host.toml`): Docker sandboxes with routes in
//! their network namespace that they can't take out, Firecracker VMs by
//! dropping their frames to them before `passt` carries those to the
//! network (docs/security.md#sandboxes).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Where clouds serve their machines' metadata and credentials.
pub const ADDRESSES: [IpAddr; 7] = [
    // AWS, Google Cloud, Azure, Oracle Cloud, DigitalOcean, Hetzner,
    // OpenStack and most others.
    IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
    // Amazon ECS's task credentials.
    IpAddr::V4(Ipv4Addr::new(169, 254, 170, 2)),
    // Amazon EKS Pod Identity's credentials.
    IpAddr::V4(Ipv4Addr::new(169, 254, 170, 23)),
    // Tencent Cloud.
    IpAddr::V4(Ipv4Addr::new(169, 254, 0, 23)),
    // Alibaba Cloud.
    IpAddr::V4(Ipv4Addr::new(100, 100, 100, 200)),
    // AWS's instance metadata, and EKS Pod Identity, over IPv6.
    IpAddr::V6(Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254)),
    IpAddr::V6(Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x23)),
];

/// Whether `ip` is one of [`ADDRESSES`], an IPv4 one written as IPv6
/// (`::ffff:169.254.169.254`) too.
pub fn is_metadata(ip: IpAddr) -> bool {
    ADDRESSES.contains(&ip.to_canonical())
}

/// Ethernet's types for IPv4, IPv6, and an 802.1Q or 802.1ad tag.
const ETHER_IPV4: u16 = 0x0800;
const ETHER_IPV6: u16 = 0x86dd;
const ETHER_VLAN: u16 = 0x8100;
const ETHER_QINQ: u16 = 0x88a8;

/// What the host's filter does with a frame a VM sends on its way to the
/// network (`fctl-agent net-filter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// On to the network.
    Carry,
    /// Not on: it goes to a metadata address, and an IPv4 one gets an
    /// answer that says so ([`refusal`]).
    Refuse(IpAddr),
    /// Not on, without a word: a frame too short to be Ethernet, one whose
    /// type says IPv4 or IPv6 but whose header doesn't read as such, and a
    /// tagged one, which a VM's own network has no use for. What would
    /// read a destination in them otherwise can't be known here.
    Drop,
}

/// What happens to `frame`, by its Ethernet type: an IP packet goes on
/// unless it goes to a metadata address, other than DNS, which Google
/// Cloud's metadata server answers as the machine's nameserver; what
/// carries no IP packet (ARP) goes on.
pub fn verdict(frame: &[u8]) -> Verdict {
    let Some(kind) = frame
        .get(12..14)
        .and_then(|b| <[u8; 2]>::try_from(b).ok())
        .map(u16::from_be_bytes)
    else {
        return Verdict::Drop;
    };
    let packet = frame.get(14..).unwrap_or_default();
    match kind {
        ETHER_IPV4 => ipv4_verdict(packet),
        ETHER_IPV6 => ipv6_verdict(packet),
        ETHER_VLAN | ETHER_QINQ => Verdict::Drop,
        _ => Verdict::Carry,
    }
}

fn ipv4_verdict(packet: &[u8]) -> Verdict {
    let Some(&first) = packet.first() else {
        return Verdict::Drop;
    };
    let header_len = usize::from(first & 0x0f) * 4;
    let header = packet.get(..header_len);
    let dst = packet.get(16..20).and_then(|d| <[u8; 4]>::try_from(d).ok());
    let (Some(header), Some(dst)) = (header, dst) else {
        return Verdict::Drop;
    };
    if first >> 4 != 4 || header_len < 20 {
        return Verdict::Drop;
    }
    let dst = IpAddr::from(dst);
    if !is_metadata(dst) {
        return Verdict::Carry;
    }
    let first_fragment = header
        .get(6..8)
        .is_some_and(|f| u16::from_be_bytes([f[0], f[1]]) & 0x1fff == 0);
    let transport = header.get(9).copied();
    if first_fragment && is_dns(transport, packet.get(header_len..)) {
        return Verdict::Carry;
    }
    Verdict::Refuse(dst)
}

fn ipv6_verdict(packet: &[u8]) -> Verdict {
    let dst = packet
        .get(24..40)
        .and_then(|d| <[u8; 16]>::try_from(d).ok());
    let Some(dst) = dst.filter(|_| packet.first().is_some_and(|b| b >> 4 == 6)) else {
        return Verdict::Drop;
    };
    let dst = IpAddr::from(dst);
    if !is_metadata(dst) {
        return Verdict::Carry;
    }
    // Without extension headers: DNS has no use for them.
    if is_dns(packet.get(6).copied(), packet.get(40..)) {
        return Verdict::Carry;
    }
    Verdict::Refuse(dst)
}

/// Whether a packet of the protocol `transport`, whose TCP or UDP header
/// starts `segment`, goes to port 53.
fn is_dns(transport: Option<u8>, segment: Option<&[u8]>) -> bool {
    matches!(transport, Some(6 | 17))
        && segment
            .and_then(|s| s.get(2..4))
            .is_some_and(|port| u16::from_be_bytes([port[0], port[1]]) == 53)
}

/// The answer to `frame`, an IPv4 packet that won't be carried to a
/// metadata address: an ICMP "destination unreachable, communication
/// administratively prohibited", as from `gateway`, so that what sent it
/// fails at once ("No route to host") rather than waiting out its
/// timeout. `None` for what gets no answer: IPv6, which is dropped alone,
/// fragments past the first, ICMP other than echo requests, and other
/// protocols than TCP and UDP.
pub fn refusal(frame: &[u8], gateway: Ipv4Addr) -> Option<Vec<u8>> {
    if frame.get(12..14)? != ETHER_IPV4.to_be_bytes() {
        return None;
    }
    let packet = frame.get(14..)?;
    let first = *packet.first()?;
    let header_len = usize::from(first & 0x0f) * 4;
    if first >> 4 != 4 || header_len < 20 {
        return None;
    }
    let header = packet.get(..header_len)?;
    let fragment = u16::from_be_bytes(header.get(6..8)?.try_into().ok()?);
    if fragment & 0x1fff != 0 {
        return None;
    }
    let payload = packet.get(header_len..)?;
    match header.get(9)? {
        // TCP and UDP.
        6 | 17 => {}
        // An echo request (ping): never an ICMP error, which gets none.
        1 if payload.first() == Some(&8) => {}
        _ => return None,
    }
    // Its header and the first 8 bytes after it, which name its ports.
    let quoted = packet.get(..header_len + payload.len().min(8))?;
    let total = u16::try_from(20 + 8 + quoted.len()).ok()?;
    let mut ip = Vec::with_capacity(usize::from(total));
    // Version 4 with a header of 20 bytes, internetwork control, its
    // length, no ID, don't fragment, a TTL of 64, ICMP, the checksum.
    ip.extend_from_slice(&[0x45, 0xc0]);
    ip.extend_from_slice(&total.to_be_bytes());
    ip.extend_from_slice(&[0, 0, 0x40, 0, 64, 1, 0, 0]);
    ip.extend_from_slice(&gateway.octets());
    ip.extend_from_slice(header.get(12..16)?);
    let sum = checksum(&ip);
    ip.get_mut(10..12)?.copy_from_slice(&sum);
    // Type 3, code 13, the checksum and four unused bytes, then the quote.
    let mut icmp = vec![3, 13, 0, 0, 0, 0, 0, 0];
    icmp.extend_from_slice(quoted);
    let sum = checksum(&icmp);
    icmp.get_mut(2..4)?.copy_from_slice(&sum);
    // Back to its sender, from where it went.
    let mut reply = Vec::with_capacity(14 + ip.len() + icmp.len());
    reply.extend_from_slice(frame.get(6..12)?);
    reply.extend_from_slice(frame.get(0..6)?);
    reply.extend_from_slice(&ETHER_IPV4.to_be_bytes());
    reply.extend(ip);
    reply.extend(icmp);
    Some(reply)
}

/// The Internet checksum (RFC 1071) of `bytes`.
fn checksum(bytes: &[u8]) -> [u8; 2] {
    let mut sum: u32 = bytes
        .chunks(2)
        .map(|pair| match pair {
            [high, low] => u32::from(u16::from_be_bytes([*high, *low])),
            [high] => u32::from(u16::from_be_bytes([*high, 0])),
            _ => 0,
        })
        .sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    let folded = u16::try_from(sum).unwrap_or(u16::MAX);
    (!folded).to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUEST_MAC: [u8; 6] = [0x02, 0, 0, 0, 0, 0x0f];
    const GATEWAY_MAC: [u8; 6] = [0x9a, 0x55, 0x9a, 0x55, 0x9a, 0x55];
    const GUEST: [u8; 4] = [10, 0, 2, 15];
    const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 2);

    /// An Ethernet frame from the guest with `payload` as `ether_type`.
    fn frame(ether_type: u16, payload: &[u8]) -> Vec<u8> {
        let mut f = GATEWAY_MAC.to_vec();
        f.extend_from_slice(&GUEST_MAC);
        f.extend_from_slice(&ether_type.to_be_bytes());
        f.extend_from_slice(payload);
        f
    }

    /// An IPv4 packet of `protocol` from the guest to `dst`, with
    /// `fragment` as its flags and offset, carrying `body`.
    fn ipv4(protocol: u8, dst: [u8; 4], fragment: u16, body: &[u8]) -> Vec<u8> {
        let total = u16::try_from(20 + body.len()).unwrap_or(u16::MAX);
        let mut p = vec![0x45, 0];
        p.extend_from_slice(&total.to_be_bytes());
        p.extend_from_slice(&[0x12, 0x34]);
        p.extend_from_slice(&fragment.to_be_bytes());
        p.extend_from_slice(&[64, protocol, 0, 0]);
        p.extend_from_slice(&GUEST);
        p.extend_from_slice(&dst);
        let sum = checksum(&p);
        p.splice(10..12, sum);
        p.extend_from_slice(body);
        p
    }

    /// A TCP SYN from port 43210 to port 80, with its options.
    fn syn() -> Vec<u8> {
        let mut t = vec![0xa8, 0xca, 0, 80, 1, 2, 3, 4, 0, 0, 0, 0, 0xa0, 0x02];
        t.extend_from_slice(&[0xfa, 0xf0, 0, 0, 0, 0]);
        t.extend_from_slice(&[2, 4, 5, 0xb4, 4, 2, 8, 10]);
        t.extend_from_slice(&[0; 12]);
        t
    }

    #[test]
    fn the_clouds_metadata_addresses_are_metadata() {
        for ip in [
            "169.254.169.254",
            "169.254.170.2",
            "fd00:ec2::254",
            "::ffff:169.254.169.254",
        ] {
            let ip: IpAddr = ip.parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
            assert!(is_metadata(ip), "{ip}");
        }
        for ip in [
            "169.254.169.253",
            "10.0.2.3",
            "1.1.1.1",
            "fd00:ec2::253",
            "::1",
        ] {
            let ip: IpAddr = ip.parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
            assert!(!is_metadata(ip), "{ip}");
        }
    }

    /// A UDP datagram from port 12345 to `port`.
    fn udp_to(port: u16) -> Vec<u8> {
        let mut u = vec![0x30, 0x39];
        u.extend_from_slice(&port.to_be_bytes());
        u.extend_from_slice(&[0, 8, 0, 0]);
        u
    }

    /// An IPv6 packet of `next` from the guest to `dst`, carrying `body`.
    fn ipv6(next: u8, dst: Ipv6Addr, body: &[u8]) -> Vec<u8> {
        let len = u16::try_from(body.len()).unwrap_or(u16::MAX);
        let mut p = vec![0x60, 0, 0, 0];
        p.extend_from_slice(&len.to_be_bytes());
        p.extend_from_slice(&[next, 64]);
        p.extend_from_slice(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0f]);
        p.extend_from_slice(&dst.octets());
        p.extend_from_slice(body);
        p
    }

    #[test]
    fn refuses_what_goes_to_metadata_but_dns() {
        let metadata = [169, 254, 169, 254];
        let refused = Verdict::Refuse(IpAddr::from(metadata));
        let v4 = |protocol, dst, fragment, body: &[u8]| {
            verdict(&frame(ETHER_IPV4, &ipv4(protocol, dst, fragment, body)))
        };
        assert_eq!(v4(6, metadata, 0x4000, &syn()), refused);
        assert_eq!(v4(17, metadata, 0, &udp_to(80)), refused);
        assert_eq!(v4(1, metadata, 0, &[8, 0, 0, 0, 0, 1, 0, 1]), refused);
        assert_eq!(v4(6, [140, 82, 112, 3], 0x4000, &syn()), Verdict::Carry);
        // DNS: Google Cloud's nameserver is its metadata server.
        assert_eq!(v4(17, metadata, 0, &udp_to(53)), Verdict::Carry);
        let mut tcp_dns = syn();
        tcp_dns.splice(2..4, [0, 53]);
        assert_eq!(v4(6, metadata, 0x4000, &tcp_dns), Verdict::Carry);
        // But not a later fragment, which has no ports to tell.
        assert_eq!(v4(17, metadata, 0x0010, &udp_to(53)), refused);

        // IPv6.
        let aws = Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254);
        let v6 = |next, dst, body: &[u8]| verdict(&frame(ETHER_IPV6, &ipv6(next, dst, body)));
        assert_eq!(v6(6, aws, &syn()), Verdict::Refuse(IpAddr::V6(aws)));
        assert_eq!(v6(17, aws, &udp_to(53)), Verdict::Carry);
        // An extension header first: no DNS to tell.
        assert_eq!(v6(0, aws, &udp_to(53)), Verdict::Refuse(IpAddr::V6(aws)));
        assert_eq!(v6(6, Ipv6Addr::LOCALHOST, &syn()), Verdict::Carry);
    }

    #[test]
    fn drops_what_says_ip_but_doesnt_read_as_ip() {
        // IPv4's type over a packet whose version isn't 4, with a metadata
        // address where IPv4's destination would be: whatever else reads
        // it, it goes nowhere.
        let mut odd = ipv4(6, [169, 254, 169, 254], 0x4000, &syn());
        odd[0] = 0x65;
        assert_eq!(verdict(&frame(ETHER_IPV4, &odd)), Verdict::Drop);
        // A header shorter than IPv4's least.
        let mut short = ipv4(6, [140, 82, 112, 3], 0x4000, &syn());
        short[0] = 0x44;
        assert_eq!(verdict(&frame(ETHER_IPV4, &short)), Verdict::Drop);
        // Cut short, before its destination, or before the header it names.
        let whole = frame(ETHER_IPV4, &ipv4(6, [140, 82, 112, 3], 0x4000, &syn()));
        assert_eq!(verdict(&whole[..30]), Verdict::Drop);
        let mut long_header = ipv4(6, [140, 82, 112, 3], 0x4000, &[]);
        long_header[0] = 0x4f;
        assert_eq!(verdict(&frame(ETHER_IPV4, &long_header)), Verdict::Drop);
        assert_eq!(verdict(&frame(ETHER_IPV6, &[0x60; 30])), Verdict::Drop);
        assert_eq!(verdict(&frame(ETHER_IPV6, &[0x45; 60])), Verdict::Drop);
        // Tagged frames, whatever they carry.
        let mut tagged = GATEWAY_MAC.to_vec();
        tagged.extend_from_slice(&GUEST_MAC);
        tagged.extend_from_slice(&[0x81, 0x00, 0, 5, 0x08, 0x00]);
        tagged.extend(ipv4(17, [140, 82, 112, 3], 0, &udp_to(443)));
        assert_eq!(verdict(&tagged), Verdict::Drop);
        // Too short to be Ethernet.
        assert_eq!(verdict(&[0; 13]), Verdict::Drop);
        assert_eq!(verdict(&[]), Verdict::Drop);
        // ARP carries no IP packet: it goes on.
        assert_eq!(verdict(&frame(0x0806, &[0; 28])), Verdict::Carry);
    }

    #[test]
    fn refuses_a_connection_to_metadata_with_an_icmp_error() -> Result<(), String> {
        let packet = ipv4(6, [169, 254, 169, 254], 0x4000, &syn());
        let sent = frame(ETHER_IPV4, &packet);
        let reply = refusal(&sent, GATEWAY).ok_or("no refusal")?;
        // Back to the guest's MAC, from the gateway's, as IPv4.
        assert_eq!(&reply[..6], &GUEST_MAC);
        assert_eq!(&reply[6..12], &GATEWAY_MAC);
        assert_eq!(&reply[12..14], &[0x08, 0x00]);
        let ip = &reply[14..34];
        assert_eq!(ip[0], 0x45);
        assert_eq!(
            usize::from(u16::from_be_bytes([ip[2], ip[3]])),
            reply.len() - 14
        );
        assert_eq!(ip[9], 1, "ICMP");
        assert_eq!(&ip[12..16], &GATEWAY.octets());
        assert_eq!(&ip[16..20], &GUEST);
        assert_eq!(checksum(ip), [0, 0], "the header's checksum adds up");
        let icmp = &reply[34..];
        assert_eq!((icmp[0], icmp[1]), (3, 13));
        assert_eq!(checksum(icmp), [0, 0], "the ICMP checksum adds up");
        // It quotes the header and the first 8 bytes of TCP: the ports.
        assert_eq!(&icmp[8..], &packet[..28]);
        Ok(())
    }

    #[test]
    fn answers_only_what_takes_an_answer() {
        let refused = |protocol, fragment, body: &[u8]| {
            refusal(
                &frame(
                    ETHER_IPV4,
                    &ipv4(protocol, [169, 254, 169, 254], fragment, body),
                ),
                GATEWAY,
            )
            .is_some()
        };
        assert!(refused(17, 0, &[0x30, 0x39, 0, 80, 0, 8, 0, 0]), "UDP");
        assert!(refused(1, 0, &[8, 0, 0, 0, 0, 1, 0, 1]), "an echo request");
        assert!(!refused(1, 0, &[3, 1, 0, 0, 0, 0, 0, 0]), "an ICMP error");
        assert!(!refused(6, 0x0010, &syn()), "a later fragment");
        assert!(!refused(47, 0, &[0; 8]), "GRE");
        // Short packets are quoted as far as they go.
        assert!(refused(17, 0, &[0x30, 0x39]));
        assert_eq!(refusal(&frame(0x0806, &[0; 28]), GATEWAY), None);
        assert_eq!(refusal(&[0; 20], GATEWAY), None);
    }
}
