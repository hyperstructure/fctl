//! Slack (docs/architecture.md#slack): what Slack sends fctl's app
//! (`/api/v1/slack/events`, `/interactivity`, `/commands`, each signed with
//! the app's signing secret), the Slack sign-ins that install the app in a
//! Slack workspace (`/api/v1/slack/connect`, back at `/installed`) and link
//! someone's account (`/api/v1/slack/link`, back at `/linked`), the Slack
//! workspaces an fctl workspace works in, with what it lets people and
//! agents do there (`/api/v1/workspaces/{id}/slack`), your Slack accounts
//! (`/api/v1/me/slack`), and the instance's app (`/api/v1/admin/slack`).
//!
//! Installing fctl's app in a Slack workspace takes someone who may add
//! apps there (Slack may ask one of its admins to approve it), and
//! connecting it to an fctl workspace one of the fctl workspace's admins:
//! the same person, signing in to Slack from fctl. A person's own Sign in
//! with Slack shows fctl who they are there, which fctl keeps, and its
//! token is revoked at once: what they ask fctl's app there then runs as
//! them, and nobody else can claim their Slack account.

use crate::crypto::{self, Keys};
use crate::ctx::Ctx;
use crate::db::{Count, n, os, q, s};
use crate::error::{AppError, AppResult, forbidden, not_found, validation};
use crate::http;
use crate::model::{PROJECT_SELECT, ProjectRow};
use crate::slack;
use crate::workspaces as ws;
use fctl_domain::cookies::Cookie;
use fctl_domain::slack::{self as sl, manifest, task};
use fctl_domain::verify::Sensitive;
use fctl_proto::slack::{
    AdminSlack, AdminSlackTeam, ConnectSlackRequest, MySlack, SetSlackChannelRequest, SlackAccount,
    SlackChannel, SlackChannelChoice, SlackChannelList, SlackChannelRef, SlackConnection,
    SlackLink, SlackProject, SlackProjectRef, SlackProjectUse, SlackSettings, SlackStatus,
    SlackTeamRef, SlackVia, SlackWorkspaceRef, UpdateMySlackRequest, UpdateSlackRequest,
    WorkspaceSlack,
};
use fctl_proto::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use worker::{Method, Request, Response};

/// How long a link that connects a Slack workspace, and the Slack sign-in
/// it starts, work.
const CONNECT_TTL_MS: i64 = 10 * 60 * 1000;

// ───────────────────────────── what Slack sends ─────────────────────────────

/// Whether `segs` are one of Slack's routes ([`route`]).
pub fn routes(segs: &[&str]) -> bool {
    matches!(
        segs,
        ["slack", ..] | ["me", "slack", ..] | ["workspaces", _, "slack", ..] | ["admin", "slack"]
    )
}

/// Slack's routes, apart from `crate::dispatch`'s: Slack's requests, the
/// ways to and back from Slack's sign-in, workspaces' Slack, your Slack
/// accounts and the instance's Slack app.
pub async fn route(
    ctx: &mut Ctx,
    req: &mut Request,
    method: Method,
    segs: &[&str],
) -> AppResult<Response> {
    use Method::*;
    match (method, segs) {
        // Slack's own requests, signed with its secret, not by a person.
        (Post, ["slack", "events"]) => events(ctx, req).await,
        (Post, ["slack", "interactivity"]) => interactivity(ctx, req).await,
        (Post, ["slack", "commands"]) => commands(ctx, req).await,
        (Get, ["slack", "connect"]) => connect_link(ctx, req).await,
        (Get, ["slack", "installed"]) => installed(ctx, req).await,
        (Get, ["slack", "link"]) => link(ctx, req).await,
        (Get, ["slack", "linked"]) => linked(ctx, req).await,
        (Get, ["workspaces", id, "slack"]) => workspace(ctx, id).await,
        (Post, ["workspaces", id, "slack", "connect"]) => connect(ctx, req, id).await,
        (Patch, ["workspaces", id, "slack", team]) => update(ctx, req, id, team).await,
        (Delete, ["workspaces", id, "slack", team]) => disconnect(ctx, id, team).await,
        (Get, ["workspaces", id, "slack", team, "channels"]) => channels(ctx, req, id, team).await,
        (Put, ["workspaces", id, "slack", team, "channels", channel]) => {
            set_channel(ctx, req, id, team, channel).await
        }
        (Get, ["me", "slack"]) => mine(ctx).await,
        (Post, ["me", "slack", "link"]) => link_for(ctx, req).await,
        (Patch, ["me", "slack", team]) => update_mine(ctx, req, team).await,
        (Delete, ["me", "slack", team]) => unlink(ctx, team).await,
        (Get, ["admin", "slack"]) => admin(ctx).await,
        _ => Err(not_found("Endpoint")),
    }
}

/// The body of a request from Slack, once its signature counts.
async fn signed_body(ctx: &Ctx, req: &mut Request) -> AppResult<Vec<u8>> {
    let cfg = slack::config(&ctx.cfg)?;
    let Some(secret) = cfg.signing_secret.clone() else {
        return Err(crate::error::not_configured(
            "This instance takes no Slack requests over HTTP: its admins set SLACK_SIGNING_SECRET",
        ));
    };
    if http::length(req).is_some_and(|l| l > sl::MAX_BODY_BYTES as u64) {
        return Err(AppError(ApiError::bad_request("The request is too large")));
    }
    let body = http::read_upto(req, sl::MAX_BODY_BYTES).await?;
    if body.len() > sl::MAX_BODY_BYTES {
        return Err(AppError(ApiError::bad_request("The request is too large")));
    }
    let timestamp = http::header(req, "X-Slack-Request-Timestamp").unwrap_or_default();
    let signature = http::header(req, "X-Slack-Signature").unwrap_or_default();
    if !sl::signed(&secret, &timestamp, &body, &signature, ctx.now) {
        return Err(AppError(ApiError::new(
            ErrorCode::Unauthorized,
            "The request isn't signed with the Slack app's signing secret, or it's more than five minutes old",
        )));
    }
    Ok(body)
}

