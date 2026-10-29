//! The two routing requests the VM's network needs, over rtnetlink: a link
//! up (the loopback) and a default route. Images have no `ip` to run.

use nix::sys::socket::{
    AddressFamily, MsgFlags, NetlinkAddr, SockFlag, SockProtocol, SockType, bind, recv, send,
    socket,
};
use std::io;
use std::os::fd::AsRawFd;

const RTM_NEWLINK: u16 = 16;
const RTM_NEWROUTE: u16 = 24;
const NLMSG_ERROR: u16 = 2;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_ACK: u16 = 0x4;
const NLM_F_REPLACE: u16 = 0x100;
const NLM_F_CREATE: u16 = 0x400;
const IFF_UP: u32 = 0x1;
const AF_INET: u8 = 2;
const RT_TABLE_MAIN: u8 = 254;
const RTPROT_BOOT: u8 = 3;
const RT_SCOPE_UNIVERSE: u8 = 0;
const RTN_UNICAST: u8 = 1;
const RTA_OIF: u16 = 4;
const RTA_GATEWAY: u16 = 5;

/// A message: its header (`nlmsghdr`), then `body`.
fn message(kind: u16, flags: u16, body: &[u8]) -> Vec<u8> {
    let len = u32::try_from(16 + body.len()).unwrap_or(u32::MAX);
    let mut m = Vec::with_capacity(16 + body.len());
    m.extend_from_slice(&len.to_ne_bytes());
    m.extend_from_slice(&kind.to_ne_bytes());
    m.extend_from_slice(&(flags | NLM_F_REQUEST | NLM_F_ACK).to_ne_bytes());
    m.extend_from_slice(&1u32.to_ne_bytes());
    m.extend_from_slice(&0u32.to_ne_bytes());
    m.extend_from_slice(body);
    m
}

/// An attribute (`rtattr`), padded to 4 bytes.
fn attribute(kind: u16, data: &[u8]) -> Vec<u8> {
    let len = u16::try_from(4 + data.len()).unwrap_or(u16::MAX);
    let mut a = Vec::with_capacity(8 + data.len());
    a.extend_from_slice(&len.to_ne_bytes());
    a.extend_from_slice(&kind.to_ne_bytes());
    a.extend_from_slice(data);
    while !a.len().is_multiple_of(4) {
        a.push(0);
    }
    a
}

/// Sets the link `index` up (`ifinfomsg` with `IFF_UP` in its flags and
/// change mask).
pub fn link_up_message(index: u32) -> Vec<u8> {
    let mut body = vec![0u8, 0];
    body.extend_from_slice(&0u16.to_ne_bytes());
    body.extend_from_slice(&index.to_ne_bytes());
    body.extend_from_slice(&IFF_UP.to_ne_bytes());
    body.extend_from_slice(&IFF_UP.to_ne_bytes());
    message(RTM_NEWLINK, 0, &body)
}

/// The default route via `gateway` on the link `index` (`rtmsg` and its
/// attributes), in place of any there.
pub fn default_route_message(index: u32, gateway: [u8; 4]) -> Vec<u8> {
    let mut body = vec![
        AF_INET,
        0,
        0,
        0,
        RT_TABLE_MAIN,
        RTPROT_BOOT,
        RT_SCOPE_UNIVERSE,
        RTN_UNICAST,
    ];
    body.extend_from_slice(&0u32.to_ne_bytes());
    body.extend(attribute(RTA_GATEWAY, &gateway));
    body.extend(attribute(RTA_OIF, &index.to_ne_bytes()));
    message(RTM_NEWROUTE, NLM_F_CREATE | NLM_F_REPLACE, &body)
}

/// What the kernel's acknowledgement says: `Ok`, or the error it carries.
pub fn acknowledged(reply: &[u8]) -> io::Result<()> {
    let kind = reply.get(4..6).map(|b| u16::from_ne_bytes([b[0], b[1]]));
    let code = reply
        .get(16..20)
        .map(|b| i32::from_ne_bytes([b[0], b[1], b[2], b[3]]));
    match (kind, code) {
        (Some(NLMSG_ERROR), Some(0)) => Ok(()),
        (Some(NLMSG_ERROR), Some(e)) => Err(io::Error::from_raw_os_error(-e)),
        _ => Err(io::Error::other(
            "the kernel's answer isn't an acknowledgement",
        )),
    }
}

/// Sends `request` to the kernel and waits for its acknowledgement.
pub fn ask(request: &[u8]) -> io::Result<()> {
    let fd = socket(
        AddressFamily::Netlink,
        SockType::Raw,
        SockFlag::SOCK_CLOEXEC,
        SockProtocol::NetlinkRoute,
    )?;
    bind(fd.as_raw_fd(), &NetlinkAddr::new(0, 0))?;
    send(fd.as_raw_fd(), request, MsgFlags::empty())?;
    let mut reply = [0u8; 1024];
    let n = recv(fd.as_raw_fd(), &mut reply, MsgFlags::empty())?;
    acknowledged(&reply[..n])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(m: &[u8], at: usize) -> u32 {
        u32::from_ne_bytes([m[at], m[at + 1], m[at + 2], m[at + 3]])
    }

    #[test]
    fn a_link_comes_up() {
        let m = link_up_message(1);
        assert_eq!(m.len(), 32);
        assert_eq!(u32_at(&m, 0), 32);
        assert_eq!(&m[4..6], &RTM_NEWLINK.to_ne_bytes());
        assert_eq!(&m[6..8], &(NLM_F_REQUEST | NLM_F_ACK).to_ne_bytes());
        // ifinfomsg: family, padding, type, index, flags, change.
        assert_eq!(u32_at(&m, 20), 1);
        assert_eq!((u32_at(&m, 24), u32_at(&m, 28)), (IFF_UP, IFF_UP));
    }

    #[test]
    fn the_default_route_goes_through_the_gateway() {
        let m = default_route_message(2, [10, 0, 2, 2]);
        // Header, rtmsg, then RTA_GATEWAY and RTA_OIF of 8 bytes each.
        assert_eq!(m.len(), 16 + 12 + 8 + 8);
        assert_eq!(u32_at(&m, 0) as usize, m.len());
        assert_eq!(&m[4..6], &RTM_NEWROUTE.to_ne_bytes());
        assert_eq!(
            &m[6..8],
            &(NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_REPLACE).to_ne_bytes()
        );
        assert_eq!(&m[16..24], &[AF_INET, 0, 0, 0, 254, 3, 0, 1]);
        assert_eq!(&m[28..30], &8u16.to_ne_bytes());
        assert_eq!(&m[30..32], &RTA_GATEWAY.to_ne_bytes());
        assert_eq!(&m[32..36], &[10, 0, 2, 2]);
        assert_eq!(&m[38..40], &RTA_OIF.to_ne_bytes());
        assert_eq!(u32_at(&m, 40), 2);
    }

    #[test]
    fn reads_the_kernels_acknowledgement() {
        let mut ack = message(NLMSG_ERROR, 0, &0i32.to_ne_bytes());
        assert!(acknowledged(&ack).is_ok());
        ack.truncate(16);
        ack.extend_from_slice(&(-17i32).to_ne_bytes());
        let e = acknowledged(&ack).err();
        assert_eq!(e.and_then(|e| e.raw_os_error()), Some(17));
        assert!(acknowledged(&[0u8; 8]).is_err());
    }
}
