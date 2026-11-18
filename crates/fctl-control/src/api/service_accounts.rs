//! A workspace's service accounts (docs/architecture.md#service-accounts,
//! ADR 0086): `/api/v1/workspaces/{id}/service-accounts`, which its admins
//! make and every member lists, `/api/v1/service-accounts/{id}` to change,
//! give a new secret, delete, answer and see who answered what, and
//! `/api/v1/service-account`, what a service account's token is.

use crate::ctx::Ctx;
use crate::db::{b, n, os, q, s};
use crate::error::{AppError, AppResult, forbidden, not_found};
use crate::service_accounts::{self as sa, Row};
use crate::{http, workspaces as ws};
use fctl_domain::service_accounts as policy;
use fctl_domain::tokens::{SecretKind, hash_secret, new_secret};
use fctl_domain::verify::Sensitive;
use fctl_proto::api::Ok as Done;
use fctl_proto::service_accounts::{
    AnswerServiceAccountRequest, CreateServiceAccountRequest, CreatedServiceAccount,
    ServiceAccountList, ServiceAccountPeople, ServiceAccountSelf, UpdateServiceAccountRequest,
};
use serde_json::json;
use worker::{Request, Response};

/// A service account and its workspace, as the caller sees them: not found
/// unless they are a member of it.
async fn load(ctx: &Ctx, id: &str) -> AppResult<(Row, ws::WorkspaceAccess)> {
    let me = ctx.user()?.id.clone();
    let row = sa::row(&ctx.db, &me, id)
        .await?
        .ok_or_else(|| not_found("Service account"))?;
    let a = ws::load(ctx, &row.workspace_id)
        .await
        .map_err(|_| not_found("Service account"))?;
    Ok((row, a))
}

/// The service account `id` again, after a change, as the caller (one of
/// its workspace's admins, `manages`) sees it.
async fn reread(
    ctx: &Ctx,
    id: &str,
    manages: bool,
) -> AppResult<fctl_proto::service_accounts::ServiceAccount> {
    let me = ctx.user()?.id.clone();
    let row = sa::row(&ctx.db, &me, id)
        .await?
        .ok_or_else(|| not_found("Service account"))?;
    Ok(row.to_api(&ctx.settings(), manages))
}

/// `GET /api/v1/workspaces/{id}/service-accounts`: every member sees them,
/// with their own answer; its admins see the rest.
pub async fn list(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    let role = a.require_open()?;
    let me = ctx.user()?.id.clone();
    let rows = sa::of_workspace(&ctx.db, &me, a.id()).await?;
    let settings = ctx.settings();
    let consent = sa::consent(&settings);
    http::ok(&ServiceAccountList {
        service_accounts: rows
            .iter()
            .map(|r| r.to_api(&settings, role.is_admin()))
            .collect(),
        consent,
        limit: policy::MAX_PER_WORKSPACE,
    })
}

/// `POST /api/v1/workspaces/{id}/service-accounts` (its admins): a new
/// service account, whose secret the answer shows once, kept by the
/// caller. Whoever holds the secret starts sessions for the people who
/// allow it: it takes confirming it's you.
pub async fn create(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: CreateServiceAccountRequest = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    let name = policy::check_name(&body.name).map_err(AppError)?;
    let description = policy::check_description(body.description.as_deref()).map_err(AppError)?;
    policy::check_room(sa::count_of(&ctx.db, a.id()).await?).map_err(AppError)?;
    ctx.verified(Sensitive::NewServiceAccount)?;
    let secret = new_secret(SecretKind::Service);
    let new_id = fctl_domain::ids::service_account_id(ctx.now);
    let me = ctx.user()?.id.clone();
    // Within the limit when it runs, not only when it was checked: two
    // requests at once can't both take the last place.
    let added = q(
        &ctx.db,
        "INSERT INTO service_accounts (id, workspace_id, name, description, hash, hint, keeper_id, secret_made_at, \
         created_by, created_at, updated_at) \
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?7, ?8, ?8 \
         WHERE (SELECT COUNT(*) FROM service_accounts WHERE workspace_id = ?2) < ?9",
        &[
            s(new_id.as_str()),
            s(a.id()),
            s(&name),
            os(description.as_deref()),
            s(&hash_secret(&secret)),
            s(&policy::hint(&secret)),
            s(&me),
            n(ctx.now),
            n(i64::from(policy::MAX_PER_WORKSPACE)),
        ],
    )?
    .run()
    .await?;
    if added == 0 {
        return Err(AppError(
            policy::check_room(policy::MAX_PER_WORKSPACE)
                .err()
                .unwrap_or_else(|| fctl_proto::ApiError::conflict("Try again")),
        ));
    }
    ctx.audit_in(
        Some(a.id()),
        "workspace.service_account_created",
        Some(new_id.as_str()),
        Some(json!({ "name": name })),
    );
    http::created(&CreatedServiceAccount {
        service_account: reread(ctx, new_id.as_str(), true).await?,
        secret,
    })
}

