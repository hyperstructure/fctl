//! A session's uplink to its SessionObject, reconnecting forever.
//!
//! Events are not buffered here while disconnected: fctl-agent's journal
//! holds them and the session replays from the acknowledged sequence number
//! after every `Welcome`. State, whether the host reaches the agent, and host
//! log lines are kept and re-sent. So are the changed files' diffs: a new
//! connection sends them all, then only those that changed.

use fctl_client::ws;
use fctl_domain::diffs;
use fctl_proto::event::{DiffKey, FileDiff, LogLine};
use fctl_proto::uplink::{AgentEvent, FEATURE_ATTACHMENTS, SessionState, UplinkDown, UplinkUp};
use fctl_proto::{PROTOCOL_VERSION, RunnerId};
use futures::StreamExt;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

pub enum Up {
    Events(Vec<AgentEvent>),
    Raw(UplinkUp),
    /// Every changed file's diff, as fctl-agent read them last.
    Diffs(Vec<FileDiff>),
}

pub struct Uplink {
    tx: mpsc::UnboundedSender<Up>,
}

impl Uplink {
    /// Starts the connection task; downstream messages arrive on the returned receiver.
    pub fn start(
        url: String,
        token: String,
        runner_id: RunnerId,
    ) -> (Self, mpsc::UnboundedReceiver<UplinkDown>) {
        let (tx, rx) = mpsc::unbounded_channel::<Up>();
        let (down_tx, down_rx) = mpsc::unbounded_channel::<UplinkDown>();
        tokio::spawn(run(url, token, runner_id, rx, down_tx));
        (Self { tx }, down_rx)
    }

    pub fn events(&self, events: Vec<AgentEvent>) {
        if !events.is_empty() {
            let _ = self.tx.send(Up::Events(events));
        }
    }

    pub fn send(&self, msg: UplinkUp) {
        let _ = self.tx.send(Up::Raw(msg));
    }

    pub fn state(&self, state: SessionState) {
        self.send(UplinkUp::State { state });
    }

    pub fn log(&self, line: LogLine) {
        self.send(UplinkUp::Log { line });
    }

    /// The changed files' diffs, all of them, for SessionDO's viewers. What
    /// runs in the sandbox can pose as its agent: diffs no agent sends, and
    /// those past what a session keeps, go no further.
    pub fn diffs(&self, mut files: Vec<FileDiff>) {
        files.retain(|f| diffs::check(f).is_ok());
        files.truncate(diffs::MAX_FILES);
        let mut left = diffs::MAX_SESSION_BYTES;
        files.retain(|f| {
            let size = f.patch.as_deref().map_or(0, diffs::json_bytes);
            let fits = size <= left;
            if fits {
                left -= size;
            }
            fits
        });
        let _ = self.tx.send(Up::Diffs(files));
    }
}

/// What SessionDO got of the diffs over this connection: each file's
/// digest. None before the connection's first frame: SessionDO may still
/// keep diffs from an earlier one, which that frame's `keep` lets go.
type Sent = Option<HashMap<(Option<String>, String), String>>;

/// The frames that bring SessionDO up to `files`, given what it `sent`
/// already over this connection, which they then add to: each lists every
/// file (`keep`), and the diffs SessionDO lacks are spread over them, at
/// most [`diffs::MAX_FRAME_BYTES`] a frame but for one larger diff alone.
/// None when SessionDO has them all, and no others; at least one over a
/// new connection, even for no files.
fn diff_frames(files: &[FileDiff], sent: &mut Sent) -> Vec<UplinkUp> {
    let had = |f: &FileDiff| sent.as_ref()?.get(&(f.repo.clone(), f.path.clone()));
    let new: Vec<&FileDiff> = files.iter().filter(|f| had(f) != Some(&f.digest)).collect();
    if new.is_empty() && sent.as_ref().is_some_and(|s| s.len() == files.len()) {
        return Vec::new();
    }
    let keep: Vec<DiffKey> = files.iter().map(FileDiff::key).collect();
    let room =
        diffs::MAX_FRAME_BYTES.saturating_sub(serde_json::to_vec(&keep).map_or(0, |k| k.len()));
    let mut frames = Vec::new();
    let mut batch: Vec<FileDiff> = Vec::new();
    let mut size = 0;
    for f in new {
        let bytes = serde_json::to_vec(f).map_or(0, |b| b.len());
        if !batch.is_empty() && size + bytes > room {
            frames.push(UplinkUp::Diff {
                keep: keep.clone(),
                files: std::mem::take(&mut batch),
            });
            size = 0;
        }
        size += bytes;
        batch.push(f.clone());
    }
    if !batch.is_empty() || frames.is_empty() {
        frames.push(UplinkUp::Diff { keep, files: batch });
    }
    *sent = Some(
        files
            .iter()
            .map(|f| ((f.repo.clone(), f.path.clone()), f.digest.clone()))
            .collect(),
    );
    frames
}

