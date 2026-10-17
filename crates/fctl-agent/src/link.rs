//! The host link: a unix socket the host reaches through `fctl-agent attach`.
//!
//! One host connection is active at a time; a new one replaces the old.
//! Frames are JSON, one per line.

use anyhow::{Context, Result};
use fctl_proto::agent::{AgentToHost, HostToAgent, MAX_FRAME_BYTES};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

pub enum LinkEvent {
    /// A host attached; frames for it go to this sender.
    Connected(u64, mpsc::UnboundedSender<String>),
    Frame(u64, HostToAgent),
    Disconnected(u64),
}

/// Binds the socket, removing a stale one. Fails if another supervisor is
/// listening there.
pub async fn bind(path: &Path) -> Result<UnixListener> {
    if path.exists() {
        if UnixStream::connect(path).await.is_ok() {
            anyhow::bail!("another fctl-agent is already serving {}", path.display());
        }
        std::fs::remove_file(path).ok();
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
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
        let events = events.clone();
        tokio::spawn(async move {
            let (read, mut write) = stream.into_split();
            let (tx, mut rx) = mpsc::unbounded_channel::<String>();
            if events.send(LinkEvent::Connected(id, tx)).is_err() {
                return;
            }
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
