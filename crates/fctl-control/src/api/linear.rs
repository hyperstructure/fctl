//! Linear (docs/architecture.md#linear): the Linear workspaces fctl
//! workspaces work in (`/api/v1/workspaces/{id}/linear`), people's Linear
//! accounts (`/api/v1/me/linear`), the Linear sign-ins that install fctl's
//! app in a Linear workspace (`/api/v1/linear/connect`) and link someone's
//! account (`/api/v1/linear/link`), which come back to
//! `/api/v1/linear/installed`, the app's redirect URI, and the app's
//! webhook, `/api/v1/linear/webhook`.
//!
//! Installing fctl's app in a Linear workspace takes one of its admins
//! (Linear asks for that), and connecting it to an fctl workspace one of
//! the fctl workspace's admins: the same person, signing in to Linear from
//! fctl. A person's own sign-in shows fctl who they are in Linear, which
//! fctl keeps, and is revoked at once: what they delegate there then runs
//! as them, and nobody else can claim their Linear account.

use crate::crypto::{self, Keys};
use crate::ctx::Ctx;
use crate::db::{n, os, q, s};
use crate::error::{AppError, AppResult, not_configured, not_found};
use crate::http;
use crate::linear;
use crate::workspaces as ws;
use fctl_domain::cookies::Cookie;
use fctl_domain::linear as lin;
use fctl_proto::tickets::{
    ConnectLinearRequest, LinearAccount, LinearLink, LinearTeamList, MyLinear, WorkspaceLinear,
};
use fctl_proto::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use worker::{Request, Response};

/// How long a link that connects a Linear workspace, and the Linear sign-in
/// it starts, work.
const CONNECT_TTL_MS: i64 = 10 * 60 * 1000;

/// A link that connects a Linear workspace to an fctl workspace
/// (`POST /api/v1/workspaces/{id}/linear/connect`), signed with the master
/// key.
#[derive(Serialize, Deserialize)]
struct ConnectTicket {
    purpose: String,
    workspace: String,
    user: String,
    #[serde(default)]
    project: Option<String>,
    exp: i64,
}

const CONNECT: &str = "linear-connect";

/// A Linear sign-in on its way, signed, in its cookie.
#[derive(Serialize, Deserialize)]
struct SignIn {
    purpose: String,
    user: String,
    /// Installing fctl's app for this workspace; `None`: linking the
    /// person's account.
    #[serde(default)]
    workspace: Option<String>,
    /// The project whose Tickets tab it started from, by slug.
    #[serde(default)]
    project: Option<String>,
    /// The agent session in Linear that asked them to link it.
    #[serde(default)]
    thread: Option<String>,
    state: String,
    verifier: String,
    exp: i64,
}

const SIGN_IN: &str = "linear-sign-in";

fn keys_for(ctx: &Ctx) -> AppResult<Keys> {
    linear::config(&ctx.cfg)?;
    ctx.keys()
}

