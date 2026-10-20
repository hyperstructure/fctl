//! `/api/v1/workspaces/*` and `/api/v1/invitations/*`: workspaces, their
//! members and invitations, avatars, runners and audit logs
//! (docs/architecture.md#workspaces). GitHub accounts are in `api::github`.

use crate::ctx::Ctx;
use crate::db::{Count, batch, n, os, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, not_found, validation};
use crate::http;
use crate::internal::{self, LiveRunners};
use crate::model::{
    PROJECT_ORDER, PROJECT_VISIBLE, RUNNER_SELECT, RunnerRow, RunnerView, UserRow, WORKSPACE_ORDER,
};
use crate::workspaces::{self as ws, INVITATION_SELECT, InvitationRow, WorkspaceRow};
use base64::Engine;
use fctl_domain::tokens::{SecretKind, hash_secret, new_secret};
use fctl_domain::validate;
use fctl_domain::workspace as policy;
use fctl_proto::api::*;
use fctl_proto::session::{WorkspaceColor, WorkspaceRole};
use fctl_proto::{ApiError, ErrorCode, UserId};
use serde::Deserialize;
use worker::{Headers, Request, Response};

const ACTIVE: &str = "('queued','provisioning','preparing','running','idle','paused','stopping')";

async fn live(ctx: &Ctx) -> LiveRunners {
    match internal::fleet_stub(&ctx.env) {
        Ok(stub) => internal::get(&stub, "/runners").await.unwrap_or_default(),
        Err(_) => LiveRunners::default(),
    }
}

async fn slug_taken(ctx: &Ctx, slug: &str, except: Option<&str>) -> AppResult<bool> {
    let c: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM workspaces WHERE slug = ? AND id != ?",
        &[s(slug), s(except.unwrap_or(""))],
    )?
    .first()
    .await?;
    Ok(c.map_or(0, |c| c.n) > 0)
}

/// A free slug from a name: the name's, or with a number after it.
async fn free_slug(ctx: &Ctx, name: &str) -> AppResult<String> {
    let base = policy::slug_from_name(name);
    // A name the app's paths use ("New", "Admin") gets a word more.
    let base = validate::slug(&base)
        .or_else(|_| validate::slug(&format!("{base}-workspace")))
        .unwrap_or_else(|_| "workspace".to_owned());
    let mut candidate = base.clone();
    let mut i = 2;
    while slug_taken(ctx, &candidate, None).await? {
        candidate = format!("{base}-{i}");
        i += 1;
        if i > 50 {
            return Err(conflict("Pick a different workspace name"));
        }
    }
    Ok(candidate)
}

/// The workspace as the caller sees it now.
async fn respond(ctx: &Ctx, id: &str) -> AppResult<Workspace> {
    let a = ws::load(ctx, id).await?;
    let live = live(ctx).await;
    Ok(a.row.to_workspace(a.blocked, &live.online))
}

/// `GET /api/v1/workspaces`.
pub async fn list(ctx: &Ctx) -> AppResult<Response> {
    let user = ctx.user()?;
    let rows = ws::mine(&ctx.db, &user.id).await?;
    let live = live(ctx).await;
    let invitations = ws::invitations_for(&ctx.db, &user.id, ctx.now).await?;
    http::ok(&WorkspaceList {
        workspaces: rows
            .iter()
            .map(|r| {
                r.to_workspace(
                    ws::blocked_for(ctx, user, r.require_two_factor),
                    &live.online,
                )
            })
            .collect(),
        invitations: invitations
            .iter()
            .map(|i| i.to_invitation(ws::blocked_for(ctx, user, i.require_two_factor)))
            .collect(),
        limit: policy::MAX_WORKSPACES as u32,
    })
}

/// The address `user` is a member as: `requested` when it is theirs, else
/// their primary one.
async fn own_address(ctx: &Ctx, user: &UserRow, requested: Option<&str>) -> AppResult<String> {
    let Some(email) = requested
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
    else {
        return Ok(user.email.clone());
    };
    let theirs: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM user_emails WHERE email = ? AND user_id = ?",
        &[s(&email), s(&user.id)],
    )?
    .first()
    .await?;
    if theirs.map_or(0, |c| c.n) == 0 && email != user.email {
        return Err(validation("email", "Use one of your own addresses"));
    }
    Ok(email)
}

