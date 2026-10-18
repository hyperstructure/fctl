//! The host link: a unix socket the host reaches through `fctl-agent attach`.
//!
//! One host connection is active at a time; a new one replaces the old.
//! Frames are JSON, one per line.

use anyhow::{Context, Result};
use fctl_proto::agent::{AgentToHost, HostToAgent, MAX_FRAME_BYTES};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

pub enum LinkEvent {
    /// A host attached; frames for it go to this sender.
    Connected(u64, mpsc::UnboundedSender<String>),
    Frame(u64, HostToAgent),
    Disconnected(u64),
}

/// What `fctl-agent serve` gets for the host socket.
pub enum Claim {
    /// The socket is this supervisor's, with the lock that tells the next
    /// `serve` so for as long as it is held.
    Serving(UnixListener, Option<File>),
    /// Another supervisor serves it.
    Taken,
}

/// Claims the host socket. The host starts `fctl-agent serve` whenever the
/// agent may be gone, so usually another one serves it. The lock on
/// `agent.lock` says so without connecting to the socket: the supervisor
/// would take that connection for a host link, in place of the host's.
/// Supervisors from before the lock are found by connecting, as before.
pub async fn claim(path: &Path) -> Result<Claim> {
    let lock = match lock(&path.with_extension("lock")) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => return Ok(Claim::Taken),
        Err(e) => {
            tracing::warn!("{e:#}; looking for another fctl-agent on the socket instead");
            None
        }
    };
    match bind(path).await {
        Ok(listener) => Ok(Claim::Serving(listener, lock)),
        Err(e) if e.is::<AlreadyServing>() => Ok(Claim::Taken),
        Err(e) => Err(e),
    }
}

/// An exclusive lock on `path`, or `None` while another process holds it.
fn lock(path: &Path) -> Result<Option<File>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => {
            Err(e).with_context(|| format!("locking {}", path.display()))
        }
    }
}

/// Another supervisor listens on the socket.
#[derive(Debug)]
pub struct AlreadyServing(PathBuf);

impl std::fmt::Display for AlreadyServing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "another fctl-agent is already serving {}",
            self.0.display()
        )
    }
}

impl std::error::Error for AlreadyServing {}

/// Binds the socket, removing a stale one. Fails with [`AlreadyServing`] if
/// another supervisor is listening there.
pub async fn bind(path: &Path) -> Result<UnixListener> {
    if path.exists() {
        if UnixStream::connect(path).await.is_ok() {
            return Err(AlreadyServing(path.to_owned()).into());
        }
        std::fs::remove_file(path).ok();
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener =
        UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
    restrict(path);
    Ok(listener)
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_: &Path) {}

pub async fn accept_loop(listener: UnixListener, events: mpsc::UnboundedSender<LinkEvent>) {
    let mut next_id = 0u64;
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        next_id += 1;
        let id = next_id;
        // Announced here, in the order they connect, so the newest
        // connection is the one that stays current.
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        if events.send(LinkEvent::Connected(id, tx)).is_err() {
            return;
        }
        let events = events.clone();
        tokio::spawn(async move {
            let (read, mut write) = stream.into_split();
            let writer = tokio::spawn(async move {
                while let Some(mut line) = rx.recv().await {
                    line.push('\n');
                    if write.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                }
                let _ = write.shutdown().await;
            });
            let mut reader = BufReader::with_capacity(64 * 1024, read);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) if n > MAX_FRAME_BYTES => break,
                    Ok(_) => {}
                }
                let text = line.trim();
                if text.is_empty() {
                    continue;
                }
                match serde_json::from_str::<HostToAgent>(text) {
                    Ok(frame) => {
                        if events.send(LinkEvent::Frame(id, frame)).is_err() {
                            break;
                        }
                    }
                    Err(e) => tracing::warn!("ignoring a malformed frame from the host: {e}"),
                }
            }
            let _ = events.send(LinkEvent::Disconnected(id));
            writer.abort();
        });
    }
}

pub fn encode(msg: &AgentToHost) -> Option<String> {
    serde_json::to_string(msg).ok()
}

/// `fctl-agent attach`: copies stdin to the socket and the socket to stdout.
pub async fn attach(path: &Path) -> Result<()> {
    let mut stream = None;
    // The supervisor may still be starting.
    for _ in 0..100 {
        match UnixStream::connect(path).await {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
    let stream = stream.with_context(|| format!("no fctl-agent is serving {}", path.display()))?;
    let (mut sock_read, mut sock_write) = stream.into_split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let up = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut stdin, &mut sock_write).await;
        let _ = sock_write.shutdown().await;
    });
    let _ = tokio::io::copy(&mut sock_read, &mut stdout).await;
    let _ = stdout.flush().await;
    up.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_second_supervisor_leaves_the_socket_alone() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let socket = dir.path().join("agent.sock");
        let Claim::Serving(listener, Some(lock)) = claim(&socket).await? else {
            anyhow::bail!("the first supervisor should get the socket and the lock");
        };
        // The host starts another in case the first is gone.
        assert!(matches!(claim(&socket).await?, Claim::Taken));
        // Without connecting: the first would take that for a host link.
        let connected = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(connected.is_err(), "the second supervisor connected");

        // Once the first is gone, the next one serves, over its stale socket.
        drop((listener, lock));
        assert!(socket.exists());
        // Processes that other tests start hold copies of the first's lock and
        // socket until they exec, so it can look taken a moment longer.
        for _ in 0..100 {
            match claim(&socket).await? {
                Claim::Serving(_, Some(_)) => return Ok(()),
                Claim::Serving(_, None) => anyhow::bail!("the next supervisor got no lock"),
                Claim::Taken => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        anyhow::bail!("the next supervisor should get the socket once the first is gone")
    }

    #[tokio::test]
    async fn finds_supervisors_from_before_the_lock_on_the_socket() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let socket = dir.path().join("agent.sock");
        let _older = bind(&socket).await?;
        assert!(matches!(claim(&socket).await?, Claim::Taken));
        Ok(())
    }
}
