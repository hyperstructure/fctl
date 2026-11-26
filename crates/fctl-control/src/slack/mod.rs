//! fctl's Slack app (docs/architecture.md#slack): its Web API client, the
//! Slack workspaces it is installed in (`slack_installations`), and who
//! people are there (`slack_accounts`).
//!
//! Slack keeps one bot token for an app in a Slack workspace, so a Slack
//! workspace has one installation here, whatever fctl workspaces work in
//! it: installing the app again, as another fctl workspace's admin does to
//! connect it, replaces the token for all of them. The token doesn't
//! expire (token rotation stays off), so it lasts until Slack revokes it:
//! uninstalled there, or its tokens revoked (`app_uninstalled`,
//! `tokens_revoked`), which ends every fctl workspace's connection to it.
//!
//! - [`inbound`]: what Slack sends (events, interactions, `/fctl`) and the
//!   sessions it starts and steers;
//! - [`outbound`]: what sessions post back (their card, replies,
//!   questions, pauses and failures, streams in direct messages);
//! - [`calls`]: what agents read and post with `fctl slack`;
//! - [`home`]: fctl's Home tab, `/fctl status`, and notifications as
//!   direct messages;
//! - [`socket`]: Socket Mode, for instances Slack can't reach.

pub mod calls;
pub mod home;
pub mod inbound;
pub mod outbound;
pub mod socket;
#[cfg(all(test, target_arch = "wasm32"))]
mod tests;

use crate::config::{Config, SlackConfig};
use crate::crypto::Keys;
use crate::db::{n, os, q, s};
use crate::error::{AppError, AppResult, internal, not_configured};
use crate::model::{UserRow, user_columns};
use fctl_domain::slack::{self as sl, access::Conversation};
use fctl_proto::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use worker::{D1Database, Fetch, Headers, Method, Request, RequestInit};

/// The instance's Slack app, or why it has none.
pub fn config(cfg: &Config) -> AppResult<&SlackConfig> {
    cfg.slack.as_ref().ok_or_else(|| {
        not_configured(
            "This instance has no Slack app: its admins set SLACK_CLIENT_ID and SLACK_CLIENT_SECRET (see the self-hosting guide)",
        )
    })
}

/// What D1 keeps sealed for an installation.
#[derive(Serialize, Deserialize)]
struct Secret {
    bot_token: String,
}

fn aad(team: &str) -> String {
    format!("slack:{team}")
}

/// Why a call to Slack failed.
#[derive(Debug)]
pub enum Failure {
    /// Slack answered with an error (`channel_not_found`, `not_in_channel`…).
    Slack(String),
    /// Slack asks fctl to wait this many seconds (`ratelimited`).
    Limited(u32),
    /// Slack didn't answer.
    Down(String),
}

impl Failure {
    /// Slack's error code, where it answered with one.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Slack(c) => Some(c),
            _ => None,
        }
    }

    /// The bot's token no longer works there.
    pub fn revoked(&self) -> bool {
        matches!(
            self.code(),
            Some(
                "invalid_auth"
                    | "account_inactive"
                    | "token_revoked"
                    | "not_authed"
                    | "team_not_found"
            )
        )
    }
}

impl From<Failure> for AppError {
    fn from(f: Failure) -> Self {
        match f {
            Failure::Limited(after) => AppError(ApiError::new(
                ErrorCode::RateLimited,
                format!("Slack asks fctl to slow down: try again in {after} seconds"),
            )),
            Failure::Down(why) => AppError(ApiError::new(
                ErrorCode::Unavailable,
                format!("Slack didn't answer: {why}"),
            )),
            Failure::Slack(code) if f_revoked(&code) => not_configured(
                "Slack no longer takes fctl's access to this Slack workspace: an admin of the fctl workspace connects it again",
            ),
            Failure::Slack(code) => AppError(ApiError::new(
                ErrorCode::Unavailable,
                format!("Slack refused it: {code}"),
            )),
        }
    }
}

fn f_revoked(code: &str) -> bool {
    Failure::Slack(code.to_owned()).revoked()
}

/// An HTTP request to Slack: its status, the `Retry-After` it asked for,
/// and its answer.
async fn send(url: &str, auth: Option<&str>, body: Body<'_>) -> Result<Value, Failure> {
    let headers = Headers::new();
    let set = |k: &str, v: &str| headers.set(k, v).map_err(|e| Failure::Down(e.to_string()));
    set("User-Agent", "fctl-control")?;
    set("Accept", "application/json")?;
    if let Some(a) = auth {
        set("Authorization", a)?;
    }
    let text = match body {
        Body::Json(v) => {
            set("Content-Type", "application/json; charset=utf-8")?;
            v.to_string()
        }
        Body::Form(fields) => {
            set("Content-Type", "application/x-www-form-urlencoded")?;
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(fields.iter().map(|(k, v)| (*k, v.as_str())))
                .finish()
        }
    };
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(text.into()));
    let req = Request::new_with_init(url, &init).map_err(|e| Failure::Down(e.to_string()))?;
    let mut res = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| Failure::Down(e.to_string()))?;
    let status = res.status_code();
    if status == 429 {
        let after = res
            .headers()
            .get("Retry-After")
            .ok()
            .flatten()
            .and_then(|a| a.trim().parse::<u32>().ok())
            .unwrap_or(60);
        return Err(Failure::Limited(after));
    }
    let answer = res.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&answer).unwrap_or(Value::Null);
    if status >= 500 || v.is_null() {
        return Err(Failure::Down(format!("it answered {status}")));
    }
    match sl::answer(&v) {
        Ok(_) => Ok(v),
        Err(code) if code == "ratelimited" => Err(Failure::Limited(60)),
        Err(code) => Err(Failure::Slack(code)),
    }
}