fn form(body: &[u8]) -> Vec<(String, String)> {
    url::form_urlencoded::parse(body)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// `POST /api/v1/slack/events`: the Events API. Slack wants an answer
/// within three seconds, so what takes longer happens after it.
pub async fn events(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body = signed_body(ctx, req).await?;
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return Err(AppError(ApiError::bad_request("The request is not JSON")));
    };
    match sl::envelope(&v) {
        sl::Envelope::UrlVerification { challenge } => http::ok(&json!({ "challenge": challenge })),
        sl::Envelope::Event(e) => {
            let env = ctx.env.clone();
            let cfg = ctx.cfg.clone();
            let db = crate::db::handle(&ctx.db);
            let now = ctx.now;
            ctx.defer(
                async move { crate::slack::inbound::event(&env, &cfg, &db, &e, &v, now).await },
            );
            http::ok(&json!({}))
        }
        sl::Envelope::Other => http::ok(&json!({})),
    }
}

/// `POST /api/v1/slack/interactivity`: buttons, menus, forms and
/// shortcuts, as a form whose `payload` is JSON.
pub async fn interactivity(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body = signed_body(ctx, req).await?;
    let fields = form(&body);
    let Some(payload) = fields
        .iter()
        .find(|(k, _)| k == "payload")
        .and_then(|(_, p)| serde_json::from_str::<Value>(p).ok())
    else {
        return Err(AppError(ApiError::bad_request(
            "The request has no payload",
        )));
    };
    let i = sl::interaction(&payload);
    let quick = slack::inbound::quick(&i);
    let env = ctx.env.clone();
    let cfg = ctx.cfg.clone();
    let db = crate::db::handle(&ctx.db);
    let now = ctx.now;
    ctx.defer(async move { crate::slack::inbound::interaction(&env, &cfg, &db, &i, now).await });
    match quick {
        Some(answer) => http::ok(&answer),
        None => Ok(Response::empty()?.with_status(200)),
    }
}

/// `POST /api/v1/slack/commands`: `/fctl`, as a form.
pub async fn commands(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body = signed_body(ctx, req).await?;
    let fields = form(&body);
    let get = |k: &str| fields.iter().find(|(f, _)| f == k).map(|(_, v)| v.clone());
    if get("ssl_check").as_deref() == Some("1") {
        return http::ok(&json!({}));
    }
    let c = sl::command(get);
    let quick = slack::inbound::quick_command(&c);
    let env = ctx.env.clone();
    let cfg = ctx.cfg.clone();
    let db = crate::db::handle(&ctx.db);
    let now = ctx.now;
    ctx.defer(async move { crate::slack::inbound::command(&env, &cfg, &db, &c, now).await });
    if quick.as_object().is_some_and(|o| o.is_empty()) {
        Ok(Response::empty()?.with_status(200))
    } else {
        http::ok(&quick)
    }
}

// ───────────────────────────── signing in to Slack ─────────────────────────────

/// A link that connects a Slack workspace to an fctl workspace, signed with
/// the master key.
#[derive(Serialize, Deserialize)]
struct ConnectTicket {
    purpose: String,
    workspace: String,
    user: String,
    /// The Slack workspace the admin asked to install fctl's app in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    team: Option<String>,
    exp: i64,
}

const CONNECT: &str = "slack-connect";

/// A Slack sign-in on its way, signed, in its cookie.
#[derive(Serialize, Deserialize)]
struct SignIn {
    purpose: String,
    user: String,
    /// Installing fctl's app for this workspace; `None`: linking the
    /// person's account.
    #[serde(default)]
    workspace: Option<String>,
    /// The Slack workspace it's for, and the thread that asked them to
    /// link their account.
    #[serde(default)]
    team: Option<String>,
    #[serde(default)]
    thread: Option<String>,
    state: String,
    #[serde(default)]
    nonce: String,
    exp: i64,
}

const SIGN_IN: &str = "slack-sign-in";

fn keys_for(ctx: &Ctx) -> AppResult<Keys> {
    slack::config(&ctx.cfg)?;
    ctx.keys()
}

fn urls(ctx: &Ctx) -> manifest::Urls {
    manifest::Urls::of(&ctx.cfg.public_url)
}

/// Sends people who aren't signed in to fctl to sign in first, and back.
fn sign_in_first(req: &Request) -> AppResult<Response> {
    let here = req.url()?;
    let next: String = url::form_urlencoded::byte_serialize(
        format!("{}?{}", here.path(), here.query().unwrap_or_default()).as_bytes(),
    )
    .collect();
    http::redirect(&format!("/login?next={next}"))
}

fn clean(v: Option<String>, max: usize) -> Option<String> {
    v.map(|t| t.trim().to_owned()).filter(|t| {
        !t.is_empty()
            && t.len() <= max
            && t.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b':' | b'_'))
    })
}

/// Goes on to Slack to sign in, with what the sign-in does in its cookie.
fn begin(ctx: &mut Ctx, keys: &Keys, mut sign_in: SignIn) -> AppResult<Response> {
    let cfg = slack::config(&ctx.cfg)?.clone();
    sign_in.purpose = SIGN_IN.into();
    sign_in.state = crypto::b64url(&fctl_domain::random_bytes::<24>());
    sign_in.nonce = crypto::b64url(&fctl_domain::random_bytes::<16>());
    sign_in.exp = ctx.now + CONNECT_TTL_MS;
    let u = urls(ctx);
    let url = match &sign_in.workspace {
        Some(_) => sl::authorize_url(
            &format!("{}/oauth/v2/authorize", cfg.url),
            &cfg.client_id,
            &u.installed,
            &sign_in.state,
            sign_in.team.as_deref(),
        ),
        None => sl::oidc_url(
            &format!("{}/openid/connect/authorize", cfg.url),
            &cfg.client_id,
            &u.linked,
            &sign_in.state,
            &sign_in.nonce,
            sign_in.team.as_deref(),
        ),
    };
    let signed = keys.sign(&serde_json::to_vec(&sign_in)?);
    ctx.cookies.push(http::set_cookie(
        Cookie::SlackConnect,
        &signed,
        (CONNECT_TTL_MS / 1000) as u64,
        ctx.cfg.secure_cookies(),
    ));
    http::redirect(&url)
}

/// The sign-in a Slack redirect comes back from, once it checks out.
fn returned(ctx: &mut Ctx, req: &Request, keys: &Keys, user: &str) -> Option<SignIn> {
    let secure = ctx.cfg.secure_cookies();
    let kept = http::cookie(req, Cookie::SlackConnect, secure);
    ctx.cookies
        .push(http::set_cookie(Cookie::SlackConnect, "", 0, secure));
    kept.and_then(|k| keys.verify(&k))
        .and_then(|b| serde_json::from_slice::<SignIn>(&b).ok())
        .filter(|st| st.purpose == SIGN_IN && st.user == user)
}

