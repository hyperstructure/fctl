//! What admin tokens do (docs/architecture.md#admin-tokens,
//! `fctl_domain::admin_tokens`): programs provision people with them,
//! where the instance's deployment turns them on. They create accounts,
//! read them with the workspaces and projects they are in, set their
//! pictures and the Claude token their sessions run on, and place them in
//! any workspace and project or take them out again. `api::admin` changes
//! their names, addresses and commit emails, disables and deletes them,
//! for admin tokens as for admins.

use crate::api::avatars;
use crate::auth;
use crate::ctx::Ctx;
use crate::db::{Query, batch, n, os, q, s};
use crate::error::{AppError, AppResult, conflict, not_found, validation};
use crate::http;
use crate::model::{CredentialRow, CredentialSecret, UserRow, workspace_ref};
use crate::workspaces as ws;
use fctl_domain::admin_tokens as policy;
use fctl_domain::validate;
use fctl_domain::workspace as wpolicy;
use fctl_proto::api::{
    AddClaudeTokenRequest, AdminClaudeToken, AdminUser, AdminUserProject, AdminUserWorkspace,
    AvatarSource, CreateUserRequest, CredentialStatus, SetRoleRequest,
    SetWorkspaceMembershipRequest,
};
use fctl_proto::session::{ProjectRole, WorkspaceRole};
use fctl_proto::{CredentialId, ProjectId};
use serde::Deserialize;
use worker::{D1Database, Request, Response};

/// The note of the allowlist entries that let in the addresses admin
/// tokens give people.
pub const ALLOWED_NOTE: &str = "Admin token";

/// The admin an admin token acts as: anyone else is refused, as the router
/// refuses them already.
fn maker(ctx: &Ctx) -> AppResult<UserRow> {
    if ctx.admin_token().is_none() {
        return Err(AppError(policy::needs_token(ctx.cfg.admin_tokens)));
    }
    Ok(ctx.admin()?.clone())
}

/// Someone on the instance, by their user id or any of their addresses;
/// never a deleted account's placeholder.
pub async fn find(ctx: &Ctx, who: &str) -> AppResult<UserRow> {
    let who = who.trim();
    let found = if who.contains('@') {
        auth::user_by_email(ctx, &who.to_lowercase()).await?
    } else {
        auth::user_by_id(ctx, who).await?
    };
    found
        .filter(|u| !fctl_domain::account::is_tombstone(&u.email))
        .ok_or_else(|| not_found("User"))
}

/// Someone whose account, or memberships, `maker`'s admin token may change
/// (`policy::check_target`).
async fn target(ctx: &Ctx, maker: &UserRow, who: &str) -> AppResult<UserRow> {
    let user = find(ctx, who).await?;
    policy::check_target(maker.role(), user.role()).map_err(AppError)?;
    Ok(user)
}

/// Refuses what a disabled account can't take: places in workspaces, and
/// a Claude token to run sessions on.
fn active(user: &UserRow) -> AppResult<()> {
    if user.is_active() {
        Ok(())
    } else {
        Err(conflict(
            "Their account is disabled. Enable it first (`PATCH /api/v1/admin/users/{user}` with `{\"status\": \"active\"}`).",
        ))
    }
}

/// The statement that lets `email` in, as a member, unless an entry names
/// it already: removing another entry disables the accounts none of whose
/// addresses an entry admits (`api::admin::remove_allow`), so the
/// addresses admin tokens give people are on the allowlist as invited
/// ones are.
pub fn allow(db: &D1Database, email: &str, by: &str, now: i64) -> AppResult<Query> {
    q(
        db,
        "INSERT OR IGNORE INTO allowlist (id, pattern, role, note, created_by, created_at) \
         VALUES (?, ?, 'member', ?, ?, ?)",
        &[
            s(fctl_domain::ids::allow_id(now).as_str()),
            s(email),
            s(ALLOWED_NOTE),
            s(by),
            n(now),
        ],
    )
}