enum Body<'a> {
    Json(Value),
    Form(&'a [(&'a str, String)]),
}

/// Posts to a `response_url` Slack gave with an interaction or a command:
/// a message only the person sees, or one that replaces theirs.
pub async fn respond(response_url: &str, message: Value) {
    if !response_url.starts_with("https://") && !response_url.starts_with("http://") {
        return;
    }
    let headers = Headers::new();
    let _ = headers.set("Content-Type", "application/json; charset=utf-8");
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(message.to_string().into()));
    let sent = async {
        let req = Request::new_with_init(response_url, &init)?;
        let mut res = Fetch::Request(req).send().await?;
        Ok::<u16, worker::Error>({
            let _ = res.text().await;
            res.status_code()
        })
    }
    .await;
    match sent {
        Ok(200) => {}
        Ok(status) => worker::console_error!("Slack's response_url answered {status}"),
        Err(e) => worker::console_error!("posting to Slack's response_url: {e}"),
    }
}

/// Trades an install's code for the bot's token (`oauth.v2.access`).
pub async fn oauth_access(
    cfg: &SlackConfig,
    code: &str,
    redirect_uri: &str,
) -> AppResult<sl::Installed> {
    let fields = [
        ("client_id", cfg.client_id.clone()),
        ("client_secret", cfg.client_secret.clone()),
        ("code", code.to_owned()),
        ("redirect_uri", redirect_uri.to_owned()),
    ];
    let v = send(&cfg.api("oauth.v2.access"), None, Body::Form(&fields))
        .await
        .map_err(|e| {
            worker::console_error!("Slack's oauth.v2.access: {e:?}");
            AppError(ApiError::new(
                ErrorCode::Unavailable,
                "Slack refused the installation: try again",
            ))
        })?;
    sl::installed(&v).map_err(|e| AppError(ApiError::new(ErrorCode::Unavailable, e)))
}

/// Trades a Sign in with Slack's code for who signed in
/// (`openid.connect.token`), then revokes its token: fctl keeps none.
pub async fn oidc_token(
    cfg: &SlackConfig,
    code: &str,
    redirect_uri: &str,
) -> AppResult<sl::SignedIn> {
    let fields = [
        ("client_id", cfg.client_id.clone()),
        ("client_secret", cfg.client_secret.clone()),
        ("code", code.to_owned()),
        ("redirect_uri", redirect_uri.to_owned()),
        ("grant_type", "authorization_code".to_owned()),
    ];
    let v = send(&cfg.api("openid.connect.token"), None, Body::Form(&fields))
        .await
        .map_err(|e| {
            worker::console_error!("Slack's openid.connect.token: {e:?}");
            AppError(ApiError::new(
                ErrorCode::Unavailable,
                "Slack refused the sign-in: try again",
            ))
        })?;
    // Who signed in: the ID token's claims, else Slack's userInfo.
    let access = v
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut who = v
        .get("id_token")
        .and_then(Value::as_str)
        .and_then(sl::signed_in);
    if let Some(token) = access.as_deref() {
        if who
            .as_ref()
            .is_none_or(|w| w.team_name.is_none() || w.team_domain.is_none())
            && let Ok(info) = send(
                &cfg.api("openid.connect.userInfo"),
                Some(&format!("Bearer {token}")),
                Body::Form(&[]),
            )
            .await
        {
            let s = |k: &str| info.get(k).and_then(Value::as_str).map(str::to_owned);
            match who.as_mut() {
                Some(w) => {
                    w.team_name = w
                        .team_name
                        .take()
                        .or_else(|| s("https://slack.com/team_name"));
                    w.team_domain = w
                        .team_domain
                        .take()
                        .or_else(|| s("https://slack.com/team_domain"));
                    w.picture = w.picture.take().or_else(|| s("picture"));
                }
                None => {
                    who = (|| {
                        Some(sl::SignedIn {
                            team: s("https://slack.com/team_id")?,
                            team_name: s("https://slack.com/team_name"),
                            team_domain: s("https://slack.com/team_domain"),
                            user: s("https://slack.com/user_id").or_else(|| s("sub"))?,
                            name: s("name").unwrap_or_else(|| "Someone".into()),
                            email: s("email"),
                            email_verified: info
                                .get("email_verified")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                            picture: s("picture"),
                            nonce: None,
                        })
                    })();
                }
            }
        }
        // fctl keeps no token of the person's.
        let _ = send(
            &cfg.api("auth.revoke"),
            Some(&format!("Bearer {token}")),
            Body::Form(&[]),
        )
        .await;
    }
    who.ok_or_else(|| internal("Slack's sign-in named nobody"))
}

/// Opens a Socket Mode connection with the app-level token: its address.
pub async fn connections_open(cfg: &SlackConfig) -> Result<String, Failure> {
    let token = cfg
        .app_token
        .as_deref()
        .ok_or_else(|| Failure::Slack("no_app_token".into()))?;
    let v = send(
        &cfg.api("apps.connections.open"),
        Some(&format!("Bearer {token}")),
        Body::Form(&[]),
    )
    .await?;
    v.get("url")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Failure::Down("Slack's answer has no address".into()))
}

