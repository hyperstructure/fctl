//! `/api/v1/sessions/*` and `/api/v1/projects/{id}/sessions`.

use crate::access;
use crate::ctx::Ctx;
use crate::db::{n, os, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, not_found, validation};
use crate::http;
use crate::internal::{
    self, ControlState, Deliver, Delivered, InitSession, Schedule, Stop, ViewerIdentity,
};
use crate::model::{CredentialRow, SESSION_SELECT, SessionRow};
use fctl_domain::{naming, validate};
use fctl_proto::api::*;
use fctl_proto::event::{Attachment, EventBody, Interrupt, MessageSource, UserMessage};
use fctl_proto::runner::StopReason;
use fctl_proto::session::{SessionRole, SessionStatus, UserRef};
use fctl_proto::uplink::{Delivery, DeliveryKind};
use fctl_proto::{ApiError, ErrorCode, UserId};
use worker::{Request, Response};

/// The credential a new session of `user` runs on.
pub async fn default_credential(
    ctx: &Ctx,
    user: &crate::model::UserRow,
) -> AppResult<CredentialRow> {
    let creds: Vec<CredentialRow> = q(
        &ctx.db,
        "SELECT * FROM claude_credentials WHERE user_id = ? AND status = 'active' ORDER BY created_at DESC",
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let chosen = user
        .default_credential_id
        .as_deref()
        .and_then(|id| creds.iter().find(|c| c.id == id))
        .or_else(|| creds.first())
        .cloned();
    chosen.ok_or_else(|| {
        AppError(ApiError {
            code: ErrorCode::NotConfigured,
            message: "Connect your Claude subscription first (Settings → Claude). Sessions always run on your own subscription.".into(),
            field: Some("claude".into()),
            retry_after: None,
        })
    })
}

#[derive(serde::Deserialize)]
struct Counter {
    #[serde(deserialize_with = "crate::db::int")]
    session_counter: i64,
}

pub async fn create(ctx: &mut Ctx, req: &mut Request, project_id: &str) -> AppResult<Response> {
    let body: CreateSessionRequest = http::body(req).await?;
    let p = access::project(ctx, project_id).await?;
    p.require_editor()?;
    if p.row.archived_at.is_some() {
        return Err(conflict("This project is archived"));
    }
    let user = ctx.user()?.clone();
    let files = validate::attachment_ids(&body.attachments)?;
    let prompt = validate::message_with_files("prompt", &body.prompt, files.len())?;
    let title = body
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(validate::session_title)
        .transpose()?;
    let model = body
        .model
        .as_deref()
        .filter(|m| !m.trim().is_empty())
        .map(validate::model)
        .transpose()?;
    let effort = body
        .effort
        .as_deref()
        .filter(|e| !e.trim().is_empty())
        .map(validate::effort)
        .transpose()?;
    let base_branch = body
        .base_branch
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .map(|b| validate::branch("base_branch", b))
        .transpose()?
        .unwrap_or_else(|| p.row.default_branch.clone());
    let credential = default_credential(ctx, &user).await?;
    let id = fctl_domain::ids::session_id(ctx.now);
    // The task's files belong to the new session from here on.
    let attachments = internal::claim_files(&ctx.env, &p.row.id, &id, &user.id, &files).await?;
    let title = title.unwrap_or_else(|| {
        if prompt.is_empty() {
            let names: Vec<&str> = attachments.iter().map(|a| a.name.as_str()).collect();
            naming::title_from_files(&names)
        } else {
            naming::title_from_prompt(&prompt)
        }
    });
    let prompt_attachments = if attachments.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&attachments)?)
    };
    // A per-project sequence number (#12).
    let counter: Option<Counter> = q(
        &ctx.db,
        "UPDATE projects SET session_counter = session_counter + 1 WHERE id = ? RETURNING session_counter",
        &[s(&p.row.id)],
    )?
    .first()
    .await?;
    let number = counter.map_or(1, |c| c.session_counter);
    let branch = match body.branch.as_deref().filter(|b| !b.trim().is_empty()) {
        Some(b) => validate::branch("branch", b)?,
        None => naming::session_branch(number as u32, &title),
    };
    let claude_session_id = fctl_domain::ids::uuid_v4();
    q(
        &ctx.db,
        "INSERT INTO sessions (id, project_id, number, title, prompt, prompt_attachments, status, created_by, credential_id, branch, base_branch, model, effort, \
         claude_session_id, stats, created_at, updated_at, last_activity_at) VALUES (?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?, ?, ?, ?, ?, '{}', ?, ?, ?)",
        &[
            s(id.as_str()),
            s(&p.row.id),
            n(number),
            s(&title),
            s(&prompt),
            os(prompt_attachments.as_deref()),
            s(&user.id),
            s(&credential.id),
            s(&branch),
            s(&base_branch),
            os(model.as_deref()),
            os(effort.as_deref()),
            s(&claude_session_id),
            n(ctx.now),
            n(ctx.now),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    // Following is not worth failing a session over: it can be done later.
    if let Err(e) = crate::notify::follow_own(&ctx.db, id.as_str(), &user.id, ctx.now).await {
        worker::console_error!("following a new session failed: {}", e.0.message);
    }
    let created = access::session(ctx, id.as_str()).await?;
    let session = created.row.to_session(Some(created.role));
    let stub = internal::session_stub(&ctx.env, id.as_str())?;
    let prompt_msg = UserMessage {
        id: fctl_domain::ids::delivery_id(ctx.now),
        author: Some(user.to_ref()),
        text: prompt,
        source: source_of(ctx),
        attachments,
    };
    let _: serde_json::Value = internal::post(
        &stub,
        "/init",
        &InitSession {
            session: session.clone(),
            prompt: prompt_msg,
        },
    )
    .await?;
    let fleet = internal::fleet_stub(&ctx.env)?;
    let _: serde_json::Value = internal::post(
        &fleet,
        "/schedule",
        &Schedule {
            session_id: id.clone(),
        },
    )
    .await?;
    ctx.audit(
        "session.created",
        Some(id.as_str()),
        Some(serde_json::json!({ "project": p.row.slug, "number": number })),
    );
    http::created(&session)
}

fn source_of(ctx: &Ctx) -> MessageSource {
    match ctx.auth.as_ref().map(|a| a.kind) {
        Some(TokenKind::Web) => MessageSource::Web,
        Some(TokenKind::Cli) => MessageSource::Cli,
        _ => MessageSource::Api,
    }
}

pub async fn list(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    let user = ctx.user()?;
    let limit = http::query(req, "limit")
        .and_then(|l| l.parse::<i64>().ok())
        .unwrap_or(50)
        .clamp(1, 200);
    let before = http::query(req, "before").and_then(|b| b.parse::<i64>().ok());
    let mut sql = format!("{SESSION_SELECT} WHERE 1 = 1");
    let mut params = vec![s(&user.id)];
    if !user.is_admin() {
        sql.push_str(" AND (m.role IS NOT NULL OR sh.role IS NOT NULL OR s.created_by = ?1)");
    }
    if let Some(project) = http::query(req, "project") {
        let p = access::project(ctx, &project).await?;
        params.push(s(&p.row.id));
        sql.push_str(&format!(" AND s.project_id = ?{}", params.len()));
    }
    if let Some(status) = http::query(req, "status") {
        match status.as_str() {
            "active" => sql.push_str(" AND s.status IN ('queued','provisioning','preparing','running','idle','paused','stopping')"),
            "ended" => sql.push_str(" AND s.status IN ('stopped','failed')"),
            other => {
                let st = SessionStatus::parse(other).ok_or_else(|| validation("status", "Unknown status"))?;
                params.push(s(st.as_str()));
                sql.push_str(&format!(" AND s.status = ?{}", params.len()));
            }
        }
    }
    if http::query(req, "mine").is_some_and(|v| v == "1" || v == "true") {
        sql.push_str(" AND s.created_by = ?1");
    }
    if !http::query(req, "archived").is_some_and(|v| v == "1" || v == "true") {
        sql.push_str(" AND s.archived = 0");
    }
    if let Some(b) = before {
        params.push(n(b));
        sql.push_str(&format!(" AND s.created_at < ?{}", params.len()));
    }
    params.push(n(limit + 1));
    sql.push_str(&format!(
        " ORDER BY s.created_at DESC LIMIT ?{}",
        params.len()
    ));
    let mut rows: Vec<SessionRow> = q(&ctx.db, &sql, &params)?.all().await?;
    let more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let next_before = if more {
        rows.last().map(|r| r.created_at)
    } else {
        None
    };
    http::ok(&SessionList {
        sessions: rows
            .iter()
            .map(|r| r.to_session(access::session_role(user, r)))
            .collect(),
        next_before,
    })
}

pub async fn get(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = access::session(ctx, id).await?;
    http::ok(&a.row.to_session(Some(a.role)))
}

pub async fn update(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdateSessionRequest = http::body(req).await?;
    let a = access::session(ctx, id).await?;
    a.require_owner()?;
    let title = body
        .title
        .as_deref()
        .map(validate::session_title)
        .transpose()?;
    // Absent: unchanged. Empty: cleared.
    let description = body
        .description
        .as_deref()
        .map(validate::session_description)
        .transpose()?;
    q(
        &ctx.db,
        "UPDATE sessions SET title = COALESCE(?, title), description = CASE WHEN ? THEN ? ELSE description END, \
         archived = COALESCE(?, archived), updated_at = ? WHERE id = ?",
        &[
            os(title.as_deref()),
            crate::db::b(description.is_some()),
            os(description.clone().flatten().as_deref()),
            body.archived.map_or(crate::db::null(), crate::db::b),
            n(ctx.now),
            s(id),
        ],
    )?
    .run()
    .await?;
    let stub = internal::session_stub(&ctx.env, id)?;
    let _: serde_json::Value = internal::post(
        &stub,
        "/control",
        &ControlState {
            title,
            description: description.map(Option::unwrap_or_default),
            archived: body.archived,
            ..Default::default()
        },
    )
    .await?;
    get(ctx, id).await
}

pub async fn send_message(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: SendMessageRequest = http::body(req).await?;
    let a = access::session(ctx, id).await?;
    a.require_editor()?;
    let files = validate::attachment_ids(&body.attachments)?;
    let text = validate::message_with_files("text", &body.text, files.len())?;
    if !a.row.status().accepts_messages() {
        return Err(conflict("The session has ended. Resume it to continue."));
    }
    let author = ctx.user()?.to_ref();
    let attachments = internal::claim_files(
        &ctx.env,
        &a.row.project_id,
        &a.row.session_id(),
        author.id.as_str(),
        &files,
    )
    .await?;
    let delivery_id = deliver_message(
        ctx,
        id,
        text,
        author,
        source_of(ctx),
        body.client_id,
        attachments,
    )
    .await?;
    http::ok(&SendMessageResponse { delivery_id })
}

/// Stores a message and queues it for the agent. Shared with SessionDO
/// viewer messages, which call the DO directly.
pub async fn deliver_message(
    ctx: &Ctx,
    session_id: &str,
    text: String,
    author: UserRef,
    source: MessageSource,
    client_key: Option<String>,
    attachments: Vec<Attachment>,
) -> AppResult<fctl_proto::DeliveryId> {
    let delivery_id = fctl_domain::ids::delivery_id(ctx.now);
    let stub = internal::session_stub(&ctx.env, session_id)?;
    let out: Delivered = internal::post(
        &stub,
        "/deliver",
        &Deliver {
            delivery: Delivery {
                id: delivery_id.clone(),
                kind: DeliveryKind::Message {
                    text: text.clone(),
                    author: Some(author.clone()),
                    attachments: attachments.clone(),
                },
            },
            client_key,
            event: EventBody::UserMessage(UserMessage {
                id: delivery_id,
                author: Some(author),
                text,
                source,
                attachments,
            }),
        },
    )
    .await?;
    Ok(out.delivery_id)
}

pub async fn interrupt(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let a = access::session(ctx, id).await?;
    a.require_editor()?;
    let by = ctx.user()?.to_ref();
    let delivery_id = fctl_domain::ids::delivery_id(ctx.now);
    let stub = internal::session_stub(&ctx.env, id)?;
    let out: Delivered = internal::post(
        &stub,
        "/deliver",
        &Deliver {
            delivery: Delivery {
                id: delivery_id.clone(),
                kind: DeliveryKind::Interrupt {
                    by: Some(by.clone()),
                },
            },
            client_key: None,
            event: EventBody::Interrupt(Interrupt {
                id: delivery_id,
                by: Some(by),
            }),
        },
    )
    .await?;
    http::ok(&SendMessageResponse {
        delivery_id: out.delivery_id,
    })
}

pub async fn stop(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let a = access::session(ctx, id).await?;
    a.require_owner()?;
    let status = a.row.status();
    if status.is_terminal() {
        return get(ctx, id).await;
    }
    let fleet = internal::fleet_stub(&ctx.env)?;
    let _: serde_json::Value = internal::post(
        &fleet,
        "/stop",
        &Stop {
            session_id: a.row.session_id(),
            reason: StopReason::User,
            destroy: false,
        },
    )
    .await?;
    ctx.audit("session.stopped", Some(id), None);
    get(ctx, id).await
}

pub async fn resume(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: ResumeSessionRequest = if http::header(req, "Content-Type").is_some() {
        http::body(req).await?
    } else {
        ResumeSessionRequest {
            prompt: None,
            attachments: Vec::new(),
        }
    };
    let a = access::session(ctx, id).await?;
    a.require_owner()?;
    if !a.row.status().is_terminal() {
        return Err(conflict("The session is still running"));
    }
    let user = ctx.user()?.clone();
    let files = validate::attachment_ids(&body.attachments)?;
    let prompt = body.prompt.as_deref().unwrap_or_default();
    let message = if files.is_empty() && prompt.trim().is_empty() {
        None
    } else {
        Some(validate::message_with_files("prompt", prompt, files.len())?)
    };
    // Resumed sessions run on the resumer's subscription.
    let credential = default_credential(ctx, &user).await?;
    let attachments = internal::claim_files(
        &ctx.env,
        &a.row.project_id,
        &a.row.session_id(),
        &user.id,
        &files,
    )
    .await?;
    q(
        &ctx.db,
        "UPDATE sessions SET status = 'queued', status_reason = NULL, credential_id = ?, ended_at = NULL, updated_at = ? WHERE id = ?",
        &[s(&credential.id), n(ctx.now), s(id)],
    )?
    .run()
    .await?;
    if let Err(e) = crate::notify::follow_own(&ctx.db, id, &user.id, ctx.now).await {
        worker::console_error!("following a resumed session failed: {}", e.0.message);
    }
    let stub = internal::session_stub(&ctx.env, id)?;
    let _: serde_json::Value = internal::post(
        &stub,
        "/control",
        &ControlState {
            status: Some(SessionStatus::Queued),
            reason: Some("Resuming".into()),
            clear_runner: true,
            ..Default::default()
        },
    )
    .await?;
    if let Some(text) = message {
        deliver_message(
            ctx,
            id,
            text,
            user.to_ref(),
            source_of(ctx),
            None,
            attachments,
        )
        .await?;
    }
    let fleet = internal::fleet_stub(&ctx.env)?;
    let _: serde_json::Value = internal::post(
        &fleet,
        "/schedule",
        &Schedule {
            session_id: a.row.session_id(),
        },
    )
    .await?;
    ctx.audit("session.resumed", Some(id), None);
    get(ctx, id).await
}

pub async fn events(ctx: &Ctx, req: &Request, id: &str) -> AppResult<Response> {
    access::session(ctx, id).await?;
    let mut path = String::from("/events?");
    for key in ["after", "before", "limit"] {
        if let Some(v) = http::query(req, key).filter(|v| v.bytes().all(|b| b.is_ascii_digit())) {
            path.push_str(&format!("{key}={v}&"));
        }
    }
    let stub = internal::session_stub(&ctx.env, id)?;
    let page: EventPage = internal::get(&stub, &path).await?;
    http::ok(&page)
}

/// Upgrades to the viewer WebSocket of a session.
pub async fn websocket(ctx: &Ctx, req: &Request, id: &str) -> AppResult<Response> {
    if !http::is_websocket_upgrade(req) {
        return Err(AppError(ApiError::bad_request(
            "Expected a WebSocket upgrade",
        )));
    }
    // Browsers: the socket must be opened by the app itself.
    if ctx.auth.as_ref().is_some_and(|a| a.via_cookie) {
        http::check_same_origin(req, ctx.cfg.origin(), ctx.cfg.dev)?;
    }
    let a = access::session(ctx, id).await?;
    let ident = ViewerIdentity {
        user: ctx.user()?.to_ref(),
        role: a.role,
    };
    let mut path = String::from("/viewer");
    if let Some(after) = http::query(req, "after").filter(|v| v.bytes().all(|b| b.is_ascii_digit()))
    {
        path.push_str(&format!("?after={after}"));
    }
    let stub = internal::session_stub(&ctx.env, id)?;
    let ident_json = serde_json::to_string(&ident)?;
    internal::upgrade(&stub, &path, &[(internal::HDR_VIEWER, &ident_json)]).await
}

/// `PUT /sessions/{id}/follow`: notifications about it for the caller.
pub async fn follow(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let a = access::session(ctx, id).await?;
    crate::notify::follow(&ctx.db, &a.row.id, &ctx.user()?.id, ctx.now).await?;
    get(ctx, id).await
}

/// `DELETE /sessions/{id}/follow`: no more notifications about it.
pub async fn unfollow(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let a = access::session(ctx, id).await?;
    crate::notify::unfollow(&ctx.db, &a.row.id, &ctx.user()?.id).await?;
    get(ctx, id).await
}

// ───────────────────────────── shares ─────────────────────────────

#[derive(serde::Deserialize)]
struct ShareRow {
    user_id: String,
    name: String,
    email: String,
    avatar_url: Option<String>,
    role: String,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
}

pub async fn shares(ctx: &Ctx, id: &str) -> AppResult<Response> {
    access::session(ctx, id).await?;
    let rows: Vec<ShareRow> = q(
        &ctx.db,
        "SELECT sh.user_id, u.name, u.email, u.avatar_url, sh.role, sh.created_at FROM session_shares sh \
         JOIN users u ON u.id = sh.user_id WHERE sh.session_id = ? ORDER BY sh.created_at",
        &[s(id)],
    )?
    .all()
    .await?;
    http::ok(&ShareList {
        shares: rows
            .into_iter()
            .map(|r| SessionShare {
                user: UserRef {
                    id: UserId(r.user_id),
                    name: r.name,
                    avatar_url: r.avatar_url,
                },
                email: r.email,
                role: SessionRole::parse(&r.role).unwrap_or(SessionRole::Viewer),
                created_at: r.created_at,
            })
            .collect(),
    })
}

pub async fn add_share(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: CreateShareRequest = http::body(req).await?;
    let a = access::session(ctx, id).await?;
    a.require_owner()?;
    if body.role == SessionRole::Owner
        && a.row.created_by != ctx.user()?.id
        && !ctx.user()?.is_admin()
    {
        return Err(forbidden(
            "Only the person who started the session can share ownership",
        ));
    }
    let target = crate::api::auth::lookup_user_by_email(ctx, &body.email)
        .await?
        .ok_or_else(|| validation("email", "No one with this email has an account yet"))?;
    q(
        &ctx.db,
        "INSERT INTO session_shares (session_id, user_id, role, created_by, created_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT (session_id, user_id) DO UPDATE SET role = excluded.role",
        &[s(id), s(&target.id), s(body.role.as_str()), s(&ctx.user()?.id), n(ctx.now)],
    )?
    .run()
    .await?;
    ctx.audit(
        "session.shared",
        Some(id),
        Some(serde_json::json!({ "user": target.id, "role": body.role.as_str() })),
    );
    shares(ctx, id).await
}

pub async fn remove_share(ctx: &mut Ctx, id: &str, user_id: &str) -> AppResult<Response> {
    let a = access::session(ctx, id).await?;
    if ctx.user()?.id != user_id {
        a.require_owner()?;
    }
    let changed = q(
        &ctx.db,
        "DELETE FROM session_shares WHERE session_id = ? AND user_id = ?",
        &[s(id), s(user_id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(not_found("Share"));
    }
    shares(ctx, id).await
}