/// `GET /api/v1/service-accounts/{id}`: as a member of its workspace sees
/// it, and all of it for its admins.
pub async fn get(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let (row, a) = load(ctx, id).await?;
    let role = a.require_open()?;
    http::ok(&row.to_api(&ctx.settings(), role.is_admin()))
}

/// `PATCH /api/v1/service-accounts/{id}` (its workspace's admins): its
/// name and what it is.
pub async fn update(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdateServiceAccountRequest = http::body(req).await?;
    let (row, a) = load(ctx, id).await?;
    a.require_admin()?;
    let name = body
        .name
        .as_deref()
        .map(policy::check_name)
        .transpose()
        .map_err(AppError)?;
    let description = match body.description.as_deref() {
        Some(d) => Some(policy::check_description(Some(d)).map_err(AppError)?),
        None => None,
    };
    let changed = name.as_deref().is_some_and(|n| n != row.name)
        || description
            .as_ref()
            .is_some_and(|d| d.as_deref() != row.description.as_deref());
    if changed {
        q(
            &ctx.db,
            "UPDATE service_accounts SET name = COALESCE(?1, name), \
             description = CASE WHEN ?2 = 1 THEN ?3 ELSE description END, updated_at = ?4 WHERE id = ?5",
            &[
                os(name.as_deref()),
                b(description.is_some()),
                os(description.clone().flatten().as_deref()),
                n(ctx.now),
                s(&row.id),
            ],
        )?
        .run()
        .await?;
        ctx.audit_in(
            Some(a.id()),
            "workspace.service_account_updated",
            Some(&row.id),
            Some(json!({
                "name": name.as_deref().unwrap_or(&row.name),
                "renamed_from": name.as_ref().filter(|n| **n != row.name).map(|_| &row.name),
                "description": description.is_some().then_some(true),
            })),
        );
    }
    http::ok(&reread(ctx, &row.id, true).await?)
}

/// `DELETE /api/v1/service-accounts/{id}` (its workspace's admins): it
/// works no more, and people's answers to it go. The sessions it started
/// stay their people's.
pub async fn delete(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let (row, a) = load(ctx, id).await?;
    a.require_admin()?;
    q(
        &ctx.db,
        "DELETE FROM service_accounts WHERE id = ?",
        &[s(&row.id)],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(a.id()),
        "workspace.service_account_deleted",
        Some(&row.id),
        Some(json!({ "name": row.name })),
    );
    http::ok(&Done { ok: true })
}

/// `POST /api/v1/service-accounts/{id}/secret` (its workspace's admins): a
/// new secret, shown once, in place of the old one, which stops working at
/// once. The caller keeps it from now on: one that stopped as its keeper
/// left works again. People's answers stay.
pub async fn new_secret_for(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let (row, a) = load(ctx, id).await?;
    a.require_admin()?;
    ctx.verified(Sensitive::NewServiceAccount)?;
    let secret = new_secret(SecretKind::Service);
    let me = ctx.user()?.id.clone();
    q(
        &ctx.db,
        "UPDATE service_accounts SET hash = ?1, hint = ?2, keeper_id = ?3, secret_made_at = ?4, updated_at = ?4 \
         WHERE id = ?5",
        &[
            s(&hash_secret(&secret)),
            s(&policy::hint(&secret)),
            s(&me),
            n(ctx.now),
            s(&row.id),
        ],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(a.id()),
        "workspace.service_account_secret_changed",
        Some(&row.id),
        Some(json!({ "name": row.name, "was_stopped": row.stopped(&ctx.settings()).is_some().then_some(true) })),
    );
    http::ok(&CreatedServiceAccount {
        service_account: reread(ctx, &row.id, true).await?,
        secret,
    })
}