/// `POST /api/v1/admin/users`: an account for someone, active, as a member
/// of the instance, who joins the workspaces their address was invited
/// to, as signing up would.
pub async fn create(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: CreateUserRequest = http::body(req).await?;
    let maker = maker(ctx)?;
    let email = validate::email(&body.email)?;
    let person = validate::person_name(
        body.given_name.as_deref(),
        body.family_name.as_deref(),
        body.name.as_deref(),
    )?;
    let preferred = match body.preferred_name.as_deref() {
        Some(p) => validate::preferred_name(p)?,
        None => None,
    };
    let commit_email = match body.commit_email.as_deref() {
        Some(c) => validate::commit_email(c)?,
        None => None,
    };
    let hash = match body.password.as_deref() {
        Some(password) => {
            validate::password(password, Some(&email))?;
            Some(auth::hash_password(password)?)
        }
        None => None,
    };
    let taken = || conflict("An account has this address already");
    if auth::user_by_email(ctx, &email).await?.is_some() {
        return Err(taken());
    }
    let id = fctl_domain::ids::user_id(ctx.now);
    let made = batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "INSERT INTO users (id, email, name, given_name, family_name, preferred_name, password_hash, role, status, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, 'member', 'active', ?, ?)",
                &[
                    s(id.as_str()),
                    s(&email),
                    s(&person.full),
                    os(person.given.as_deref()),
                    os(person.family.as_deref()),
                    os(preferred.as_deref()),
                    os(hash.as_deref()),
                    n(ctx.now),
                    n(ctx.now),
                ],
            )?,
            q(
                &ctx.db,
                "INSERT INTO user_emails (email, user_id, commit_email, created_at) VALUES (?, ?, ?, ?)",
                &[s(&email), s(id.as_str()), os(commit_email.as_deref()), n(ctx.now)],
            )?,
            allow(&ctx.db, &email, &maker.id, ctx.now)?,
        ],
    )
    .await;
    if let Err(e) = made {
        // Another request made it a moment ago.
        if auth::user_by_email(ctx, &email).await?.is_some() {
            return Err(taken());
        }
        return Err(e);
    }
    let user = auth::user_by_id(ctx, id.as_str())
        .await?
        .ok_or_else(|| crate::error::internal("user vanished after insert"))?;
    let user = crate::api::auth::new_gravatar(ctx, user).await?;
    ctx.audit(
        "user.created",
        Some(&user.id),
        Some(serde_json::json!({ "email": email, "password": hash.is_some() })),
    );
    crate::api::workspaces::joined_after_sign_up(ctx, &user, &email).await?;
    http::created(&read(ctx, &user.id).await?)
}

/// `GET /api/v1/admin/users/{user}`: anyone's account, by id or address.
pub async fn get(ctx: &Ctx, who: &str) -> AppResult<Response> {
    maker(ctx)?;
    let user = find(ctx, who).await?;
    http::ok(&read(ctx, &user.id).await?)
}

#[derive(Deserialize)]
struct WorkspaceRow {
    id: String,
    slug: String,
    name: String,
    color: Option<String>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    avatar_updated_at: Option<i64>,
    role: String,
    email: String,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
}

#[derive(Deserialize)]
struct ProjectRow {
    id: String,
    slug: String,
    name: String,
    role: String,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    workspace_id: String,
    workspace_slug: String,
    workspace_name: String,
    workspace_color: Option<String>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    workspace_avatar_at: Option<i64>,
}

#[derive(Deserialize)]
struct ClaudeRow {
    id: String,
    label: Option<String>,
    status: String,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    #[serde(deserialize_with = "crate::db::int")]
    updated_at: i64,
    /// The id of the connection their sessions run on when they chose
    /// none: their newest working one (`api::sessions::credential_for`).
    newest: Option<String>,
}

