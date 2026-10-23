//! `/api/v1/admin/*`: people, the allowlist, invitations, workspaces, the
//! primary instance owner's role, the audit log, and the instance's
//! settings. GitHub connections are in `api::github`.

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
use fctl_domain::verify::{self, Sensitive};
use fctl_proto::api::*;
use fctl_proto::session::{InstanceRole, UserRef, WorkspaceRef};
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
        && role != target.role()
    {
        if target.role() == InstanceRole::Owner {
            return Err(forbidden(
                "Nobody can change the primary instance owner's role. They can hand it to an admin.",
            ));
        }
        if role == InstanceRole::Owner {
            return Err(validation(
                "role",
                "The instance has one primary owner, who hands the role over",
            ));
        }
        if !perm::can_assign_instance_role(actor.role(), target.role(), role) {
            return Err(forbidden(if role == InstanceRole::Admin {
                "Only the primary instance owner makes someone an admin"
            } else {
                "You cannot give this role"
            }));
        }
    }
    if let Some(status) = body.status {
        if target.id == actor.id && status == UserStatus::Disabled {
            return Err(conflict("You cannot disable yourself"));
        }
        if target.role() == InstanceRole::Owner {
            return Err(forbidden("Nobody can disable the primary instance owner"));
        }
        if target.role() == InstanceRole::Admin && actor.role() != InstanceRole::Owner {
            return Err(forbidden(
                "Only the primary instance owner can disable admins",
            ));
        }
    }
    // Names as roles and statuses: the primary instance owner's is theirs,
    // and only they rename admins, so that nobody below them can make
    // another admin look like someone else.
    if body.name.is_some() && target.id != actor.id {
        if target.role() == InstanceRole::Owner {
            return Err(forbidden(
                "Nobody renames the primary instance owner. They change their name in Settings.",
            ));
        }
        if target.role() == InstanceRole::Admin && actor.role() != InstanceRole::Owner {
            return Err(forbidden(
                "Only the primary instance owner can rename admins",
            ));
        }
    }
    let name = body
        .name
        .as_deref()
        .map(|v| validate::name("name", v))
        .transpose()?;
    if body
        .role
        .is_some_and(|role| verify::makes_admin(target.role(), role))
    {
        ctx.verified(Sensitive::MakeAdmin)?;
    }
    // Not if they became the primary instance owner in the meantime.
    let changed = q(
        &ctx.db,
        "UPDATE users SET role = COALESCE(?1, role), status = COALESCE(?2, status), name = COALESCE(?3, name), updated_at = ?4 \
         WHERE id = ?5 AND (role != 'owner' OR (?1 IS NULL AND ?2 IS NULL AND (?3 IS NULL OR id = ?6)))",
        &[
            os(body.role.map(|r| r.as_str())),
            os(body.status.map(|st| st.as_str())),
            os(name.as_deref()),
            n(ctx.now),
            s(&target.id),
            s(&actor.id),
        ],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(forbidden(
            "Nobody can change the primary instance owner's role. They can hand it to an admin.",
        ));
    }
    if body.status == Some(UserStatus::Disabled) {
        auth::revoke_tokens(ctx, &target.id, None).await?;
        crate::api::workspaces::stop_sessions_on(ctx, &target.id).await?;
    }
    if body.status.is_some() {
        crate::api::workspaces::owner_runners_changed(ctx, &target.id).await?;
    }
    if target.role() == InstanceRole::Admin && body.role == Some(InstanceRole::Member) {
        demote_allowed(ctx, &target.id).await?;
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

/// The allowlist entries that admit an admin's addresses as an admin, for
/// those addresses alone, admit them as a member from now on: they were
/// made an admin or removed, and an entry that still said admin would make
/// whoever signs up with one of them (with Google, which needs no
/// invitation) an admin again.
async fn demote_allowed(ctx: &Ctx, user_id: &str) -> AppResult<()> {
    q(
        &ctx.db,
        "UPDATE allowlist SET role = 'member' WHERE role = 'admin' AND pattern IN \
         (SELECT email FROM user_emails WHERE user_id = ?1 UNION SELECT email FROM users WHERE id = ?1)",
        &[s(user_id)],
    )?
    .run()
    .await?;
    Ok(())
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
            "The primary instance owner can't be deleted. They can hand the role to an admin first.",
        ));
    }
    if target.role() == InstanceRole::Admin && actor.role() != InstanceRole::Owner {
        return Err(forbidden(
            "Only the primary instance owner can delete admins",
        ));
    }
    // The same as deleting your own account (api::account): their
    // workspaces with no other members go, what others use stays, without
    // their name.
    let plan = crate::api::account::plan(ctx, &target).await?;
    if plan.running() > 0 {
        return Err(conflict("Stop their running sessions first"));
    }
    ctx.verified(Sensitive::DeleteUser)?;
    if target.role() == InstanceRole::Admin {
        demote_allowed(ctx, &target.id).await?;
    }
    crate::api::account::erase(ctx, &target, &plan).await?;
    ctx.audit(
        "user.deleted",
        Some(&target.id),
        Some(serde_json::json!({ "email": target.email })),
    );
    http::ok(&Ok { ok: true })
}