/// `POST /api/v1/workspaces`: the caller is its primary admin.
pub async fn create(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: CreateWorkspaceRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    policy::check_limit(ws::membership_count(&ctx.db, &user.id).await?)?;
    let name = policy::name(&body.name)?;
    let slug = match body.slug.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(sl) => {
            let sl = validate::slug(sl)?;
            if slug_taken(ctx, &sl, None).await? {
                return Err(validation("slug", "Another workspace uses this address"));
            }
            sl
        }
        None => free_slug(ctx, &name).await?,
    };
    let email = own_address(ctx, &user, body.email.as_deref()).await?;
    let id = fctl_domain::ids::workspace_id(ctx.now);
    let color = match body.color {
        Some(c) => c,
        None => {
            // A color their other workspaces don't have, while one is left.
            let taken: Vec<Option<String>> = ws::mine(&ctx.db, &user.id)
                .await?
                .into_iter()
                .map(|w| w.color)
                .collect();
            WorkspaceColor::ALL
                .into_iter()
                .find(|c| !taken.iter().any(|t| t.as_deref() == Some(c.as_str())))
                .unwrap_or_else(|| {
                    WorkspaceColor::ALL
                        .get(taken.len() % WorkspaceColor::ALL.len())
                        .copied()
                        .unwrap_or_default()
                })
        }
    };
    let created = batch(
        &ctx.db,
        vec![
            // Within the limit when it runs (two at once can't both take the
            // last place); the membership follows it.
            q(
                &ctx.db,
                "INSERT INTO workspaces (id, slug, name, color, base_role, require_two_factor, created_by, created_at, updated_at) \
                 SELECT ?1, ?2, ?3, ?4, 'editor', 0, ?5, ?6, ?6 \
                 WHERE (SELECT COUNT(*) FROM workspace_members WHERE user_id = ?5) < ?7",
                &[
                    s(id.as_str()),
                    s(&slug),
                    s(&name),
                    s(color.as_str()),
                    s(&user.id),
                    n(ctx.now),
                    n(policy::MAX_WORKSPACES as i64),
                ],
            )?,
            ws::add_member(
                &ctx.db,
                id.as_str(),
                &user.id,
                &email,
                WorkspaceRole::Owner,
                Some(&user.id),
                ctx.now,
            )?,
        ],
    )
    .await;
    if let Err(e) = created {
        // Its address went to another workspace a moment ago.
        if slug_taken(ctx, &slug, None).await? {
            return Err(validation(
                "slug",
                "Another workspace took this address a moment ago. Try again.",
            ));
        }
        return Err(e);
    }
    if ws::member(&ctx.db, id.as_str(), &user.id).await?.is_none() {
        policy::check_limit(ws::membership_count(&ctx.db, &user.id).await?)?;
        return Err(not_found("Workspace"));
    }
    let data = serde_json::json!({ "name": name, "slug": slug });
    ctx.audit("workspace.created", Some(id.as_str()), Some(data.clone()));
    ctx.audit_in(
        Some(id.as_str()),
        "workspace.created",
        Some(id.as_str()),
        Some(data),
    );
    ws::changed(ctx, vec![user.id.clone()]);
    http::created(&respond(ctx, id.as_str()).await?)
}

/// `GET /api/v1/workspaces/{id}`.
pub async fn get(ctx: &Ctx, id: &str) -> AppResult<Response> {
    http::ok(&respond(ctx, id).await?)
}

/// `PATCH /api/v1/workspaces/{id}` (admins).
pub async fn update(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdateWorkspaceRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let user = ctx.user()?.clone();
    let name = body.name.as_deref().map(policy::name).transpose()?;
    let slug = match body.slug.as_deref() {
        Some(sl) => {
            let sl = validate::slug(sl)?;
            if slug_taken(ctx, &sl, Some(a.id())).await? {
                return Err(validation("slug", "Another workspace uses this address"));
            }
            Some(sl)
        }
        None => None,
    };
    if body.require_two_factor == Some(true)
        && !a.row.require_two_factor
        && user.blocked_by(&ctx.settings(), true)
    {
        return Err(AppError(ApiError::new(
            ErrorCode::Conflict,
            "Set up two-factor sign-in yourself first (Settings → Security): the workspace would close to you too.",
        )));
    }
    let updated = q(
        &ctx.db,
        "UPDATE workspaces SET name = COALESCE(?, name), slug = COALESCE(?, slug), color = COALESCE(?, color), \
         base_role = COALESCE(?, base_role), require_two_factor = COALESCE(?, require_two_factor), \
         skip_invites = COALESCE(?, skip_invites), updated_at = ? WHERE id = ?",
        &[
            os(name.as_deref()),
            os(slug.as_deref()),
            os(body.color.map(WorkspaceColor::as_str)),
            os(body.base_role.map(BaseRole::as_str)),
            body.require_two_factor.map_or(crate::db::null(), crate::db::b),
            body.skip_invites.map_or(crate::db::null(), crate::db::b),
            n(ctx.now),
            s(a.id()),
        ],
    )?
    .run()
    .await;
    if let Err(e) = updated {
        if let Some(sl) = &slug
            && slug_taken(ctx, sl, Some(a.id())).await?
        {
            return Err(validation(
                "slug",
                "Another workspace took this address a moment ago",
            ));
        }
        return Err(e);
    }
    ctx.audit_in(
        Some(a.id()),
        "workspace.updated",
        Some(a.id()),
        Some(serde_json::to_value(&body)?),
    );
    // Who sees what changes with the base role and two-factor.
    ws::changed(ctx, ws::member_ids(&ctx.db, a.id()).await?);
    http::ok(&respond(ctx, a.id()).await?)
}

/// Deletes a workspace with its projects, their sessions and files, after
/// checking that none of its sessions runs.
pub async fn erase(ctx: &mut Ctx, row: &WorkspaceRow, typed: &str) -> AppResult<()> {
    if !typed.trim().eq_ignore_ascii_case(row.name.trim()) {
        return Err(validation("name", "Type the workspace's name to delete it"));
    }
    let running: Option<Count> = q(
        &ctx.db,
        &format!(
            "SELECT COUNT(*) AS n FROM sessions s JOIN projects p ON p.id = s.project_id \
             WHERE p.workspace_id = ? AND s.status IN {ACTIVE}"
        ),
        &[s(&row.id)],
    )?
    .first()
    .await?;
    if running.map_or(0, |c| c.n) > 0 {
        return Err(conflict("Stop the workspace's sessions before deleting it"));
    }
    let members = ws::member_ids(&ctx.db, &row.id).await?;
    // Rows cascade from the workspace, and its audit log goes with it; the
    // sessions' event logs and the projects' files live in Durable
    // Objects, so they are queued for purging in the same batch.
    batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "INSERT OR IGNORE INTO session_purges (session_id, queued_at) \
                 SELECT s.id, ? FROM sessions s JOIN projects p ON p.id = s.project_id WHERE p.workspace_id = ?",
                &[n(ctx.now), s(&row.id)],
            )?,
            q(
                &ctx.db,
                "INSERT OR IGNORE INTO file_purges (project_id, queued_at) SELECT id, ? FROM projects WHERE workspace_id = ?",
                &[n(ctx.now), s(&row.id)],
            )?,
            q(&ctx.db, "DELETE FROM audit_log WHERE workspace_id = ?", &[s(&row.id)])?,
            q(&ctx.db, "DELETE FROM workspaces WHERE id = ?", &[s(&row.id)])?,
        ],
    )
    .await?;
    let env = ctx.env.clone();
    let db = crate::db::handle(&ctx.db);
    ctx.defer(async move { crate::housekeeping::purge_deleted(&env, &db).await });
    ctx.audit(
        "workspace.deleted",
        Some(&row.id),
        Some(serde_json::json!({ "name": row.name, "slug": row.slug })),
    );
    ws::changed(ctx, members);
    Ok(())
}