/// Someone's account as admin tokens read it, as it is now: never their
/// Claude token, nor their own Claude connections.
pub async fn read(ctx: &Ctx, user_id: &str) -> AppResult<AdminUser> {
    let user = auth::user_by_id(ctx, user_id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    let workspaces: Vec<WorkspaceRow> = q(
        &ctx.db,
        "SELECT w.id, w.slug, w.name, w.color, w.avatar_updated_at, m.role, m.email, m.created_at \
         FROM workspace_members m JOIN workspaces w ON w.id = m.workspace_id \
         WHERE m.user_id = ? ORDER BY m.created_at",
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let projects: Vec<ProjectRow> = q(
        &ctx.db,
        "SELECT p.id, p.slug, p.name, m.role, m.created_at, w.id AS workspace_id, w.slug AS workspace_slug, \
         w.name AS workspace_name, w.color AS workspace_color, w.avatar_updated_at AS workspace_avatar_at \
         FROM project_members m JOIN projects p ON p.id = m.project_id JOIN workspaces w ON w.id = p.workspace_id \
         WHERE m.user_id = ? ORDER BY m.created_at",
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let claude: Option<ClaudeRow> = q(
        &ctx.db,
        "SELECT c.id, c.label, c.status, c.created_at, c.updated_at, \
         (SELECT n.id FROM claude_credentials n WHERE n.user_id = c.user_id AND n.status = 'active' \
          ORDER BY n.created_at DESC LIMIT 1) AS newest \
         FROM claude_credentials c WHERE c.user_id = ? AND c.provisioned = 1 LIMIT 1",
        &[s(&user.id)],
    )?
    .first()
    .await?;
    let default_id = user.default_credential_id.clone();
    Ok(AdminUser {
        commit_email: crate::api::emails::primary_commit_email(ctx, &user).await?,
        workspaces: workspaces
            .into_iter()
            .map(|w| AdminUserWorkspace {
                workspace: workspace_ref(
                    &w.id,
                    &w.slug,
                    &w.name,
                    w.color.as_deref(),
                    w.avatar_updated_at,
                ),
                role: WorkspaceRole::parse(&w.role).unwrap_or(WorkspaceRole::Member),
                email: w.email,
                joined_at: w.created_at,
            })
            .collect(),
        projects: projects
            .into_iter()
            .map(|p| AdminUserProject {
                id: ProjectId(p.id),
                slug: p.slug,
                name: p.name,
                workspace: workspace_ref(
                    &p.workspace_id,
                    &p.workspace_slug,
                    &p.workspace_name,
                    p.workspace_color.as_deref(),
                    p.workspace_avatar_at,
                ),
                role: ProjectRole::parse(&p.role).unwrap_or(ProjectRole::Viewer),
                added_at: p.created_at,
            })
            .collect(),
        claude: claude.map(|c| AdminClaudeToken {
            is_default: default_id.as_ref().or(c.newest.as_ref()) == Some(&c.id),
            id: CredentialId(c.id),
            label: c.label,
            status: if c.status == "active" {
                CredentialStatus::Active
            } else {
                CredentialStatus::Invalid
            },
            created_at: c.created_at,
            updated_at: c.updated_at,
        }),
        user: user.to_user(),
    })
}

// ───────────────────────────── addresses ─────────────────────────────

/// What makes `new` the primary address of `user_id` in place of `old`: it
/// signs in from now on, keeping `old`'s commit email unless it has one;
/// the workspaces they joined as `old` know them by it; the allowlist
/// admits it. `old` stops signing in, and goes as a deleted account's
/// addresses go (`api::account::erase`), with its invitations and
/// allowlist entries, and the password reset links that may have gone to
/// it stop working.
pub fn change_email(
    db: &D1Database,
    user_id: &str,
    old: &str,
    new: &str,
    by: &str,
    now: i64,
) -> AppResult<Vec<Query>> {
    Ok(vec![
        q(
            db,
            "INSERT INTO user_emails (email, user_id, commit_email, created_at) \
             SELECT ?1, ?2, (SELECT commit_email FROM user_emails WHERE email = ?3 AND user_id = ?2), ?4 \
             WHERE NOT EXISTS (SELECT 1 FROM user_emails WHERE email = ?1)",
            &[s(new), s(user_id), s(old), n(now)],
        )?,
        q(
            db,
            "UPDATE user_emails SET commit_email = (SELECT o.commit_email FROM user_emails o WHERE o.email = ?3 AND o.user_id = ?2) \
             WHERE email = ?1 AND user_id = ?2 AND commit_email IS NULL",
            &[s(new), s(user_id), s(old)],
        )?,
        // Only while it is still theirs, and the new one is theirs too.
        q(
            db,
            "UPDATE users SET email = ?1, gravatar_checked_at = NULL, updated_at = ?4 \
             WHERE id = ?2 AND email = ?3 AND EXISTS (SELECT 1 FROM user_emails WHERE email = ?1 AND user_id = ?2)",
            &[s(new), s(user_id), s(old), n(now)],
        )?,
        q(
            db,
            "UPDATE workspace_members SET email = ?1 WHERE user_id = ?2 AND email = ?3 \
             AND EXISTS (SELECT 1 FROM users WHERE id = ?2 AND email = ?1)",
            &[s(new), s(user_id), s(old)],
        )?,
        q(
            db,
            "DELETE FROM user_emails WHERE email = ?1 AND user_id = ?2 \
             AND EXISTS (SELECT 1 FROM users WHERE id = ?2 AND email = ?3)",
            &[s(old), s(user_id), s(new)],
        )?,
        q(
            db,
            "DELETE FROM workspace_invitations WHERE email = ?1 \
             AND NOT EXISTS (SELECT 1 FROM user_emails WHERE email = ?1)",
            &[s(old)],
        )?,
        q(
            db,
            "DELETE FROM invites WHERE email = ?1 AND NOT EXISTS (SELECT 1 FROM user_emails WHERE email = ?1)",
            &[s(old)],
        )?,
        q(
            db,
            "DELETE FROM allowlist WHERE pattern = ?1 AND NOT EXISTS (SELECT 1 FROM user_emails WHERE email = ?1) \
             AND NOT EXISTS (SELECT 1 FROM users WHERE email = ?1)",
            &[s(old)],
        )?,
        q(
            db,
            "INSERT OR IGNORE INTO allowlist (id, pattern, role, note, created_by, created_at) \
             SELECT ?1, ?2, 'member', ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM users WHERE id = ?6 AND email = ?2)",
            &[
                s(fctl_domain::ids::allow_id(now).as_str()),
                s(new),
                s(ALLOWED_NOTE),
                s(by),
                n(now),
                s(user_id),
            ],
        )?,
        q(
            db,
            "UPDATE password_resets SET used_at = ?2 WHERE user_id = ?1 AND used_at IS NULL \
             AND EXISTS (SELECT 1 FROM users WHERE id = ?1 AND email = ?3)",
            &[s(user_id), n(now), s(new)],
        )?,
    ])
}

/// Makes `new` `user`'s primary address ([`change_email`]): refused when it
/// is someone else's.
pub async fn set_email(ctx: &Ctx, user: &UserRow, new: &str) -> AppResult<()> {
    if new == user.email {
        return Ok(());
    }
    if auth::user_by_email(ctx, new)
        .await?
        .is_some_and(|other| other.id != user.id)
    {
        return Err(conflict("This address belongs to another account"));
    }
    let by = ctx.user()?.id.clone();
    batch(
        &ctx.db,
        change_email(&ctx.db, &user.id, &user.email, new, &by, ctx.now)?,
    )
    .await?;
    let now = auth::user_by_id(ctx, &user.id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    if now.email != new {
        return Err(conflict(
            "Their address changed in the meantime. Read their account again, then try again.",
        ));
    }
    ctx.audit(
        "user.email_changed",
        Some(&user.id),
        Some(serde_json::json!({ "from": user.email, "to": new })),
    );
    ws::changed(ctx, vec![user.id.clone()]);
    Ok(())
}

// ───────────────────────────── picture ─────────────────────────────

/// `PUT /api/v1/admin/users/{user}/avatar`: their picture, as an
/// `application/octet-stream` body, as an upload of their own would be
/// (`PUT /api/v1/me/avatar`). It replaces the one they had.
pub async fn set_avatar(ctx: &mut Ctx, req: &mut Request, who: &str) -> AppResult<Response> {
    let maker = maker(ctx)?;
    let user = target(ctx, &maker, who).await?;
    let (data, media_type) = avatars::read(req).await?;
    avatars::store(
        &ctx.db,
        ctx.now,
        &user.id,
        &data,
        media_type,
        AvatarSource::Upload,
    )
    .await?;
    ctx.audit("user.avatar_set", Some(&user.id), None);
    http::ok(&read(ctx, &user.id).await?)
}

/// `DELETE /api/v1/admin/users/{user}/avatar`: their initials again, and
/// not their Gravatar either, as removing their own does.
pub async fn remove_avatar(ctx: &mut Ctx, who: &str) -> AppResult<Response> {
    let maker = maker(ctx)?;
    let user = target(ctx, &maker, who).await?;
    batch(&ctx.db, avatars::remove(ctx, &user.id)?).await?;
    ctx.audit("user.avatar_removed", Some(&user.id), None);
    http::ok(&read(ctx, &user.id).await?)
}

// ───────────────────────────── Claude ─────────────────────────────

/// `PUT /api/v1/admin/users/{user}/claude`: the Claude token (`claude
/// setup-token`'s) the sessions they start run on. The one an admin token
/// set before takes it in place, so that their choices of it stay, and
/// sessions that run on it get the new one as they next start; a first one
/// becomes their default. Their own connections stay as they are.
pub async fn set_claude(ctx: &mut Ctx, req: &mut Request, who: &str) -> AppResult<Response> {
    let body: AddClaudeTokenRequest = http::body(req).await?;
    let maker = maker(ctx)?;
    let user = target(ctx, &maker, who).await?;
    active(&user)?;
    let token = body.token.trim().to_owned();
    if let Some(problem) = fctl_domain::claude::setup_token_problem(&token) {
        return Err(validation("token", problem));
    }
    let label = body
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| validate::name("label", l))
        .transpose()?;
    let keys = ctx.keys()?;
    let secret = CredentialSecret {
        access_token: token,
        refresh_token: None,
        expires_at: None,
    };
    #[derive(Deserialize)]
    struct Id {
        id: String,
    }
    let before: Option<Id> = q(
        &ctx.db,
        "SELECT id FROM claude_credentials WHERE user_id = ? AND provisioned = 1 LIMIT 1",
        &[s(&user.id)],
    )?
    .first()
    .await?;
    let expires_at = ctx.now + 365 * auth::DAY_MS;
    let id = match before {
        Some(before) => {
            let sealed = keys.seal_json(&CredentialRow::aad(&before.id, &user.id), &secret)?;
            // What Claude said of the token it replaces isn't this one's.
            batch(
                &ctx.db,
                vec![
                    q(
                        &ctx.db,
                        "UPDATE claude_credentials SET secret_enc = ?, label = COALESCE(?, label), status = 'active', \
                         account_email = NULL, subscription_type = NULL, account = NULL, rate_limit_tier = NULL, \
                         expires_at = ?, updated_at = ? WHERE id = ? AND user_id = ?",
                        &[
                            s(&sealed),
                            os(label.as_deref()),
                            n(expires_at),
                            n(ctx.now),
                            s(&before.id),
                            s(&user.id),
                        ],
                    )?,
                    q(
                        &ctx.db,
                        "DELETE FROM claude_usage WHERE credential_id = ?",
                        &[s(&before.id)],
                    )?,
                ],
            )
            .await?;
            before.id
        }
        None => {
            let id = fctl_domain::ids::credential_id(ctx.now);
            let sealed = keys.seal_json(&CredentialRow::aad(id.as_str(), &user.id), &secret)?;
            batch(
                &ctx.db,
                vec![
                    q(
                        &ctx.db,
                        "INSERT INTO claude_credentials (id, user_id, kind, label, secret_enc, scopes, status, expires_at, created_at, updated_at, provisioned) \
                         VALUES (?, ?, 'setup_token', ?, ?, 'user:inference', 'active', ?, ?, ?, 1)",
                        &[
                            s(id.as_str()),
                            s(&user.id),
                            os(label.as_deref()),
                            s(&sealed),
                            n(expires_at),
                            n(ctx.now),
                            n(ctx.now),
                        ],
                    )?,
                    q(
                        &ctx.db,
                        "UPDATE users SET default_credential_id = ? WHERE id = ?",
                        &[s(id.as_str()), s(&user.id)],
                    )?,
                ],
            )
            .await?;
            id.0
        }
    };
    ctx.audit(
        "claude.token_set",
        Some(&user.id),
        Some(serde_json::json!({ "credential": id })),
    );
    crate::autopilots::owner_moved(&ctx.env, &ctx.db, &user.id, ctx.now).await;
    http::ok(&read(ctx, &user.id).await?)
}

/// `DELETE /api/v1/admin/users/{user}/claude`: the Claude token an admin
/// token set for them goes, as removing a connection of theirs would: the
/// sessions that run on it stop.
pub async fn remove_claude(ctx: &mut Ctx, who: &str) -> AppResult<Response> {
    let maker = maker(ctx)?;
    let user = target(ctx, &maker, who).await?;
    #[derive(Deserialize)]
    struct Id {
        id: String,
    }
    let set: Option<Id> = q(
        &ctx.db,
        "SELECT id FROM claude_credentials WHERE user_id = ? AND provisioned = 1 LIMIT 1",
        &[s(&user.id)],
    )?
    .first()
    .await?;
    let Some(set) = set else {
        return Err(not_found("Claude token"));
    };
    crate::api::claude::remove_credential(ctx, &user.id, &set.id).await?;
    ctx.audit(
        "claude.token_removed",
        Some(&user.id),
        Some(serde_json::json!({ "credential": set.id })),
    );
    crate::autopilots::owner_moved(&ctx.env, &ctx.db, &user.id, ctx.now).await;
    http::ok(&read(ctx, &user.id).await?)
}

// ───────────────────────────── workspaces ─────────────────────────────

/// What makes `user_id` a member of workspace `workspace_id`, as their
/// primary address `email`, with `role`, while they are in fewer than
/// `limit` workspaces: and uses up the invitations of their addresses
/// there, which would let them back in after they leave.
#[allow(clippy::too_many_arguments)]
pub fn join(
    db: &D1Database,
    workspace_id: &str,
    user_id: &str,
    email: &str,
    role: WorkspaceRole,
    by: &str,
    limit: Option<usize>,
    now: i64,
) -> AppResult<Vec<Query>> {
    Ok(vec![
        ws::add_member(db, workspace_id, user_id, email, role, Some(by), limit, now)?,
        q(
            db,
            "DELETE FROM workspace_invitations WHERE workspace_id = ?1 \
             AND email IN (SELECT email FROM user_emails WHERE user_id = ?2) \
             AND EXISTS (SELECT 1 FROM workspace_members WHERE workspace_id = ?1 AND user_id = ?2)",
            &[s(workspace_id), s(user_id)],
        )?,
    ])
}

/// A workspace, by id or slug, whoever is in it.
async fn workspace(ctx: &Ctx, maker: &UserRow, id_or_slug: &str) -> AppResult<ws::WorkspaceRow> {
    ws::find(&ctx.db, &maker.id, id_or_slug)
        .await?
        .ok_or_else(|| not_found("Workspace"))
}

/// `PUT /api/v1/admin/users/{user}/workspaces/{workspace}`: makes them a
/// member of any workspace, or one of its admins, as their primary
/// address; a member already gets the role. Its primary admin stays its
/// primary admin. A workspace that requires two-factor sign-in stays
/// closed to them until they set it up.
pub async fn set_workspace(
    ctx: &mut Ctx,
    req: &mut Request,
    who: &str,
    id_or_slug: &str,
) -> AppResult<Response> {
    let body: SetWorkspaceMembershipRequest = http::body(req).await?;
    let maker = maker(ctx)?;
    let user = target(ctx, &maker, who).await?;
    let role = wpolicy::invitation_role(body.role).map_err(AppError)?;
    let w = workspace(ctx, &maker, id_or_slug).await?;
    match ws::member(&ctx.db, &w.id, &user.id).await? {
        None => {
            active(&user)?;
            let limit = wpolicy::workspace_limit(user.role());
            let count = ws::membership_count(&ctx.db, &user.id).await?;
            wpolicy::check_their_limit(count, limit).map_err(AppError)?;
            batch(
                &ctx.db,
                join(
                    &ctx.db,
                    &w.id,
                    &user.id,
                    &user.email,
                    role,
                    &maker.id,
                    limit,
                    ctx.now,
                )?,
            )
            .await?;
            if ws::member(&ctx.db, &w.id, &user.id).await?.is_none() {
                // They joined another one in the meantime, or it went.
                let count = ws::membership_count(&ctx.db, &user.id).await?;
                wpolicy::check_their_limit(count, limit).map_err(AppError)?;
                return Err(not_found("Workspace"));
            }
            ctx.audit_in(
                Some(&w.id),
                "workspace.member_added",
                Some(&user.id),
                Some(serde_json::json!({ "email": user.email, "role": role.as_str() })),
            );
        }
        Some(m) => {
            let from = WorkspaceRole::parse(&m.role).unwrap_or(WorkspaceRole::Member);
            // Its primary admin is one of its admins already.
            let same = from == role || (from == WorkspaceRole::Owner && role.is_admin());
            if !same {
                if from == WorkspaceRole::Owner {
                    return Err(conflict(
                        "They are its primary admin: they hand the role to another admin first.",
                    ));
                }
                let changed = q(
                    &ctx.db,
                    "UPDATE workspace_members SET role = ? WHERE workspace_id = ? AND user_id = ? AND role = ?",
                    &[
                        s(role.as_str()),
                        s(&w.id),
                        s(&user.id),
                        s(from.as_str()),
                    ],
                )?
                .run()
                .await?;
                if changed == 0 {
                    return Err(conflict(
                        "Their role there changed in the meantime. Read their account again, then try again.",
                    ));
                }
                if from.is_admin() && !role.is_admin() {
                    crate::api::workspaces::demoted(ctx, &w.id, &user.id).await?;
                }
                ctx.audit_in(
                    Some(&w.id),
                    "workspace.member_role",
                    Some(&user.id),
                    Some(serde_json::json!({ "from": from.as_str(), "to": role.as_str() })),
                );
            }
        }
    }
    ws::changed(ctx, vec![user.id.clone()]);
    http::ok(&read(ctx, &user.id).await?)
}

/// `DELETE /api/v1/admin/users/{user}/workspaces/{workspace}`: takes them
/// out of it as its admins would, with their project memberships there;
/// their sessions there stop. Not its primary admin, who hands the role
/// over first.
pub async fn leave_workspace(ctx: &mut Ctx, who: &str, id_or_slug: &str) -> AppResult<Response> {
    let maker = maker(ctx)?;
    let user = target(ctx, &maker, who).await?;
    let w = workspace(ctx, &maker, id_or_slug).await?;
    let m = ws::member(&ctx.db, &w.id, &user.id)
        .await?
        .ok_or_else(|| not_found("Member"))?;
    if m.role == WorkspaceRole::Owner.as_str() {
        return Err(conflict(
            "They are its primary admin: they hand the role to another admin first.",
        ));
    }
    crate::api::workspaces::remove_from(ctx, &w.id, &user.id).await?;
    ctx.audit_in(
        Some(&w.id),
        "workspace.member_removed",
        Some(&user.id),
        Some(serde_json::json!({ "email": m.email })),
    );
    ws::changed(ctx, vec![user.id.clone()]);
    http::ok(&read(ctx, &user.id).await?)
}

// ───────────────────────────── projects ─────────────────────────────

#[derive(Deserialize)]
struct Project {
    id: String,
    workspace_id: Option<String>,
    workspace_name: Option<String>,
    role: Option<String>,
    #[serde(deserialize_with = "crate::db::int", default)]
    owners: i64,
}

/// A project, by id or slug, with `user_id`'s role as its member and how
/// many owners it has.
async fn project(ctx: &Ctx, id_or_slug: &str, user_id: &str) -> AppResult<(Project, String)> {
    let p: Option<Project> = q(
        &ctx.db,
        "SELECT p.id, p.workspace_id, w.name AS workspace_name, \
         (SELECT role FROM project_members WHERE project_id = p.id AND user_id = ?2) AS role, \
         (SELECT COUNT(*) FROM project_members WHERE project_id = p.id AND role = 'owner') AS owners \
         FROM projects p LEFT JOIN workspaces w ON w.id = p.workspace_id WHERE p.id = ?1 OR p.slug = ?1",
        &[s(id_or_slug), s(user_id)],
    )?
    .first()
    .await?;
    let p = p.ok_or_else(|| not_found("Project"))?;
    let workspace_id = p.workspace_id.clone().ok_or_else(|| not_found("Project"))?;
    Ok((p, workspace_id))
}

/// Refuses taking the last owner a project has as its member from it.
fn keeps_an_owner(p: &Project, to: Option<ProjectRole>) -> AppResult<()> {
    if p.role.as_deref() == Some(ProjectRole::Owner.as_str())
        && to != Some(ProjectRole::Owner)
        && p.owners <= 1
    {
        return Err(conflict("A project needs at least one owner"));
    }
    Ok(())
}

/// `PUT /api/v1/admin/users/{user}/projects/{project}`: makes a member of
/// its workspace a member of the project, with a role, or gives them the
/// role.
pub async fn set_project(
    ctx: &mut Ctx,
    req: &mut Request,
    who: &str,
    id_or_slug: &str,
) -> AppResult<Response> {
    let body: SetRoleRequest = http::body(req).await?;
    let maker = maker(ctx)?;
    let user = target(ctx, &maker, who).await?;
    let (p, workspace_id) = project(ctx, id_or_slug, &user.id).await?;
    if ws::member(&ctx.db, &workspace_id, &user.id)
        .await?
        .is_none()
    {
        return Err(conflict(&format!(
            "They aren't a member of its workspace, {}: add them to it first.",
            p.workspace_name.as_deref().unwrap_or("its workspace")
        )));
    }
    keeps_an_owner(&p, Some(body.role))?;
    let added = q(
        &ctx.db,
        "INSERT INTO project_members (project_id, user_id, role, added_by, created_at) \
         SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM workspace_members WHERE workspace_id = ?6 AND user_id = ?2) \
         ON CONFLICT (project_id, user_id) DO UPDATE SET role = excluded.role",
        &[
            s(&p.id),
            s(&user.id),
            s(body.role.as_str()),
            s(&maker.id),
            n(ctx.now),
            s(&workspace_id),
        ],
    )?
    .run()
    .await?;
    if added == 0 {
        return Err(conflict(
            "They left its workspace in the meantime: add them to it first.",
        ));
    }
    ctx.audit_in(
        Some(&workspace_id),
        "project.member_added",
        Some(&p.id),
        Some(serde_json::json!({ "user": user.id, "role": body.role.as_str() })),
    );
    ws::changed(ctx, vec![user.id.clone()]);
    http::ok(&read(ctx, &user.id).await?)
}

/// `DELETE /api/v1/admin/users/{user}/projects/{project}`: they stop being
/// a member of the project; the workspace's base role still gives them
/// what it gives everyone there.
pub async fn leave_project(ctx: &mut Ctx, who: &str, id_or_slug: &str) -> AppResult<Response> {
    let maker = maker(ctx)?;
    let user = target(ctx, &maker, who).await?;
    let (p, workspace_id) = project(ctx, id_or_slug, &user.id).await?;
    if p.role.is_none() {
        return Err(not_found("Member"));
    }
    keeps_an_owner(&p, None)?;
    q(
        &ctx.db,
        "DELETE FROM project_members WHERE project_id = ? AND user_id = ?",
        &[s(&p.id), s(&user.id)],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(&workspace_id),
        "project.member_removed",
        Some(&p.id),
        Some(serde_json::json!({ "user": user.id })),
    );
    ws::changed(ctx, vec![user.id.clone()]);
    http::ok(&read(ctx, &user.id).await?)
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::db::{Count, batch};
    use crate::testing;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// How many accounts `email` signs in to.
    async fn owners_of(db: &D1Database, email: &str) -> AppResult<i64> {
        let c: Option<Count> = q(
            db,
            "SELECT COUNT(*) AS n FROM user_emails WHERE email = ?",
            &[s(email)],
        )?
        .first()
        .await?;
        Ok(c.map_or(0, |c| c.n))
    }

    const NOW: i64 = 1_800_000_000_000;

    /// Ada, an admin; Bob, a member, who joined the acme workspace as his
    /// primary address and has a commit email for it, an invitation to the
    /// beta workspace and a password reset link waiting.
    async fn world(db: &D1Database) -> AppResult<()> {
        for sql in [
            "INSERT INTO users (id, email, name, role, created_at, updated_at) VALUES \
             ('usr_ada', 'ada@example.com', 'Ada', 'admin', 0, 0), ('usr_bob', 'bob@old.example', 'Bob', 'member', 0, 0)",
            "INSERT INTO user_emails (email, user_id, commit_email, created_at) VALUES \
             ('ada@example.com', 'usr_ada', NULL, 0), ('bob@old.example', 'usr_bob', '1+bob@users.noreply.github.com', 0)",
            "INSERT INTO allowlist (id, pattern, role, note, created_at) VALUES ('alw_bob', 'bob@old.example', 'member', 'Invited', 0)",
            "INSERT INTO workspaces (id, slug, name, created_at, updated_at) VALUES \
             ('wsp_acme', 'acme', 'Acme', 0, 0), ('wsp_beta', 'beta', 'Beta', 0, 0)",
            "INSERT INTO workspace_members (workspace_id, user_id, email, role, created_at) VALUES \
             ('wsp_acme', 'usr_bob', 'bob@old.example', 'member', 0), ('wsp_beta', 'usr_ada', 'ada@example.com', 'owner', 0)",
            "INSERT INTO workspace_invitations (id, workspace_id, email, role, invited_by, created_at, expires_at) VALUES \
             ('wsi_1', 'wsp_beta', 'bob@old.example', 'member', 'usr_ada', 0, 1900000000000)",
            "INSERT INTO password_resets (token_hash, user_id, created_at, expires_at) VALUES ('h', 'usr_bob', 0, 1900000000000)",
        ] {
            q(db, sql, &[])?.run().await?;
        }
        Ok(())
    }

    #[derive(Deserialize)]
    struct Text {
        v: Option<String>,
    }

    async fn text(db: &D1Database, sql: &str) -> AppResult<Option<String>> {
        let r: Option<Text> = q(db, sql, &[])?.first().await?;
        Ok(r.and_then(|r| r.v))
    }

    async fn count(db: &D1Database, sql: &str) -> AppResult<i64> {
        let c: Option<Count> = q(db, sql, &[])?.first().await?;
        Ok(c.map_or(0, |c| c.n))
    }

    #[wasm_bindgen_test]
    async fn a_new_address_takes_the_old_ones_place_everywhere() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        let statements = change_email(
            &db,
            "usr_bob",
            "bob@old.example",
            "bob@new.example",
            "usr_ada",
            NOW,
        )?;
        batch(&db, statements).await?;
        assert_eq!(
            text(&db, "SELECT email AS v FROM users WHERE id = 'usr_bob'").await?,
            Some("bob@new.example".into())
        );
        // The new address signs in, the old one doesn't.
        assert_eq!(owners_of(&db, "bob@new.example").await?, 1);
        assert_eq!(owners_of(&db, "bob@old.example").await?, 0);
        // It credits his commits as the old one did.
        assert_eq!(
            text(
                &db,
                "SELECT commit_email AS v FROM user_emails WHERE email = 'bob@new.example'"
            )
            .await?,
            Some("1+bob@users.noreply.github.com".into())
        );
        // Acme knows him by it.
        assert_eq!(
            text(
                &db,
                "SELECT email AS v FROM workspace_members WHERE workspace_id = 'wsp_acme' AND user_id = 'usr_bob'"
            )
            .await?,
            Some("bob@new.example".into())
        );
        // The allowlist admits the new one, and lets the old one go with
        // its invitation.
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM allowlist WHERE pattern = 'bob@new.example' AND note = 'Admin token'"
            )
            .await?,
            1
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM allowlist WHERE pattern = 'bob@old.example'"
            )
            .await?,
            0
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM workspace_invitations WHERE email = 'bob@old.example'"
            )
            .await?,
            0
        );
        // A reset link that may have gone to the old one works no more.
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM password_resets WHERE user_id = 'usr_bob' AND used_at IS NULL"
            )
            .await?,
            0
        );
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn an_address_of_someone_elses_changes_nothing() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        // Taken between the check and the change: Ada's.
        let statements = change_email(
            &db,
            "usr_bob",
            "bob@old.example",
            "ada@example.com",
            "usr_ada",
            NOW,
        )?;
        batch(&db, statements).await?;
        assert_eq!(
            text(&db, "SELECT email AS v FROM users WHERE id = 'usr_bob'").await?,
            Some("bob@old.example".into())
        );
        assert_eq!(owners_of(&db, "bob@old.example").await?, 1);
        assert_eq!(
            text(
                &db,
                "SELECT user_id AS v FROM user_emails WHERE email = 'ada@example.com'"
            )
            .await?,
            Some("usr_ada".into())
        );
        // Nor does it touch the allowlist, his workspaces or reset links.
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM allowlist WHERE pattern = 'ada@example.com'"
            )
            .await?,
            0
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM allowlist WHERE pattern = 'bob@old.example'"
            )
            .await?,
            1
        );
        assert_eq!(
            text(
                &db,
                "SELECT email AS v FROM workspace_members WHERE workspace_id = 'wsp_acme' AND user_id = 'usr_bob'"
            )
            .await?,
            Some("bob@old.example".into())
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM password_resets WHERE user_id = 'usr_bob' AND used_at IS NULL"
            )
            .await?,
            1
        );
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn joining_holds_to_the_limit_and_uses_up_invitations() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        // Bob is in one workspace; with a limit of one he joins no other.
        batch(
            &db,
            join(
                &db,
                "wsp_beta",
                "usr_bob",
                "bob@old.example",
                WorkspaceRole::Member,
                "usr_ada",
                Some(1),
                NOW,
            )?,
        )
        .await?;
        assert!(ws::member(&db, "wsp_beta", "usr_bob").await?.is_none());
        // Within it, he joins as an admin, and his invitation there goes.
        batch(
            &db,
            join(
                &db,
                "wsp_beta",
                "usr_bob",
                "bob@old.example",
                WorkspaceRole::Admin,
                "usr_ada",
                Some(5),
                NOW,
            )?,
        )
        .await?;
        let m = ws::member(&db, "wsp_beta", "usr_bob").await?;
        assert_eq!(m.map(|m| m.role), Some("admin".to_owned()));
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM workspace_invitations WHERE email = 'bob@old.example'"
            )
            .await?,
            0
        );
        Ok(())
    }
}
