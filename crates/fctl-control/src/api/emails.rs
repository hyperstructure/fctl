//! `/api/v1/me/emails/*` and `/api/v1/invites/{token}/connect`: the email
//! addresses of an account (docs/architecture.md#email-addresses).
//!
//! An account has a primary address and may have more, each invited to the
//! instance on its own. Accepting an invitation while signed in connects
//! its address to the account instead of making a new one. Every address
//! signs in, and has its own commit email; a workspace knows its members
//! by the address they joined as.

use crate::auth;
use crate::ctx::Ctx;
use crate::db::{Count, batch, n, os, q, s};
use crate::error::{AppError, AppResult, conflict, not_found};
use crate::http;
use crate::model::{UserRow, workspace_ref};
use fctl_domain::validate;
use fctl_proto::ApiError;
use fctl_proto::api::{EmailAddress, UpdateEmailRequest};
use fctl_proto::session::WorkspaceRef;
use serde::Deserialize;
use worker::{Request, Response};

#[derive(Deserialize)]
struct EmailRow {
    email: String,
    commit_email: Option<String>,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    /// The workspaces they are a member as it (JSON).
    workspaces: Option<String>,
}

#[derive(Deserialize)]
struct EmailWorkspace {
    id: String,
    slug: String,
    name: String,
    color: Option<String>,
    #[serde(default)]
    avatar_at: Option<f64>,
}

/// `user`'s addresses, the primary one first.
pub async fn list(ctx: &Ctx, user: &UserRow) -> AppResult<Vec<EmailAddress>> {
    let rows: Vec<EmailRow> = q(
        &ctx.db,
        "SELECT e.email, e.commit_email, e.created_at, \
         (SELECT json_group_array(json_object('id', w.id, 'slug', w.slug, 'name', w.name, 'color', w.color, \
          'avatar_at', w.avatar_updated_at)) FROM workspace_members m JOIN workspaces w ON w.id = m.workspace_id \
          WHERE m.user_id = e.user_id AND m.email = e.email) AS workspaces \
         FROM user_emails e WHERE e.user_id = ? ORDER BY e.email = ? DESC, e.created_at",
        &[s(&user.id), s(&user.email)],
    )?
    .all()
    .await?;
    let mut out: Vec<EmailAddress> = rows
        .into_iter()
        .map(|r| EmailAddress {
            primary: r.email == user.email,
            workspaces: r
                .workspaces
                .as_deref()
                .and_then(|j| serde_json::from_str::<Vec<EmailWorkspace>>(j).ok())
                .unwrap_or_default()
                .into_iter()
                .map(|w| {
                    workspace_ref(
                        &w.id,
                        &w.slug,
                        &w.name,
                        w.color.as_deref(),
                        w.avatar_at.map(|t| t as i64),
                    )
                })
                .collect::<Vec<WorkspaceRef>>(),
            email: r.email,
            commit_email: r.commit_email,
            added_at: r.created_at,
        })
        .collect();
    // An account from before addresses had their own rows.
    if !out.iter().any(|e| e.primary) {
        out.insert(
            0,
            EmailAddress {
                email: user.email.clone(),
                primary: true,
                commit_email: None,
                workspaces: Vec::new(),
                added_at: user.created_at,
            },
        );
    }
    Ok(out)
}

/// The commit email of `user`'s primary address (`Me.commit_email`).
pub async fn primary_commit_email(ctx: &Ctx, user: &UserRow) -> AppResult<Option<String>> {
    #[derive(Deserialize)]
    struct R {
        commit_email: Option<String>,
    }
    let r: Option<R> = q(
        &ctx.db,
        "SELECT commit_email FROM user_emails WHERE email = ? AND user_id = ?",
        &[s(&user.email), s(&user.id)],
    )?
    .first()
    .await?;
    Ok(r.and_then(|r| r.commit_email))
}

/// Sets the commit email of one of `user`'s addresses.
pub async fn set_commit_email(
    ctx: &Ctx,
    user: &UserRow,
    email: &str,
    commit_email: Option<&str>,
) -> AppResult<()> {
    q(
        &ctx.db,
        "INSERT INTO user_emails (email, user_id, commit_email, created_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT (email) DO UPDATE SET commit_email = excluded.commit_email WHERE user_emails.user_id = excluded.user_id",
        &[s(email), s(&user.id), os(commit_email), n(ctx.now)],
    )?
    .run()
    .await?;
    Ok(())
}

async fn own(ctx: &Ctx, user: &UserRow, email: &str) -> AppResult<String> {
    let email = email.trim().to_lowercase();
    let c: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM user_emails WHERE email = ? AND user_id = ?",
        &[s(&email), s(&user.id)],
    )?
    .first()
    .await?;
    if c.map_or(0, |c| c.n) == 0 && email != user.email {
        return Err(not_found("Email address"));
    }
    Ok(email)
}