/// `POST /api/v1/workspaces/{id}/delete` (its primary admin), with its
/// name typed to confirm.
pub async fn delete(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: DeleteWorkspaceRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    a.require_primary()?;
    erase(ctx, &a.row, &body.name).await?;
    http::ok(&Ok { ok: true })
}

// ───────────────────────────── avatar ─────────────────────────────

/// `PUT /api/v1/workspaces/{id}/avatar` (admins): the image as an
/// `application/octet-stream` body.
pub async fn set_avatar(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let ct = http::header(req, "Content-Type").unwrap_or_default();
    if !ct
        .to_ascii_lowercase()
        .starts_with("application/octet-stream")
    {
        return Err(AppError(ApiError::bad_request(
            "Send the image as application/octet-stream",
        )));
    }
    if http::header(req, "Content-Length")
        .and_then(|l| l.parse::<usize>().ok())
        .is_some_and(|l| l > policy::MAX_AVATAR_BYTES)
    {
        return Err(validation("avatar", "Use a smaller image"));
    }
    let bytes = req.bytes().await?;
    let media_type = policy::avatar(&bytes)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    q(
        &ctx.db,
        "UPDATE workspaces SET avatar = ?, avatar_type = ?, avatar_updated_at = ?, updated_at = ? WHERE id = ?",
        &[
            s(&encoded),
            s(media_type),
            n(ctx.now),
            n(ctx.now),
            s(a.id()),
        ],
    )?
    .run()
    .await?;
    ctx.audit_in(Some(a.id()), "workspace.avatar_set", Some(a.id()), None);
    ws::changed(ctx, ws::member_ids(&ctx.db, a.id()).await?);
    http::ok(&respond(ctx, a.id()).await?)
}

/// `DELETE /api/v1/workspaces/{id}/avatar` (admins): its initial again.
pub async fn remove_avatar(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    q(
        &ctx.db,
        "UPDATE workspaces SET avatar = NULL, avatar_type = NULL, avatar_updated_at = NULL, updated_at = ? WHERE id = ?",
        &[n(ctx.now), s(a.id())],
    )?
    .run()
    .await?;
    ctx.audit_in(Some(a.id()), "workspace.avatar_removed", Some(a.id()), None);
    ws::changed(ctx, ws::member_ids(&ctx.db, a.id()).await?);
    http::ok(&respond(ctx, a.id()).await?)
}