/// fctl's bot in one Slack workspace: the Web API with its token.
pub struct Slack {
    cfg: SlackConfig,
    token: String,
    pub team: String,
    /// The bot's user id there, which people mention.
    pub bot_user: String,
    /// The Slack workspace's subdomain, for links.
    pub domain: Option<String>,
    pub team_name: String,
    db: Option<D1Database>,
}

impl Slack {
    /// A client with a bot token fctl just got, before it keeps it.
    pub fn with_token(cfg: &SlackConfig, token: &str, team: &str, bot_user: &str) -> Self {
        Self {
            cfg: cfg.clone(),
            token: token.to_owned(),
            team: team.to_owned(),
            bot_user: bot_user.to_owned(),
            domain: None,
            team_name: String::new(),
            db: None,
        }
    }

    /// Calls a Web API method with a JSON body. A token Slack no longer
    /// takes revokes the installation.
    pub async fn call(&self, method: &str, args: Value) -> Result<Value, Failure> {
        let r = send(
            &self.cfg.api(method),
            Some(&format!("Bearer {}", self.token)),
            Body::Json(args),
        )
        .await;
        self.after(method, r).await
    }

    /// Calls a method that takes a form (those that read).
    pub async fn read(&self, method: &str, fields: &[(&str, String)]) -> Result<Value, Failure> {
        let r = send(
            &self.cfg.api(method),
            Some(&format!("Bearer {}", self.token)),
            Body::Form(fields),
        )
        .await;
        self.after(method, r).await
    }

    async fn after(&self, method: &str, r: Result<Value, Failure>) -> Result<Value, Failure> {
        if let Err(f) = &r {
            if f.revoked()
                && let Some(db) = &self.db
            {
                worker::console_error!(
                    "Slack no longer takes the bot token of {} ({method})",
                    self.team
                );
                if let Err(e) = mark_revoked(db, &self.team, crate::ctx::now_ms()).await {
                    worker::console_error!("revoking {}: {}", self.team, e.0.message);
                }
            } else if !matches!(f, Failure::Slack(c) if c == "already_reacted") {
                worker::console_log!("Slack's {method} for {}: {f:?}", self.team);
            }
        }
        r
    }