async fn me(ctx: &Ctx, user_id: &str) -> AppResult<Response> {
    let user = auth::user_by_id(ctx, user_id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::ok(&auth::me(ctx, &user).await?)
}

/// `PATCH /api/v1/me/emails/{email}`: its commit email, or makes it the
/// primary address.
pub async fn update(ctx: &mut Ctx, req: &mut Request, email: &str) -> AppResult<Response> {
    let body: UpdateEmailRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    let email = own(ctx, &user, email).await?;
    if let Some(commit) = body.commit_email.as_deref() {
        let commit = validate::commit_email(commit)?;
        set_commit_email(ctx, &user, &email, commit.as_deref()).await?;
    }
    if body.primary == Some(true) && email != user.email {
        q(
            &ctx.db,
            "UPDATE users SET email = ?, updated_at = ? WHERE id = ?",
            &[s(&email), n(ctx.now), s(&user.id)],
        )?
        .run()
        .await?;
        ctx.audit(
            "user.primary_email",
            Some(&user.id),
            Some(serde_json::json!({ "from": user.email, "to": email })),
        );
    }
    me(ctx, &user.id).await
}

/// `DELETE /api/v1/me/emails/{email}`: disconnects an address that isn't
/// the primary one. The workspaces you are a member as it see you leave.
pub async fn remove(ctx: &mut Ctx, email: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let email = own(ctx, &user, email).await?;
    if email == user.email {
        return Err(conflict(
            "This is your primary address. Make another one primary first.",
        ));
    }
    #[derive(Deserialize)]
    struct M {
        workspace_id: String,
        role: String,
        name: String,
    }
    let memberships: Vec<M> = q(
        &ctx.db,
        "SELECT m.workspace_id, m.role, w.name FROM workspace_members m JOIN workspaces w ON w.id = m.workspace_id \
         WHERE m.user_id = ? AND m.email = ?",
        &[s(&user.id), s(&email)],
    )?
    .all()
    .await?;
    if let Some(m) = memberships.iter().find(|m| m.role == "owner") {
        return Err(conflict(&format!(
            "You are the primary admin of {} as this address. Hand the role to another admin first.",
            m.name
        )));
    }
    for m in &memberships {
        crate::api::workspaces::remove_from(ctx, &m.workspace_id, &user.id).await?;
        ctx.audit_in(
            Some(&m.workspace_id),
            "workspace.left",
            Some(&user.id),
            Some(serde_json::json!({ "email": email, "disconnected": true })),
        );
    }
    q(
        &ctx.db,
        "DELETE FROM user_emails WHERE email = ? AND user_id = ?",
        &[s(&email), s(&user.id)],
    )?
    .run()
    .await?;
    ctx.audit(
        "user.email_disconnected",
        Some(&user.id),
        Some(serde_json::json!({ "email": email })),
    );
    crate::workspaces::changed(ctx, vec![user.id.clone()]);
    me(ctx, &user.id).await
}

/// Connects `email` to `user`: it signs in to their account from now on,
/// and they join the workspaces it was invited to.
pub async fn connect(ctx: &Ctx, user: &UserRow, email: &str) -> AppResult<Vec<WorkspaceRef>> {
    if auth::user_by_email(ctx, email).await?.is_some() {
        return Err(conflict(
            "This address belongs to an account already. Sign in with it instead.",
        ));
    }
    q(
        &ctx.db,
        "INSERT INTO user_emails (email, user_id, created_at) VALUES (?, ?, ?)",
        &[s(email), s(&user.id), n(ctx.now)],
    )?
    .run()
    .await?;
    ctx.audit(
        "user.email_connected",
        Some(&user.id),
        Some(serde_json::json!({ "email": email })),
    );
    crate::api::workspaces::joined_after_sign_up(ctx, user, email).await
}

/// `POST /api/v1/invites/{token}/connect`: accepts an invitation to the
/// instance by connecting its address to the signed-in account.
pub async fn connect_invite(ctx: &mut Ctx, token: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    ctx.rate_limit(&format!("connect:{}", user.id)).await?;
    let invite = crate::api::auth::live_invite(ctx, token)
        .await?
        .ok_or_else(|| not_found("Invitation"))?;
    let email = validate::email(&invite.email)?;
    if email == user.email {
        return Err(conflict("This is your account's address already"));
    }
    if crate::api::auth::allow_match(ctx, &email).await?.is_none() {
        return Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::AccessDenied,
            "This email is no longer allowed on this instance.",
        )));
    }
    connect(ctx, &user, &email).await?;
    batch(
        &ctx.db,
        vec![q(
            &ctx.db,
            "UPDATE invites SET accepted_at = ? WHERE id = ?",
            &[n(ctx.now), s(&invite.id)],
        )?],
    )
    .await?;
    me(ctx, &user.id).await
}
