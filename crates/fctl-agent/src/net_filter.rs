//! `fctl-agent net-filter`, on a Firecracker host: carries a VM's network
//! between its link, on stdin (the VM's vsock connection), and `passt`, on
//! stdout, in passt's frames (each after its length: 4 bytes, big endian),
//! and drops the VM's frames to cloud metadata services
//! (`fctl_domain::metadata`), but DNS, which Google Cloud's metadata server
//! answers as the machine's nameserver. An IPv4 one gets an ICMP
//! "communication administratively prohibited" back, so that what sent it
//! fails at once rather than waiting out its timeout. Frames whose type
//! says IP but that don't read as IP, and tagged ones, go nowhere.
//!
//! In the VM its root can change anything, routes too; this runs on the
//! host, out of its reach. The host starts it between the VM and passt
//! (crates/fctl-host/src/driver/firecracker/vm.rs, `link_network`): passt
//! exits once its end closes, and this once either end does, and the host
//! links the VM again.

use anyhow::{Context, Result};
use fctl_domain::metadata::{Verdict, refusal, verdict};
use fctl_proto::vm::GATEWAY;
use std::collections::HashSet;
use std::io::{self, BufReader, Read, Write};
use std::net::Ipv4Addr;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, PoisonError};

/// The longest frame passt's protocol carries, and then some.
const MAX_FRAME: usize = 65_536 + 18;

pub fn run() -> Result<()> {
    let vm = std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .context("the VM's link, on stdin")?;
    let passt = std::io::stdout()
        .as_fd()
        .try_clone_to_owned()
        .context("passt's, on stdout")?;
    let why = relay(UnixStream::from(vm), UnixStream::from(passt));
    tracing::info!("the VM's network link ended: {why}");
    Ok(())
}

/// Carries frames both ways, until either side closes or fails, and says
/// why.
pub fn relay(vm: UnixStream, passt: UnixStream) -> io::Error {
    let (to_vm, from_passt) = match (vm.try_clone(), passt.try_clone()) {
        (Ok(to_vm), Ok(from_passt)) => (Arc::new(Mutex::new(to_vm)), from_passt),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let (ended, first) = std::sync::mpsc::channel();
    {
        let ended = ended.clone();
        let to_vm = to_vm.clone();
        std::thread::spawn(move || {
            let _ = ended.send(up(vm, passt, &to_vm));
        });
    }
    std::thread::spawn(move || {
        let _ = ended.send(down(from_passt, &to_vm));
    });
    first
        .recv()
        .unwrap_or_else(|_| io::Error::other("the link's threads ended"))
}

/// The VM's frames to passt, but those to metadata services, other than
/// DNS, and those that say IP but don't read as IP
/// ([`fctl_domain::metadata::verdict`]).
fn up(vm: UnixStream, mut passt: UnixStream, to_vm: &Mutex<UnixStream>) -> io::Error {
    let mut from_vm = BufReader::with_capacity(256 * 1024, vm);
    let (mut frame, mut out) = (Vec::new(), Vec::new());
    let mut told = HashSet::new();
    let mut told_dropped = false;
    loop {
        if let Err(e) = read_frame(&mut from_vm, &mut frame) {
            return e;
        }
        match verdict(&frame) {
            Verdict::Carry => {
                if let Err(e) = write_frame(&mut passt, &frame, &mut out) {
                    return e;
                }
            }
            Verdict::Drop => {
                if !told_dropped {
                    told_dropped = true;
                    tracing::warn!(
                        "dropped a frame from the VM whose type says IP but which doesn't read as IP, or that has a tag"
                    );
                }
            }
            Verdict::Refuse(ip) => {
                if told.insert(ip) {
                    tracing::warn!(
                        "dropped the VM's packets to {ip}, a cloud metadata service (allow_cloud_metadata in host.toml lets sandboxes reach it)"
                    );
                }
                let Some(reply) = refusal(&frame, Ipv4Addr::from(GATEWAY)) else {
                    continue;
                };
                let mut vm = to_vm.lock().unwrap_or_else(PoisonError::into_inner);
                if let Err(e) = write_frame(&mut *vm, &reply, &mut out) {
                    return e;
                }
            }
        }
    }
}

/// passt's frames to the VM, as they are.
fn down(passt: UnixStream, to_vm: &Mutex<UnixStream>) -> io::Error {
    let mut from_passt = BufReader::with_capacity(256 * 1024, passt);
    let (mut frame, mut out) = (Vec::new(), Vec::new());
    loop {
        if let Err(e) = read_frame(&mut from_passt, &mut frame) {
            return e;
        }
        let mut vm = to_vm.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(e) = write_frame(&mut *vm, &frame, &mut out) {
            return e;
        }
    }
}

/// The next frame into `frame`. An error at the end: the other side closed.
fn read_frame(from: &mut impl Read, frame: &mut Vec<u8>) -> io::Result<()> {
    let mut len = [0u8; 4];
    from.read_exact(&mut len).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => io::Error::new(e.kind(), "the other side closed it"),
        _ => e,
    })?;
    let len = usize::try_from(u32::from_be_bytes(len)).unwrap_or(usize::MAX);
    if len > MAX_FRAME {
        return Err(io::Error::other(format!("a frame of {len} bytes")));
    }
    frame.resize(len, 0);
    from.read_exact(frame)
}

