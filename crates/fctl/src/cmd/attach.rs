//! `fctl attach`: a live session in the terminal.
//!
//! Lines typed are sent to the agent. `/interrupt` stops its turn, `/stop`
//! stops the session, Ctrl-C or Ctrl-D detaches (the session keeps running).

use crate::config;
use crate::render::Renderer;
use crate::stall::Watch;
use crate::style;
use anyhow::Result;
use fctl_client::{Api, ws};
use fctl_proto::viewer::{ViewerClient, ViewerServer};
use futures::StreamExt;
use std::io::IsTerminal;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};

pub async fn attach(server: Option<&str>, reference: &str, read_only: bool) -> Result<()> {
    let api = config::api(server)?;
    let s = crate::cmd::sessions::resolve(&api, reference).await?;
    follow(&api, &s.id.0, read_only).await
}

pub async fn follow(api: &Api, session_id: &str, read_only: bool) -> Result<()> {
    follow_after(api, session_id, read_only, None, Renderer::default()).await
}

pub async fn follow_after(
    api: &Api,
    session_id: &str,
    read_only: bool,
    mut after: Option<u64>,
    mut r: Renderer,
) -> Result<()> {
    let interactive = !read_only && std::io::stdin().is_terminal();
    if interactive {
        eprintln!(
            "{}",
            style::dim("Type to message the agent · /interrupt · /stop · Ctrl-C to detach")
        );
    }
    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    if interactive {
        tokio::spawn(async move {
            let mut lines = BufReader::new(tokio::io::stdin()).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line_tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    let mut counter = 0u64;
    let mut watch = Watch::default();
    loop {
        let url = api.session_ws_url(session_id, after);
        let socket = match ws::connect(&url, api.token()).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", style::dim(&format!("reconnecting ({e:#})…")));
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let (mut sink, mut stream) = socket.split();
        let mut ping = tokio::time::interval(Duration::from_secs(25));
        let closed = loop {
            let due = watch.due().map(tokio::time::Instant::from_std);
            tokio::select! {
                msg = ws::next_text(&mut stream) => {
                    let Some(Ok(text)) = msg else { break true };
                    let m = match serde_json::from_str::<ViewerServer>(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            tracing::warn!("ignoring a message this fctl does not understand: {e}");
                            continue;
                        }
                    };
                    match m {
                        ViewerServer::Snapshot(snap) => {
                            if after.is_none() && !r.json {
                                println!("{} {}", style::bold(&format!("{}#{}", snap.session.project_slug, snap.session.number)), style::plain(&snap.session.title));
                            }
                            for ev in &snap.events {
                                r.event(ev);
                            }
                            for st in &snap.live {
                                r.stream(st);
                            }
                            after = Some(r.last_seq);
                            if snap.session.status.is_terminal() && read_only {
                                return Ok(());
                            }
                            watch.session(&snap.session);
                            let agent = snap.agent_unreachable.as_ref();
                            if let Some(l) = watch.runner(snap.runner_connected, snap.runner_last_seen_at, agent, Instant::now(), style::now_ms()) {
                                r.note(&l);
                            }
                        }
                        ViewerServer::Events { events } => {
                            for ev in &events {
                                r.event(ev);
                            }
                            after = Some(r.last_seq);
                        }
                        ViewerServer::Stream { event } => r.stream(&event),
                        ViewerServer::Session { session } => watch.session(&session),
                        ViewerServer::RunnerConnected { connected, last_seen_at, agent_unreachable } => {
                            if let Some(l) = watch.runner(connected, last_seen_at, agent_unreachable.as_ref(), Instant::now(), style::now_ms()) {
                                r.note(&l);
                            }
                        }
                        ViewerServer::Result { ok: false, error: Some(e), .. } => eprintln!("{}", style::red(&e.message)),
                        ViewerServer::Error { error } => eprintln!("{}", style::red(&error.message)),
                        _ => {}
                    }
                }
                line = line_rx.recv(), if interactive => {
                    let Some(line) = line else { return Ok(()) };
                    let text = line.trim();
                    if text.is_empty() { continue; }
                    counter += 1;
                    let client_id = format!("cli-{}-{counter}", std::process::id());
                    let msg = match text {
                        "/interrupt" => ViewerClient::Interrupt { client_id },
                        "/stop" => {
                            api.stop(session_id).await?;
                            continue;
                        }
                        "/detach" | "/quit" | "/exit" => return Ok(()),
                        _ => ViewerClient::Send { client_id, text: text.to_owned(), attachments: Vec::new() },
                    };
                    if ws::send_json(&mut sink, &msg).await.is_err() { break true; }
                }
                _ = ping.tick() => {
                    if ws::send_json(&mut sink, &ViewerClient::Ping).await.is_err() { break true; }
                }
                _ = async { if let Some(at) = due { tokio::time::sleep_until(at).await } }, if due.is_some() => {
                    if let Some(l) = watch.tick(Instant::now(), style::now_ms()) {
                        r.note(&l);
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\n{}", style::dim("Detached. The session keeps running."));
                    return Ok(());
                }
            }
        };
        if closed {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