/// `GET /api/v1/workspaces/{id}/avatar`: for its members, people invited to
/// it, and instance admins. Its URL changes with it, so it is cached.
pub async fn avatar(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let user = ctx.user()?;
    #[derive(Deserialize)]
    struct Row {
        avatar: Option<String>,
        avatar_type: Option<String>,
        #[serde(deserialize_with = "crate::db::int_bool")]
        allowed: bool,
    }
    let row: Option<Row> = q(
        &ctx.db,
        "SELECT w.avatar, w.avatar_type, (EXISTS (SELECT 1 FROM workspace_members m WHERE m.workspace_id = w.id AND m.user_id = ?1) \
         OR EXISTS (SELECT 1 FROM workspace_invitations i WHERE i.workspace_id = w.id AND i.email IN \
         (SELECT email FROM user_emails WHERE user_id = ?1)) OR ?3) AS allowed \
         FROM workspaces w WHERE w.id = ?2",
        &[s(&user.id), s(id), crate::db::b(user.is_admin())],
    )?
    .first()
    .await?;
    let Some(Row {
        avatar: Some(data),
        avatar_type: Some(media_type),
        allowed: true,
    }) = row
    else {
        return Err(not_found("Avatar"));
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(crate::error::internal)?;
    let h = Headers::new();
    h.set("Content-Type", &media_type)?;
    h.set("Cache-Control", "private, max-age=31536000, immutable")?;
    h.set("X-Content-Type-Options", "nosniff")?;
    h.set("Content-Security-Policy", "default-src 'none'; sandbox")?;
    h.set("Cross-Origin-Resource-Policy", "same-origin")?;
    Ok(Response::from_bytes(bytes)?.with_headers(h))
}

// ───────────────────────────── members ─────────────────────────────

#[derive(Deserialize)]
struct MemberRow {
    user_id: String,
    name: String,
    avatar_url: Option<String>,
    email: String,
    role: String,
    added_by: Option<String>,
    adder_name: Option<String>,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    #[serde(deserialize_with = "crate::db::int_bool")]
    two_factor: bool,
}

/// `GET /api/v1/workspaces/{id}/members`: its members, and for its admins
/// the invitations nobody has answered yet.
pub async fn members(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    let role = a.require_open()?;
    let rows: Vec<MemberRow> = q(
        &ctx.db,
        "SELECT m.user_id, u.name, u.avatar_url, m.email, m.role, m.added_by, au.name AS adder_name, m.created_at, \
         (EXISTS (SELECT 1 FROM authenticator_apps x WHERE x.user_id = m.user_id) \
          OR EXISTS (SELECT 1 FROM passkeys p WHERE p.user_id = m.user_id)) AS two_factor \
         FROM workspace_members m JOIN users u ON u.id = m.user_id LEFT JOIN users au ON au.id = m.added_by \
         WHERE m.workspace_id = ? ORDER BY CASE m.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 ELSE 2 END, u.name",
        &[s(a.id())],
    )?
    .all()
    .await?;
    let invitations: Vec<InvitationRow> = if role.is_admin() {
        q(
            &ctx.db,
            &format!("{INVITATION_SELECT} WHERE i.workspace_id = ? AND i.expires_at > ? ORDER BY i.created_at DESC"),
            &[s(a.id()), n(ctx.now)],
        )?
        .all()
        .await?
    } else {
        Vec::new()
    };
    http::ok(&WorkspaceMemberList {
        members: rows
            .into_iter()
            .map(|r| WorkspaceMember {
                user: fctl_proto::session::UserRef {
                    id: UserId(r.user_id),
                    name: r.name,
                    avatar_url: r.avatar_url,
                },
                email: r.email,
                role: WorkspaceRole::parse(&r.role).unwrap_or(WorkspaceRole::Member),
                // Who lacks a second factor is for its admins to know.
                two_factor: role.is_admin().then_some(r.two_factor),
                added_by: match (r.added_by, r.adder_name) {
                    (Some(id), Some(name)) => Some(fctl_proto::session::UserRef {
                        id: UserId(id),
                        name,
                        avatar_url: None,
                    }),
                    _ => None,
                },
                joined_at: r.created_at,
            })
            .collect(),
        invitations: invitations.iter().map(|i| i.to_invitation(false)).collect(),
    })
}

/// `PATCH /api/v1/workspaces/{id}/members/{user}`: admin or member.
pub async fn set_member_role(
    ctx: &mut Ctx,
    req: &mut Request,
    id: &str,
    user_id: &str,
) -> AppResult<Response> {
    let body: UpdateWorkspaceMemberRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    let actor = a.require_open()?;
    let target = ws::member(&ctx.db, a.id(), user_id)
        .await?
        .ok_or_else(|| not_found("Member"))?;
    let from = WorkspaceRole::parse(&target.role).unwrap_or(WorkspaceRole::Member);
    policy::check_role_change(Some(actor), from, body.role)?;
    // Only from the role it was checked against: a handover in between
    // could have made them the primary admin.
    let changed = q(
        &ctx.db,
        "UPDATE workspace_members SET role = ? WHERE workspace_id = ? AND user_id = ? AND role = ?",
        &[
            s(body.role.as_str()),
            s(a.id()),
            s(user_id),
            s(from.as_str()),
        ],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(conflict(
            "Their role changed in the meantime. Reload to see it, then try again.",
        ));
    }
    if from.is_admin() && !body.role.is_admin() {
        // Adding runners takes being an admin: theirs stop taking its
        // sessions (the ones running there carry on).
        #[derive(Deserialize)]
        struct R {
            runner_id: String,
        }
        let runners: Vec<R> = q(
            &ctx.db,
            "SELECT rw.runner_id FROM runner_workspaces rw JOIN runners r ON r.id = rw.runner_id \
             WHERE rw.workspace_id = ? AND r.created_by = ?",
            &[s(a.id()), s(user_id)],
        )?
        .all()
        .await?;
        if !runners.is_empty() {
            q(
                &ctx.db,
                "DELETE FROM runner_workspaces WHERE workspace_id = ? AND runner_id IN \
                 (SELECT id FROM runners WHERE created_by = ?)",
                &[s(a.id()), s(user_id)],
            )?
            .run()
            .await?;
            for r in &runners {
                runner_changed(ctx, &r.runner_id);
            }
        }
    }
    ctx.audit_in(
        Some(a.id()),
        "workspace.member_role",
        Some(user_id),
        Some(serde_json::json!({ "from": from.as_str(), "to": body.role.as_str() })),
    );
    ws::changed(ctx, vec![user_id.to_owned()]);
    members(ctx, a.id()).await
}

/// Stops `user_id`'s active sessions in a workspace they no longer are in:
/// they run on their Claude subscription.
async fn stop_sessions_of(ctx: &Ctx, workspace_id: &str, user_id: &str) -> AppResult<u32> {
    #[derive(Deserialize)]
    struct S {
        id: String,
    }
    // Theirs, and the workspace's on their machines, which it no longer
    // trusts with its code and tokens.
    let running: Vec<S> = q(
        &ctx.db,
        &format!(
            "SELECT s.id FROM sessions s JOIN projects p ON p.id = s.project_id \
             WHERE p.workspace_id = ?1 AND s.status IN {ACTIVE} \
             AND (s.created_by = ?2 OR s.runner_id IN (SELECT id FROM runners WHERE created_by = ?2))"
        ),
        &[s(workspace_id), s(user_id)],
    )?
    .all()
    .await?;
    let fleet = internal::fleet_stub(&ctx.env)?;
    for session in &running {
        let _: serde_json::Value = internal::post(
            &fleet,
            "/stop",
            &internal::Stop {
                session_id: fctl_proto::SessionId(session.id.clone()),
                reason: fctl_proto::runner::StopReason::User,
                destroy: false,
            },
        )
        .await?;
    }
    Ok(running.len() as u32)
}

/// Takes someone out of a workspace, with their project memberships and
/// shares there, their runners and the invitations of their other
/// addresses; their sessions in it stop, and its sessions on their runners.
pub async fn remove_from(ctx: &Ctx, workspace_id: &str, user_id: &str) -> AppResult<()> {
    stop_sessions_of(ctx, workspace_id, user_id).await?;
    #[derive(Deserialize)]
    struct R {
        runner_id: String,
    }
    let runners: Vec<R> = q(
        &ctx.db,
        "SELECT rw.runner_id FROM runner_workspaces rw JOIN runners r ON r.id = rw.runner_id \
         WHERE rw.workspace_id = ? AND r.created_by = ?",
        &[s(workspace_id), s(user_id)],
    )?
    .all()
    .await?;
    batch(
        &ctx.db,
        vec![
            // Their runners leave with them.
            q(
                &ctx.db,
                "DELETE FROM runner_workspaces WHERE workspace_id = ? AND runner_id IN \
                 (SELECT id FROM runners WHERE created_by = ?)",
                &[s(workspace_id), s(user_id)],
            )?,
            // So do the invitations of their other addresses, which would
            // let them back in.
            q(
                &ctx.db,
                "DELETE FROM workspace_invitations WHERE workspace_id = ? AND email IN \
                 (SELECT email FROM user_emails WHERE user_id = ?)",
                &[s(workspace_id), s(user_id)],
            )?,
            q(
                &ctx.db,
                "DELETE FROM project_members WHERE user_id = ? AND project_id IN (SELECT id FROM projects WHERE workspace_id = ?)",
                &[s(user_id), s(workspace_id)],
            )?,
            q(
                &ctx.db,
                "DELETE FROM session_shares WHERE user_id = ? AND session_id IN \
                 (SELECT s.id FROM sessions s JOIN projects p ON p.id = s.project_id WHERE p.workspace_id = ?)",
                &[s(user_id), s(workspace_id)],
            )?,
            q(
                &ctx.db,
                "DELETE FROM workspace_members WHERE workspace_id = ? AND user_id = ? AND role != 'owner'",
                &[s(workspace_id), s(user_id)],
            )?,
        ],
    )
    .await?;
    for r in &runners {
        runner_changed(ctx, &r.runner_id);
    }
    Ok(())
}

/// `DELETE /api/v1/workspaces/{id}/members/{user}`: an admin removes
/// someone, or someone leaves.
pub async fn remove_member(ctx: &mut Ctx, id: &str, user_id: &str) -> AppResult<Response> {
    let me = ctx.user()?.id.clone();
    let a = ws::load(ctx, id).await?;
    let own = me == user_id;
    // Leaving works while the workspace is closed to you, too.
    let actor = if own { a.role } else { Some(a.require_open()?) };
    let target = ws::member(&ctx.db, a.id(), user_id)
        .await?
        .ok_or_else(|| not_found("Member"))?;
    let role = WorkspaceRole::parse(&target.role).unwrap_or(WorkspaceRole::Member);
    policy::check_removal(actor, role, own)?;
    remove_from(ctx, a.id(), user_id).await?;
    ctx.audit_in(
        Some(a.id()),
        if own {
            "workspace.left"
        } else {
            "workspace.member_removed"
        },
        Some(user_id),
        Some(serde_json::json!({ "email": target.email })),
    );
    ws::changed(ctx, vec![user_id.to_owned()]);
    if own {
        return http::ok(&Ok { ok: true });
    }
    members(ctx, a.id()).await
}

/// `POST /api/v1/workspaces/{id}/transfer` (its primary admin): to another
/// admin, and the primary admin stays an admin.
pub async fn transfer(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: TransferRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    let me = ctx.user()?.id.clone();
    let to = ws::member(&ctx.db, a.id(), body.user_id.as_str())
        .await?
        .and_then(|m| WorkspaceRole::parse(&m.role));
    a.require_open()?;
    policy::check_transfer(a.role, to)?;
    // The old primary admin goes first (one per workspace), and only while
    // the new one is still an admin, so that the workspace always has one.
    batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "UPDATE workspace_members SET role = 'admin' WHERE workspace_id = ?1 AND user_id = ?2 AND role = 'owner' \
                 AND EXISTS (SELECT 1 FROM workspace_members t WHERE t.workspace_id = ?1 AND t.user_id = ?3 AND t.role = 'admin')",
                &[s(a.id()), s(&me), s(body.user_id.as_str())],
            )?,
            q(
                &ctx.db,
                "UPDATE workspace_members SET role = 'owner' WHERE workspace_id = ?1 AND user_id = ?2 AND role = 'admin' \
                 AND NOT EXISTS (SELECT 1 FROM workspace_members o WHERE o.workspace_id = ?1 AND o.role = 'owner')",
                &[s(a.id()), s(body.user_id.as_str())],
            )?,
        ],
    )
    .await?;
    let became = ws::member(&ctx.db, a.id(), body.user_id.as_str())
        .await?
        .and_then(|m| WorkspaceRole::parse(&m.role));
    if became != Some(WorkspaceRole::Owner) {
        return Err(conflict(
            "They stopped being an admin in the meantime, so you are still the primary admin",
        ));
    }
    ctx.audit_in(
        Some(a.id()),
        "workspace.transferred",
        Some(body.user_id.as_str()),
        Some(serde_json::json!({ "from": me })),
    );
    ws::changed(ctx, vec![me, body.user_id.0.clone()]);
    members(ctx, a.id()).await
}