/// `frame` after its length, in one write.
fn write_frame(to: &mut impl Write, frame: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
    let len = u32::try_from(frame.len()).map_err(io::Error::other)?;
    out.clear();
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(frame);
    to.write_all(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// An Ethernet frame from the guest with a TCP SYN to `dst`.
    fn syn_to(dst: [u8; 4]) -> Vec<u8> {
        let mut f = vec![
            0x9a, 0x55, 0x9a, 0x55, 0x9a, 0x55, 2, 0, 0, 0, 0, 0x0f, 0x08, 0,
        ];
        f.extend_from_slice(&[0x45, 0, 0, 40, 0, 1, 0x40, 0, 64, 6, 0, 0]);
        f.extend_from_slice(&[10, 0, 2, 15]);
        f.extend_from_slice(&dst);
        f.extend_from_slice(&[
            0xa8, 0xca, 0, 80, 1, 2, 3, 4, 0, 0, 0, 0, 0x50, 0x02, 0xfa, 0xf0,
        ]);
        f.extend_from_slice(&[0, 0, 0, 0]);
        f
    }

    fn pair() -> io::Result<(UnixStream, UnixStream)> {
        let (a, b) = UnixStream::pair()?;
        b.set_read_timeout(Some(Duration::from_secs(5)))?;
        Ok((a, b))
    }

    #[test]
    fn carries_frames_and_refuses_those_to_metadata() -> Result<()> {
        let (vm, mut guest) = pair()?;
        let (ours, mut passt) = pair()?;
        std::thread::spawn(move || relay(vm, ours));
        let (mut got, mut out) = (Vec::new(), Vec::new());

        let metadata = syn_to([169, 254, 169, 254]);
        let github = syn_to([140, 82, 112, 3]);
        // DNS to the metadata server, Google Cloud's nameserver.
        let mut dns = syn_to([169, 254, 169, 254]);
        dns.splice(36..38, [0, 53]);
        // A frame of IPv4's type that isn't IPv4, to the metadata server.
        let mut odd = syn_to([169, 254, 169, 254]);
        odd[14] = 0x65;
        write_frame(&mut guest, &metadata, &mut out)?;
        write_frame(&mut guest, &odd, &mut out)?;
        write_frame(&mut guest, &github, &mut out)?;
        write_frame(&mut guest, &dns, &mut out)?;
        // passt gets the frames to GitHub and the DNS alone…
        read_frame(&mut passt, &mut got)?;
        assert_eq!(got, github);
        read_frame(&mut passt, &mut got)?;
        assert_eq!(got, dns);
        // …and the guest an ICMP "administratively prohibited".
        read_frame(&mut guest, &mut got)?;
        assert_eq!(got.get(34..36), Some(&[3u8, 13][..]));
        assert_eq!(got.get(..6), metadata.get(6..12));

        // passt's frames reach the guest as they are.
        write_frame(&mut passt, &github, &mut out)?;
        read_frame(&mut guest, &mut got)?;
        assert_eq!(got, github);
        Ok(())
    }

    #[test]
    fn ends_when_either_side_closes() -> Result<()> {
        let (vm, guest) = pair()?;
        let (ours, _passt) = pair()?;
        let relayed = std::thread::spawn(move || relay(vm, ours));
        drop(guest);
        let why = relayed
            .join()
            .map_err(|_| anyhow::anyhow!("the relay panicked"))?;
        assert_eq!(why.kind(), io::ErrorKind::UnexpectedEof);

        // A length past what passt sends ends it too.
        let (vm, mut guest) = pair()?;
        let (ours, _passt) = pair()?;
        let relayed = std::thread::spawn(move || relay(vm, ours));
        guest.write_all(&u32::MAX.to_be_bytes())?;
        let why = relayed
            .join()
            .map_err(|_| anyhow::anyhow!("the relay panicked"))?;
        assert!(why.to_string().contains("bytes"), "{why}");
        Ok(())
    }
}