fn redirect_uri(ctx: &Ctx) -> String {
    ctx.cfg.url("/api/v1/linear/installed")
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

/// A slug as a project's is: what goes back into a URL.
fn clean_slug(slug: Option<&str>) -> Option<String> {
    slug.map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .filter(|s| s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .map(str::to_owned)
}

// ───────────────────────────── a workspace's ─────────────────────────────

/// `GET /api/v1/workspaces/{id}/linear`.
pub async fn workspace(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_open()?;
    http::ok(&respond(ctx, a.id()).await?)
}

async fn respond(ctx: &Ctx, workspace_id: &str) -> AppResult<WorkspaceLinear> {
    Ok(WorkspaceLinear {
        configured: ctx.cfg.linear.is_some(),
        webhook: ctx
            .cfg
            .linear
            .as_ref()
            .is_some_and(|l| l.webhook_secret.is_some()),
        connections: crate::tickets::connections(&ctx.db, workspace_id).await?,
    })
}

/// `POST /api/v1/workspaces/{id}/linear/connect` (admins): the link to the
/// Linear sign-in that installs fctl's app in a Linear workspace and
/// connects it.
pub async fn connect(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: ConnectLinearRequest = if http::header(req, "Content-Type").is_some() {
        http::body(req).await?
    } else {
        ConnectLinearRequest::default()
    };
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let keys = keys_for(ctx)?;
    let expires_at = ctx.now + CONNECT_TTL_MS;
    let ticket = keys.sign(&serde_json::to_vec(&ConnectTicket {
        purpose: CONNECT.into(),
        workspace: a.row.id.clone(),
        user: ctx.user()?.id.clone(),
        project: clean_slug(body.project.as_deref()),
        exp: expires_at,
    })?);
    let encoded: String = url::form_urlencoded::byte_serialize(ticket.as_bytes()).collect();
    http::ok(&LinearLink {
        url: ctx
            .cfg
            .url(&format!("/api/v1/linear/connect?ticket={encoded}")),
        expires_at,
    })
}

/// `DELETE /api/v1/workspaces/{id}/linear/{organization}` (admins): the
/// workspace stops working in a Linear workspace. Its projects that took
/// tickets from it take none, and once no fctl workspace works there,
/// fctl's app gives up its access.
pub async fn disconnect(ctx: &mut Ctx, id: &str, organization: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let removed = q(
        &ctx.db,
        "DELETE FROM workspace_linear WHERE workspace_id = ? AND organization_id = ?",
        &[s(a.id()), s(organization)],
    )?
    .run()
    .await?;
    if removed == 0 {
        return Err(not_found("Linear workspace"));
    }
    if let Ok(keys) = ctx.keys() {
        linear::uninstall_if_unused(&ctx.db, &ctx.cfg, &keys, organization).await?;
    }
    ctx.audit_in(
        Some(a.id()),
        "workspace.linear_disconnected",
        Some(organization),
        None,
    );
    http::ok(&respond(ctx, a.id()).await?)
}

/// `GET /api/v1/workspaces/{id}/linear/{organization}/teams`: the teams
/// fctl's app reaches there, for a project to choose from.
pub async fn teams(ctx: &Ctx, id: &str, organization: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_open()?;
    let connected = crate::tickets::connections(&ctx.db, a.id())
        .await?
        .into_iter()
        .any(|c| c.organization == organization);
    if !connected {
        return Err(not_found("Linear workspace"));
    }
    let keys = ctx.keys()?;
    let client = linear::client(&ctx.db, &ctx.cfg, &keys, organization, ctx.now).await?;
    http::ok(&LinearTeamList {
        teams: crate::tickets::teams(&client).await?,
    })
}

// ───────────────────────────── yours ─────────────────────────────

#[derive(Deserialize)]
struct AccountRow {
    organization_id: String,
    organization_name: String,
    url_key: Option<String>,
    linear_user_id: String,
    name: String,
    email: Option<String>,
    avatar_url: Option<String>,
    #[serde(deserialize_with = "crate::db::int")]
    linked_at: i64,
}

impl AccountRow {
    fn to_account(&self) -> LinearAccount {
        LinearAccount {
            organization: self.organization_id.clone(),
            organization_name: self.organization_name.clone(),
            url_key: self.url_key.clone(),
            linear_user: self.linear_user_id.clone(),
            name: self.name.clone(),
            email: self.email.clone(),
            avatar_url: self.avatar_url.clone(),
            linked_at: self.linked_at,
        }
    }
}

/// Someone's Linear accounts, as Settings and their data export list them.
pub async fn accounts(ctx: &Ctx, user_id: &str) -> AppResult<Vec<LinearAccount>> {
    let rows: Vec<AccountRow> = q(
        &ctx.db,
        "SELECT organization_id, organization_name, url_key, linear_user_id, name, email, avatar_url, linked_at \
         FROM linear_accounts WHERE user_id = ? ORDER BY linked_at",
        &[s(user_id)],
    )?
    .all()
    .await?;
    Ok(rows.iter().map(AccountRow::to_account).collect())
}

/// `GET /api/v1/me/linear`.
pub async fn mine(ctx: &Ctx) -> AppResult<Response> {
    let user = ctx.user()?;
    http::ok(&MyLinear {
        configured: ctx.cfg.linear.is_some(),
        accounts: accounts(ctx, &user.id).await?,
    })
}

/// `DELETE /api/v1/me/linear/{organization}`: fctl forgets who you are
/// there, and what you delegate there starts nothing until you link it
/// again.
pub async fn unlink(ctx: &mut Ctx, organization: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let removed = q(
        &ctx.db,
        "DELETE FROM linear_accounts WHERE user_id = ? AND organization_id = ?",
        &[s(&user.id), s(organization)],
    )?
    .run()
    .await?;
    if removed == 0 {
        return Err(not_found("Linear account"));
    }
    ctx.audit("user.linear_unlinked", Some(organization), None);
    mine(ctx).await
}

// ───────────────────────────── signing in to Linear ─────────────────────────────

/// Goes on to Linear to sign in, with what the sign-in does in its cookie.
fn begin(ctx: &mut Ctx, keys: &Keys, mut sign_in: SignIn) -> AppResult<Response> {
    let cfg = linear::config(&ctx.cfg)?;
    let (verifier, challenge) = crypto::pkce_pair();
    sign_in.purpose = SIGN_IN.into();
    sign_in.state = crypto::b64url(&fctl_domain::random_bytes::<24>());
    sign_in.verifier = verifier;
    sign_in.exp = ctx.now + CONNECT_TTL_MS;
    let url = lin::authorize_url(
        &cfg.authorize_url,
        &cfg.client_id,
        &redirect_uri(ctx),
        &sign_in.state,
        &challenge,
        sign_in.workspace.is_some(),
    );
    let signed = keys.sign(&serde_json::to_vec(&sign_in)?);
    ctx.cookies.push(http::set_cookie(
        Cookie::LinearConnect,
        &signed,
        (CONNECT_TTL_MS / 1000) as u64,
        ctx.cfg.secure_cookies(),
    ));
    http::redirect(&url)
}

/// `GET /api/v1/linear/connect?ticket=…`: a link that connects a Linear
/// workspace goes on to Linear, in the browser of the admin it is for.
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
    let back = Back::Workspace {
        slug: a.row.slug.clone(),
        project: ticket.project.clone(),
    };
    if ticket.exp <= ctx.now {
        return http::redirect(&back.url("expired"));
    }
    if ticket.user != user.id {
        return http::redirect(&back.url("other_user"));
    }
    if a.require_admin().is_err() {
        return http::redirect(&back.url("failed"));
    }
    begin(
        ctx,
        &keys,
        SignIn {
            purpose: String::new(),
            user: user.id,
            workspace: Some(a.row.id.clone()),
            project: ticket.project,
            thread: None,
            state: String::new(),
            verifier: String::new(),
            exp: 0,
        },
    )
}

/// `GET /api/v1/linear/link[?project=…][&thread=…]`: links the signed-in
/// person's Linear account, from Settings, a project's Tickets tab, or
/// Linear's own button in an agent session that waits for it. Anyone can
/// send anyone this link: it only ever links the account they sign in to
/// Linear with to their own.
pub async fn link(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let Ok(keys) = keys_for(ctx) else {
        return http::redirect("/settings");
    };
    let thread = http::query(req, "thread").filter(|t| {
        !t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    begin(
        ctx,
        &keys,
        SignIn {
            purpose: String::new(),
            user: user.id,
            workspace: None,
            project: clean_slug(http::query(req, "project").as_deref()),
            thread,
            state: String::new(),
            verifier: String::new(),
            exp: 0,
        },
    )
}

/// Where the browser goes back to after a Linear sign-in.
enum Back {
    /// The project's Tickets tab, else the workspace's Linear tab.
    Workspace {
        slug: String,
        project: Option<String>,
    },
    /// The project's Tickets tab, else Settings.
    Person { project: Option<String> },
}

impl Back {
    fn url(&self, outcome: &str) -> String {
        match self {
            Self::Workspace {
                project: Some(p), ..
            }
            | Self::Person { project: Some(p) } => {
                format!("/p/{p}?tab=tickets&linear={outcome}")
            }
            Self::Workspace {
                slug,
                project: None,
            } => {
                format!("/w/{slug}?tab=linear&linear={outcome}")
            }
            Self::Person { project: None } => format!("/settings?linear={outcome}#linear"),
        }
    }
}

/// `GET /api/v1/linear/installed`: the Linear app's redirect URI, where
/// Linear sends people back from signing in.
pub async fn installed(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let secure = ctx.cfg.secure_cookies();
    let kept = http::cookie(req, Cookie::LinearConnect, secure);
    ctx.cookies
        .push(http::set_cookie(Cookie::LinearConnect, "", 0, secure));
    let Ok(keys) = keys_for(ctx) else {
        return http::redirect("/");
    };
    let Some(sign_in) = kept
        .and_then(|k| keys.verify(&k))
        .and_then(|b| serde_json::from_slice::<SignIn>(&b).ok())
        .filter(|st| st.purpose == SIGN_IN && st.user == user.id)
    else {
        return http::redirect("/");
    };
    let back = match &sign_in.workspace {
        Some(w) => match ws::load(ctx, w).await {
            Ok(a) => Back::Workspace {
                slug: a.row.slug,
                project: sign_in.project.clone(),
            },
            Err(_) => return http::redirect("/"),
        },
        None => Back::Person {
            project: sign_in.project.clone(),
        },
    };
    if sign_in.exp <= ctx.now
        || http::query(req, "state").as_deref() != Some(sign_in.state.as_str())
    {
        return http::redirect(&back.url("expired"));
    }
    if http::query(req, "error").is_some() {
        return http::redirect(&back.url("denied"));
    }
    let Some(code) = http::query(req, "code") else {
        return http::redirect(&back.url("failed"));
    };
    let outcome = match &sign_in.workspace {
        Some(w) => install(ctx, &keys, w, &code, &sign_in.verifier).await,
        None => linked(ctx, &code, &sign_in.verifier, sign_in.thread.as_deref()).await,
    };
    match outcome {
        Ok(Some(session)) => http::redirect(&format!("/s/{session}")),
        Ok(None) => http::redirect(&back.url(if sign_in.workspace.is_some() {
            "connected"
        } else {
            "linked"
        })),
        Err(e) => {
            worker::console_error!("a Linear sign-in: {}", e.0.message);
            http::redirect(&back.url(if e.0.code == ErrorCode::Forbidden {
                "not_app"
            } else {
                "failed"
            }))
        }
    }
}

/// Keeps fctl's app's tokens for the Linear workspace an admin installed it
/// in, and connects it to their fctl workspace.
async fn install(
    ctx: &mut Ctx,
    keys: &Keys,
    workspace: &str,
    code: &str,
    verifier: &str,
) -> AppResult<Option<String>> {
    let a = ws::load(ctx, workspace).await?;
    a.require_admin()?;
    let cfg = linear::config(&ctx.cfg)?.clone();
    let tokens = linear::exchange(&cfg, code, verifier, &redirect_uri(ctx)).await?;
    let viewer = linear::Linear::with_token(&cfg, &tokens.access_token)
        .viewer()
        .await?;
    if !viewer.app {
        // A sign-in as a person, which can't be delegated anything.
        linear::revoke(&cfg, &tokens.access_token).await;
        return Err(AppError(ApiError::forbidden(
            "Linear signed in a person rather than installing fctl's app",
        )));
    }
    let user = ctx.user()?.clone();
    linear::store(
        &ctx.db,
        keys,
        &linear::Installed {
            organization: viewer.organization.clone(),
            name: viewer.organization_name.clone(),
            url_key: viewer.url_key.clone(),
            logo_url: viewer.logo_url.clone(),
            app_user: Some(viewer.id.clone()),
            app_name: Some(viewer.name.clone()),
            app_handle: viewer.handle.clone(),
        },
        &tokens,
        &user.id,
        ctx.now,
    )
    .await?;
    q(
        &ctx.db,
        "INSERT INTO workspace_linear (workspace_id, organization_id, connected_by, linear_user, created_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT (workspace_id, organization_id) DO UPDATE SET connected_by = excluded.connected_by",
        &[
            s(a.id()),
            s(&viewer.organization),
            s(&user.id),
            os(None),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(a.id()),
        "workspace.linear_connected",
        Some(&viewer.organization),
        Some(json!({ "name": viewer.organization_name, "url_key": viewer.url_key })),
    );
    Ok(None)
}

/// Keeps who the person signed in to Linear as, revokes the sign-in, and
/// takes up what waited for them: returns the session to show them, when
/// the agent session that asked them started one.
async fn linked(
    ctx: &mut Ctx,
    code: &str,
    verifier: &str,
    thread: Option<&str>,
) -> AppResult<Option<String>> {
    let cfg = linear::config(&ctx.cfg)?.clone();
    let tokens = linear::exchange(&cfg, code, verifier, &redirect_uri(ctx)).await?;
    let viewer = linear::Linear::with_token(&cfg, &tokens.access_token)
        .viewer()
        .await;
    linear::revoke(&cfg, &tokens.access_token).await;
    let viewer = viewer?;
    let user = ctx.user()?.clone();
    // A Linear account is one person's: whoever proves it now has it.
    q(
        &ctx.db,
        "DELETE FROM linear_accounts WHERE organization_id = ? AND linear_user_id = ? AND user_id != ?",
        &[s(&viewer.organization), s(&viewer.id), s(&user.id)],
    )?
    .run()
    .await?;
    q(
        &ctx.db,
        "INSERT INTO linear_accounts (user_id, organization_id, organization_name, url_key, linear_user_id, name, email, avatar_url, linked_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (user_id, organization_id) DO UPDATE SET organization_name = excluded.organization_name, \
         url_key = excluded.url_key, linear_user_id = excluded.linear_user_id, name = excluded.name, email = excluded.email, \
         avatar_url = excluded.avatar_url, linked_at = excluded.linked_at",
        &[
            s(&user.id),
            s(&viewer.organization),
            s(&viewer.organization_name),
            os(Some(viewer.url_key.as_str()).filter(|k| !k.is_empty())),
            s(&viewer.id),
            s(&viewer.name),
            os(viewer.email.as_deref()),
            os(viewer.avatar_url.as_deref()),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    ctx.audit(
        "user.linear_linked",
        Some(&viewer.organization),
        Some(json!({ "name": viewer.organization_name })),
    );
    let reached = crate::delegations::linked(
        &ctx.env,
        &ctx.cfg,
        &ctx.db,
        &viewer.organization,
        &viewer.id,
        ctx.now,
    )
    .await
    .unwrap_or_else(|e| {
        worker::console_error!("taking up delegations after a link: {}", e.0.message);
        Vec::new()
    });
    // From Linear's button in an agent session: on to what it started.
    Ok(thread.and(reached.first().cloned()))
}

// ───────────────────────────── the webhook ─────────────────────────────

/// `POST /api/v1/linear/webhook`: Linear says that people delegated an
/// issue to fctl, mentioned it or replied to it, or that fctl's app lost
/// its access somewhere. Linear signs each delivery with the webhook's
/// secret, and wants an answer within five seconds, so what takes longer
/// happens after it.
pub async fn webhook(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let Some(secret) = ctx
        .cfg
        .linear
        .as_ref()
        .and_then(|l| l.webhook_secret.clone())
    else {
        return Err(not_configured(
            "This instance takes no Linear webhooks: set LINEAR_WEBHOOK_SECRET",
        ));
    };
    if http::length(req).is_some_and(|l| l > lin::MAX_DELIVERY_BYTES as u64) {
        return Err(AppError(ApiError::bad_request("The delivery is too large")));
    }
    let body = req.bytes().await?;
    if body.len() > lin::MAX_DELIVERY_BYTES {
        return Err(AppError(ApiError::bad_request("The delivery is too large")));
    }
    let signature = http::header(req, "Linear-Signature").unwrap_or_default();
    if !lin::signed(&secret, &body, &signature) {
        return Err(AppError(ApiError::new(
            ErrorCode::Unauthorized,
            "The delivery isn't signed with the webhook's secret",
        )));
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return Err(AppError(ApiError::bad_request("The delivery is not JSON")));
    };
    if !lin::fresh(
        payload.get("webhookTimestamp").and_then(Value::as_i64),
        ctx.now,
    ) {
        return Err(AppError(ApiError::bad_request(
            "The delivery is too old: its webhookTimestamp is more than a minute away",
        )));
    }
    let delivery = http::header(req, "Linear-Delivery").unwrap_or_default();
    let env = ctx.env.clone();
    let cfg = ctx.cfg.clone();
    let db = crate::db::handle(&ctx.db);
    let now = ctx.now;
    match lin::delivery(&payload) {
        lin::Delivery::Created(e) => {
            ctx.defer(async move {
                if let Err(err) =
                    crate::delegations::created(&env, &cfg, &db, &e, &payload, now).await
                {
                    worker::console_error!(
                        "Linear delivery {delivery} (created): {}",
                        err.0.message
                    );
                }
                Ok(())
            });
        }
        lin::Delivery::Prompted(e) => {
            ctx.defer(async move {
                if let Err(err) =
                    crate::delegations::prompted(&env, &cfg, &db, &e, &payload, now).await
                {
                    worker::console_error!(
                        "Linear delivery {delivery} (prompted): {}",
                        err.0.message
                    );
                }
                Ok(())
            });
        }
        lin::Delivery::Revoked { organization } => {
            crate::delegations::revoked(&ctx.db, &organization, ctx.now).await?;
            worker::console_log!("Linear delivery {delivery}: fctl's app lost {organization}");
        }
        lin::Delivery::Teams(p) => {
            worker::console_log!(
                "Linear delivery {delivery}: the teams fctl reaches in {} changed (+{}, -{})",
                p.organization_id,
                p.added_team_ids.len(),
                p.removed_team_ids.len()
            );
        }
        lin::Delivery::Other => {}
    }
    http::ok(&json!({ "ok": true }))
}