/// `POST /api/v1/workspaces/{id}/slack/connect` (admins): the link to the
/// Slack sign-in that installs fctl's app in a Slack workspace and
/// connects it.
pub async fn connect(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: ConnectSlackRequest = if http::header(req, "Content-Type").is_some() {
        http::body(req).await?
    } else {
        ConnectSlackRequest::default()
    };
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let team = match body
        .team
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        Some(t) if sl::is_id(t) && t.starts_with('T') => Some(t.to_owned()),
        Some(_) => {
            return Err(AppError(ApiError::bad_request(
                "team is a Slack workspace's id, as T0123ABCD",
            )));
        }
        None => None,
    };
    let keys = keys_for(ctx)?;
    let expires_at = ctx.now + CONNECT_TTL_MS;
    let ticket = keys.sign(&serde_json::to_vec(&ConnectTicket {
        purpose: CONNECT.into(),
        workspace: a.row.id.clone(),
        user: ctx.user()?.id.clone(),
        team,
        exp: expires_at,
    })?);
    let encoded: String = url::form_urlencoded::byte_serialize(ticket.as_bytes()).collect();
    http::ok(&SlackLink {
        url: ctx
            .cfg
            .url(&format!("/api/v1/slack/connect?ticket={encoded}")),
        expires_at,
    })
}

/// Where the browser goes back to in the workspace, after a Slack sign-in.
fn back_to_workspace(slug: &str, outcome: &str) -> String {
    format!("/w/{slug}?tab=integrations&integration=slack&slack={outcome}")
}

/// `GET /api/v1/slack/connect?ticket=…`: a link that connects a Slack
/// workspace goes on to Slack, in the browser of the admin it is for.
pub async fn connect_link(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let Ok(keys) = keys_for(ctx) else {
        return http::redirect("/");
    };
    let ticket = http::query(req, "ticket")
        .and_then(|t| keys.verify(&t))
        .and_then(|b| serde_json::from_slice::<ConnectTicket>(&b).ok())
        .filter(|t| t.purpose == CONNECT);
    let Some(ticket) = ticket else {
        return http::redirect("/");
    };
    let Ok(a) = ws::load(ctx, &ticket.workspace).await else {
        return http::redirect("/");
    };
    let slug = a.row.slug.clone();
    if ticket.exp <= ctx.now {
        return http::redirect(&back_to_workspace(&slug, "expired"));
    }
    if ticket.user != user.id {
        return http::redirect(&back_to_workspace(&slug, "other_user"));
    }
    if a.require_admin().is_err() {
        return http::redirect(&back_to_workspace(&slug, "failed"));
    }
    begin(
        ctx,
        &keys,
        SignIn {
            purpose: String::new(),
            user: user.id,
            workspace: Some(a.row.id.clone()),
            team: ticket.team,
            thread: None,
            state: String::new(),
            nonce: String::new(),
            exp: 0,
        },
    )
}

/// `GET /api/v1/slack/installed`: Slack sends the admin back here from
/// installing fctl's app.
pub async fn installed(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let Ok(keys) = keys_for(ctx) else {
        return http::redirect("/");
    };
    let Some(sign_in) = returned(ctx, req, &keys, &user.id) else {
        return http::redirect("/");
    };
    let Some(workspace) = sign_in.workspace.clone() else {
        return http::redirect("/");
    };
    let Ok(a) = ws::load(ctx, &workspace).await else {
        return http::redirect("/");
    };
    let slug = a.row.slug.clone();
    if sign_in.exp <= ctx.now
        || http::query(req, "state").as_deref() != Some(sign_in.state.as_str())
    {
        return http::redirect(&back_to_workspace(&slug, "expired"));
    }
    if http::query(req, "error").is_some() {
        return http::redirect(&back_to_workspace(&slug, "denied"));
    }
    let Some(code) = http::query(req, "code") else {
        return http::redirect(&back_to_workspace(&slug, "failed"));
    };
    match install(ctx, &keys, &workspace, &code).await {
        Ok(team) => http::redirect(&format!(
            "{}&team={}",
            back_to_workspace(&slug, "connected"),
            url::form_urlencoded::byte_serialize(team.as_bytes()).collect::<String>()
        )),
        Err(e) => {
            worker::console_error!("a Slack install: {}", e.0.message);
            let outcome = if e.0.message.contains("org-wide") {
                "enterprise"
            } else {
                "failed"
            };
            http::redirect(&back_to_workspace(&slug, outcome))
        }
    }
}

