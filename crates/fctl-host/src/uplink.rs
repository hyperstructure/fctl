//! A session's uplink to its SessionObject, reconnecting forever.
//!
//! Events are not buffered here while disconnected: fctl-agent's journal
//! holds them and the session replays from the acknowledged sequence number
//! after every `Welcome`. State, whether the host reaches the agent, and host
//! log lines are kept and re-sent.

use fctl_client::ws;
use fctl_proto::event::LogLine;
use fctl_proto::uplink::{AgentEvent, FEATURE_ATTACHMENTS, SessionState, UplinkDown, UplinkUp};
use fctl_proto::{PROTOCOL_VERSION, RunnerId};
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

pub enum Up {
    Events(Vec<AgentEvent>),
    Raw(UplinkUp),
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