/// `PATCH /api/v1/workspaces/{id}/membership`: the caller's preferences.
pub async fn update_membership(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdateMembershipRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    a.require_open()?;
    let user = ctx.user()?.clone();
    if let Some(wanted) = &body.project_order {
        // The projects they see there, in their order now.
        let current: Vec<Named> = q(
            &ctx.db,
            &format!(
                "SELECT p.id, p.slug FROM projects p JOIN workspaces w ON w.id = p.workspace_id \
                 JOIN workspace_members wm ON wm.workspace_id = w.id AND wm.user_id = ?1 \
                 LEFT JOIN project_members m ON m.project_id = p.id AND m.user_id = ?1 \
                 WHERE w.id = ?2 AND {PROJECT_VISIBLE} ORDER BY {PROJECT_ORDER}"
            ),
            &[s(&user.id), s(a.id())],
        )?
        .all()
        .await?;
        let order = fctl_domain::order::arrange(
            &Named::pairs(&current),
            wanted,
            "project_order",
            &format!("your projects in {}", a.row.name),
        )?;
        q(
            &ctx.db,
            "UPDATE workspace_members SET project_order = ? WHERE workspace_id = ? AND user_id = ?",
            &[s(&serde_json::to_string(&order)?), s(a.id()), s(&user.id)],
        )?
        .run()
        .await?;
    }
    if let Some(claude) = body.claude.as_deref() {
        let claude = claude.trim();
        if !claude.is_empty() {
            let theirs: Option<Count> = q(
                &ctx.db,
                "SELECT COUNT(*) AS n FROM claude_credentials WHERE id = ? AND user_id = ? AND status = 'active'",
                &[s(claude), s(&user.id)],
            )?
            .first()
            .await?;
            if theirs.map_or(0, |c| c.n) == 0 {
                return Err(validation("claude", "Pick one of your Claude connections"));
            }
        }
        q(
            &ctx.db,
            "UPDATE workspace_members SET credential_id = ? WHERE workspace_id = ? AND user_id = ?",
            &[
                os(Some(claude).filter(|c| !c.is_empty())),
                s(a.id()),
                s(&user.id),
            ],
        )?
        .run()
        .await?;
    }
    http::ok(&respond(ctx, a.id()).await?)
}