async fn run(
    url: String,
    token: String,
    runner_id: RunnerId,
    mut rx: mpsc::UnboundedReceiver<Up>,
    down: mpsc::UnboundedSender<UplinkDown>,
) {
    let mut backoff = Duration::from_millis(500);
    let mut last_state: Option<SessionState> = None;
    // SessionDO forgets it with each connection.
    let mut last_link: Option<UplinkUp> = None;
    let mut last_diffs: Option<Vec<FileDiff>> = None;
    let mut pending_logs: Vec<UplinkUp> = Vec::new();
    loop {
        let socket = match ws::connect(&url, Some(&token)).await {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!("uplink {url}: {e:#}");
                // Keep draining so senders never block; keep what must survive.
                let deadline = tokio::time::sleep(backoff);
                tokio::pin!(deadline);
                loop {
                    tokio::select! {
                        _ = &mut deadline => break,
                        msg = rx.recv() => match msg {
                            None => return,
                            Some(Up::Raw(UplinkUp::State { state })) => last_state = Some(state),
                            Some(Up::Raw(m @ UplinkUp::AgentLink { .. })) => last_link = Some(m),
                            Some(Up::Raw(m @ UplinkUp::Log { .. })) | Some(Up::Raw(m @ UplinkUp::Delivered { .. })) => {
                                if pending_logs.len() < 500 { pending_logs.push(m); }
                            }
                            Some(Up::Diffs(files)) => last_diffs = Some(files),
                            Some(_) => {}
                        }
                    }
                }
                backoff = (backoff * 2).min(Duration::from_secs(15));
                continue;
            }
        };
        backoff = Duration::from_millis(500);
        let (mut sink, mut stream) = socket.split();
        if ws::send_json(
            &mut sink,
            &UplinkUp::Hello {
                runner_id: runner_id.clone(),
                protocol: PROTOCOL_VERSION,
                features: vec![FEATURE_ATTACHMENTS.to_owned()],
            },
        )
        .await
        .is_err()
        {
            continue;
        }
        if let Some(s) = last_state.clone() {
            let _ = ws::send_json(&mut sink, &UplinkUp::State { state: s }).await;
        }
        if let Some(m) = &last_link {
            let _ = ws::send_json(&mut sink, m).await;
        }
        for m in pending_logs.drain(..) {
            let _ = ws::send_json(&mut sink, &m).await;
        }
        let mut sent: Sent = None;
        if let Some(files) = &last_diffs {
            for frame in diff_frames(files, &mut sent) {
                let _ = ws::send_json(&mut sink, &frame).await;
            }
        }
        let mut ping = tokio::time::interval(Duration::from_secs(20));
        let mut batch: Vec<AgentEvent> = Vec::new();
        let mut flush = tokio::time::interval(Duration::from_millis(30));
        let broken = loop {
            tokio::select! {
                msg = ws::next_text(&mut stream) => match msg {
                    Some(Ok(text)) => match serde_json::from_str::<UplinkDown>(&text) {
                        Ok(d) => { let _ = down.send(d); }
                        Err(e) => tracing::warn!("bad uplink message: {e}"),
                    },
                    _ => break true,
                },
                up = rx.recv() => match up {
                    None => break false,
                    Some(Up::Events(evs)) => {
                        batch.extend(evs);
                        if batch.len() >= 200
                            && ws::send_json(&mut sink, &UplinkUp::Events { events: std::mem::take(&mut batch) }).await.is_err() { break true; }
                    }
                    Some(Up::Diffs(files)) => {
                        // Before the events that follow them: the `git`
                        // event with their digests.
                        if !batch.is_empty() && ws::send_json(&mut sink, &UplinkUp::Events { events: std::mem::take(&mut batch) }).await.is_err() { break true; }
                        let mut failed = false;
                        for frame in diff_frames(&files, &mut sent) {
                            if ws::send_json(&mut sink, &frame).await.is_err() { failed = true; break; }
                        }
                        last_diffs = Some(files);
                        if failed { break true; }
                    }
                    Some(Up::Raw(m)) => {
                        if let UplinkUp::State { state } = &m { last_state = Some(state.clone()); }
                        if matches!(m, UplinkUp::AgentLink { .. }) { last_link = Some(m.clone()); }
                        // Keep ordering: events before anything that follows them.
                        if !batch.is_empty() && ws::send_json(&mut sink, &UplinkUp::Events { events: std::mem::take(&mut batch) }).await.is_err() { break true; }
                        if ws::send_json(&mut sink, &m).await.is_err() { break true; }
                    }
                },
                _ = flush.tick() => {
                    if !batch.is_empty() && ws::send_json(&mut sink, &UplinkUp::Events { events: std::mem::take(&mut batch) }).await.is_err() { break true; }
                }
                _ = ping.tick() => {
                    if ws::send_json(&mut sink, &UplinkUp::Ping).await.is_err() { break true; }
                }
            }
        };
        if !broken {
            return;
        }
        tracing::info!("uplink {url} dropped; reconnecting");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, text: &str) -> FileDiff {
        let mut files = vec![diffs::untracked(path, text.as_bytes(), true)];
        diffs::fit(&mut files);
        files.remove(0)
    }

    fn sent_in(frames: &[UplinkUp]) -> Vec<Vec<String>> {
        frames
            .iter()
            .map(|f| match f {
                UplinkUp::Diff { files, .. } => files.iter().map(|d| d.path.clone()).collect(),
                _ => vec!["not a diff".to_owned()],
            })
            .collect()
    }

    #[test]
    fn a_connection_gets_every_diff_then_those_that_change() {
        let mut sent: Sent = None;
        let (a, b) = (file("a.rs", "a\n"), file("b.rs", "b\n"));
        let frames = diff_frames(&[a.clone(), b.clone()], &mut sent);
        assert_eq!(sent_in(&frames), [vec!["a.rs", "b.rs"]]);
        // Nothing new: nothing to send.
        assert!(diff_frames(&[a.clone(), b.clone()], &mut sent).is_empty());
        // One changes: that one, with both listed.
        let b2 = file("b.rs", "b\nc\n");
        let frames = diff_frames(&[a.clone(), b2.clone()], &mut sent);
        assert_eq!(sent_in(&frames), [vec!["b.rs"]]);
        let UplinkUp::Diff { keep, .. } = &frames[0] else {
            panic!("expected a diff frame");
        };
        assert_eq!(keep, &[a.key(), b2.key()]);
        // One goes: a frame that lists the other alone, so SessionDO forgets it.
        let frames = diff_frames(std::slice::from_ref(&a), &mut sent);
        assert_eq!(sent_in(&frames), [Vec::<String>::new()]);
        let UplinkUp::Diff { keep, .. } = &frames[0] else {
            panic!("expected a diff frame");
        };
        assert_eq!(keep, &[a.key()]);
        // A new connection starts over.
        let frames = diff_frames(std::slice::from_ref(&a), &mut None);
        assert_eq!(sent_in(&frames), [vec!["a.rs"]]);
        // And none at all, once there were some: SessionDO forgets them.
        let frames = diff_frames(&[], &mut sent);
        assert_eq!(sent_in(&frames), [Vec::<String>::new()]);
        assert!(diff_frames(&[], &mut sent).is_empty());
        // None over a new connection: SessionDO may keep an earlier one's.
        let frames = diff_frames(&[], &mut None);
        assert_eq!(sent_in(&frames), [Vec::<String>::new()]);
    }

    #[test]
    fn large_diffs_spread_over_frames() {
        let big = |path: &str| {
            let mut f = file(path, "x\n");
            f.patch = Some("y".repeat(diffs::MAX_FRAME_BYTES / 2 - 100));
            f
        };
        let files: Vec<FileDiff> = ["a", "b", "c", "d", "e"].into_iter().map(big).collect();
        let frames = diff_frames(&files, &mut None);
        // Each takes about half a frame, which also lists every file: one
        // a frame, each frame within bounds, every file once.
        assert_eq!(
            sent_in(&frames),
            [vec!["a"], vec!["b"], vec!["c"], vec!["d"], vec!["e"]]
        );
        for frame in &frames {
            assert!(serde_json::to_vec(frame).map_or(0, |f| f.len()) <= diffs::MAX_FRAME_BYTES);
        }
        // Small ones share a frame.
        let small: Vec<FileDiff> = (0..50).map(|i| file(&format!("f{i}"), "z\n")).collect();
        assert_eq!(diff_frames(&small, &mut None).len(), 1);
    }

    #[test]
    fn diffs_no_agent_sends_go_no_further() {
        let mut bad = file("a.rs", "a\n");
        bad.digest = "not hex".into();
        let (tx, mut rx) = mpsc::unbounded_channel::<Up>();
        let uplink = Uplink { tx };
        uplink.diffs(vec![bad, file("b.rs", "b\n")]);
        let Some(Up::Diffs(files)) = rx.blocking_recv() else {
            panic!("expected diffs");
        };
        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["b.rs"]
        );
    }
}
