//! Socket Mode (docs/architecture.md#slack): for an instance Slack can't
//! reach (a private network, plain HTTP), a Durable Object, `SlackObject`,
//! opens an outbound WebSocket to Slack with the app-level token
//! (`SLACK_APP_TOKEN`) and takes Slack's events, interactions and commands
//! over it, as HTTP would bring them, on Cloudflare and on the standalone
//! server alike. It acknowledges each envelope at once (with fctl's quick
//! answer, where Slack takes one), then handles it as the HTTP endpoints
//! do. Slack refreshes connections every few hours: a `disconnect` opens a
//! new one, and the alarm, every minute, opens one where none is open or
//! none has heard from Slack for a while. Anything that wants Slack's
//! events kicks the object (`kick`), which then keeps itself going.

use crate::config::Config;
use crate::ctx::now_ms;
use crate::internal;
use fctl_domain::slack as sl;
use fctl_proto::ApiError;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;
use worker::{
    DurableObject, Env, Request, Response, Result, State, WebSocket, WebsocketEvent, durable_object,
};

/// How often the alarm sees that a connection is open.
const CHECK_MS: i64 = 60_000;
/// A connection that heard nothing from Slack for this long (Slack pings
/// and refreshes them) is opened again.
const QUIET_MS: i64 = 10 * 60_000;

/// How Socket Mode stands, for the admin's page.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Status {
    pub connected: bool,
    pub since: Option<i64>,
    pub error: Option<String>,
}

#[derive(Default)]
struct Live {
    /// The open connection's number: an older one that closes changes
    /// nothing.
    generation: u64,
    connected: bool,
    since: Option<i64>,
    heard: i64,
    error: Option<String>,
    connecting: bool,
}

#[durable_object]
pub struct SlackObject {
    state: State,
    env: Env,
    live: Rc<RefCell<Live>>,
}

impl DurableObject for SlackObject {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            live: Rc::new(RefCell::new(Live::default())),
        }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        let path = req.path();
        let result: std::result::Result<Value, ApiError> = match path.as_str() {
            "/kick" => {
                self.ensure().await;
                Ok(serde_json::to_value(self.status()).unwrap_or(Value::Null))
            }
            "/status" => Ok(serde_json::to_value(self.status()).unwrap_or(Value::Null)),
            _ => Err(ApiError::not_found("Slack endpoint")),
        };
        match result {
            Ok(v) => internal::do_json(&v),
            Err(e) => internal::do_error(&e),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        self.ensure().await;
        Response::ok("ok")
    }
}

impl SlackObject {
    fn status(&self) -> Status {
        let l = self.live.borrow();
        Status {
            connected: l.connected,
            since: l.since,
            error: l.error.clone(),
        }
    }

    /// Opens a connection where none works, and has the alarm look again
    /// in a minute, while the instance has an app-level token.
    async fn ensure(&self) {
        let cfg = Config::from_env(&self.env);
        let Some(slack) = cfg.slack.clone().filter(|s| s.app_token.is_some()) else {
            let _ = self.state.storage().delete_alarm().await;
            return;
        };
        let now = now_ms();
        let (stale, busy) = {
            let l = self.live.borrow();
            (!l.connected || now - l.heard > QUIET_MS, l.connecting)
        };
        if stale && !busy {
            connect(self.env.clone(), slack, self.live.clone()).await;
        }
        if let Err(e) = self.state.storage().set_alarm(CHECK_MS).await {
            worker::console_error!("Slack's socket alarm: {e}");
        }
    }
}

/// Opens a connection, and listens on it until it closes.
async fn connect(env: Env, slack: crate::config::SlackConfig, live: Rc<RefCell<Live>>) {
    live.borrow_mut().connecting = true;
    let opened = async {
        let url = super::connections_open(&slack)
            .await
            .map_err(|e| format!("Slack refused to open a connection: {e:?}"))?;
        let url: url::Url = url
            .parse()
            .map_err(|_| "Slack's connection address isn't one".to_owned())?;
        let ws = WebSocket::connect(url)
            .await
            .map_err(|e| format!("connecting to Slack: {e}"))?;
        ws.accept()
            .map_err(|e| format!("accepting Slack's socket: {e}"))?;
        Ok::<WebSocket, String>(ws)
    }
    .await;
    let ws = match opened {
        Ok(ws) => ws,
        Err(e) => {
            worker::console_error!("Slack's Socket Mode: {e}");
            let mut l = live.borrow_mut();
            l.connecting = false;
            l.connected = false;
            l.error = Some(e);
            return;
        }
    };
    let generation = {
        let mut l = live.borrow_mut();
        l.generation += 1;
        l.connected = true;
        l.connecting = false;
        l.since = Some(now_ms());
        l.heard = now_ms();
        l.error = None;
        l.generation
    };
    worker::console_log!("Slack's Socket Mode connected");
    wasm_bindgen_futures::spawn_local(listen(env, slack, live, ws, generation));
}

