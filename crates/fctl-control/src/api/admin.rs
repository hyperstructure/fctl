//! `/api/v1/admin/*`: users, the allowlist, invitations, the audit log.

use crate::auth;
use crate::ctx::Ctx;
use crate::db::{n, os, q, s};
use crate::error::{AppResult, conflict, forbidden, not_found, validation};
use crate::http;
use crate::model::{AllowRow, UserRow, user_columns};
use fctl_domain::allowlist::AllowPattern;
use fctl_domain::perm;
use fctl_domain::tokens::{SecretKind, hash_secret, new_secret};
use fctl_domain::validate;
use fctl_proto::api::*;
use fctl_proto::session::{InstanceRole, UserRef};
use fctl_proto::{InviteId, UserId};
use worker::{Request, Response};

const INVITE_TTL_MS: i64 = 14 * auth::DAY_MS;

pub async fn users(ctx: &Ctx) -> AppResult<Response> {
    ctx.admin()?;
    let rows: Vec<UserRow> = q(
        &ctx.db,
        &format!(
            "SELECT {} FROM users u ORDER BY u.created_at",
            user_columns("u")
        ),
        &[],
    )?
    .all()
    .await?;
    http::ok(&UserList {
        users: rows.iter().map(UserRow::to_user).collect(),
    })
}

