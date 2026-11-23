//! Session snapshots through the host (docs/architecture.md#session-snapshots):
//! the parts fctl-agent hands over as its sandbox stops, spooled to a file
//! and uploaded to the control plane before the run is said to have ended,
//! and the parts of a snapshot fetched for an agent that restores one.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fctl_client::Api;
use fctl_proto::SessionId;
use fctl_proto::agent::{HostToAgent, SnapshotSaved};
use fctl_proto::api::SnapshotCommit;
use fctl_proto::runner::{SNAPSHOT_PART_BYTES, SnapshotRef};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;

/// How long the host waits for a snapshot's upload before it says the run
/// ended anyway.
pub const UPLOAD_LIMIT: Duration = Duration::from_secs(10 * 60);

/// How long a stop waits for an agent that saves a snapshot as it stops,
/// rather than the minute others get: its turn's grace, its push, then
/// the snapshot.
pub const STOP_SAVING: Duration = Duration::from_secs(3 * 60);

/// How long a stop waits after each part of a snapshot, while they come.
pub const STOP_PER_PART: Duration = Duration::from_secs(60);

/// The longest a stop waits for a snapshot, however its parts come.
pub const STOP_MOST: Duration = Duration::from_secs(15 * 60);

/// A snapshot whose parts come from the agent, spooled to a file, at most
/// `limit` bytes: an agent runs what the session runs, which mustn't fill
/// the disk the runner's other sandboxes share.
pub struct Spool {
    pub id: String,
    path: PathBuf,
    file: std::fs::File,
    next: u32,
    bytes: u64,
    limit: u64,
}

impl Spool {
    /// Starts spooling snapshot `id` of `session` under `data_dir`, for a
    /// snapshot of at most `max_bytes`, as the session's spec allows.
    pub fn start(
        data_dir: &Path,
        session: &SessionId,
        id: &str,
        max_bytes: u64,
    ) -> std::io::Result<Self> {
        let dir = data_dir.join("snapshots");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}-{id}", session.as_str()));
        Ok(Self {
            id: id.to_owned(),
            file: std::fs::File::create(&path)?,
            path,
            next: 0,
            bytes: 0,
            limit: max_bytes.saturating_add(SNAPSHOT_PART_BYTES as u64),
        })
    }

    /// Adds part `n`, base64, which must be the next.
    pub fn add(&mut self, n: u32, data: &str) -> Result<(), String> {
        if n != self.next {
            return Err(format!("part {n} came where part {} was due", self.next));
        }
        // As many parts as its size takes, none of them empty: an agent
        // only sends a part with something in it.
        if u64::from(n) > self.limit / SNAPSHOT_PART_BYTES as u64 {
            return Err(format!("part {n} is more parts than its size takes"));
        }
        let bytes = STANDARD
            .decode(data.as_bytes())
            .map_err(|e| format!("part {n} isn't base64: {e}"))?;
        if bytes.is_empty() {
            return Err(format!("part {n} is empty"));
        }
        self.bytes += bytes.len() as u64;
        if self.bytes > self.limit {
            return Err(format!(
                "it passed the {} bytes its spec allows",
                self.limit - SNAPSHOT_PART_BYTES as u64
            ));
        }
        self.file
            .write_all(&bytes)
            .map_err(|e| format!("spooling it failed: {e}"))?;
        self.next += 1;
        Ok(())
    }

    /// Forgets it, and its file.
    pub fn discard(self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Uploads a spooled snapshot, part by part, and makes it the session's:
/// done when the task ends, the spool's file gone either way.
///
/// With a bucket of the host's own (`store`), it goes there, and the
/// control plane hears only where; the one before (`previous`, the
/// session's last), when this host's bucket has it, goes once it has.
pub fn upload(
    api: Api,
    session: SessionId,
    spool: Spool,
    saved: SnapshotSaved,
    store: Option<std::sync::Arc<crate::s3::Store>>,
    previous: Option<String>,
) -> tokio::task::JoinHandle<Result<(), String>> {
    tokio::spawn(async move {
        let Spool { id, path, file, .. } = spool;
        drop(file);
        let saved_to = match &store {
            Some(store) => upload_to_bucket(&api, &session, &id, &path, &saved, store)
                .await
                .map(|at| format!("in {at}")),
            None => upload_parts(&api, &session, &id, &path, &saved)
                .await
                .map(|parts| format!("{parts} parts")),
        };
        let done = match saved_to {
            Ok(where_) => {
                tracing::info!(
                    "saved {session}'s snapshot ({} bytes, {where_})",
                    saved.size
                );
                // The one before, in this host's bucket, is the session's no more.
                if let (Some(store), Some(previous)) = (&store, previous)
                    && let Some(key) = store.bucket.key_of(&previous)
                    && let Err(e) = store.delete(&key).await
                {
                    tracing::warn!("deleting {session}'s snapshot before: {e}");
                }
                Ok(())
            }
            Err(e) => {
                tracing::warn!("saving {session}'s snapshot: {e}");
                Err(e)
            }
        };
        let _ = std::fs::remove_file(&path);
        done
    })
}

/// Puts the snapshot at `path` in the host's bucket and tells the control
/// plane where: that location.
async fn upload_to_bucket(
    api: &Api,
    session: &SessionId,
    id: &str,
    path: &Path,
    saved: &SnapshotSaved,
    store: &crate::s3::Store,
) -> Result<String, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size != saved.size {
        return Err(format!(
            "it has {size} bytes where fctl-agent said {}",
            saved.size
        ));
    }
    let key = store.bucket.key(session.as_str(), id);
    store.put(&key, path, &saved.sha256).await?;
    let location = store.bucket.location(&key);
    let commit = SnapshotCommit {
        saved: saved.clone(),
        parts: 0,
        location: Some(location.clone()),
    };
    if let Err(e) = retry(|| api.runner_snapshot_commit(session.as_str(), id, &commit)).await {
        // Not the session's: it would only take room.
        let _ = store.delete(&key).await;
        return Err(format!("keeping it: {e}"));
    }
    Ok(location)
}