/// A link that resets someone's password: emailed to them where the
/// instance sends email, else for the admin to hand over.
pub async fn reset_link(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let actor = ctx.admin()?.clone();
    let target = auth::user_by_id(ctx, id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    if target.role() != InstanceRole::Member && actor.role() != InstanceRole::Owner {
        return Err(forbidden(
            "Only the primary instance owner can reset an admin's password",
        ));
    }
    let url = crate::api::auth::create_reset_link(ctx, &target.id, Some(&actor.id)).await?;
    ctx.audit("user.reset_link", Some(&target.id), None);
    let instance = crate::instance::name(&ctx.db, &ctx.cfg).await?;
    // Whoever has the link signs in as them, into their workspaces: where
    // the instance sends email, it goes to them alone, never to the admin.
    if let Some(mail) = ctx.cfg.email.clone() {
        crate::email::send(
            &mail,
            &target.email,
            "Reset your password",
            &format!(
                "{} sent you a link that sets a new password for your {instance} account ({}). It works for an hour:\n\n{url}\n\nIf you didn't ask for it, ignore this email and tell another admin of the instance.",
                actor.name, target.email
            ),
        )
        .await?;
        return http::ok(&serde_json::json!({ "emailed": true, "expires_in": 3600 }));
    }
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
            return Err(validation(
                "role",
                "The primary instance owner hands their role over to an admin",
            ));
        }
        InstanceRole::Admin if actor.role() != InstanceRole::Owner => {
            return Err(forbidden(
                "Only the primary instance owner can allow new admins",
            ));
        }
        r => r,
    };
    let note = body
        .note
        .as_deref()
        .map(|v| validate::text("note", v, 200))
        .transpose()?
        .filter(|v| !v.is_empty());
    // Whoever has an address it admits as an admin signs up as one.
    if role == InstanceRole::Admin {
        ctx.verified(Sensitive::InviteAdmin)?;
    }
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
    // Re-enable an existing account that was disabled by an earlier removal:
    // an admin's only for the primary instance owner, who alone enables and
    // disables admins.
    if let AllowPattern::Email(email) = &pattern {
        q(
            &ctx.db,
            "UPDATE users SET status = 'active', updated_at = ? WHERE email = ? AND status = 'disabled' \
             AND (role = 'member' OR ?)",
            &[
                n(ctx.now),
                s(email),
                crate::db::b(actor.role() == InstanceRole::Owner),
            ],
        )?
        .run()
        .await?;
    }
    let invite = match (&pattern, body.invite.unwrap_or(true)) {
        (AllowPattern::Email(email), true) if auth::user_by_email(ctx, email).await?.is_none() => {
            Some(create_invite_for(ctx, email, role, Vec::new()).await?)
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
    // An account none of whose addresses an entry admits any more is
    // disabled (never the primary instance owner's, which may predate the
    // allowlist). Of the others, the addresses no entry admits are
    // disconnected, and the primary address moves to one that still is.
    let remaining: Vec<AllowRow> = q(
        &ctx.db,
        &format!("{ALLOW_SELECT} WHERE a.id != ? ORDER BY a.created_at"),
        &[s(id)],
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
    #[derive(serde::Deserialize)]
    struct Address {
        user_id: String,
        email: String,
    }
    let addresses: Vec<Address> = q(
        &ctx.db,
        "SELECT e.user_id, e.email FROM user_emails e JOIN users u ON u.id = e.user_id \
         WHERE u.status = 'active' AND u.role != 'owner' ORDER BY e.created_at",
        &[],
    )?
    .all()
    .await?;
    let admitted = |email: &str| {
        fctl_domain::allowlist::best_match(&remaining, email, |r| r.pattern.as_str()).is_some()
    };
    // Only the primary instance owner disables admins, or takes their
    // addresses (`update_user`).
    if actor.role() != InstanceRole::Owner {
        let admins: Vec<&str> = users
            .iter()
            .filter(|u| u.is_admin())
            .filter(|u| {
                std::iter::once(u.email.as_str())
                    .chain(
                        addresses
                            .iter()
                            .filter(|a| a.user_id == u.id)
                            .map(|a| a.email.as_str()),
                    )
                    .any(|e| !admitted(e))
            })
            .map(|u| u.email.as_str())
            .collect();
        if !admins.is_empty() {
            return Err(forbidden(&format!(
                "Removing this entry would take access from admins ({}). Only the primary instance owner can.",
                admins.join(", ")
            )));
        }
    }
    q(&ctx.db, "DELETE FROM allowlist WHERE id = ?", &[s(id)])?
        .run()
        .await?;
    let mut disabled = Vec::new();
    let mut disconnected = Vec::new();
    let mut changed = Vec::new();
    for u in users {
        let mut theirs: Vec<&str> = addresses
            .iter()
            .filter(|a| a.user_id == u.id)
            .map(|a| a.email.as_str())
            .collect();
        if !theirs.contains(&u.email.as_str()) {
            theirs.insert(0, &u.email);
        }
        let (kept, gone): (Vec<&str>, Vec<&str>) = theirs.into_iter().partition(|e| admitted(e));
        let Some(first) = kept.first().copied() else {
            q(
                &ctx.db,
                "UPDATE users SET status = 'disabled', updated_at = ? WHERE id = ?",
                &[n(ctx.now), s(&u.id)],
            )?
            .run()
            .await?;
            auth::revoke_tokens(ctx, &u.id, None).await?;
            crate::api::workspaces::stop_sessions_on(ctx, &u.id).await?;
            crate::api::workspaces::owner_runners_changed(ctx, &u.id).await?;
            disabled.push(u.email.clone());
            continue;
        };
        if gone.is_empty() {
            continue;
        }
        let primary = if gone.contains(&u.email.as_str()) {
            q(
                &ctx.db,
                "UPDATE users SET email = ?, updated_at = ? WHERE id = ?",
                &[s(first), n(ctx.now), s(&u.id)],
            )?
            .run()
            .await?;
            first
        } else {
            u.email.as_str()
        };
        for email in gone {
            changed.extend(crate::api::emails::disallow(ctx, &u.id, email, primary).await?);
            disconnected.push(email.to_owned());
        }
    }
    ctx.audit(
        "allowlist.removed",
        Some(&row.pattern),
        Some(serde_json::json!({ "disabled": disabled, "disconnected": disconnected })),
    );
    crate::workspaces::changed(ctx, changed);
    http::ok(&serde_json::json!({ "ok": true, "disabled": disabled, "disconnected": disconnected }))
}

async fn create_invite_for(
    ctx: &Ctx,
    email: &str,
    role: InstanceRole,
    workspaces: Vec<WorkspaceRef>,
) -> AppResult<Invite> {
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
    // An address that asked for early access has its answer.
    crate::api::early_access::invited(ctx, email).await?;
    let url = ctx.cfg.url(&format!("/invite#token={secret}"));
    let mut emailed = false;
    if let Some(mail) = ctx.cfg.email.clone() {
        let inviter = ctx
            .auth
            .as_ref()
            .map(|a| a.user.name.clone())
            .unwrap_or_else(|| "An admin".into());
        let instance = crate::instance::name(&ctx.db, &ctx.cfg).await?;
        let joins = match workspaces.as_slice() {
            [] => String::new(),
            [w] => format!(" You join {} with it.", w.name),
            more => format!(
                " You join {} with it.",
                more.iter()
                    .map(|w| w.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let text = format!(
            "{inviter} invited you to {instance}, a software factory where Claude agents build software.{joins}\n\n\
             Create your account, or connect this address to the account you have: {url}\n\nThe link works for 14 days."
        );
        emailed = crate::email::send(
            &mail,
            email,
            &format!("You're invited to {instance}"),
            &text,
        )
        .await
        .is_ok();
    }
    Ok(Invite {
        id,
        email: email.to_owned(),
        // Where the instance sends email, the link goes to the address
        // alone: whoever holds it creates the account, or connects the
        // address to theirs, and joins the workspaces that invited it, so
        // an admin who kept it could join workspaces they aren't in. When
        // the email didn't go out, they invite the address again.
        url: ctx.cfg.email.is_none().then_some(url),
        role,
        expires_at,
        emailed,
        workspaces,
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
        return Err(conflict(
            "This address belongs to an account already. Invite it to workspaces from their settings.",
        ));
    }
    // The workspaces to invite them to as well: ones the inviter is an
    // admin of.
    let mut workspaces = Vec::new();
    for w in body.workspaces.unwrap_or_default() {
        let a = crate::workspaces::load(ctx, &w.workspace).await?;
        a.require_admin().map_err(|_| {
            forbidden(&format!(
                "You can invite people to workspaces you are an admin of; you aren't one of {}",
                a.row.name
            ))
        })?;
        workspaces.push((a.row, fctl_domain::workspace::invitation_role(w.role)?));
    }
    if role == InstanceRole::Admin {
        ctx.verified(Sensitive::InviteAdmin)?;
    } else if workspaces.iter().any(|(_, role)| role.is_admin()) {
        ctx.verified(Sensitive::MakeWorkspaceAdmin)?;
    }
    // Inviting someone also allows them.
    q(
        &ctx.db,
        "INSERT OR IGNORE INTO allowlist (id, pattern, role, note, created_by, created_at) VALUES (?, ?, ?, 'Invited', ?, ?)",
        &[s(fctl_domain::ids::allow_id(ctx.now).as_str()), s(&email), s(role.as_str()), s(&actor.id), n(ctx.now)],
    )?
    .run()
    .await?;
    let mut refs = Vec::new();
    for (row, role) in &workspaces {
        // The instance invitation's email says it.
        crate::api::workspaces::invite(ctx, row, &email, *role, false, true).await?;
        refs.push(row.to_ref());
    }
    let invite = create_invite_for(ctx, &email, role, refs).await?;
    ctx.audit(
        "invite.created",
        Some(&email),
        Some(serde_json::json!({
            "workspaces": workspaces.iter().map(|(w, _)| w.id.as_str()).collect::<Vec<_>>(),
        })),
    );
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

/// `GET /api/v1/admin/audit`: the instance's audit log (what happened in
/// workspaces is in theirs).
pub async fn audit(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    ctx.admin()?;
    audit_page(ctx, req, None).await
}

/// A page of the audit log of a workspace, or of the instance (`None`).
pub async fn audit_page(ctx: &Ctx, req: &Request, workspace: Option<&str>) -> AppResult<Response> {
    let before = http::query(req, "before")
        .and_then(|b| b.parse::<i64>().ok())
        .unwrap_or(i64::MAX / 2);
    let select = "SELECT a.id, a.ts, a.actor_id, u.name AS actor_name, a.action, a.target, a.data, a.ip \
        FROM audit_log a LEFT JOIN users u ON u.id = a.actor_id";
    let rows: Vec<AuditRow> = match workspace {
        Some(w) => q(
            &ctx.db,
            &format!("{select} WHERE a.workspace_id = ? AND a.ts < ? ORDER BY a.ts DESC LIMIT 101"),
            &[s(w), n(before)],
        )?,
        None => q(
            &ctx.db,
            &format!(
                "{select} WHERE a.ts < ? AND a.workspace_id IS NULL ORDER BY a.ts DESC LIMIT 101"
            ),
            &[n(before)],
        )?,
    }
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
    #[derive(serde::Deserialize)]
    struct Revoked {
        email: String,
    }
    let revoked: Option<Revoked> = q(
        &ctx.db,
        "DELETE FROM invites WHERE id = ? AND accepted_at IS NULL RETURNING email",
        &[s(id)],
    )?
    .first()
    .await?;
    let Some(revoked) = revoked else {
        return Err(not_found("Invitation"));
    };
    // Inviting the address allowed it too (`create_invite`): that goes with
    // the invitation, unless the address has an account or another one, or
    // an admin allowed it on its own. Otherwise a Google sign-up, which
    // needs no invitation, would still let its holder in.
    q(
        &ctx.db,
        "DELETE FROM allowlist WHERE pattern = ?1 AND note = 'Invited' \
         AND NOT EXISTS (SELECT 1 FROM users WHERE email = ?1) \
         AND NOT EXISTS (SELECT 1 FROM user_emails WHERE email = ?1) \
         AND NOT EXISTS (SELECT 1 FROM invites WHERE email = ?1 AND accepted_at IS NULL AND expires_at > ?2)",
        &[s(&revoked.email), n(ctx.now)],
    )?
    .run()
    .await?;
    ctx.audit("invite.revoked", Some(&revoked.email), None);
    let _ = InviteId(id.to_owned());
    http::ok(&Ok { ok: true })
}

/// `GET /api/v1/admin/settings`.
pub async fn settings(ctx: &Ctx) -> AppResult<Response> {
    ctx.admin()?;
    http::ok(&crate::instance::settings(&ctx.db).await?)
}

/// `PATCH /api/v1/admin/settings`. Only the primary instance owner turns
/// confirming sensitive actions on or off, and turning off what makes
/// people prove it's them takes confirming it's you
/// (`fctl_domain::verify::instance_settings_change`).
pub async fn update_settings(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: UpdateInstanceSettingsRequest = http::body(req).await?;
    let actor = ctx.admin()?.clone();
    let mut settings = crate::instance::settings(&ctx.db).await?;
    if let Some(on) = body.verify_sensitive_actions {
        verify::check_verify_setting(actor.role(), settings.verify_sensitive_actions, on)
            .map_err(crate::error::AppError)?;
    }
    if let Some(action) = verify::instance_settings_change(&settings, &body) {
        ctx.verified(action)?;
    }
    if let Some(on) = body.publish_screenshots {
        settings.publish_screenshots = on;
    }
    if let Some(on) = body.require_two_factor {
        // Without the master key nobody could set two-factor up, and
        // everyone it applies to would be stuck.
        if on {
            ctx.keys()?;
        }
        settings.require_two_factor = on;
    }
    if let Some(on) = body.two_factor_google {
        settings.two_factor_google = on;
    }
    if let Some(on) = body.verify_sensitive_actions {
        settings.verify_sensitive_actions = on;
    }
    crate::instance::save(&ctx.db, &settings).await?;
    ctx.audit(
        "instance.settings_updated",
        None,
        Some(serde_json::to_value(&body)?),
    );
    http::ok(&settings)
}

// ─────────────────────────── the primary instance owner ───────────────────────────

/// `POST /api/v1/admin/transfer` (the primary instance owner): hands the
/// role to an admin, and becomes an admin.
pub async fn transfer(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: TransferRequest = http::body(req).await?;
    let actor = ctx.admin()?.clone();
    if actor.role() != InstanceRole::Owner {
        return Err(forbidden(
            "Only the primary instance owner can hand the role over",
        ));
    }
    let target = auth::user_by_id(ctx, body.user_id.as_str())
        .await?
        .ok_or_else(|| not_found("User"))?;
    if target.id == actor.id {
        return Err(conflict("You are the primary instance owner already"));
    }
    if target.role() != InstanceRole::Admin || !target.is_active() {
        return Err(validation(
            "user_id",
            "Make them an admin first: the role of primary instance owner goes to an admin",
        ));
    }
    ctx.verified(Sensitive::TransferInstance)?;
    crate::db::batch(
        &ctx.db,
        vec![
            // Only while the new one is still an active admin, so that the
            // instance always has its primary owner.
            q(
                &ctx.db,
                "UPDATE users SET role = 'admin', updated_at = ?1 WHERE id = ?2 AND role = 'owner' \
                 AND EXISTS (SELECT 1 FROM users t WHERE t.id = ?3 AND t.role = 'admin' AND t.status = 'active')",
                &[n(ctx.now), s(&actor.id), s(&target.id)],
            )?,
            q(
                &ctx.db,
                "UPDATE users SET role = 'owner', updated_at = ?1 WHERE id = ?2 AND role = 'admin' \
                 AND NOT EXISTS (SELECT 1 FROM users o WHERE o.role = 'owner')",
                &[n(ctx.now), s(&target.id)],
            )?,
        ],
    )
    .await?;
    let updated = auth::user_by_id(ctx, &target.id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    if updated.role() != InstanceRole::Owner {
        return Err(conflict(
            "They stopped being an admin in the meantime, so you are still the primary instance owner",
        ));
    }
    ctx.audit(
        "instance.transferred",
        Some(&target.id),
        Some(serde_json::json!({ "from": actor.id })),
    );
    http::ok(&updated.to_user())
}

// ───────────────────────────── workspaces ─────────────────────────────

/// `GET /api/v1/admin/workspaces`: every workspace, as instance admins see
/// them (what it is and who runs it, not what's inside).
pub async fn workspaces(ctx: &Ctx) -> AppResult<Response> {
    let admin = ctx.admin()?;
    let rows: Vec<crate::workspaces::WorkspaceRow> = q(
        &ctx.db,
        &format!(
            "{} ORDER BY lower(w.name)",
            crate::workspaces::WORKSPACE_SELECT
        ),
        &[s(&admin.id)],
    )?
    .all()
    .await?;
    let settings = ctx.settings();
    let mut workspaces = Vec::with_capacity(rows.len());
    for r in &rows {
        let needs_primary = r.needs_primary();
        workspaces.push(AdminWorkspace {
            workspace: r.to_ref(),
            primary: r.primary(),
            members: r.members.max(0) as u32,
            projects: r.projects.max(0) as u32,
            runners: r.runner_ids().len() as u32,
            github: crate::workspaces::strings(r.github.as_deref()),
            require_two_factor: r.requires_two_factor(&settings),
            created_at: r.created_at,
            role: r.role(),
            needs_primary,
            // Who it could go to, only for the workspaces that need one.
            candidates: if needs_primary {
                primary_candidates(ctx, &r.id).await?
            } else {
                Vec::new()
            },
        });
    }
    http::ok(&AdminWorkspaceList { workspaces })
}

/// A workspace's members with active accounts, admins first: who may
/// become its primary admin when nobody can hand the role over.
async fn primary_candidates(ctx: &Ctx, workspace_id: &str) -> AppResult<Vec<UserRef>> {
    #[derive(serde::Deserialize)]
    struct C {
        id: String,
        name: String,
        avatar_url: Option<String>,
    }
    let rows: Vec<C> = q(
        &ctx.db,
        "SELECT u.id, u.name, u.avatar_url FROM workspace_members m JOIN users u ON u.id = m.user_id \
         WHERE m.workspace_id = ? AND u.status = 'active' ORDER BY m.role = 'admin' DESC, m.created_at",
        &[s(workspace_id)],
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .map(|c| UserRef {
            id: UserId(c.id),
            name: c.name,
            avatar_url: c.avatar_url,
        })
        .collect())
}

/// `POST /api/v1/admin/workspaces/{id}/primary` (instance admins): gives
/// the primary admin role of a workspace whose primary admin's account is
/// disabled or gone to one of its members. Otherwise its primary admin
/// hands it over: instance admins don't run workspaces.
pub async fn workspace_primary(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: TransferRequest = http::body(req).await?;
    let admin = ctx.admin()?.clone();
    let row = crate::workspaces::find(&ctx.db, &admin.id, id)
        .await?
        .ok_or_else(|| not_found("Workspace"))?;
    if !row.needs_primary() {
        return Err(conflict(
            "Its primary admin can hand the role over themselves",
        ));
    }
    let to = body.user_id.as_str();
    if !primary_candidates(ctx, &row.id)
        .await?
        .iter()
        .any(|c| c.id.as_str() == to)
    {
        return Err(validation(
            "user_id",
            "Choose one of its members whose account is active",
        ));
    }
    // Instance admins can disable the primary admin, so one who is a member
    // could otherwise take a workspace over: they make only themselves its
    // primary admin where they are one of its admins already.
    fctl_domain::workspace::check_assign_primary(to == admin.id, row.role())?;
    ctx.verified(Sensitive::TransferWorkspace)?;
    let from = row.primary_id.clone();
    crate::db::batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "UPDATE workspace_members SET role = 'admin' WHERE workspace_id = ?1 AND role = 'owner' AND user_id != ?2",
                &[s(&row.id), s(to)],
            )?,
            q(
                &ctx.db,
                "UPDATE workspace_members SET role = 'owner' WHERE workspace_id = ?1 AND user_id = ?2 \
                 AND NOT EXISTS (SELECT 1 FROM workspace_members o WHERE o.workspace_id = ?1 AND o.role = 'owner')",
                &[s(&row.id), s(to)],
            )?,
        ],
    )
    .await?;
    let data = serde_json::json!({ "from": from, "by": admin.id });
    ctx.audit_in(
        Some(&row.id),
        "workspace.transferred",
        Some(to),
        Some(data.clone()),
    );
    ctx.audit("workspace.primary_assigned", Some(&row.id), Some(data));
    crate::workspaces::changed(
        ctx,
        [Some(to.to_owned()), from].into_iter().flatten().collect(),
    );
    http::ok(&Ok { ok: true })
}

/// `POST /api/v1/admin/workspaces/{id}/delete` (instance admins): deletes a
/// workspace and everything in it, its name typed to confirm.
pub async fn delete_workspace(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: DeleteWorkspaceRequest = http::body(req).await?;
    let admin = ctx.admin()?.clone();
    let row = crate::workspaces::find(&ctx.db, &admin.id, id)
        .await?
        .ok_or_else(|| not_found("Workspace"))?;
    crate::api::workspaces::erase(ctx, &row, &body.name).await?;
    http::ok(&Ok { ok: true })
}