/// Something with an id and a slug, which both name it.
#[derive(Deserialize)]
struct Named {
    id: String,
    slug: String,
}

impl Named {
    fn pairs(list: &[Named]) -> Vec<(&str, &str)> {
        list.iter()
            .map(|n| (n.id.as_str(), n.slug.as_str()))
            .collect()
    }
}

/// `PUT /api/v1/me/workspace-order`: the caller's workspaces in the order
/// they want them; the ones they don't name follow as they were.
pub async fn order(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: WorkspaceOrderRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    let current: Vec<Named> = q(
        &ctx.db,
        &format!(
            "SELECT w.id, w.slug FROM workspace_members wm JOIN workspaces w ON w.id = wm.workspace_id \
             WHERE wm.user_id = ? ORDER BY {WORKSPACE_ORDER}"
        ),
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let order = fctl_domain::order::arrange(
        &Named::pairs(&current),
        &body.workspaces,
        "workspaces",
        "your workspaces",
    )?;
    let statements = order
        .iter()
        .zip(0_i64..)
        .map(|(id, position)| {
            q(
                &ctx.db,
                "UPDATE workspace_members SET position = ? WHERE workspace_id = ? AND user_id = ?",
                &[n(position), s(id), s(&user.id)],
            )
        })
        .collect::<AppResult<Vec<_>>>()?;
    if !statements.is_empty() {
        batch(&ctx.db, statements).await?;
    }
    list(ctx).await
}

// ───────────────────────────── invitations ─────────────────────────────

/// Whether `email` can sign in to the instance: an account has it, or the
/// allowlist admits it.
async fn on_instance(ctx: &Ctx, email: &str) -> AppResult<bool> {
    if crate::auth::user_by_email(ctx, email).await?.is_some() {
        return Ok(true);
    }
    Ok(crate::api::auth::allow_match(ctx, email).await?.is_some())
}

/// Creates (or renews) an invitation to a workspace, and emails it
/// (`notify`). The caller checked that they may invite.
pub async fn invite(
    ctx: &Ctx,
    workspace: &WorkspaceRow,
    email: &str,
    role: WorkspaceRole,
    notify: bool,
) -> AppResult<WorkspaceInvitation> {
    let already: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM workspace_members m JOIN user_emails e ON e.user_id = m.user_id \
         WHERE m.workspace_id = ? AND e.email = ?",
        &[s(&workspace.id), s(email)],
    )?
    .first()
    .await?;
    if already.map_or(0, |c| c.n) > 0 {
        return Err(conflict("They are a member of this workspace already"));
    }
    let id = fctl_domain::ids::workspace_invite_id(ctx.now);
    let actor = ctx.user()?;
    q(
        &ctx.db,
        "INSERT INTO workspace_invitations (id, workspace_id, email, role, invited_by, created_at, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT (workspace_id, email) DO UPDATE SET role = excluded.role, \
         invited_by = excluded.invited_by, created_at = excluded.created_at, expires_at = excluded.expires_at",
        &[
            s(id.as_str()),
            s(&workspace.id),
            s(email),
            s(role.as_str()),
            s(&actor.id),
            n(ctx.now),
            n(ctx.now + policy::INVITATION_TTL_MS),
        ],
    )?
    .run()
    .await?;
    let row: InvitationRow = q(
        &ctx.db,
        &format!("{INVITATION_SELECT} WHERE i.workspace_id = ? AND i.email = ?"),
        &[s(&workspace.id), s(email)],
    )?
    .first()
    .await?
    .ok_or_else(|| not_found("Invitation"))?;
    let mut emailed = false;
    if let Some(mail) = ctx.cfg.email.clone().filter(|_| notify) {
        let instance = crate::instance::name(&ctx.db, &ctx.cfg).await?;
        let text = format!(
            "{} invited you to join {} on {instance}, where you and its people give Claude agents work in its projects.\n\n\
             Open {instance} to join: {}\n\nThe invitation works for 30 days.",
            actor.name,
            workspace.name,
            ctx.cfg.url("/"),
        );
        emailed = crate::email::send(
            &mail,
            email,
            &format!("Join {} on {instance}", workspace.name),
            &text,
        )
        .await
        .is_ok();
    }
    ctx.audit_in(
        Some(&workspace.id),
        "workspace.invited",
        Some(email),
        Some(serde_json::json!({ "role": role.as_str() })),
    );
    if let Some(invitee) = crate::auth::user_by_email(ctx, email).await? {
        ws::changed(ctx, vec![invitee.id]);
    }
    let mut out = row.to_invitation(false);
    out.emailed = Some(emailed);
    Ok(out)
}

