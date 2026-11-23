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

/// Ethernet's types for IPv4, IPv6 and an 802.1Q tag.
const ETHER_IPV4: u16 = 0x0800;
const ETHER_IPV6: u16 = 0x86dd;
const ETHER_VLAN: u16 = 0x8100;

/// The type of what an Ethernet frame carries, and where that starts,
/// after the frame's 802.1Q tag if it has one.
fn ether_type(frame: &[u8]) -> Option<(u16, usize)> {
    let kind = u16::from_be_bytes(frame.get(12..14)?.try_into().ok()?);
    if kind == ETHER_VLAN {
        let inner = u16::from_be_bytes(frame.get(16..18)?.try_into().ok()?);
        return Some((inner, 18));
    }
    Some((kind, 14))
}

/// Where the IPv4 or IPv6 packet in an Ethernet frame goes. `None` for
/// other frames (ARP, say), and for one too short to say.
pub fn destination(frame: &[u8]) -> Option<IpAddr> {
    let (kind, at) = ether_type(frame)?;
    let packet = frame.get(at..)?;
    let version = packet.first()? >> 4;
    match (kind, version) {
        (ETHER_IPV4, 4) => {
            let dst: [u8; 4] = packet.get(16..20)?.try_into().ok()?;
            Some(IpAddr::from(dst))
        }
        (ETHER_IPV6, 6) => {
            let dst: [u8; 16] = packet.get(24..40)?.try_into().ok()?;
            Some(IpAddr::from(dst))
        }
        _ => None,
    }
}

/// The metadata address an Ethernet frame goes to, if it goes to one.
pub fn blocked(frame: &[u8]) -> Option<IpAddr> {
    destination(frame).filter(|ip| is_metadata(*ip))
}

/// The answer to `frame`, an IPv4 packet that won't be carried to a
/// metadata address: an ICMP "destination unreachable, communication
/// administratively prohibited", as from `gateway`, so that what sent it
/// fails at once ("No route to host") rather than waiting out its
/// timeout. `None` for what gets no answer: IPv6, which is dropped alone,
/// fragments past the first, ICMP other than echo requests, and other
/// protocols than TCP and UDP.
pub fn refusal(frame: &[u8], gateway: Ipv4Addr) -> Option<Vec<u8>> {
    let (kind, at) = ether_type(frame)?;
    if kind != ETHER_IPV4 {
        return None;
    }
    let packet = frame.get(at..)?;
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

    #[test]
    fn reads_where_a_frame_goes() {
        let to_metadata = frame(ETHER_IPV4, &ipv4(6, [169, 254, 169, 254], 0x4000, &syn()));
        assert_eq!(
            destination(&to_metadata),
            Some(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)))
        );
        assert_eq!(blocked(&to_metadata), destination(&to_metadata));
        let elsewhere = frame(ETHER_IPV4, &ipv4(6, [140, 82, 112, 3], 0x4000, &syn()));
        assert!(destination(&elsewhere).is_some());
        assert_eq!(blocked(&elsewhere), None);

        // Behind an 802.1Q tag.
        let mut tagged = GATEWAY_MAC.to_vec();
        tagged.extend_from_slice(&GUEST_MAC);
        tagged.extend_from_slice(&[0x81, 0x00, 0, 5, 0x08, 0x00]);
        tagged.extend(ipv4(17, [169, 254, 169, 254], 0, &[0; 8]));
        assert!(blocked(&tagged).is_some());

        // IPv6.
        let mut v6 = vec![0x60, 0, 0, 0, 0, 0, 6, 64];
        v6.extend_from_slice(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0f]);
        v6.extend_from_slice(&Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254).octets());
        let v6 = frame(ETHER_IPV6, &v6);
        assert!(blocked(&v6).is_some());

        // ARP, and frames too short to say, go nowhere.
        assert_eq!(destination(&frame(0x0806, &[0; 28])), None);
        assert_eq!(destination(&to_metadata[..30]), None);
        assert_eq!(destination(&[]), None);
        // An IPv4 type with an IPv6 packet in it says nothing.
        assert_eq!(destination(&frame(ETHER_IPV4, &[0x60; 40])), None);
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