/// Takes what comes over a connection until it closes.
async fn listen(
    env: Env,
    slack: crate::config::SlackConfig,
    live: Rc<RefCell<Live>>,
    ws: WebSocket,
    generation: u64,
) {
    let Ok(mut events) = ws.events() else {
        return;
    };
    while let Some(event) = events.next().await {
        match event {
            Ok(WebsocketEvent::Message(m)) => {
                live.borrow_mut().heard = now_ms();
                let Some(text) = m.text() else { continue };
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                match v.get("type").and_then(Value::as_str) {
                    Some("hello") => {}
                    Some("disconnect") => {
                        // A new connection first, then this one goes.
                        let reason = v.get("reason").and_then(Value::as_str).unwrap_or_default();
                        if reason == "link_disabled" {
                            live.borrow_mut().error =
                                Some("Socket Mode is off in the Slack app's settings".into());
                        }
                        if reason != "warning" {
                            wasm_bindgen_futures::spawn_local(connect(
                                env.clone(),
                                slack.clone(),
                                live.clone(),
                            ));
                        }
                    }
                    _ => {
                        if let Some(envelope_id) = v.get("envelope_id").and_then(Value::as_str) {
                            let answer = quick(&v);
                            let ack = match answer {
                                Some(p)
                                    if v.get("accepts_response_payload")
                                        .and_then(Value::as_bool)
                                        == Some(true) =>
                                {
                                    json!({ "envelope_id": envelope_id, "payload": p })
                                }
                                _ => json!({ "envelope_id": envelope_id }),
                            };
                            if let Err(e) = ws.send_with_str(ack.to_string()) {
                                worker::console_error!("acknowledging Slack's envelope: {e}");
                            }
                            wasm_bindgen_futures::spawn_local(handle(env.clone(), v));
                        }
                    }
                }
            }
            Ok(WebsocketEvent::Close(_)) | Err(_) => break,
        }
    }
    let mut l = live.borrow_mut();
    if l.generation == generation {
        l.connected = false;
    }
}

/// What fctl answers an envelope with at once, where Slack takes an
/// answer: help for `/fctl help`, a form's errors.
fn quick(v: &Value) -> Option<Value> {
    let payload = v.get("payload")?;
    match v.get("type").and_then(Value::as_str) {
        Some("interactive") => super::inbound::quick(&sl::interaction(payload)),
        Some("slash_commands") => {
            let c = sl::command(|k| payload.get(k).and_then(Value::as_str).map(str::to_owned));
            Some(super::inbound::quick_command(&c))
        }
        _ => None,
    }
}

/// Handles an envelope as the HTTP endpoints would its request.
async fn handle(env: Env, v: Value) {
    let cfg = Config::from_env(&env);
    let Ok(db) = env.d1("DB") else { return };
    let now = now_ms();
    let Some(payload) = v.get("payload") else {
        return;
    };
    let done = match v.get("type").and_then(Value::as_str) {
        Some("events_api") => match sl::envelope(payload) {
            sl::Envelope::Event(e) => {
                super::inbound::event(&env, &cfg, &db, &e, payload, now).await
            }
            _ => Ok(()),
        },
        Some("interactive") => {
            super::inbound::interaction(&env, &cfg, &db, &sl::interaction(payload), now).await
        }
        Some("slash_commands") => {
            let c = sl::command(|k| payload.get(k).and_then(Value::as_str).map(str::to_owned));
            super::inbound::command(&env, &cfg, &db, &c, now).await
        }
        _ => Ok(()),
    };
    if let Err(e) = done {
        worker::console_error!("Slack over Socket Mode: {}", e.0.message);
    }
}

/// Has the Socket Mode object open a connection where the instance takes
/// Slack's events that way: as anything that wants them (the hourly job,
/// the pages that show Slack) asks. Its status, or why it can't say.
pub async fn kick(env: &Env) -> Option<Status> {
    let cfg = Config::from_env(env);
    cfg.slack.as_ref()?.app_token.as_ref()?;
    let stub = env
        .durable_object("SLACK")
        .ok()?
        .id_from_name("socket")
        .ok()?
        .get_stub()
        .ok()?;
    let r: std::result::Result<Status, crate::error::AppError> =
        internal::post(&stub, "/kick", &json!({})).await;
    match r {
        Ok(s) => Some(s),
        Err(e) => {
            worker::console_error!("kicking Slack's Socket Mode: {}", e.0.message);
            Some(Status {
                connected: false,
                since: None,
                error: Some(e.0.message),
            })
        }
    }
}
