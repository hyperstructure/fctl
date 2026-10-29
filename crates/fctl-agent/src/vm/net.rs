//! The VM's network: a TAP device, `eth0`, whose frames travel over the
//! vsock to the host's `passt`, which carries them to the network as the
//! host's own connections. The host needs no bridge, TAP or root for it,
//! and each VM's network is its own.

use super::netlink;
use fctl_proto::vm::{ADDRESS, GATEWAY, MTU, NET_PORT, PREFIX};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tun::Tun;
use tokio_vsock::{OwnedReadHalf, OwnedWriteHalf, VMADDR_CID_HOST, VsockAddr, VsockStream};

/// The longest frame: the MTU, an Ethernet header and a VLAN tag.
const MAX_FRAME: usize = MTU as usize + 18;

/// Brings the network up and keeps it linked to the host, connecting again
/// when the link closes (the host restarts `passt`, say).
pub async fn run() {
    if let Err(e) = netlink::ask(&netlink::link_up_message(1)) {
        tracing::warn!("bringing the loopback up: {e}");
    }
    let tap = match tap() {
        Ok(tap) => Arc::new(tap),
        Err(e) => {
            tracing::error!("the VM has no network: {e}");
            return;
        }
    };
    let mut wait = Duration::from_secs(1);
    loop {
        match VsockStream::connect(VsockAddr::new(VMADDR_CID_HOST, NET_PORT)).await {
            Ok(stream) => {
                tracing::info!("the network is linked to the host");
                wait = Duration::from_secs(1);
                let e = forward(&tap, stream).await;
                tracing::warn!("the network's link to the host closed: {e}");
            }
            Err(e) => tracing::debug!("linking the network to the host: {e}"),
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_secs(10));
    }
}

/// `eth0`, with its address, and the default route through it.
fn tap() -> std::io::Result<Tun> {
    let mask = u32::MAX
        .checked_shl(32 - u32::from(PREFIX))
        .unwrap_or_default();
    let tun = Tun::builder()
        .name("eth0")
        .tap()
        .mtu(i32::try_from(MTU).unwrap_or(1500))
        .address(Ipv4Addr::from(ADDRESS))
        .netmask(Ipv4Addr::from(mask))
        .up()
        .build()
        .map_err(std::io::Error::other)?
        .pop()
        .ok_or_else(|| std::io::Error::other("no queue for eth0"))?;
    let index = nix::net::if_::if_nametoindex("eth0")?;
    netlink::ask(&netlink::default_route_message(index, GATEWAY))?;
    Ok(tun)
}

/// Carries frames both ways until either side fails, and says why.
async fn forward(tap: &Arc<Tun>, stream: VsockStream) -> std::io::Error {
    let (mut from_host, mut to_host) = stream.into_split();
    let result = tokio::select! {
        r = up(tap, &mut to_host) => r,
        r = down(tap, &mut from_host) => r,
    };
    match result {
        Err(e) => e,
        Ok(()) => std::io::Error::other("closed"),
    }
}

/// The VM's frames to the host, each after its length, in one write.
async fn up(tap: &Tun, to_host: &mut OwnedWriteHalf) -> std::io::Result<()> {
    let mut buf = vec![0u8; 4 + MAX_FRAME];
    loop {
        let n = tap.recv(&mut buf[4..]).await?;
        let len = u32::try_from(n).unwrap_or(u32::MAX).to_be_bytes();
        buf[..4].copy_from_slice(&len);
        to_host.write_all(&buf[..4 + n]).await?;
    }
}

/// The host's frames to the VM.
async fn down(tap: &Tun, from_host: &mut OwnedReadHalf) -> std::io::Result<()> {
    let mut buf = vec![0u8; MAX_FRAME];
    loop {
        let len = from_host.read_u32().await? as usize;
        if len > buf.len() {
            return Err(std::io::Error::other(format!(
                "a frame of {len} bytes, more than the MTU allows"
            )));
        }
        from_host.read_exact(&mut buf[..len]).await?;
        tap.send_all(&buf[..len]).await?;
    }
}