/// `PUT /api/v1/service-accounts/{id}/answer`: whether it may start
/// sessions for the caller, a member of its workspace. Allowing it hands
/// whoever holds its secret their sessions, on their subscription: it
/// takes confirming it's you, and fctl emails them that they did. They
/// change their answer whenever they like.
pub async fn answer(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: AnswerServiceAccountRequest = http::body(req).await?;
    let (row, a) = load(ctx, id).await?;
    let role = a.require_open()?;
    let was = sa::answer(row.my_answer);
    let allowed = Some(fctl_proto::service_accounts::ServiceAccountAnswer::Allowed);
    if body.allow && was != allowed {
        ctx.verified(Sensitive::AllowServiceAccount)?;
    }
    let me = ctx.user()?.clone();
    q(
        &ctx.db,
        "INSERT INTO service_account_answers (service_account_id, user_id, allowed, answered_at) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT (service_account_id, user_id) DO UPDATE SET allowed = excluded.allowed, \
         answered_at = CASE WHEN service_account_answers.allowed = excluded.allowed \
           THEN service_account_answers.answered_at ELSE excluded.answered_at END",
        &[s(&row.id), s(&me.id), b(body.allow), n(ctx.now)],
    )?
    .run()
    .await?;
    let now = if body.allow {
        allowed
    } else {
        Some(fctl_proto::service_accounts::ServiceAccountAnswer::Declined)
    };
    if now != was {
        ctx.audit_in(
            Some(a.id()),
            if body.allow {
                "workspace.service_account_allowed"
            } else {
                "workspace.service_account_declined"
            },
            Some(&row.id),
            Some(json!({ "name": row.name })),
        );
        if body.allow && ctx.cfg.email.is_some() {
            let instance = crate::instance::name(&ctx.db, &ctx.cfg)
                .await
                .unwrap_or_else(|_| ctx.cfg.instance_name.clone());
            let (subject, text) = allowed_notice(ctx, &instance, &me.email, &row, &a.row.slug);
            crate::email::notice(ctx, &me, &subject, text);
        }
    }
    http::ok(&reread(ctx, &row.id, role.is_admin()).await?)
}

/// What fctl emails someone who just let a service account start sessions
/// for them.
fn allowed_notice(
    ctx: &Ctx,
    instance: &str,
    email: &str,
    row: &Row,
    slug: &str,
) -> (String, String) {
    (
        format!("You let {} start sessions for you", row.name),
        format!(
            "You let {}, a service account of {}, start sessions for you on {instance} ({email}): the program that holds it starts them as you, on your Claude subscription, and messages their agents for you.\n\nYou can take that back at any time in {}'s Service accounts: {}\n\nIf you didn't allow it, take it back there now, sign out your other devices in Settings and change your password.",
            row.name,
            row.workspace_name,
            row.workspace_name,
            ctx.cfg.url(&format!("/w/{slug}?tab=service-accounts")),
        ),
    )
}

/// `GET /api/v1/service-accounts/{id}/people`: the workspace's people, with
/// their answers and whether it may act for each, for its admins, and for
/// the service account itself.
pub async fn people(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let row = match ctx.service_account() {
        Some(caller) => {
            if caller.id != id {
                return Err(not_found("Service account"));
            }
            sa::row(&ctx.db, "", id)
                .await?
                .ok_or_else(|| not_found("Service account"))?
        }
        None => {
            let (row, a) = load(ctx, id).await?;
            a.require_admin()?;
            row
        }
    };
    #[derive(serde::Deserialize)]
    struct W {
        #[serde(deserialize_with = "crate::db::int_bool")]
        require_two_factor: bool,
    }
    let w: Option<W> = q(
        &ctx.db,
        "SELECT require_two_factor FROM workspaces WHERE id = ?",
        &[s(&row.workspace_id)],
    )?
    .first()
    .await?;
    let settings = ctx.settings();
    let require = w.is_some_and(|w| w.require_two_factor) || settings.require_two_factor;
    http::ok(&ServiceAccountPeople {
        people: sa::people(ctx, &row, require).await?,
        consent: sa::consent(&settings),
    })
}

/// `GET /api/v1/service-account`, with a service account's token: what it
/// is, and whom it acts for in this request.
pub async fn whoami(ctx: &Ctx) -> AppResult<Response> {
    let Some(caller) = ctx.service_account() else {
        return Err(forbidden(
            "This is a service account's: send its token (fctl_svc_…) as the bearer token.",
        ));
    };
    let row = sa::row(&ctx.db, "", &caller.id)
        .await?
        .ok_or_else(|| not_found("Service account"))?;
    let acting_for = match &caller.acting_for {
        Some(_) => {
            let user = ctx.user()?;
            let member = ws::member(&ctx.db, &caller.workspace_id, &user.id).await?;
            Some((user.to_ref(), member.map(|m| m.email)))
        }
        None => None,
    };
    http::ok(&ServiceAccountSelf {
        // Its own view of itself: what its admins see, but whose secret.
        service_account: fctl_proto::service_accounts::ServiceAccount {
            secret_hint: None,
            secret_made_at: None,
            can_manage: false,
            answer: None,
            answered_at: None,
            acts_for_you: false,
            ..row.to_api(&ctx.settings(), true)
        },
        acting_for_email: acting_for.as_ref().and_then(|(_, e)| e.clone()),
        acting_for: acting_for.map(|(u, _)| u),
    })
}
