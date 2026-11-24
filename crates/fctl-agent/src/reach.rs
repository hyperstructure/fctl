//! `fctl-agent reach HOST:PORT`: whether the sandbox it runs in reaches the
//! network: the name resolves, as the sandbox's programs resolve names
//! (its `/etc/hosts`, then the nameservers of its `/etc/resolv.conf`), and
//! a TCP connection to it opens, within a time limit. It prints one line,
//! which `fctl_domain::reach::Reach` reads, and exits 0 either way: the
//! host, which asks (crates/fctl-host/src/driver/network.rs), takes a
//! failure to run as no answer.

use fctl_domain::reach::{self, Reach};
use std::io;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long one address may take to answer, of the time there is: an
/// address that doesn't answer leaves the others time.
const PER_ADDRESS: Duration = Duration::from_secs(5);

/// Looks up the addresses of `HOST:PORT`.
type Resolve = fn(&str) -> io::Result<Vec<SocketAddr>>;
/// Opens a TCP connection to an address within a time limit.
type Connect = fn(&SocketAddr, Duration) -> io::Result<()>;

pub fn run(target: &str, within: Duration) -> anyhow::Result<()> {
    let found = reach(target, within, resolve, connect);
    println!("{}", found.line(target));
    Ok(())
}

fn resolve(target: &str) -> io::Result<Vec<SocketAddr>> {
    Ok(target.to_socket_addrs()?.collect())
}

fn connect(addr: &SocketAddr, within: Duration) -> io::Result<()> {
    TcpStream::connect_timeout(addr, within).map(drop)
}

/// Whether `target` resolves with `resolve` and `connect` reaches one of
/// its addresses, all within `within`.
fn reach(target: &str, within: Duration, resolve: Resolve, connect: Connect) -> Reach {
    let deadline = Instant::now() + within;
    let host = reach::host_of(target);
    // A lookup has no time limit of its own: the resolver's, in
    // resolv.conf, may be longer.
    let (tx, rx) = mpsc::channel();
    let looked_up = target.to_owned();
    std::thread::spawn(move || {
        let _ = tx.send(resolve(&looked_up));
    });
    let addrs = match rx.recv_timeout(within) {
        Ok(Ok(addrs)) if !addrs.is_empty() => reach::order(addrs),
        Ok(Ok(_)) => return Reach::Unreachable(format!("{host} resolved to no address")),
        Ok(Err(e)) => return Reach::Unreachable(format!("{host} didn't resolve: {e}")),
        Err(_) => {
            return Reach::Unreachable(format!("{host} didn't resolve within {}", took(within)));
        }
    };
    let mut failed = Vec::new();
    for addr in &addrs {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match connect(addr, left.min(PER_ADDRESS)) {
            Ok(()) => return Reach::Reached,
            Err(e) => failed.push(format!("{addr}: {e}")),
        }
    }
    Reach::Unreachable(if failed.is_empty() {
        format!(
            "{host} resolved, but no connection to it opened within {}",
            took(within)
        )
    } else {
        format!("connecting to {host} failed ({})", failed.join("; "))
    })
}

/// `10 seconds`, `1 second`, `200 ms`.
fn took(d: Duration) -> String {
    match d.as_secs() {
        0 => format!("{} ms", d.as_millis()),
        1 => "1 second".into(),
        s => format!("{s} seconds"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    const WITHIN: Duration = Duration::from_secs(2);

    #[test]
    fn reaches_what_answers() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let target = listener.local_addr()?.to_string();
        assert_eq!(reach(&target, WITHIN, resolve, connect), Reach::Reached);

        // What refuses the connection isn't reached.
        drop(listener);
        let Reach::Unreachable(why) = reach(&target, WITHIN, resolve, connect) else {
            anyhow::bail!("reached a closed port");
        };
        assert!(
            why.starts_with("connecting to 127.0.0.1 failed (127.0.0.1:"),
            "{why}"
        );
        Ok(())
    }

    #[test]
    fn says_when_a_name_doesnt_resolve() {
        fn again(_: &str) -> io::Result<Vec<SocketAddr>> {
            Err(io::Error::other(
                "failed to lookup address information: Try again",
            ))
        }
        assert_eq!(
            reach(reach::TARGET, WITHIN, again, connect),
            Reach::Unreachable(
                "github.com didn't resolve: failed to lookup address information: Try again".into()
            )
        );
        fn nothing(_: &str) -> io::Result<Vec<SocketAddr>> {
            Ok(Vec::new())
        }
        assert_eq!(
            reach(reach::TARGET, WITHIN, nothing, connect),
            Reach::Unreachable("github.com resolved to no address".into())
        );
    }

    #[test]
    fn a_lookup_that_hangs_runs_out_of_time() {
        fn hangs(_: &str) -> io::Result<Vec<SocketAddr>> {
            std::thread::sleep(Duration::from_secs(30));
            Ok(Vec::new())
        }
        let started = Instant::now();
        assert_eq!(
            reach(reach::TARGET, Duration::from_millis(200), hangs, connect),
            Reach::Unreachable("github.com didn't resolve within 200 ms".into())
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn tries_each_address_ipv4_first() -> anyhow::Result<()> {
        fn both(_: &str) -> io::Result<Vec<SocketAddr>> {
            Ok(vec![
                SocketAddr::from(([0x2606, 0x50c0, 0x8000, 0, 0, 0, 0, 0x153], 443)),
                SocketAddr::from(([140, 82, 121, 4], 443)),
            ])
        }
        // Only IPv6 answers: IPv4 is tried first, and fails.
        fn v6_only(addr: &SocketAddr, _: Duration) -> io::Result<()> {
            match addr {
                SocketAddr::V6(_) => Ok(()),
                SocketAddr::V4(_) => Err(io::Error::from(io::ErrorKind::TimedOut)),
            }
        }
        assert_eq!(reach(reach::TARGET, WITHIN, both, v6_only), Reach::Reached);
        fn none(_: &SocketAddr, _: Duration) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::ConnectionRefused))
        }
        let Reach::Unreachable(why) = reach(reach::TARGET, WITHIN, both, none) else {
            anyhow::bail!("reached what refuses");
        };
        let v4 = why.find("140.82.121.4:443");
        let v6 = why.find("[2606:50c0:8000::153]:443");
        assert!(v4.is_some() && v6.is_some() && v4 < v6, "{why}");
        Ok(())
    }

    #[test]
    fn says_how_long_it_waited() {
        assert_eq!(took(Duration::from_millis(200)), "200 ms");
        assert_eq!(took(Duration::from_secs(1)), "1 second");
        assert_eq!(took(Duration::from_secs(10)), "10 seconds");
    }
}