/// Keeps fctl's app's token for the Slack workspace an admin installed it
/// in, and connects it to their fctl workspace.
/// Answers the Slack workspace's id.
async fn install(ctx: &mut Ctx, keys: &Keys, workspace: &str, code: &str) -> AppResult<String> {
    let a = ws::load(ctx, workspace).await?;
    a.require_admin()?;
    let cfg = slack::config(&ctx.cfg)?.clone();
    let installed = slack::oauth_access(&cfg, code, &urls(ctx).installed).await?;
    if installed.enterprise || installed.team.is_empty() {
        return Err(forbidden(
            "fctl doesn't take Enterprise Grid org-wide installs yet: install it in one Slack workspace",
        ));
    }
    let bot = slack::Slack::with_token(
        &cfg,
        &installed.bot_token,
        &installed.team,
        &installed.bot_user,
    );
    // The bot's id, the Slack workspace's address and its icon.
    let test = bot.call("auth.test", json!({})).await.ok();
    let bot_id = test
        .as_ref()
        .and_then(|t| t.get("bot_id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let domain = test
        .as_ref()
        .and_then(|t| t.get("url"))
        .and_then(Value::as_str)
        .and_then(|u| url::Url::parse(u).ok())
        .and_then(|u| u.host_str().map(str::to_owned))
        .and_then(|h| h.strip_suffix(".slack.com").map(str::to_owned));
    let info = bot
        .read("team.info", &[("team", installed.team.clone())])
        .await
        .ok();
    let icon = info
        .as_ref()
        .and_then(|i| {
            i.pointer("/team/icon/image_88")
                .or_else(|| i.pointer("/team/icon/image_68"))
        })
        .and_then(Value::as_str)
        .filter(|u| !u.contains("default"))
        .map(str::to_owned);
    let team = slack::Team {
        id: installed.team.clone(),
        name: info
            .as_ref()
            .and_then(|i| i.pointer("/team/name"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| installed.team_name.clone()),
        domain: domain.or_else(|| {
            info.as_ref()
                .and_then(|i| i.pointer("/team/domain"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        }),
        icon_url: icon,
    };
    let user = ctx.user()?.clone();
    slack::store(
        &ctx.db,
        keys,
        &team,
        &installed,
        bot_id.as_deref(),
        &user.id,
        ctx.now,
    )
    .await?;
    q(
        &ctx.db,
        "INSERT INTO workspace_slack (workspace_id, team_id, connected_by, slack_user, settings, created_at) VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT (workspace_id, team_id) DO UPDATE SET connected_by = excluded.connected_by, slack_user = excluded.slack_user, revoked_at = NULL",
        &[
            s(a.id()),
            s(&team.id),
            s(&user.id),
            os(installed.installer.as_deref()),
            s(&serde_json::to_string(&SlackSettings::default())?),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(a.id()),
        "workspace.slack_connected",
        Some(&team.id),
        Some(json!({ "name": team.name, "domain": team.domain })),
    );
    Ok(team.id)
}

/// `GET /api/v1/slack/link[?team=…][&thread=…]`: links the signed-in
/// person's Slack account, from Settings, or from the message fctl sent
/// them in Slack the first time they asked. Anyone can send anyone this
/// link: it only ever links the account they sign in to Slack with to
/// their own.
pub async fn link(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let Ok(keys) = keys_for(ctx) else {
        return http::redirect("/settings?tab=slack");
    };
    let team = clean(http::query(req, "team"), 32).filter(|t| sl::is_id(t));
    let thread = clean(http::query(req, "thread"), 128);
    // What the account asks then starts sessions as them, so a stolen
    // session can't link one alone, where the instance asks for that: the
    // app confirms, then comes back here.
    if ctx.verified(Sensitive::LinkSlack).is_err() {
        let mut back = url::form_urlencoded::Serializer::new(String::new());
        back.append_pair("tab", "slack")
            .append_pair("slack", "confirm");
        if let Some(t) = &team {
            back.append_pair("team", t);
        }
        if let Some(t) = &thread {
            back.append_pair("thread", t);
        }
        return http::redirect(&format!("/settings?{}", back.finish()));
    }
    begin(
        ctx,
        &keys,
        SignIn {
            purpose: String::new(),
            user: user.id,
            workspace: None,
            team,
            thread,
            state: String::new(),
            nonce: String::new(),
            exp: 0,
        },
    )
}

/// `GET /api/v1/slack/linked`: Slack sends people back here from signing
/// in with Slack.
pub async fn linked(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let back = |outcome: &str| format!("/settings?tab=slack&slack={outcome}");
    let Ok(keys) = keys_for(ctx) else {
        return http::redirect("/");
    };
    let Some(sign_in) = returned(ctx, req, &keys, &user.id).filter(|s| s.workspace.is_none())
    else {
        return http::redirect(&back("failed"));
    };
    if sign_in.exp <= ctx.now
        || http::query(req, "state").as_deref() != Some(sign_in.state.as_str())
    {
        return http::redirect(&back("expired"));
    }
    if http::query(req, "error").is_some() {
        return http::redirect(&back("denied"));
    }
    let Some(code) = http::query(req, "code") else {
        return http::redirect(&back("failed"));
    };
    match link_account(ctx, &code, &sign_in).await {
        Ok(Some(session)) => http::redirect(&format!("/s/{session}")),
        Ok(None) => http::redirect(&back("linked")),
        Err(e) => {
            worker::console_error!("a Slack sign-in: {}", e.0.message);
            http::redirect(&back("failed"))
        }
    }
}

/// Keeps who the person signed in to Slack as, and takes up what waited
/// for them: returns the session to show them, when the thread that asked
/// them to link their account started one.
async fn link_account(ctx: &mut Ctx, code: &str, sign_in: &SignIn) -> AppResult<Option<String>> {
    let cfg = slack::config(&ctx.cfg)?.clone();
    let who = slack::oidc_token(&cfg, code, &urls(ctx).linked).await?;
    if who.nonce.as_deref().is_some_and(|n| n != sign_in.nonce) {
        return Err(forbidden(
            "The Slack sign-in doesn't match the one fctl started",
        ));
    }
    let user = ctx.user()?.clone();
    let person = sl::User {
        id: who.user.clone(),
        name: who.name.clone(),
        handle: None,
        email: who.email.clone(),
        avatar_url: who.picture.clone(),
        bot: false,
        deleted: false,
        guest: false,
        tz: None,
    };
    let team_name = who.team_name.clone().unwrap_or_else(|| "Slack".into());
    slack::keep_account(
        &ctx.db,
        &user.id,
        &who.team,
        &team_name,
        who.team_domain.as_deref(),
        &person,
        SlackVia::Linked,
        ctx.now,
    )
    .await?;
    ctx.audit(
        "user.slack_linked",
        Some(&who.team),
        Some(json!({ "team": team_name, "slack_user": who.user })),
    );
    if ctx.cfg.email.is_some() {
        let instance = crate::instance::name(&ctx.db, &ctx.cfg)
            .await
            .unwrap_or_else(|_| ctx.cfg.instance_name.clone());
        let who_words = match who.email.as_deref() {
            Some(e) if !e.is_empty() => format!("{} ({e})", who.name),
            _ => who.name.clone(),
        };
        let text = format!(
            "{who_words}, in the Slack workspace {team_name}, is now linked to your {instance} account ({}): what it asks fctl's app there starts sessions as you, on your Claude subscription, and its replies reach your sessions' agents.\n\nIf you didn't link it, unlink it in Settings now, sign out your other devices there and change your password: {}",
            user.email,
            ctx.cfg.url("/settings?tab=slack")
        );
        crate::email::notice(
            ctx,
            &user,
            "A Slack account was linked to your account",
            text,
        );
    }
    let reached =
        crate::slack::inbound::linked(&ctx.env, &ctx.cfg, &ctx.db, &who.team, &who.user, ctx.now)
            .await
            .unwrap_or_else(|e| {
                worker::console_error!("taking up requests after a link: {}", e.0.message);
                Vec::new()
            });
    // From the thread that asked them: on to what it started.
    Ok(sign_in.thread.as_ref().and(reached.first().cloned()))
}

// ───────────────────────────── a workspace's ─────────────────────────────

#[derive(Deserialize)]
struct ConnectionRow {
    team_id: String,
    name: String,
    domain: Option<String>,
    icon_url: Option<String>,
    bot_user_id: String,
    status: String,
    settings: String,
    slack_user: Option<String>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    revoked_at: Option<i64>,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    connected_by: Option<String>,
    connector_name: Option<String>,
    connector_avatar: Option<String>,
}

#[derive(Deserialize)]
struct ChannelRow {
    team_id: String,
    channel_id: String,
    name: String,
    #[serde(deserialize_with = "crate::db::int_bool", default)]
    private: bool,
    project_id: Option<String>,
    #[serde(deserialize_with = "crate::db::int_bool", default)]
    readable: bool,
    #[serde(deserialize_with = "crate::db::int")]
    updated_at: i64,
}

/// `GET /api/v1/workspaces/{id}/slack`.
pub async fn workspace(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_open()?;
    http::ok(&respond(ctx, a.id()).await?)
}

async fn respond(ctx: &Ctx, workspace_id: &str) -> AppResult<WorkspaceSlack> {
    let user = ctx.user()?.clone();
    let rows: Vec<ConnectionRow> = q(
        &ctx.db,
        "SELECT ws.team_id, i.name, i.domain, i.icon_url, i.bot_user_id, i.status, ws.settings, ws.slack_user, ws.revoked_at, \
         ws.created_at, ws.connected_by, u.name AS connector_name, u.avatar_url AS connector_avatar FROM workspace_slack ws \
         JOIN slack_installations i ON i.team_id = ws.team_id LEFT JOIN users u ON u.id = ws.connected_by \
         WHERE ws.workspace_id = ? ORDER BY ws.created_at",
        &[s(workspace_id)],
    )?
    .all()
    .await?;
    let channels: Vec<ChannelRow> = q(
        &ctx.db,
        "SELECT team_id, channel_id, name, private, project_id, readable, updated_at FROM slack_channels WHERE workspace_id = ? ORDER BY name",
        &[s(workspace_id)],
    )?
    .all()
    .await?;
    let projects: Vec<ProjectRow> = q(
        &ctx.db,
        &format!(
            "{PROJECT_SELECT} WHERE p.workspace_id = ?2 AND p.archived_at IS NULL ORDER BY p.name"
        ),
        &[s(&user.id), s(workspace_id)],
    )?
    .all()
    .await?;
    let instance = crate::instance::settings(&ctx.db).await?;
    let seen: Vec<&ProjectRow> = projects
        .iter()
        .filter(|p| crate::access::project_role(&user, &instance, ctx.limits(), p).is_some())
        .collect();
    // Only the projects the caller sees: a channel mapped to another says
    // nothing of it.
    let project_ref = |pid: &str| {
        seen.iter().find(|p| p.id == pid).map(|p| SlackProjectRef {
            id: fctl_proto::ProjectId(p.id.clone()),
            slug: p.slug.clone(),
            name: p.name.clone(),
        })
    };
    let linked: Vec<String> = {
        #[derive(Deserialize)]
        struct T {
            team_id: String,
        }
        let t: Vec<T> = q(
            &ctx.db,
            "SELECT team_id FROM slack_accounts WHERE user_id = ?",
            &[s(&user.id)],
        )?
        .all()
        .await?;
        t.into_iter().map(|t| t.team_id).collect()
    };
    let mut connections = Vec::new();
    for r in rows {
        let settings: SlackSettings = serde_json::from_str(&r.settings).unwrap_or_default();
        let mine: Vec<&ChannelRow> = channels.iter().filter(|c| c.team_id == r.team_id).collect();
        let readable: Vec<String> = mine
            .iter()
            .filter(|c| c.readable && !c.private)
            .map(|c| c.name.clone())
            .collect();
        let count: Option<Count> = q(
            &ctx.db,
            "SELECT COUNT(*) AS n FROM slack_accounts a JOIN workspace_members m ON m.user_id = a.user_id \
             WHERE a.team_id = ? AND m.workspace_id = ?",
            &[s(&r.team_id), s(workspace_id)],
        )?
        .first()
        .await?;
        connections.push(SlackConnection {
            team: r.team_id.clone(),
            name: r.name,
            domain: r.domain,
            icon_url: r.icon_url,
            bot_user: Some(r.bot_user_id),
            app_name: Some(ctx.cfg.instance_name.clone()),
            status: if r.status == "active" && r.revoked_at.is_none() {
                SlackStatus::Active
            } else {
                SlackStatus::Revoked
            },
            connected_by: r.connected_by.map(|id| fctl_proto::session::UserRef {
                id: fctl_proto::UserId(id),
                name: r.connector_name.unwrap_or_default(),
                avatar_url: r.connector_avatar,
            }),
            slack_user: r.slack_user,
            created_at: r.created_at,
            reach: task::reach_sentence(&readable, settings.search),
            channels: mine
                .iter()
                .map(|c| SlackChannel {
                    id: c.channel_id.clone(),
                    name: c.name.clone(),
                    project: c.project_id.as_deref().and_then(project_ref),
                    readable: c.readable,
                    private: c.private,
                    updated_at: c.updated_at,
                })
                .collect(),
            projects: seen
                .iter()
                .map(|p| {
                    let st: SlackProject = p.settings().slack;
                    SlackProjectUse {
                        id: fctl_proto::ProjectId(p.id.clone()),
                        slug: p.slug.clone(),
                        name: p.name.clone(),
                        read_channels: st.read_channels,
                        events: st.events.filter(|e| e.team == r.team_id),
                        mapped: mine
                            .iter()
                            .filter(|c| c.project_id.as_deref() == Some(p.id.as_str()))
                            .map(|c| SlackChannelRef {
                                id: c.channel_id.clone(),
                                name: c.name.clone(),
                            })
                            .collect(),
                    }
                })
                .collect(),
            settings,
            linked: count.map_or(0, |c| c.n as u32),
        });
    }
    let slack_cfg = ctx.cfg.slack.as_ref();
    let socket = slack_cfg.is_some_and(|c| c.app_token.is_some());
    let socket_connected = if socket {
        crate::slack::socket::kick(&ctx.env)
            .await
            .map(|s| s.connected)
    } else {
        None
    };
    Ok(WorkspaceSlack {
        configured: slack_cfg.is_some(),
        events: slack_cfg.is_some_and(|c| c.events()),
        socket_mode: socket,
        socket_connected,
        match_email: instance.slack_match_email,
        instance_admin: user.role == "owner" || user.role == "admin",
        connections,
        linked_teams: linked,
    })
}

/// A connection of the workspace's, for its admins to change.
async fn connection(
    ctx: &Ctx,
    id: &str,
    team: &str,
) -> AppResult<(ws::WorkspaceAccess, SlackSettings)> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let st = settings_in(ctx, a.id(), team).await?;
    Ok((a, st))
}

/// What workspace `workspace` lets people and agents do in Slack
/// workspace `team`, where it works there.
async fn settings_in(ctx: &Ctx, workspace: &str, team: &str) -> AppResult<SlackSettings> {
    #[derive(Deserialize)]
    struct Row {
        settings: String,
    }
    let row: Option<Row> = q(
        &ctx.db,
        "SELECT settings FROM workspace_slack WHERE workspace_id = ? AND team_id = ?",
        &[s(workspace), s(team)],
    )?
    .first()
    .await?;
    let row = row.ok_or_else(|| not_found("Slack workspace"))?;
    Ok(serde_json::from_str(&row.settings).unwrap_or_default())
}

/// `PATCH /api/v1/workspaces/{id}/slack/{team}` (admins): what the
/// workspace lets people and agents do there.
pub async fn update(ctx: &mut Ctx, req: &mut Request, id: &str, team: &str) -> AppResult<Response> {
    let body: UpdateSlackRequest = http::body(req).await?;
    let (a, mut st) = connection(ctx, id, team).await?;
    let before = st.clone();
    if let Some(v) = body.channels {
        st.channels = v;
    }
    if let Some(v) = body.direct_messages {
        st.direct_messages = v;
    }
    if let Some(v) = body.search {
        st.search = v;
    }
    if let Some(v) = body.agent_replies {
        st.agent_replies = v;
    }
    if let Some(v) = body.notifications {
        st.notifications = v;
    }
    if let Some(v) = body.project_events {
        st.project_events = v;
    }
    if let Some(v) = body.transcripts {
        st.transcripts = v;
    }
    if let Some(v) = body.match_email {
        if v && !crate::instance::settings(&ctx.db).await?.slack_match_email {
            return Err(validation(
                "match_email",
                "This instance's admins don't let workspaces know people by their address in Slack",
            ));
        }
        st.match_email = v;
    }
    q(
        &ctx.db,
        "UPDATE workspace_slack SET settings = ? WHERE workspace_id = ? AND team_id = ?",
        &[s(&serde_json::to_string(&st)?), s(a.id()), s(team)],
    )?
    .run()
    .await?;
    if st != before {
        ctx.audit_in(
            Some(a.id()),
            "workspace.slack_updated",
            Some(team),
            Some(serde_json::to_value(&st)?),
        );
    }
    http::ok(&respond(ctx, a.id()).await?)
}

/// `DELETE /api/v1/workspaces/{id}/slack/{team}` (admins): the workspace
/// stops working in a Slack workspace. Once no fctl workspace works there,
/// fctl's app uninstalls itself.
pub async fn disconnect(ctx: &mut Ctx, id: &str, team: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let removed = q(
        &ctx.db,
        "DELETE FROM workspace_slack WHERE workspace_id = ? AND team_id = ?",
        &[s(a.id()), s(team)],
    )?
    .run()
    .await?;
    if removed == 0 {
        return Err(not_found("Slack workspace"));
    }
    q(
        &ctx.db,
        "DELETE FROM slack_channels WHERE workspace_id = ? AND team_id = ?",
        &[s(a.id()), s(team)],
    )?
    .run()
    .await?;
    if let Ok(keys) = ctx.keys() {
        slack::uninstall_if_unused(&ctx.db, &ctx.cfg, &keys, team).await?;
    }
    ctx.audit_in(
        Some(a.id()),
        "workspace.slack_disconnected",
        Some(team),
        None,
    );
    http::ok(&respond(ctx, a.id()).await?)
}

/// `GET /api/v1/workspaces/{id}/slack/{team}/channels[?query=…][&project=…]`:
/// public channels there, for its admins to map and make readable, and for
/// the maintainers of `project`, one of the workspace's, to post its events
/// in where the workspace lets projects do that.
pub async fn channels(ctx: &Ctx, req: &Request, id: &str, team: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    let st = settings_in(ctx, a.id(), team).await?;
    if a.require_admin().is_err() {
        let Some(project) = http::query(req, "project") else {
            return Err(forbidden(
                "Only the workspace's admins list its Slack channels, and projects' maintainers to choose where events go",
            ));
        };
        let p = crate::access::project(ctx, &project).await?;
        if p.workspace_id() != a.id() {
            return Err(not_found("project"));
        }
        p.require_maintainer()?;
        if !st.project_events {
            return Err(forbidden(
                "The workspace's admins keep projects' events out of Slack: they turn it on under Integrations → Slack",
            ));
        }
    }
    let keys = ctx.keys()?;
    let client = slack::client(&ctx.db, &ctx.cfg, &keys, team).await?;
    let query = http::query(req, "query")
        .map(|q| q.trim().trim_start_matches('#').to_lowercase())
        .filter(|q| !q.is_empty());
    let mut out: Vec<SlackChannelChoice> = Vec::new();
    let mut cursor = String::new();
    let mut more = false;
    for _ in 0..5 {
        let mut fields = vec![
            ("types", "public_channel".to_owned()),
            ("exclude_archived", "true".to_owned()),
            ("limit", "500".to_owned()),
        ];
        if !cursor.is_empty() {
            fields.push(("cursor", cursor.clone()));
        }
        let v = client.read("conversations.list", &fields).await?;
        for c in v
            .get("channels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let name = c.get("name").and_then(Value::as_str).unwrap_or_default();
            if query.as_deref().is_some_and(|q| !name.contains(q)) {
                continue;
            }
            if c.get("is_ext_shared").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            out.push(SlackChannelChoice {
                id: c
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                name: name.to_owned(),
                topic: c
                    .pointer("/topic/value")
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .map(|t| sl::text::clip(t, 120)),
                members: c
                    .get("num_members")
                    .and_then(Value::as_u64)
                    .map(|n| n as u32),
                member: c.get("is_member").and_then(Value::as_bool).unwrap_or(false),
                private: false,
            });
        }
        cursor = v
            .pointer("/response_metadata/next_cursor")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if cursor.is_empty() {
            break;
        }
        more = true;
    }
    more = more && !cursor.is_empty();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    let total = out.len();
    out.truncate(200);
    http::ok(&SlackChannelList {
        channels: out,
        more: more || total > 200,
    })
}

/// `PUT /api/v1/workspaces/{id}/slack/{team}/channels/{channel}` (admins):
/// the project a channel's mentions start sessions in, and whether agents
/// read it. fctl's app joins a public channel it maps or makes readable,
/// which shows there.
pub async fn set_channel(
    ctx: &mut Ctx,
    req: &mut Request,
    id: &str,
    team: &str,
    channel: &str,
) -> AppResult<Response> {
    let body: SetSlackChannelRequest = http::body(req).await?;
    let (a, _) = connection(ctx, id, team).await?;
    if !sl::is_id(channel) {
        return Err(not_found("Channel"));
    }
    let user = ctx.user()?.clone();
    #[derive(Deserialize)]
    struct Kept {
        name: String,
        project_id: Option<String>,
        #[serde(deserialize_with = "crate::db::int_bool", default)]
        readable: bool,
        #[serde(deserialize_with = "crate::db::int_bool", default)]
        private: bool,
    }
    let kept: Option<Kept> = q(
        &ctx.db,
        "SELECT name, project_id, readable, private FROM slack_channels WHERE workspace_id = ? AND team_id = ? AND channel_id = ?",
        &[s(a.id()), s(team), s(channel)],
    )?
    .first()
    .await?;
    let project = match body.project.as_deref().map(str::trim) {
        None => kept.as_ref().and_then(|k| k.project_id.clone()),
        Some("") => None,
        Some(p) => {
            let row: Option<ProjectRow> = q(
                &ctx.db,
                &format!(
                    "{PROJECT_SELECT} WHERE (p.id = ?2 OR p.slug = ?2) AND p.workspace_id = ?3 AND p.archived_at IS NULL"
                ),
                &[s(&user.id), s(p), s(a.id())],
            )?
            .first()
            .await?;
            Some(
                row.ok_or_else(|| {
                    validation("project", "That isn't one of this workspace's projects")
                })?
                .id,
            )
        }
    };
    let readable = body
        .readable
        .unwrap_or_else(|| kept.as_ref().is_some_and(|k| k.readable));
    let keys = ctx.keys()?;
    let client = slack::client(&ctx.db, &ctx.cfg, &keys, team).await?;
    let info = client.conversation(channel).await?;
    if info.shared {
        return Err(validation(
            "channel",
            "fctl doesn't work in channels shared with other organizations (Slack Connect) yet",
        ));
    }
    let private = info.place != fctl_proto::slack::SlackPlace::Channel;
    if readable && private {
        return Err(validation(
            "readable",
            "Agents read public channels only: in a private channel, they read their own thread alone",
        ));
    }
    let name = info
        .name
        .clone()
        .or(body.name.clone())
        .or(kept.as_ref().map(|k| k.name.clone()))
        .unwrap_or_else(|| channel.to_owned());
    if project.is_none() && !readable {
        q(
            &ctx.db,
            "DELETE FROM slack_channels WHERE workspace_id = ? AND team_id = ? AND channel_id = ?",
            &[s(a.id()), s(team), s(channel)],
        )?
        .run()
        .await?;
    } else {
        // In public channels, fctl's app joins to hear its mentions and
        // read it; a private one's members invite it themselves.
        if !private && !info.member {
            client.join(channel).await?;
        }
        q(
            &ctx.db,
            "INSERT INTO slack_channels (workspace_id, team_id, channel_id, name, private, project_id, readable, added_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (workspace_id, team_id, channel_id) DO UPDATE SET name = excluded.name, \
             private = excluded.private, project_id = excluded.project_id, readable = excluded.readable, updated_at = excluded.updated_at",
            &[
                s(a.id()),
                s(team),
                s(channel),
                s(&name),
                n(i64::from(private)),
                os(project.as_deref()),
                n(i64::from(readable)),
                s(&user.id),
                n(ctx.now),
                n(ctx.now),
            ],
        )?
        .run()
        .await?;
    }
    let _ = kept.map(|k| k.private);
    ctx.audit_in(
        Some(a.id()),
        "workspace.slack_channel_set",
        Some(channel),
        Some(json!({ "name": name, "project": project, "readable": readable, "team": team })),
    );
    http::ok(&respond(ctx, a.id()).await?)
}

// ───────────────────────────── yours ─────────────────────────────

#[derive(Deserialize)]
struct AccountRow {
    team_id: String,
    team_name: String,
    team_domain: Option<String>,
    slack_user_id: String,
    name: String,
    email: Option<String>,
    avatar_url: Option<String>,
    via: String,
    dm_project_id: Option<String>,
    dm_slug: Option<String>,
    dm_name: Option<String>,
    #[serde(deserialize_with = "crate::db::int")]
    linked_at: i64,
}

/// Someone's Slack accounts, as Settings and their data export list them.
pub async fn accounts(ctx: &Ctx, user_id: &str) -> AppResult<Vec<SlackAccount>> {
    let rows: Vec<AccountRow> = q(
        &ctx.db,
        "SELECT a.team_id, a.team_name, a.team_domain, a.slack_user_id, a.name, a.email, a.avatar_url, a.via, a.dm_project_id, \
         p.slug AS dm_slug, p.name AS dm_name, a.linked_at FROM slack_accounts a LEFT JOIN projects p ON p.id = a.dm_project_id \
         WHERE a.user_id = ? ORDER BY a.linked_at",
        &[s(user_id)],
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| SlackAccount {
            team: r.team_id,
            team_name: r.team_name,
            team_domain: r.team_domain,
            slack_user: r.slack_user_id,
            name: r.name,
            email: r.email,
            avatar_url: r.avatar_url,
            via: if r.via == "email" {
                SlackVia::Email
            } else {
                SlackVia::Linked
            },
            dm_project: match (r.dm_project_id, r.dm_slug, r.dm_name) {
                (Some(id), Some(slug), Some(name)) => Some(SlackProjectRef {
                    id: fctl_proto::ProjectId(id),
                    slug,
                    name,
                }),
                _ => None,
            },
            linked_at: r.linked_at,
        })
        .collect())
}

/// The Slack workspaces someone's workspaces work in.
async fn teams(ctx: &Ctx, user_id: &str) -> AppResult<Vec<SlackTeamRef>> {
    #[derive(Deserialize)]
    struct Row {
        team_id: String,
        name: String,
        domain: Option<String>,
        icon_url: Option<String>,
        workspace_id: String,
        slug: String,
        workspace_name: String,
    }
    let rows: Vec<Row> = q(
        &ctx.db,
        "SELECT i.team_id, i.name, i.domain, i.icon_url, w.id AS workspace_id, w.slug, w.name AS workspace_name \
         FROM workspace_slack ws JOIN slack_installations i ON i.team_id = ws.team_id JOIN workspaces w ON w.id = ws.workspace_id \
         JOIN workspace_members m ON m.workspace_id = w.id AND m.user_id = ? WHERE ws.revoked_at IS NULL AND i.status = 'active' \
         ORDER BY i.name, w.name",
        &[s(user_id)],
    )?
    .all()
    .await?;
    let mut out: Vec<SlackTeamRef> = Vec::new();
    for r in rows {
        let w = SlackWorkspaceRef {
            id: fctl_proto::WorkspaceId(r.workspace_id),
            slug: r.slug,
            name: r.workspace_name,
        };
        match out.iter_mut().find(|t| t.team == r.team_id) {
            Some(t) => t.workspaces.push(w),
            None => out.push(SlackTeamRef {
                team: r.team_id,
                name: r.name,
                domain: r.domain,
                icon_url: r.icon_url,
                workspaces: vec![w],
            }),
        }
    }
    Ok(out)
}

/// `GET /api/v1/me/slack`.
pub async fn mine(ctx: &Ctx) -> AppResult<Response> {
    let user = ctx.user()?;
    http::ok(&MySlack {
        configured: ctx.cfg.slack.is_some(),
        accounts: accounts(ctx, &user.id).await?,
        teams: teams(ctx, &user.id).await?,
    })
}

/// `POST /api/v1/me/slack/link[?team=…]`: the link that links your Slack
/// account, for a browser to open (the CLI's `fctl account slack link`).
pub async fn link_for(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    ctx.user()?;
    slack::config(&ctx.cfg)?;
    let team = clean(http::query(req, "team"), 32).filter(|t| sl::is_id(t));
    let url = match team {
        Some(t) => ctx.cfg.url(&format!("/api/v1/slack/link?team={t}")),
        None => ctx.cfg.url("/api/v1/slack/link"),
    };
    http::ok(&SlackLink {
        url,
        expires_at: ctx.now + CONNECT_TTL_MS,
    })
}

/// `PATCH /api/v1/me/slack/{team}`: the project your direct messages with
/// fctl's app start sessions in.
pub async fn update_mine(ctx: &mut Ctx, req: &mut Request, team: &str) -> AppResult<Response> {
    let body: UpdateMySlackRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    if let Some(p) = body.dm_project.as_deref().map(str::trim) {
        let project = if p.is_empty() {
            None
        } else {
            let row: Option<ProjectRow> = q(
                &ctx.db,
                &format!(
                    "{PROJECT_SELECT} WHERE (p.id = ?2 OR p.slug = ?2) AND p.archived_at IS NULL"
                ),
                &[s(&user.id), s(p)],
            )?
            .first()
            .await?;
            let instance = crate::instance::settings(&ctx.db).await?;
            let row = row
                .filter(|r| {
                    fctl_domain::perm::can_start_sessions(crate::access::project_role(
                        &user, &instance, None, r,
                    ))
                })
                .ok_or_else(|| {
                    validation("dm_project", "You can't start sessions in that project")
                })?;
            Some(row.id)
        };
        let changed = q(
            &ctx.db,
            "UPDATE slack_accounts SET dm_project_id = ? WHERE user_id = ? AND team_id = ?",
            &[os(project.as_deref()), s(&user.id), s(team)],
        )?
        .run()
        .await?;
        if changed == 0 {
            return Err(not_found("Slack account"));
        }
    }
    mine(ctx).await
}

/// `DELETE /api/v1/me/slack/{team}`: fctl forgets who you are there, and
/// what you ask fctl's app there starts nothing until you link it again.
pub async fn unlink(ctx: &mut Ctx, team: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let removed = q(
        &ctx.db,
        "DELETE FROM slack_accounts WHERE user_id = ? AND team_id = ?",
        &[s(&user.id), s(team)],
    )?
    .run()
    .await?;
    if removed == 0 {
        return Err(not_found("Slack account"));
    }
    ctx.audit("user.slack_unlinked", Some(team), None);
    mine(ctx).await
}

// ───────────────────────────── the instance's ─────────────────────────────

/// `GET /api/v1/admin/slack` (instance admins): the Slack app, and the
/// manifest that creates one with this instance's addresses.
pub async fn admin(ctx: &Ctx) -> AppResult<Response> {
    ctx.admin()?;
    let cfg = ctx.cfg.slack.as_ref();
    let reachable = manifest::reachable(&ctx.cfg.public_url);
    let socket = cfg.is_some_and(|c| c.app_token.is_some());
    // Socket Mode where it's set up, or where Slack can't reach the
    // instance anyway.
    let m = manifest::manifest(
        &ctx.cfg.instance_name,
        &ctx.cfg.public_url,
        socket || !reachable,
    );
    let u = urls(ctx);
    let status = if socket {
        crate::slack::socket::kick(&ctx.env).await
    } else {
        None
    };
    #[derive(Deserialize)]
    struct Row {
        team_id: String,
        name: String,
        domain: Option<String>,
        status: String,
        #[serde(deserialize_with = "crate::db::int")]
        n: i64,
        #[serde(deserialize_with = "crate::db::int")]
        created_at: i64,
    }
    let rows: Vec<Row> = q(
        &ctx.db,
        "SELECT i.team_id, i.name, i.domain, i.status, i.created_at, (SELECT COUNT(*) FROM workspace_slack ws WHERE ws.team_id = i.team_id) AS n \
         FROM slack_installations i ORDER BY i.name",
        &[],
    )?
    .all()
    .await?;
    http::ok(&AdminSlack {
        configured: cfg.is_some(),
        signing_secret: cfg.is_some_and(|c| c.signing_secret.is_some()),
        socket_mode: socket,
        socket_connected: status.as_ref().map(|s| s.connected),
        socket_since: status.as_ref().and_then(|s| s.since),
        socket_error: status.and_then(|s| s.error),
        reachable,
        manifest_yaml: manifest::yaml(&m),
        create_url: manifest::create_url(&m),
        manifest: m,
        events_url: u.events,
        interactivity_url: u.interactivity,
        commands_url: u.commands,
        redirect_urls: vec![u.installed, u.linked],
        teams: rows
            .into_iter()
            .map(|r| AdminSlackTeam {
                team: r.team_id,
                name: r.name,
                domain: r.domain,
                status: if r.status == "active" {
                    SlackStatus::Active
                } else {
                    SlackStatus::Revoked
                },
                workspaces: r.n as u32,
                installed_at: r.created_at,
            })
            .collect(),
        match_email: crate::instance::settings(&ctx.db).await?.slack_match_email,
    })
}