async fn upload_parts(
    api: &Api,
    session: &SessionId,
    id: &str,
    path: &Path,
    saved: &SnapshotSaved,
) -> Result<u32, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size != saved.size {
        return Err(format!(
            "it has {size} bytes where fctl-agent said {}",
            saved.size
        ));
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut n = 0u32;
    loop {
        let mut buf = vec![0u8; SNAPSHOT_PART_BYTES];
        let len = read_full(&mut file, &mut buf).map_err(|e| e.to_string())?;
        if len == 0 && n > 0 {
            break;
        }
        buf.truncate(len);
        retry(|| api.runner_snapshot_put(session.as_str(), id, n, buf.clone()))
            .await
            .map_err(|e| format!("uploading part {n}: {e}"))?;
        n += 1;
        if len < SNAPSHOT_PART_BYTES {
            break;
        }
    }
    let commit = SnapshotCommit {
        saved: saved.clone(),
        parts: n,
        location: None,
    };
    retry(|| api.runner_snapshot_commit(session.as_str(), id, &commit))
        .await
        .map_err(|e| format!("keeping it: {e}"))?;
    Ok(n)
}

/// Fetches a snapshot's parts for the agent that asked for it, as
/// `SnapshotPart` frames on `to`, or why not, as `SnapshotMissing`: from
/// the control plane, or, for one kept in a bucket, from the host's bucket
/// when it is the same.
pub fn fetch(
    api: Api,
    session: SessionId,
    snapshot: SnapshotRef,
    store: Option<std::sync::Arc<crate::s3::Store>>,
    data_dir: PathBuf,
    to: mpsc::UnboundedSender<HostToAgent>,
) {
    tokio::spawn(async move {
        let id = snapshot.id.clone();
        if let Some(location) = &snapshot.location {
            let missing = |why: String| HostToAgent::SnapshotMissing {
                id: id.clone(),
                why,
            };
            let Some((store, key)) = store
                .as_ref()
                .and_then(|s| s.bucket.key_of(location).map(|k| (s, k)))
            else {
                let bucket = fctl_domain::snapshots::bucket_of(location).unwrap_or(location);
                let _ = to.send(missing(format!(
                    "it is in the bucket {bucket}, which this runner doesn't keep snapshots in"
                )));
                return;
            };
            let dir = data_dir.join("snapshots");
            let _ = std::fs::create_dir_all(&dir);
            let file = dir.join(format!("{}-{id}.in", session.as_str()));
            let got = store.get(&key, &file, snapshot.size.max(1)).await;
            let sent = match got {
                Ok(_) => send_file(&file, &id, &to),
                Err(e) => {
                    tracing::warn!("fetching {session}'s snapshot: {e}");
                    let _ = to.send(missing(e));
                    Ok(())
                }
            };
            if let Err(e) = sent {
                let _ = to.send(missing(format!("reading it here failed: {e}")));
            }
            let _ = std::fs::remove_file(&file);
            return;
        }
        let parts = snapshot.parts.max(1);
        for n in 0..parts {
            match retry(|| api.runner_snapshot_part(session.as_str(), &id, n)).await {
                Ok(bytes) => {
                    let frame = HostToAgent::SnapshotPart {
                        id: id.clone(),
                        n,
                        data: STANDARD.encode(bytes),
                        last: n + 1 == parts,
                    };
                    if to.send(frame).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    tracing::warn!("fetching {session}'s snapshot: {e}");
                    let _ = to.send(HostToAgent::SnapshotMissing {
                        id,
                        why: format!("fetching it from the control plane failed: {e}"),
                    });
                    return;
                }
            }
        }
    });
}