pub async fn update_user(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdateUserRequest = http::body(req).await?;
    let actor = ctx.admin()?.clone();
    let target = auth::user_by_id(ctx, id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    if let Some(role) = body.role
        && role != target.role() {
            if !perm::can_assign_instance_role(actor.role(), target.role(), role) {
                return Err(forbidden("You cannot give this role"));
            }
            if target.role() == InstanceRole::Owner && role != InstanceRole::Owner {
                let owners: Vec<crate::db::Count> = q(
                    &ctx.db,
                    "SELECT COUNT(*) AS n FROM users WHERE role = 'owner' AND status = 'active'",
                    &[],
                )?
                .all()
                .await?;
                if owners.first().map_or(0, |c| c.n) <= 1 {
                    return Err(conflict("The instance needs at least one owner"));
                }
            }
        }
    if let Some(status) = body.status {
        if target.id == actor.id && status == UserStatus::Disabled {
            return Err(conflict("You cannot disable yourself"));
        }
        if target.role() == InstanceRole::Owner && actor.role() != InstanceRole::Owner {
            return Err(forbidden("Only owners can disable owners"));
        }
    }
    let name = body
        .name
        .as_deref()
        .map(|v| validate::name("name", v))
        .transpose()?;
    q(
        &ctx.db,
        "UPDATE users SET role = COALESCE(?, role), status = COALESCE(?, status), name = COALESCE(?, name), updated_at = ? WHERE id = ?",
        &[
            os(body.role.map(|r| r.as_str())),
            os(body.status.map(|st| st.as_str())),
            os(name.as_deref()),
            n(ctx.now),
            s(&target.id),
        ],
    )?
    .run()
    .await?;
    if body.status == Some(UserStatus::Disabled) {
        auth::revoke_tokens(ctx, &target.id, None).await?;
    }
    ctx.audit(
        "user.updated",
        Some(&target.id),
        Some(serde_json::json!({ "role": body.role.map(|r| r.as_str()), "status": body.status.map(|s| s.as_str()) })),
    );
    let updated = auth::user_by_id(ctx, &target.id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::ok(&updated.to_user())
}

pub async fn delete_user(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let actor = ctx.admin()?.clone();
    let target = auth::user_by_id(ctx, id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    if target.id == actor.id {
        return Err(conflict("You cannot delete yourself"));
    }
    if target.role() == InstanceRole::Owner {
        return Err(forbidden(
            "Owners cannot be deleted; make them a member first",
        ));
    }
    // Sessions and projects they created stay; their memberships and tokens go.
    let created: Vec<crate::db::Count> = q(
        &ctx.db,
        "SELECT (SELECT COUNT(*) FROM sessions WHERE created_by = ?1) + (SELECT COUNT(*) FROM projects WHERE created_by = ?1) AS n",
        &[s(&target.id)],
    )?
    .all()
    .await?;
    if created.first().map_or(0, |c| c.n) > 0 {
        // Keep the account for attribution, but lock it.
        q(&ctx.db, "UPDATE users SET status = 'disabled', password_hash = NULL, updated_at = ? WHERE id = ?", &[n(ctx.now), s(&target.id)])?
            .run()
            .await?;
        auth::revoke_tokens(ctx, &target.id, None).await?;
    } else {
        q(&ctx.db, "DELETE FROM users WHERE id = ?", &[s(&target.id)])?
            .run()
            .await?;
    }
    q(
        &ctx.db,
        "DELETE FROM allowlist WHERE pattern = ?",
        &[s(&target.email)],
    )?
    .run()
    .await?;
    ctx.audit(
        "user.deleted",
        Some(&target.id),
        Some(serde_json::json!({ "email": target.email })),
    );
    http::ok(&Ok { ok: true })
}

/// A password reset link an admin can hand over (when email is not set up).
pub async fn reset_link(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let actor = ctx.admin()?.clone();
    let target = auth::user_by_id(ctx, id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    if target.role() == InstanceRole::Owner && actor.role() != InstanceRole::Owner {
        return Err(forbidden("Only owners can reset an owner's password"));
    }
    let url = crate::api::auth::create_reset_link(ctx, &target.id, Some(&actor.id)).await?;
    ctx.audit("user.reset_link", Some(&target.id), None);
    http::ok(&serde_json::json!({ "url": url, "expires_in": 3600 }))
}

// ───────────────────────────── allowlist ─────────────────────────────

const ALLOW_SELECT: &str = "SELECT a.id, a.pattern, a.role, a.note, a.created_at, a.created_by AS created_by_id, \
    cu.name AS created_by_name, u.id AS user_id FROM allowlist a \
    LEFT JOIN users cu ON cu.id = a.created_by LEFT JOIN users u ON u.email = a.pattern";

pub async fn allowlist(ctx: &Ctx) -> AppResult<Response> {
    ctx.admin()?;
    let rows: Vec<AllowRow> = q(
        &ctx.db,
        &format!("{ALLOW_SELECT} ORDER BY a.created_at"),
        &[],
    )?
    .all()
    .await?;
    http::ok(&AllowList {
        entries: rows.iter().map(AllowRow::to_entry).collect(),
    })
}

pub async fn add_allow(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: CreateAllowRequest = http::body(req).await?;
    let actor = ctx.admin()?.clone();
    let pattern = AllowPattern::parse(&body.pattern)?;
    let role = match body.role.unwrap_or(InstanceRole::Member) {
        InstanceRole::Owner => {
            return Err(validation("role", "Owners are made from existing members"));
        }
        InstanceRole::Admin if actor.role() != InstanceRole::Owner => {
            return Err(forbidden("Only owners can allow new admins"));
        }
        r => r,
    };
    let note = body
        .note
        .as_deref()
        .map(|v| validate::text("note", v, 200))
        .transpose()?
        .filter(|v| !v.is_empty());
    let canonical = pattern.canonical();
    let id = fctl_domain::ids::allow_id(ctx.now);
    let inserted = q(
        &ctx.db,
        "INSERT OR IGNORE INTO allowlist (id, pattern, role, note, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        &[s(id.as_str()), s(&canonical), s(role.as_str()), os(note.as_deref()), s(&actor.id), n(ctx.now)],
    )?
    .run()
    .await?;
    if inserted == 0 {
        return Err(conflict("This email or domain is already allowed"));
    }
    // Re-enable an existing account that was disabled by an earlier removal.
    if let AllowPattern::Email(email) = &pattern {
        q(&ctx.db, "UPDATE users SET status = 'active', updated_at = ? WHERE email = ? AND status = 'disabled'", &[n(ctx.now), s(email)])?
            .run()
            .await?;
    }
    let invite = match (&pattern, body.invite.unwrap_or(true)) {
        (AllowPattern::Email(email), true) if auth::user_by_email(ctx, email).await?.is_none() => {
            Some(create_invite_for(ctx, email, role).await?)
        }
        _ => None,
    };
    ctx.audit(
        "allowlist.added",
        Some(&canonical),
        Some(serde_json::json!({ "role": role.as_str() })),
    );
    let row: Option<AllowRow> = q(
        &ctx.db,
        &format!("{ALLOW_SELECT} WHERE a.id = ?"),
        &[s(id.as_str())],
    )?
    .first()
    .await?;
    let entry = row.ok_or_else(|| not_found("Allowlist entry"))?.to_entry();
    http::created(&CreatedAllow { entry, invite })
}

/// Removing an entry disables the accounts it alone admitted.
pub async fn remove_allow(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let actor = ctx.admin()?.clone();
    let row: Option<AllowRow> = q(&ctx.db, &format!("{ALLOW_SELECT} WHERE a.id = ?"), &[s(id)])?
        .first()
        .await?;
    let row = row.ok_or_else(|| not_found("Allowlist entry"))?;
    if row.pattern == actor.email {
        return Err(conflict("You cannot remove your own access"));
    }
    q(&ctx.db, "DELETE FROM allowlist WHERE id = ?", &[s(id)])?
        .run()
        .await?;
    // Every non-owner user no longer matching any entry is disabled.
    let remaining: Vec<AllowRow> = q(
        &ctx.db,
        &format!("{ALLOW_SELECT} ORDER BY a.created_at"),
        &[],
    )?
    .all()
    .await?;
    let users: Vec<UserRow> = q(
        &ctx.db,
        &format!(
            "SELECT {} FROM users u WHERE u.status = 'active' AND u.role != 'owner'",
            user_columns("u")
        ),
        &[],
    )?
    .all()
    .await?;
    let mut disabled = Vec::new();
    for u in users {
        let still =
            fctl_domain::allowlist::best_match(&remaining, &u.email, |r| r.pattern.as_str())
                .is_some();
        if !still {
            q(
                &ctx.db,
                "UPDATE users SET status = 'disabled', updated_at = ? WHERE id = ?",
                &[n(ctx.now), s(&u.id)],
            )?
            .run()
            .await?;
            auth::revoke_tokens(ctx, &u.id, None).await?;
            disabled.push(u.email);
        }
    }
    ctx.audit(
        "allowlist.removed",
        Some(&row.pattern),
        Some(serde_json::json!({ "disabled": disabled })),
    );
    http::ok(&serde_json::json!({ "ok": true, "disabled": disabled }))
}

async fn create_invite_for(ctx: &Ctx, email: &str, role: InstanceRole) -> AppResult<Invite> {
    let secret = new_secret(SecretKind::Invite);
    let id = fctl_domain::ids::invite_id(ctx.now);
    let expires_at = ctx.now + INVITE_TTL_MS;
    q(
        &ctx.db,
        "INSERT INTO invites (id, token_hash, email, role, created_by, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        &[
            s(id.as_str()),
            s(&hash_secret(&secret)),
            s(email),
            s(if role == InstanceRole::Admin { "admin" } else { "member" }),
            os(ctx.auth.as_ref().map(|a| a.user.id.as_str())),
            n(ctx.now),
            n(expires_at),
        ],
    )?
    .run()
    .await?;
    let url = ctx.cfg.url(&format!("/invite#token={secret}"));
    let mut emailed = false;
    if let Some(mail) = ctx.cfg.email.clone() {
        let inviter = ctx
            .auth
            .as_ref()
            .map(|a| a.user.name.clone())
            .unwrap_or_else(|| "An admin".into());
        let text = format!(
            "{inviter} invited you to {}, a software factory where Claude agents build software.\n\nCreate your account: {url}\n\nThe link works for 14 days.",
            ctx.cfg.instance_name
        );
        emailed = crate::email::send(
            &mail,
            email,
            &format!("You're invited to {}", ctx.cfg.instance_name),
            &text,
        )
        .await
        .is_ok();
    }
    Ok(Invite {
        id,
        email: email.to_owned(),
        url: Some(url),
        role,
        expires_at,
        emailed,
    })
}

pub async fn create_invite(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: CreateInviteRequest = http::body(req).await?;
    let actor = ctx.admin()?.clone();
    let email = validate::email(&body.email)?;
    let role = body.role.unwrap_or(InstanceRole::Member);
    if role == InstanceRole::Owner
        || (role == InstanceRole::Admin && actor.role() != InstanceRole::Owner)
    {
        return Err(forbidden("You cannot invite with this role"));
    }
    if auth::user_by_email(ctx, &email).await?.is_some() {
        return Err(conflict("This person already has an account"));
    }
    // Inviting someone also allows them.
    q(
        &ctx.db,
        "INSERT OR IGNORE INTO allowlist (id, pattern, role, note, created_by, created_at) VALUES (?, ?, ?, 'Invited', ?, ?)",
        &[s(fctl_domain::ids::allow_id(ctx.now).as_str()), s(&email), s(role.as_str()), s(&actor.id), n(ctx.now)],
    )?
    .run()
    .await?;
    let invite = create_invite_for(ctx, &email, role).await?;
    ctx.audit("invite.created", Some(&email), None);
    http::created(&invite)
}

// ───────────────────────────── audit ─────────────────────────────

#[derive(serde::Deserialize)]
struct AuditRow {
    id: String,
    #[serde(deserialize_with = "crate::db::int")]
    ts: i64,
    actor_id: Option<String>,
    actor_name: Option<String>,
    action: String,
    target: Option<String>,
    data: Option<String>,
    ip: Option<String>,
}

pub async fn audit(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    ctx.admin()?;
    let before = http::query(req, "before")
        .and_then(|b| b.parse::<i64>().ok())
        .unwrap_or(i64::MAX / 2);
    let rows: Vec<AuditRow> = q(
        &ctx.db,
        "SELECT a.id, a.ts, a.actor_id, u.name AS actor_name, a.action, a.target, a.data, a.ip FROM audit_log a \
         LEFT JOIN users u ON u.id = a.actor_id WHERE a.ts < ? ORDER BY a.ts DESC LIMIT 101",
        &[n(before)],
    )?
    .all()
    .await?;
    let more = rows.len() > 100;
    let entries: Vec<AuditEntry> = rows
        .into_iter()
        .take(100)
        .map(|r| AuditEntry {
            id: r.id,
            ts: r.ts,
            actor: match (r.actor_id, r.actor_name) {
                (Some(id), Some(name)) => Some(UserRef {
                    id: UserId(id),
                    name,
                    avatar_url: None,
                }),
                _ => None,
            },
            action: r.action,
            target: r.target,
            data: r.data.and_then(|d| serde_json::from_str(&d).ok()),
            ip: r.ip,
        })
        .collect();
    let next_before = if more {
        entries.last().map(|e| e.ts)
    } else {
        None
    };
    http::ok(&AuditList {
        entries,
        next_before,
    })
}

/// Unused invite ids are listed for revocation.
pub async fn revoke_invite(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    ctx.admin()?;
    let changed = q(
        &ctx.db,
        "DELETE FROM invites WHERE id = ? AND accepted_at IS NULL",
        &[s(id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(not_found("Invitation"));
    }
    let _ = InviteId(id.to_owned());
    http::ok(&Ok { ok: true })
}