    /// Posts a message (in a thread, with `thread_ts`): its `ts`. What
    /// fctl posts never unfurls links.
    pub async fn post(
        &self,
        channel: &str,
        thread_ts: Option<&str>,
        text: &str,
        blocks: Option<&Value>,
    ) -> Result<String, Failure> {
        let mut args = json!({
            "channel": channel, "text": fallback(text),
            "unfurl_links": false, "unfurl_media": false,
        });
        if let Some(t) = thread_ts {
            put(&mut args, "thread_ts", json!(t));
        }
        if let Some(b) = blocks {
            put(&mut args, "blocks", b.clone());
        }
        let v = self.call("chat.postMessage", args).await?;
        Ok(v.get("ts")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    /// Changes a message fctl posted.
    pub async fn update(
        &self,
        channel: &str,
        ts: &str,
        text: &str,
        blocks: &Value,
    ) -> Result<(), Failure> {
        self.call(
            "chat.update",
            json!({ "channel": channel, "ts": ts, "text": fallback(text), "blocks": blocks }),
        )
        .await
        .map(|_| ())
    }

    /// A message only `user` sees.
    pub async fn ephemeral(
        &self,
        channel: &str,
        user: &str,
        thread_ts: Option<&str>,
        text: &str,
        blocks: &Value,
    ) -> Result<(), Failure> {
        let mut args =
            json!({ "channel": channel, "user": user, "text": fallback(text), "blocks": blocks });
        if let Some(t) = thread_ts {
            put(&mut args, "thread_ts", json!(t));
        }
        self.call("chat.postEphemeral", args).await.map(|_| ())
    }

    /// Reacts to a message: an emoji's name, without colons.
    pub async fn react(&self, channel: &str, ts: &str, name: &str) {
        let _ = self
            .call(
                "reactions.add",
                json!({ "channel": channel, "timestamp": ts, "name": name }),
            )
            .await;
    }

    /// What Slack says a conversation is.
    pub async fn conversation(&self, channel: &str) -> Result<Conversation, Failure> {
        let v = self
            .read("conversations.info", &[("channel", channel.to_owned())])
            .await?;
        fctl_domain::slack::access::conversation(&v)
            .ok_or_else(|| Failure::Down("Slack's answer has no conversation".into()))
    }

    /// A thread's messages, oldest first, from `oldest` (exclusive) on, at
    /// most `limit`: and whether more follow.
    pub async fn replies(
        &self,
        channel: &str,
        ts: &str,
        oldest: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<Value>, bool), Failure> {
        let mut fields = vec![
            ("channel", channel.to_owned()),
            ("ts", ts.to_owned()),
            ("limit", limit.to_string()),
        ];
        if let Some(o) = oldest {
            fields.push(("oldest", o.to_owned()));
        }
        let v = self.read("conversations.replies", &fields).await?;
        Ok(messages(&v))
    }

    /// A thread's messages, oldest first, from `oldest` (exclusive) on: page
    /// after page, up to `pages` of `limit`, as far as Slack lets fctl read
    /// at once. Slack gives an unlisted app 15 a page and a page a minute,
    /// so a short page ends it there, with whether more were left; a page
    /// Slack holds back ends it too, once one came.
    pub async fn thread(
        &self,
        channel: &str,
        ts: &str,
        oldest: Option<&str>,
        limit: u32,
        pages: u32,
    ) -> Result<(Vec<Value>, bool), Failure> {
        let mut all = Vec::new();
        let mut cursor = String::new();
        for page in 0..pages.max(1) {
            let mut fields = vec![
                ("channel", channel.to_owned()),
                ("ts", ts.to_owned()),
                ("limit", limit.to_string()),
            ];
            if let Some(o) = oldest {
                fields.push(("oldest", o.to_owned()));
            }
            if !cursor.is_empty() {
                fields.push(("cursor", cursor.clone()));
            }
            let v = match self.read("conversations.replies", &fields).await {
                Ok(v) => v,
                Err(Failure::Limited(_)) if page > 0 => return Ok((all, true)),
                Err(e) => return Err(e),
            };
            let (found, more) = messages(&v);
            let n = found.len();
            // Later pages start with the thread's first message again.
            all.extend(
                found
                    .into_iter()
                    .filter(|m| page == 0 || m.get("ts").and_then(Value::as_str) != Some(ts)),
            );
            cursor = v
                .pointer("/response_metadata/next_cursor")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if !more || cursor.is_empty() {
                return Ok((all, false));
            }
            if (n as u32) < limit {
                return Ok((all, true));
            }
        }
        Ok((all, !cursor.is_empty()))
    }

    /// A channel's messages since `oldest`, newest first, at most `limit`:
    /// and whether more follow.
    pub async fn history(
        &self,
        channel: &str,
        oldest: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<Value>, bool), Failure> {
        let mut fields = vec![
            ("channel", channel.to_owned()),
            ("limit", limit.to_string()),
        ];
        if let Some(o) = oldest {
            fields.push(("oldest", o.to_owned()));
        }
        let v = self.read("conversations.history", &fields).await?;
        Ok(messages(&v))
    }

    /// Joins a public channel, as a readable one needs.
    pub async fn join(&self, channel: &str) -> Result<(), Failure> {
        self.call("conversations.join", json!({ "channel": channel }))
            .await
            .map(|_| ())
    }

    /// The bot's direct messages with `user`: their conversation's id.
    pub async fn open_dm(&self, user: &str) -> Result<String, Failure> {
        let v = self
            .call("conversations.open", json!({ "users": user }))
            .await?;
        v.pointer("/channel/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Failure::Down("Slack's answer has no conversation".into()))
    }

    /// Who someone is in Slack.
    pub async fn user(&self, user: &str) -> Result<sl::User, Failure> {
        let v = self
            .read("users.info", &[("user", user.to_owned())])
            .await?;
        sl::user(&v).ok_or_else(|| Failure::Down("Slack's answer has no user".into()))
    }

    /// The link to a message.
    pub async fn permalink(&self, channel: &str, ts: &str) -> Option<String> {
        if self.domain.is_some() {
            return None;
        }
        let v = self
            .read(
                "chat.getPermalink",
                &[
                    ("channel", channel.to_owned()),
                    ("message_ts", ts.to_owned()),
                ],
            )
            .await
            .ok()?;
        v.get("permalink")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    /// The link to a message, as fctl writes it: from the Slack
    /// workspace's address, or asked of Slack.
    pub async fn link(&self, channel: &str, ts: &str, thread_ts: Option<&str>) -> String {
        match self.domain.as_deref() {
            Some(d) => sl::permalink(Some(d), channel, ts, thread_ts),
            None => self
                .permalink(channel, ts)
                .await
                .unwrap_or_else(|| sl::permalink(None, channel, ts, thread_ts)),
        }
    }

    /// Opens a form for the person whose click gave `trigger_id`.
    pub async fn open_view(&self, trigger_id: &str, view: &Value) -> Result<(), Failure> {
        self.call(
            "views.open",
            json!({ "trigger_id": trigger_id, "view": view }),
        )
        .await
        .map(|_| ())
    }

    /// Shows someone fctl's Home tab, made of `blocks`.
    pub async fn publish_home(&self, user: &str, blocks: &Value) -> Result<(), Failure> {
        self.call(
            "views.publish",
            json!({ "user_id": user, "view": { "type": "home", "blocks": blocks } }),
        )
        .await
        .map(|_| ())
    }

    /// Sets the agent session of a thread: `processing` while the agent
    /// works, `suspended` while it waits for an answer, `active` once its
    /// turn ended, `closed` once the session stopped. Slack shows it, with
    /// a stop button, where its agent view does; elsewhere it's refused,
    /// which changes nothing.
    pub async fn set_status(
        &self,
        channel: &str,
        thread_ts: &str,
        status: &str,
        title: Option<&str>,
        initiator: Option<&str>,
    ) {
        let mut args = json!({ "channel_id": channel, "thread_ts": thread_ts, "status": status });
        if let Some(t) = title {
            put(
                &mut args,
                "title",
                json!(fctl_domain::slack::text::clip(t, 200)),
            );
        }
        if let Some(u) = initiator {
            put(&mut args, "initiator_user_id", json!(u));
        }
        let _ = self.call("agents.sessions.setStatus", args).await;
    }

    /// Searches Slack's public channels with the Real-time Search API, as
    /// an event's action token lets fctl's bot: Slack's own results.
    pub async fn search(
        &self,
        query: &str,
        action_token: &str,
        limit: u32,
    ) -> Result<Value, Failure> {
        self.call(
            "assistant.search.context",
            json!({
                "query": query, "action_token": action_token, "channel_types": "public_channel",
                "content_types": "messages", "limit": limit.min(20),
            }),
        )
        .await
    }

    /// Starts a stream of the agent's words in a thread of someone's direct
    /// messages: its message's `ts`.
    pub async fn start_stream(
        &self,
        channel: &str,
        thread_ts: &str,
        markdown: &str,
    ) -> Result<String, Failure> {
        let v = self
            .call(
                "chat.startStream",
                json!({ "channel": channel, "thread_ts": thread_ts, "markdown_text": markdown }),
            )
            .await?;
        Ok(v.get("ts")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    pub async fn append_stream(
        &self,
        channel: &str,
        ts: &str,
        markdown: &str,
    ) -> Result<(), Failure> {
        self.call(
            "chat.appendStream",
            json!({ "channel": channel, "ts": ts, "markdown_text": markdown }),
        )
        .await
        .map(|_| ())
    }

    pub async fn stop_stream(
        &self,
        channel: &str,
        ts: &str,
        markdown: Option<&str>,
        status: &str,
    ) -> Result<(), Failure> {
        let mut args = json!({ "channel": channel, "ts": ts, "session_status": status });
        if let Some(m) = markdown.filter(|m| !m.is_empty()) {
            put(&mut args, "markdown_text", json!(m));
        }
        self.call("chat.stopStream", args).await.map(|_| ())
    }
}

/// A message's text, which Slack shows where it can't show its blocks (in
/// notifications): what anyone wrote in it can't make a link or a mention
/// there.
fn fallback(text: &str) -> String {
    sl::text::defuse(&sl::text::escape(text))
}

/// Sets `key` of a JSON object.
fn put(v: &mut Value, key: &str, value: Value) {
    if let Some(o) = v.as_object_mut() {
        o.insert(key.to_owned(), value);
    }
}

/// A list of messages from `conversations.replies` or `history`, and
/// whether more follow.
fn messages(v: &Value) -> (Vec<Value>, bool) {
    (
        v.get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        v.get("has_more").and_then(Value::as_bool).unwrap_or(false),
    )
}

// ───────────────────────────── installations ─────────────────────────────

/// fctl's app in a Slack workspace, as D1 keeps it.
#[derive(Clone, Debug, Deserialize)]
pub struct InstallationRow {
    pub team_id: String,
    pub name: String,
    pub domain: Option<String>,
    pub bot_user_id: String,
    secret_enc: String,
    pub status: String,
}

impl InstallationRow {
    pub fn active(&self) -> bool {
        self.status == "active"
    }
}

const INSTALLATION_SELECT: &str =
    "SELECT team_id, name, domain, bot_user_id, secret_enc, status FROM slack_installations";

pub async fn installation(db: &D1Database, team: &str) -> AppResult<Option<InstallationRow>> {
    q(
        db,
        &format!("{INSTALLATION_SELECT} WHERE team_id = ?"),
        &[s(team)],
    )?
    .first()
    .await
}

/// What fctl knows of a Slack workspace as its app is installed there.
pub struct Team {
    pub id: String,
    pub name: String,
    pub domain: Option<String>,
    pub icon_url: Option<String>,
}

/// Keeps the bot token of fctl's app in a Slack workspace, in place of
/// any it had there, and makes the installation active again.
pub async fn store(
    db: &D1Database,
    keys: &Keys,
    team: &Team,
    installed: &sl::Installed,
    bot_id: Option<&str>,
    by: &str,
    now: i64,
) -> AppResult<()> {
    let sealed = keys.seal_json(
        &aad(&team.id),
        &Secret {
            bot_token: installed.bot_token.clone(),
        },
    )?;
    q(
        db,
        "INSERT INTO slack_installations (team_id, name, domain, icon_url, app_id, bot_user_id, bot_id, secret_enc, scopes, status, \
         installed_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?) ON CONFLICT (team_id) DO UPDATE SET \
         name = excluded.name, domain = excluded.domain, icon_url = excluded.icon_url, app_id = excluded.app_id, \
         bot_user_id = excluded.bot_user_id, bot_id = excluded.bot_id, secret_enc = excluded.secret_enc, scopes = excluded.scopes, \
         status = 'active', installed_by = excluded.installed_by, updated_at = excluded.updated_at",
        &[
            s(&team.id),
            s(&team.name),
            os(team.domain.as_deref()),
            os(team.icon_url.as_deref()),
            os(installed.app_id.as_deref()),
            s(&installed.bot_user),
            os(bot_id),
            s(&sealed),
            s(&installed.scopes.join(",")),
            s(by),
            n(now),
            n(now),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// Marks fctl's app in a Slack workspace revoked, and forgets its token,
/// which no longer works: uninstalled there, or its tokens revoked. Every
/// fctl workspace connected to it stops working there until its admins
/// connect it again, which installs the app again.
pub async fn mark_revoked(db: &D1Database, team: &str, now: i64) -> AppResult<()> {
    crate::db::batch(
        db,
        vec![
            q(
                db,
                "UPDATE slack_installations SET status = 'revoked', secret_enc = '', updated_at = ?1 \
                 WHERE team_id = ?2 AND status = 'active'",
                &[n(now), s(team)],
            )?,
            q(
                db,
                "UPDATE workspace_slack SET revoked_at = ?1 WHERE team_id = ?2 AND revoked_at IS NULL",
                &[n(now), s(team)],
            )?,
        ],
    )
    .await
}

/// Removes fctl's app from a Slack workspace no fctl workspace works in
/// any more: Slack uninstalls it, and D1 forgets it.
pub async fn uninstall_if_unused(
    db: &D1Database,
    cfg: &Config,
    keys: &Keys,
    team: &str,
) -> AppResult<()> {
    let used: Option<crate::db::Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM workspace_slack WHERE team_id = ?",
        &[s(team)],
    )?
    .first()
    .await?;
    if used.is_some_and(|u| u.n > 0) {
        return Ok(());
    }
    if let (Some(row), Some(slack)) = (installation(db, team).await?, cfg.slack.as_ref())
        && row.active()
        && let Ok(secret) = keys.open_json::<Secret>(&aad(team), &row.secret_enc)
    {
        let fields = [
            ("client_id", slack.client_id.clone()),
            ("client_secret", slack.client_secret.clone()),
        ];
        if let Err(e) = send(
            &slack.api("apps.uninstall"),
            Some(&format!("Bearer {}", secret.bot_token)),
            Body::Form(&fields),
        )
        .await
        {
            worker::console_error!("uninstalling fctl's app from {team}: {e:?}");
        }
    }
    q(
        db,
        "DELETE FROM slack_installations WHERE team_id = ?",
        &[s(team)],
    )?
    .run()
    .await?;
    Ok(())
}

/// fctl's bot in Slack workspace `team`, while its installation works.
pub async fn client(db: &D1Database, cfg: &Config, keys: &Keys, team: &str) -> AppResult<Slack> {
    let slack = config(cfg)?;
    let row = installation(db, team).await?.ok_or_else(|| {
        not_configured("fctl's app isn't installed in this Slack workspace: an admin of the fctl workspace connects it")
    })?;
    if !row.active() {
        return Err(not_configured(&format!(
            "Slack no longer takes fctl's access to {}: an admin of the fctl workspace connects it again",
            row.name
        )));
    }
    let secret: Secret = keys.open_json(&aad(team), &row.secret_enc)?;
    Ok(Slack {
        cfg: slack.clone(),
        token: secret.bot_token,
        team: row.team_id,
        bot_user: row.bot_user_id,
        domain: row.domain,
        team_name: row.name,
        db: Some(crate::db::handle(db)),
    })
}

/// The same, from a Durable Object or a deferred task's environment.
pub async fn client_for(env: &worker::Env, team: &str) -> AppResult<(Slack, D1Database, Config)> {
    let cfg = Config::from_env(env);
    let db = env.d1("DB")?;
    let keys = crate::crypto::keys(&cfg)?;
    let c = client(&db, &cfg, &keys, team).await?;
    Ok((c, db, cfg))
}

// ───────────────────────────── people ─────────────────────────────

/// The person fctl knows as `slack_user` in Slack workspace `team`, while
/// their account is active.
pub async fn person(db: &D1Database, team: &str, slack_user: &str) -> AppResult<Option<UserRow>> {
    let user: Option<UserRow> = q(
        db,
        &format!(
            "SELECT {} FROM slack_accounts a JOIN users u ON u.id = a.user_id WHERE a.team_id = ? AND a.slack_user_id = ?",
            user_columns("u")
        ),
        &[s(team), s(slack_user)],
    )?
    .first()
    .await?;
    Ok(user.filter(UserRow::is_active))
}

/// Keeps who someone is in Slack workspace `team` for fctl account
/// `user`, in place of whoever had that Slack account: one person each.
#[allow(clippy::too_many_arguments)]
pub async fn keep_account(
    db: &D1Database,
    user: &str,
    team: &str,
    team_name: &str,
    team_domain: Option<&str>,
    who: &sl::User,
    via: fctl_proto::slack::SlackVia,
    now: i64,
) -> AppResult<()> {
    let via = match via {
        fctl_proto::slack::SlackVia::Linked => "linked",
        fctl_proto::slack::SlackVia::Email => "email",
    };
    crate::db::batch(
        db,
        vec![
            q(
                db,
                "DELETE FROM slack_accounts WHERE team_id = ? AND slack_user_id = ? AND user_id != ?",
                &[s(team), s(&who.id), s(user)],
            )?,
            q(
                db,
                "INSERT INTO slack_accounts (user_id, team_id, team_name, team_domain, slack_user_id, name, email, avatar_url, via, linked_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (user_id, team_id) DO UPDATE SET team_name = excluded.team_name, \
                 team_domain = excluded.team_domain, slack_user_id = excluded.slack_user_id, name = excluded.name, email = excluded.email, \
                 avatar_url = excluded.avatar_url, via = excluded.via, linked_at = excluded.linked_at",
                &[
                    s(user),
                    s(team),
                    s(team_name),
                    os(team_domain),
                    s(&who.id),
                    s(&who.name),
                    os(who.email.as_deref()),
                    os(who.avatar_url.as_deref()),
                    s(via),
                    n(now),
                ],
            )?,
        ],
    )
    .await
}

/// The fctl workspaces connected to Slack workspace `team`, with what each
/// lets people and agents do there, while their connection works.
#[derive(Clone, Debug, Deserialize)]
pub struct Connected {
    pub workspace_id: String,
    pub settings: String,
}

impl Connected {
    pub fn settings(&self) -> fctl_proto::slack::SlackSettings {
        serde_json::from_str(&self.settings).unwrap_or_default()
    }
}

pub async fn connected(db: &D1Database, team: &str) -> AppResult<Vec<Connected>> {
    q(
        db,
        "SELECT workspace_id, settings FROM workspace_slack WHERE team_id = ? AND revoked_at IS NULL",
        &[s(team)],
    )?
    .all()
    .await
}

/// fctl knows someone by their verified address in Slack, where they
/// haven't linked their account: where the instance allows it, and a
/// workspace connected to `team` that they are a member of turned it on
/// (`SlackSettings::match_email`). The account it matched, kept as
/// matched by address.
pub async fn match_email(
    db: &D1Database,
    slack: &Slack,
    slack_user: &str,
    now: i64,
) -> AppResult<Option<UserRow>> {
    if !crate::instance::settings(db).await?.slack_match_email {
        return Ok(None);
    }
    let workspaces: Vec<String> = connected(db, &slack.team)
        .await?
        .into_iter()
        .filter(|c| c.settings().match_email)
        .map(|c| c.workspace_id)
        .collect();
    if workspaces.is_empty() {
        return Ok(None);
    }
    let Ok(who) = slack.user(slack_user).await else {
        return Ok(None);
    };
    let Some(email) = who
        .email
        .clone()
        .filter(|_| !who.bot && !who.deleted && !who.guest)
    else {
        return Ok(None);
    };
    // An address of an active account (fctl verified each as it was
    // added) that is a member of one of those workspaces: an address alone
    // says nothing of anyone else.
    let user: Option<UserRow> = q(
        db,
        &format!(
            "SELECT {} FROM user_emails e JOIN users u ON u.id = e.user_id WHERE e.email = lower(?1) \
             AND u.status = 'active' AND EXISTS (SELECT 1 FROM workspace_members wm \
             WHERE wm.user_id = u.id AND wm.workspace_id IN (SELECT value FROM json_each(?2))) LIMIT 1",
            user_columns("u")
        ),
        &[s(&email), s(&serde_json::to_string(&workspaces)?)],
    )?
    .first()
    .await?;
    let Some(user) = user else {
        return Ok(None);
    };
    if !may_match(db, &user.id, &slack.team).await? {
        return Ok(None);
    }
    keep_account(
        db,
        &user.id,
        &slack.team,
        &slack.team_name,
        slack.domain.as_deref(),
        &who,
        fctl_proto::slack::SlackVia::Email,
        now,
    )
    .await?;
    Ok(Some(user))
}

/// Whether an address may make fctl account `user` someone in Slack
/// workspace `team`: not where fctl knows them there already, as another
/// Slack user. An address never takes over an account they linked, or
/// one an earlier match found.
pub(crate) async fn may_match(db: &D1Database, user: &str, team: &str) -> AppResult<bool> {
    let known: Option<crate::db::Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM slack_accounts WHERE user_id = ? AND team_id = ?",
        &[s(user), s(team)],
    )?
    .first()
    .await?;
    Ok(known.is_none_or(|k| k.n == 0))
}

/// Who `slack_user` is in fctl: their linked account, or the one their
/// verified address matches where that's allowed. An account matched by
/// its address counts only while that's allowed still: once nobody allows
/// it, fctl forgets it.
pub async fn who(
    db: &D1Database,
    slack: &Slack,
    slack_user: &str,
    now: i64,
) -> AppResult<Option<UserRow>> {
    #[derive(Deserialize)]
    struct Via {
        via: String,
    }
    let Some(user) = person(db, &slack.team, slack_user).await? else {
        return match_email(db, slack, slack_user, now).await;
    };
    let via: Option<Via> = q(
        db,
        "SELECT via FROM slack_accounts WHERE team_id = ? AND slack_user_id = ?",
        &[s(&slack.team), s(slack_user)],
    )?
    .first()
    .await?;
    if via.is_none_or(|v| v.via != "email") || email_allowed(db, &slack.team, &user.id).await? {
        return Ok(Some(user));
    }
    q(
        db,
        "DELETE FROM slack_accounts WHERE team_id = ? AND slack_user_id = ? AND via = 'email'",
        &[s(&slack.team), s(slack_user)],
    )?
    .run()
    .await?;
    Ok(None)
}

/// Whether fctl may know `user` by their address in Slack workspace
/// `team`: the instance allows it, and a workspace connected to it that
/// they are a member of turned it on.
async fn email_allowed(db: &D1Database, team: &str, user: &str) -> AppResult<bool> {
    if !crate::instance::settings(db).await?.slack_match_email {
        return Ok(false);
    }
    let workspaces: Vec<String> = connected(db, team)
        .await?
        .into_iter()
        .filter(|c| c.settings().match_email)
        .map(|c| c.workspace_id)
        .collect();
    if workspaces.is_empty() {
        return Ok(false);
    }
    let member: Option<crate::db::Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM workspace_members WHERE user_id = ? AND workspace_id IN (SELECT value FROM json_each(?))",
        &[s(user), s(&serde_json::to_string(&workspaces)?)],
    )?
    .first()
    .await?;
    Ok(member.is_some_and(|m| m.n > 0))
}

/// Gives up fctl's app in the Slack workspaces no fctl workspace works in
/// any more: one whose last workspace was deleted (the hourly job).
pub async fn uninstall_unused(env: &worker::Env, db: &D1Database) -> AppResult<()> {
    #[derive(Deserialize)]
    struct Row {
        team_id: String,
    }
    let unused: Vec<Row> = q(
        db,
        "SELECT team_id FROM slack_installations i WHERE NOT EXISTS \
         (SELECT 1 FROM workspace_slack ws WHERE ws.team_id = i.team_id) LIMIT 20",
        &[],
    )?
    .all()
    .await?;
    if unused.is_empty() {
        return Ok(());
    }
    let cfg = Config::from_env(env);
    let keys = crate::crypto::keys(&cfg)?;
    for r in unused {
        uninstall_if_unused(db, &cfg, &keys, &r.team_id).await?;
    }
    Ok(())
}

/// Drops what Slack's messages said from the transcripts of sessions that
/// ended, where their workspace asks for that
/// (`SlackTranscripts::Drop`): sessions from Slack, stopped, failed or
/// archived, not dropped yet, a few at a time (the hourly job). Returns
/// how many.
pub async fn drop_ended(env: &worker::Env, db: &D1Database, now: i64) -> AppResult<usize> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
        settings: String,
    }
    let rows: Vec<Row> = q(
        db,
        "SELECT s.id, ws.settings FROM sessions s JOIN projects p ON p.id = s.project_id \
         JOIN workspace_slack ws ON ws.workspace_id = p.workspace_id \
         WHERE s.slack IS NOT NULL AND json_extract(s.slack, '$.team') = ws.team_id \
         AND (s.status IN ('stopped', 'failed') OR s.archived = 1) AND s.updated_at < ? \
         AND json_extract(ws.settings, '$.transcripts') = 'drop' \
         AND (json_extract(s.slack, '$.dropped_at') IS NULL OR json_extract(s.slack, '$.dropped_at') < s.updated_at) \
         ORDER BY s.updated_at LIMIT 25",
        &[n(now - 10 * 60 * 1000)],
    )?
    .all()
    .await?;
    let mut dropped = 0;
    for r in rows {
        let settings: fctl_proto::slack::SlackSettings =
            serde_json::from_str(&r.settings).unwrap_or_default();
        if settings.transcripts != fctl_proto::slack::SlackTranscripts::Drop {
            continue;
        }
        let stub = crate::internal::session_stub(env, &r.id)?;
        let answer: AppResult<Value> =
            crate::internal::post(&stub, "/slack-drop", &json!({})).await;
        match answer {
            Ok(_) => {
                q(
                    db,
                    "UPDATE sessions SET slack = json_set(slack, '$.dropped_at', ?) WHERE id = ?",
                    &[n(now), s(&r.id)],
                )?
                .run()
                .await?;
                dropped += 1;
            }
            Err(e) => worker::console_error!("dropping Slack from {}: {}", r.id, e.0.message),
        }
    }
    Ok(dropped)
}

/// A project's choice of a channel for its events, checked as it is
/// saved: a channel of a Slack workspace its workspace works in, that lets
/// projects post there, not shared with another organization, that fctl's
/// app is in (it joins a public one) or can be; and events fctl knows.
pub async fn check_events(
    ctx: &crate::ctx::Ctx,
    workspace: &str,
    events: Option<fctl_proto::slack::SlackEvents>,
) -> AppResult<Option<fctl_proto::slack::SlackEvents>> {
    let Some(mut e) = events else {
        return Ok(None);
    };
    let field = |m: &str| crate::error::validation("slack", m);
    let conn = connected(&ctx.db, &e.team)
        .await?
        .into_iter()
        .find(|c| c.workspace_id == workspace)
        .ok_or_else(|| field("The project's workspace doesn't work in that Slack workspace"))?;
    if !conn.settings().project_events {
        return Err(field(
            "The workspace's admins keep projects' events out of Slack: they turn it on under Integrations → Slack",
        ));
    }
    for k in &e.kinds {
        if fctl_domain::hooks::Kind::parse(k).is_none() {
            return Err(field(&format!("fctl has no event called {k}")));
        }
    }
    let keys = ctx.keys()?;
    let client = client(&ctx.db, &ctx.cfg, &keys, &e.team).await?;
    let channel = e.channel.trim().to_owned();
    if !sl::is_id(&channel) {
        return Err(field("Choose a channel"));
    }
    let info = client.conversation(&channel).await?;
    if info.shared {
        return Err(field(
            "fctl doesn't post in channels shared with other organizations (Slack Connect)",
        ));
    }
    if !info.member {
        if info.place == fctl_proto::slack::SlackPlace::Channel {
            client.join(&channel).await?;
        } else {
            return Err(field(&format!(
                "Invite fctl's app to #{} first: it posts only where it's a member",
                info.name.as_deref().unwrap_or("the channel")
            )));
        }
    }
    e.channel = channel;
    e.channel_name = info.name.unwrap_or(e.channel_name);
    Ok(Some(e))
}