/// `POST /api/v1/workspaces/{id}/invitations` (admins).
pub async fn create_invitation(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: InviteToWorkspaceRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let email = validate::email(&body.email)?;
    let role = policy::invitation_role(body.role)?;
    if !on_instance(ctx, &email).await? {
        return Err(AppError(ApiError::new(
            ErrorCode::AccessDenied,
            format!(
                "{email} can't sign in to this instance yet. An instance admin invites them first, and can invite them to this workspace at the same time."
            ),
        )));
    }
    http::created(&invite(ctx, &a.row, &email, role, true).await?)
}

/// `DELETE /api/v1/workspaces/{id}/invitations/{invitation}` (admins).
pub async fn revoke_invitation(ctx: &mut Ctx, id: &str, invitation: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let removed = q(
        &ctx.db,
        "DELETE FROM workspace_invitations WHERE id = ? AND workspace_id = ? RETURNING email",
        &[s(invitation), s(a.id())],
    )?
    .all::<serde_json::Value>()
    .await?;
    let Some(email) = removed
        .first()
        .and_then(|r| r.get("email"))
        .and_then(serde_json::Value::as_str)
    else {
        return Err(not_found("Invitation"));
    };
    ctx.audit_in(
        Some(a.id()),
        "workspace.invitation_revoked",
        Some(email),
        None,
    );
    members(ctx, a.id()).await
}

/// An invitation for one of the caller's addresses.
async fn my_invitation(ctx: &Ctx, id: &str) -> AppResult<InvitationRow> {
    let user = ctx.user()?;
    q(
        &ctx.db,
        &format!(
            "{INVITATION_SELECT} WHERE i.id = ?1 AND i.expires_at > ?2 \
             AND i.email IN (SELECT email FROM user_emails WHERE user_id = ?3)"
        ),
        &[s(id), n(ctx.now), s(&user.id)],
    )?
    .first()
    .await?
    .ok_or_else(|| not_found("Invitation"))
}

/// `POST /api/v1/invitations/{id}/accept`.
pub async fn accept(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let invitation = my_invitation(ctx, id).await?;
    let user = ctx.user()?.clone();
    if !ws::join(&ctx.db, &ctx.settings(), &user, &invitation, ctx.now).await? {
        return Err(conflict("You are a member of this workspace already"));
    }
    ctx.audit_in(
        Some(&invitation.workspace_id),
        "workspace.joined",
        Some(&user.id),
        Some(serde_json::json!({ "email": invitation.email, "role": invitation.role })),
    );
    ws::changed(ctx, vec![user.id.clone()]);
    http::ok(&respond(ctx, &invitation.workspace_id).await?)
}

/// `POST /api/v1/invitations/{id}/decline`.
pub async fn decline(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let invitation = my_invitation(ctx, id).await?;
    q(
        &ctx.db,
        "DELETE FROM workspace_invitations WHERE id = ?",
        &[s(&invitation.id)],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(&invitation.workspace_id),
        "workspace.invitation_declined",
        Some(&invitation.email),
        None,
    );
    http::ok(&Ok { ok: true })
}

// ───────────────────────────── runners ─────────────────────────────

fn runner_view<'a>(user: &'a UserRow, workspaces: &'a [String]) -> RunnerView<'a> {
    RunnerView {
        user_id: Some(&user.id),
        workspaces: Some(workspaces),
    }
}

async fn my_workspace_ids(ctx: &Ctx, user: &UserRow) -> AppResult<Vec<String>> {
    Ok(ws::mine(&ctx.db, &user.id)
        .await?
        .into_iter()
        .filter(|w| !ws::blocked_for(ctx, user, w.require_two_factor))
        .map(|w| w.id)
        .collect())
}

/// `GET /api/v1/workspaces/{id}/runners`: the runners its sessions run on.
pub async fn runners(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_open()?;
    let user = ctx.user()?;
    let rows: Vec<RunnerRow> = q(
        &ctx.db,
        &format!(
            "{RUNNER_SELECT} WHERE EXISTS (SELECT 1 FROM runner_workspaces rw WHERE rw.runner_id = r.id \
             AND rw.workspace_id = ? AND {}) ORDER BY r.created_at",
            crate::serves!()
        ),
        &[s(a.id())],
    )?
    .all()
    .await?;
    let live = live(ctx).await;
    let mine = my_workspace_ids(ctx, user).await?;
    let view = runner_view(user, &mine);
    http::ok(&RunnerList {
        runners: rows.iter().map(|r| r.to_runner(&live, &view)).collect(),
    })
}