/// Hands the snapshot in the file at `path` to the agent in parts.
fn send_file(
    path: &Path,
    id: &str,
    to: &mpsc::UnboundedSender<HostToAgent>,
) -> std::io::Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; SNAPSHOT_PART_BYTES];
    let mut len = read_full(&mut file, &mut buf)?;
    let mut n = 0;
    loop {
        let mut next = vec![0u8; SNAPSHOT_PART_BYTES];
        let next_len = read_full(&mut file, &mut next)?;
        let frame = HostToAgent::SnapshotPart {
            id: id.to_owned(),
            n,
            data: STANDARD.encode(buf.get(..len).unwrap_or_default()),
            last: next_len == 0,
        };
        if to.send(frame).is_err() || next_len == 0 {
            return Ok(());
        }
        buf = next;
        len = next_len;
        n += 1;
    }
}

/// Calls `f` up to three times, while it fails in a way that may pass.
async fn retry<T, F, Fut>(f: F) -> Result<T, fctl_client::Error>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, fctl_client::Error>>,
{
    let mut delay = Duration::from_secs(1);
    let mut attempt = 1;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if e.is_transient() && attempt < 3 => {
                tokio::time::sleep(delay).await;
                delay *= 3;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Reads until `buf` is full or the input ends: how much it read.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(buf.get_mut(n..).unwrap_or_default())? {
            0 => break,
            m => n += m,
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_spool_in_order_only() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let session = SessionId("ses_1".into());
        let mut spool = Spool::start(dir.path(), &session, "snp_1", 1_000)?;
        spool
            .add(0, &STANDARD.encode(b"abc"))
            .map_err(anyhow::Error::msg)?;
        assert!(spool.add(2, &STANDARD.encode(b"x")).is_err());
        spool
            .add(1, &STANDARD.encode(b"def"))
            .map_err(anyhow::Error::msg)?;
        assert!(spool.add(2, "not base64!").is_err());
        spool.file.flush()?;
        assert_eq!(std::fs::read(&spool.path)?, b"abcdef");
        let path = spool.path.clone();
        spool.discard();
        assert!(!path.exists());
        Ok(())
    }

    /// An agent can't spool more than its spec allows, and a part over it
    /// leaves nothing more to add to.
    #[test]
    fn a_spool_stops_at_its_limit() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let session = SessionId("ses_1".into());
        let part = vec![7u8; SNAPSHOT_PART_BYTES];
        let mut spool = Spool::start(dir.path(), &session, "snp_2", 10)?;
        // Its limit and one part's room for what seals it.
        spool
            .add(0, &STANDARD.encode(&part))
            .map_err(anyhow::Error::msg)?;
        let over = spool.add(1, &STANDARD.encode(b"0123456789a"));
        assert!(over.is_err_and(|e| e.contains("10 bytes")));
        // Empty parts, and more parts than its size takes, are refused too.
        let mut empty = Spool::start(dir.path(), &session, "snp_3", 10)?;
        assert!(empty.add(0, "").is_err_and(|e| e.contains("empty")));
        let mut many = Spool::start(dir.path(), &session, "snp_4", 10)?;
        // Its size and one part's room take two parts at most.
        many.add(0, &STANDARD.encode(b"x"))
            .map_err(anyhow::Error::msg)?;
        many.add(1, &STANDARD.encode(b"y"))
            .map_err(anyhow::Error::msg)?;
        assert!(
            many.add(2, &STANDARD.encode(b"z"))
                .is_err_and(|e| e.contains("more parts"))
        );
        empty.discard();
        many.discard();
        spool.discard();
        assert_eq!(std::fs::read_dir(dir.path().join("snapshots"))?.count(), 0);
        Ok(())
    }
}
