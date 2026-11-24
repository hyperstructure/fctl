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
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

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
        // The agent names it, and the name goes in a path and a key.
        if !fctl_domain::snapshots::valid_id(id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{id:?} isn't a snapshot's id"),
            ));
        }
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
/// control plane hears only where; the one it replaced, when the control
/// plane says it was in this host's bucket, goes once it has.
pub fn upload(
    api: Api,
    session: SessionId,
    spool: Spool,
    saved: SnapshotSaved,
    store: Option<std::sync::Arc<crate::s3::Store>>,
) -> tokio::task::JoinHandle<Result<(), String>> {
    tokio::spawn(async move {
        let Spool { id, path, file, .. } = spool;
        drop(file);
        let saved_to = match &store {
            Some(store) => upload_to_bucket(&api, &session, &id, &path, &saved, store)
                .await
                .map(|(at, replaced)| (format!("in {at}"), replaced)),
            None => upload_parts(&api, &session, &id, &path, &saved)
                .await
                .map(|parts| (format!("{parts} parts"), None)),
        };
        let done = match saved_to {
            Ok((where_, replaced)) => {
                tracing::info!(
                    "saved {session}'s snapshot ({} bytes, {where_})",
                    saved.size
                );
                // The one before, in this host's bucket, is the session's no more.
                if let (Some(store), Some(replaced)) = (&store, replaced)
                    && let Some(key) = store.bucket.key_of(&replaced, session.as_str())
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
/// plane where: that location, and where the snapshot it replaced is when
/// that one was in a bucket.
async fn upload_to_bucket(
    api: &Api,
    session: &SessionId,
    id: &str,
    path: &Path,
    saved: &SnapshotSaved,
    store: &crate::s3::Store,
) -> Result<(String, Option<String>), String> {
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
    match retry(|| api.runner_snapshot_commit(session.as_str(), id, &commit)).await {
        Ok(committed) => Ok((location, committed.replaced)),
        Err(e) => {
            // Not the session's: it would only take room.
            let _ = store.delete(&key).await;
            Err(format!("keeping it: {e}"))
        }
    }
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
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| e.to_string())?;
    let mut n = 0u32;
    loop {
        let mut buf = vec![0u8; SNAPSHOT_PART_BYTES];
        let len = read_full(&mut file, &mut buf)
            .await
            .map_err(|e| e.to_string())?;
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

/// A frame for the agent from [`fetch`], with what hears once the agent
/// link has written it (`written`): a snapshot goes at the link's pace,
/// rather than into the host's memory whole.
pub struct Paced {
    pub frame: HostToAgent,
    pub written: Option<oneshot::Sender<()>>,
}

/// Sends `frame` on `to` once the frame before it is written (`before`):
/// what hears when this one is, or `None` once the link that would have
/// written the one before is gone.
async fn paced(
    to: &mpsc::UnboundedSender<Paced>,
    before: Option<oneshot::Receiver<()>>,
    frame: HostToAgent,
) -> Option<oneshot::Receiver<()>> {
    if let Some(before) = before {
        before.await.ok()?;
    }
    let (written, heard) = oneshot::channel();
    to.send(Paced {
        frame,
        written: Some(written),
    })
    .ok()?;
    Some(heard)
}

/// Fetches a snapshot's parts for the agent that asked for it, as
/// `SnapshotPart` frames on `to`, at the agent link's pace, or why not, as
/// `SnapshotMissing`: from the control plane, or, for one kept in a
/// bucket, from the host's bucket when it is the same.
pub fn fetch(
    api: Api,
    session: SessionId,
    snapshot: SnapshotRef,
    store: Option<std::sync::Arc<crate::s3::Store>>,
    data_dir: PathBuf,
    to: mpsc::UnboundedSender<Paced>,
) {
    tokio::spawn(async move {
        let id = snapshot.id.clone();
        let missing = |why: String| {
            let _ = to.send(Paced {
                frame: HostToAgent::SnapshotMissing {
                    id: id.clone(),
                    why,
                },
                written: None,
            });
        };
        // The link broke while parts went: the agent hears it when it is
        // back, rather than wait for parts that went with it.
        let broke =
            || missing("the host's link to the sandbox broke while it handed it over".into());
        if let Some(location) = &snapshot.location {
            let Some(store) = store
                .as_ref()
                .filter(|s| *location == s.bucket.location(&s.bucket.key(session.as_str(), &id)))
            else {
                missing(format!(
                    "it is at {location}, where this runner doesn't keep snapshots"
                ));
                return;
            };
            if snapshot.size > store.max_bytes {
                missing(format!(
                    "it is {}, more than the {} MiB this runner takes",
                    fctl_domain::snapshots::size_words(snapshot.size),
                    store.max_bytes / 1024 / 1024
                ));
                return;
            }
            let dir = data_dir.join("snapshots");
            let _ = std::fs::create_dir_all(&dir);
            let file = dir.join(format!("{}-{id}.in", session.as_str()));
            let key = store.bucket.key(session.as_str(), &id);
            match store.get(&key, &file, snapshot.size.max(1)).await {
                Ok(_) => match send_file(&file, &id, &to).await {
                    Ok(true) => {}
                    Ok(false) => broke(),
                    Err(e) => missing(format!("reading it here failed: {e}")),
                },
                Err(e) => {
                    tracing::warn!("fetching {session}'s snapshot: {e}");
                    missing(e);
                }
            }
            let _ = tokio::fs::remove_file(&file).await;
            return;
        }
        let parts = snapshot.parts.max(1);
        let mut before = None;
        for n in 0..parts {
            // The next part comes while the link writes the one before.
            match retry(|| api.runner_snapshot_part(session.as_str(), &id, n)).await {
                Ok(bytes) => {
                    let frame = HostToAgent::SnapshotPart {
                        id: id.clone(),
                        n,
                        data: STANDARD.encode(bytes),
                        last: n + 1 == parts,
                    };
                    match paced(&to, before.take(), frame).await {
                        Some(heard) => before = Some(heard),
                        None => return broke(),
                    }
                }
                Err(e) => {
                    tracing::warn!("fetching {session}'s snapshot: {e}");
                    return missing(format!("fetching it from the control plane failed: {e}"));
                }
            }
        }
    });
}

/// Hands the snapshot in the file at `path` to the agent in parts, at the
/// link's pace: whether the link took them all.
async fn send_file(
    path: &Path,
    id: &str,
    to: &mpsc::UnboundedSender<Paced>,
) -> std::io::Result<bool> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut buf = vec![0u8; SNAPSHOT_PART_BYTES];
    let mut len = read_full(&mut file, &mut buf).await?;
    let mut n = 0;
    let mut before = None;
    loop {
        let mut next = vec![0u8; SNAPSHOT_PART_BYTES];
        let next_len = read_full(&mut file, &mut next).await?;
        let frame = HostToAgent::SnapshotPart {
            id: id.to_owned(),
            n,
            data: STANDARD.encode(buf.get(..len).unwrap_or_default()),
            last: next_len == 0,
        };
        match paced(to, before.take(), frame).await {
            Some(heard) => before = Some(heard),
            None => return Ok(false),
        }
        if next_len == 0 {
            return Ok(true);
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
async fn read_full(
    r: &mut (impl tokio::io::AsyncRead + Unpin),
    buf: &mut [u8],
) -> std::io::Result<usize> {
    use tokio::io::AsyncReadExt;
    let mut n = 0;
    while n < buf.len() {
        match r.read(buf.get_mut(n..).unwrap_or_default()).await? {
            0 => break,
            m => n += m,
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot from a file goes to the agent a part at a time: the next
    /// waits until the link has written the one before, and none goes once
    /// the link is gone.
    #[tokio::test]
    async fn a_snapshot_goes_to_the_agent_at_the_links_pace() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("snap");
        std::fs::write(&path, vec![1u8; SNAPSHOT_PART_BYTES * 2 + 10])?;
        let part = |p: &Paced| match &p.frame {
            HostToAgent::SnapshotPart { n, last, .. } => Some((*n, *last)),
            _ => None,
        };
        let (to, mut rx) = mpsc::unbounded_channel();
        let sending = {
            let path = path.clone();
            tokio::spawn(async move { send_file(&path, "snp_1", &to).await })
        };
        let first = rx.recv().await.ok_or_else(|| anyhow::anyhow!("no part"))?;
        assert_eq!(part(&first), Some((0, false)));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            rx.try_recv().is_err(),
            "the second came before the first was written"
        );
        if let Some(w) = first.written {
            let _ = w.send(());
        }
        let second = rx.recv().await.ok_or_else(|| anyhow::anyhow!("no part"))?;
        assert_eq!(part(&second), Some((1, false)));
        // The link went before it wrote that one.
        drop(second);
        assert!(!sending.await??);
        assert!(rx.recv().await.is_none());

        // Written as they go, all of it goes, the last said.
        let (to, mut rx) = mpsc::unbounded_channel();
        let sending = tokio::spawn(async move { send_file(&path, "snp_1", &to).await });
        let mut got = Vec::new();
        while let Some(p) = rx.recv().await {
            got.extend(part(&p));
            if let Some(w) = p.written {
                let _ = w.send(());
            }
        }
        assert_eq!(got, [(0, false), (1, false), (2, true)]);
        assert!(sending.await??);
        Ok(())
    }

    /// An agent names its snapshot, and the name goes in a path and a key:
    /// only a snapshot's id is taken.
    #[test]
    fn a_spool_takes_only_a_snapshots_id() {
        let dir = tempfile::tempdir().ok();
        let dir = dir
            .as_ref()
            .map(|d| d.path())
            .unwrap_or(Path::new("/nonexistent"));
        let session = SessionId("ses_1".into());
        for bad in ["../snp_1", "snp_1/../../x", "snp_", "x"] {
            assert!(Spool::start(dir, &session, bad, 10).is_err(), "{bad}");
        }
    }

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