/// Registers a runner for `workspace_id`: the caller owns it.
pub async fn register_runner(
    ctx: &Ctx,
    name: &str,
    workspace_id: Option<&str>,
) -> AppResult<RunnerRegistration> {
    let actor = ctx.user()?;
    let name = validate::name("name", name)?;
    let token = new_secret(SecretKind::Runner);
    let id = fctl_domain::ids::runner_id(ctx.now);
    let mut statements = vec![q(
        &ctx.db,
        "INSERT INTO runners (id, name, token_hash, enabled, created_by, created_at) VALUES (?, ?, ?, 1, ?, ?)",
        &[
            s(id.as_str()),
            s(&name),
            s(&hash_secret(&token)),
            s(&actor.id),
            n(ctx.now),
        ],
    )?];
    if let Some(w) = workspace_id {
        statements.push(q(
            &ctx.db,
            "INSERT INTO runner_workspaces (runner_id, workspace_id, added_by, created_at) VALUES (?, ?, ?, ?)",
            &[s(id.as_str()), s(w), s(&actor.id), n(ctx.now)],
        )?);
    }
    batch(&ctx.db, statements).await?;
    ctx.audit(
        "runner.created",
        Some(id.as_str()),
        Some(serde_json::json!({ "name": name, "workspace": workspace_id })),
    );
    if let Some(w) = workspace_id {
        ctx.audit_in(
            Some(w),
            "workspace.runner_added",
            Some(id.as_str()),
            Some(serde_json::json!({ "name": name })),
        );
    }
    let row: Option<RunnerRow> = q(
        &ctx.db,
        &format!("{RUNNER_SELECT} WHERE r.id = ?"),
        &[s(id.as_str())],
    )?
    .first()
    .await?;
    let mine = my_workspace_ids(ctx, actor).await?;
    let runner = row
        .ok_or_else(|| not_found("Runner"))?
        .to_runner(&LiveRunners::default(), &runner_view(actor, &mine));
    Ok(RunnerRegistration { runner, token })
}

/// `POST /api/v1/workspaces/{id}/runners` (admins): registers a runner
/// for it, which the caller then installs (`fctl host install`).
pub async fn create_runner(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: CreateRunnerRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    http::created(&register_runner(ctx, &body.name, Some(a.id())).await?)
}

#[derive(Deserialize)]
struct RunnerOwner {
    created_by: Option<String>,
    name: String,
}

async fn runner_owner(ctx: &Ctx, runner_id: &str) -> AppResult<RunnerOwner> {
    q(
        &ctx.db,
        "SELECT created_by, name FROM runners WHERE id = ?",
        &[s(runner_id)],
    )?
    .first()
    .await?
    .ok_or_else(|| not_found("Runner"))
}

/// Tells FleetObject that a runner's workspaces changed: feeds update and
/// queued sessions may start on it.
fn runner_changed(ctx: &Ctx, runner_id: &str) {
    let env = ctx.env.clone();
    let runner_id = runner_id.to_owned();
    ctx.defer(async move {
        let stub = internal::fleet_stub(&env)?;
        let _: serde_json::Value = internal::post(
            &stub,
            "/runner-changed",
            &internal::RunnerChanged { runner_id },
        )
        .await?;
        Ok(())
    });
}

/// `PUT /api/v1/workspaces/{id}/runners/{runner}`: its owner, an admin of
/// the workspace, lets it run the workspace's sessions.
pub async fn add_runner(ctx: &mut Ctx, id: &str, runner_id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    let role = a.require_open()?;
    let me = ctx.user()?.id.clone();
    let owner = runner_owner(ctx, runner_id).await?;
    policy::check_add_runner(owner.created_by.as_deref() == Some(me.as_str()), Some(role))?;
    q(
        &ctx.db,
        "INSERT OR IGNORE INTO runner_workspaces (runner_id, workspace_id, added_by, created_at) VALUES (?, ?, ?, ?)",
        &[s(runner_id), s(a.id()), s(&me), n(ctx.now)],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(a.id()),
        "workspace.runner_added",
        Some(runner_id),
        Some(serde_json::json!({ "name": owner.name })),
    );
    runner_changed(ctx, runner_id);
    runners(ctx, a.id()).await
}

/// `DELETE /api/v1/workspaces/{id}/runners/{runner}`: the workspace's
/// sessions no longer start on it (the ones running there carry on).
pub async fn remove_runner(ctx: &mut Ctx, id: &str, runner_id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    let role = a.require_open()?;
    let user = ctx.user()?.clone();
    let owner = runner_owner(ctx, runner_id).await?;
    if !policy::can_remove_runner(
        owner.created_by.as_deref() == Some(user.id.as_str()),
        Some(role),
        user.is_admin(),
    ) {
        return Err(forbidden(
            "Only the workspace's admins and the runner's owner can take it out",
        ));
    }
    let removed = q(
        &ctx.db,
        "DELETE FROM runner_workspaces WHERE runner_id = ? AND workspace_id = ?",
        &[s(runner_id), s(a.id())],
    )?
    .run()
    .await?;
    if removed == 0 {
        return Err(not_found("Runner"));
    }
    ctx.audit_in(
        Some(a.id()),
        "workspace.runner_removed",
        Some(runner_id),
        Some(serde_json::json!({ "name": owner.name })),
    );
    runner_changed(ctx, runner_id);
    runners(ctx, a.id()).await
}

// ───────────────────────────── audit ─────────────────────────────

/// `GET /api/v1/workspaces/{id}/audit?before=` (admins).
pub async fn audit(ctx: &Ctx, req: &Request, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    crate::api::admin::audit_page(ctx, req, Some(a.id())).await
}

/// Instance invitations, after an account is made or an address is
/// connected: the workspaces it was invited to.
pub async fn joined_after_sign_up(
    ctx: &Ctx,
    user: &UserRow,
    email: &str,
) -> AppResult<Vec<fctl_proto::session::WorkspaceRef>> {
    let joined = ws::join_invited(ctx, user, email).await?;
    if !joined.is_empty() {
        ws::changed(ctx, vec![user.id.clone()]);
    }
    Ok(joined)
}

/// Makes sure the instance-wide view of a runner list carries what the
/// caller may see.
pub async fn runner_view_for(ctx: &Ctx) -> AppResult<(Option<Vec<String>>, String)> {
    let user = ctx.user()?;
    if user.is_admin() {
        return Ok((None, user.id.clone()));
    }
    Ok((Some(my_workspace_ids(ctx, user).await?), user.id.clone()))
}
