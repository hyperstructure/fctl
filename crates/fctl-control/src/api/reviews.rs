//! Review agents (docs/architecture.md#review-agents): a project's settings
//! for them (`/api/v1/projects/{id}/review-agents`), its reviews, and a
//! review someone asks for (`/api/v1/projects/{id}/reviews`). The reviews
//! themselves start in `crate::reviews`.

use crate::access;
use crate::ctx::Ctx;
use crate::db::{n, os, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, not_configured, validation};
use crate::http;
use crate::reviews::{self, Started};
use fctl_domain::reviews as rv;
use fctl_domain::validate;
use fctl_proto::ApiError;
use fctl_proto::api::{
    ReviewAgents, ReviewBotList, ReviewList, StartReviewRequest, UpdateReviewAgentsRequest,
};
use serde_json::json;
use worker::{Request, Response};

/// `GET /api/v1/projects/{id}/review-agents`: for everyone who sees the
/// project.
pub async fn get_settings(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    http::ok(&agents(ctx, &p.row).await?)
}

async fn agents(ctx: &Ctx, project: &crate::model::ProjectRow) -> AppResult<ReviewAgents> {
    let conf = reviews::settings(&ctx.db, &project.id).await?;
    let problem = reviews::problem(&ctx.cfg, &ctx.db, project, conf.as_ref()).await?;
    let repos = reviews::repos(&ctx.db, project).await?;
    Ok(ReviewAgents {
        enabled: conf.as_ref().is_some_and(|c| c.enabled),
        operator: conf.as_ref().and_then(reviews::SettingsRow::operator),
        model: conf.as_ref().and_then(|c| c.model.clone()),
        effort: conf.as_ref().and_then(|c| c.effort.clone()),
        instructions: conf.as_ref().and_then(|c| c.instructions.clone()),
        bots: reviews::bots(conf.as_ref()),
        repos,
        problem,
        updated_by: conf.as_ref().and_then(reviews::SettingsRow::updater),
        updated_at: conf.as_ref().map(|c| c.updated_at),
    })
}

/// `PATCH /api/v1/projects/{id}/review-agents`: the project's owners turn
/// them on (on their own Claude subscription) or off, take them over and
/// say how they run; the person they run for may turn them off.
pub async fn update_settings(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdateReviewAgentsRequest = http::body(req).await?;
    let p = access::project(ctx, id).await?;
    let me = ctx.user()?.clone();
    let conf = reviews::settings(&ctx.db, &p.row.id).await?;
    let mine = conf
        .as_ref()
        .and_then(|c| c.operator_id.as_deref())
        .is_some_and(|o| o == me.id);
    let only_off = body.enabled == Some(false)
        && body.run_as_me.is_none()
        && body.model.is_none()
        && body.effort.is_none()
        && body.instructions.is_none()
        && body.bots.is_none();
    if !fctl_domain::perm::can_own_project(p.role) && !(mine && only_off) {
        return Err(forbidden(
            "Only the project's owners and its workspace's admins change its review agents. They run on your Claude subscription: you can turn them off.",
        ));
    }
    let model = match body.model.as_deref().map(str::trim) {
        Some("") => Some(None),
        Some(m) => Some(Some(validate::model(m)?)),
        None => None,
    };
    let effort = match body.effort.as_deref().map(str::trim) {
        Some("") => Some(None),
        Some(e) => Some(Some(validate::effort(e)?)),
        None => None,
    };
    let instructions = body
        .instructions
        .as_deref()
        .map(|i| validate::text("instructions", i, 20_000))
        .transpose()?
        .map(|i| (!i.is_empty()).then_some(i));
    let bots = match body.bots.as_deref() {
        Some(list) => rv::bots(list)?,
        None => reviews::bots(conf.as_ref()),
    };
    let was_on = conf.as_ref().is_some_and(|c| c.enabled);
    let enabled = body.enabled.unwrap_or(was_on);
    // Turning them on, or taking them over, runs them on the caller's
    // subscription: nobody spends someone else's.
    let takes_over = (enabled && !was_on) || body.run_as_me == Some(true);
    if takes_over {
        if ctx.cfg.github.is_none() {
            return Err(not_configured(
                "This instance has no GitHub App, so nothing reviews pull requests. Its admins set one up first.",
            ));
        }
        if reviews::repos(&ctx.db, &p.row).await?.is_empty() {
            return Err(validation(
                "enabled",
                "This project has no repository its agents may write to: link one under Repositories first.",
            ));
        }
        if crate::api::sessions::credential_for(&ctx.db, &me, p.workspace_id())
            .await?
            .is_none()
        {
            return Err(AppError(ApiError {
                code: fctl_proto::ErrorCode::NotConfigured,
                message:
                    "Connect your Claude subscription first (Settings → Claude): reviews run on it."
                        .into(),
                field: Some("claude".into()),
                retry_after: None,
            }));
        }
    }
    let operator = if takes_over {
        Some(me.id.clone())
    } else {
        conf.as_ref().and_then(|c| c.operator_id.clone())
    };
    let keep = |new: Option<Option<String>>, old: Option<String>| new.unwrap_or(old);
    let model = keep(model, conf.as_ref().and_then(|c| c.model.clone()));
    let effort = keep(effort, conf.as_ref().and_then(|c| c.effort.clone()));
    let instructions = keep(
        instructions,
        conf.as_ref().and_then(|c| c.instructions.clone()),
    );
    q(
        &ctx.db,
        "INSERT INTO project_reviews (project_id, enabled, operator_id, model, effort, instructions, bots, updated_by, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT (project_id) DO UPDATE SET enabled = excluded.enabled, \
         operator_id = excluded.operator_id, model = excluded.model, effort = excluded.effort, \
         instructions = excluded.instructions, bots = excluded.bots, updated_by = excluded.updated_by, \
         updated_at = excluded.updated_at",
        &[
            s(&p.row.id),
            crate::db::b(enabled),
            os(operator.as_deref()),
            os(model.as_deref()),
            os(effort.as_deref()),
            os(instructions.as_deref()),
            s(&serde_json::to_string(&bots)?),
            s(&me.id),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    let action = match (was_on, enabled, takes_over) {
        (false, true, _) => "reviews.enabled",
        (true, false, _) => "reviews.disabled",
        (_, _, true) => "reviews.taken_over",
        _ => "reviews.updated",
    };
    ctx.audit_in(
        Some(p.workspace_id()),
        action,
        Some(&p.row.id),
        Some(json!({
            "project": p.row.slug,
            "operator": operator,
            "model": model,
            "effort": effort,
            "bots": bots,
        })),
    );
    http::ok(&agents(ctx, &p.row).await?)
}

/// `GET /api/v1/projects/{id}/review-agents/bots`: the apps that opened
/// the newest pull requests in the project's repositories, for its owners,
/// who choose which of them start reviews by themselves.
pub async fn bots(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    p.require_owner()?;
    http::ok(&ReviewBotList {
        bots: reviews::seen_bots(ctx, &p.row).await?,
    })
}

/// `GET /api/v1/projects/{id}/review-agents/bots/{name}`: an app, as
/// GitHub shows it (its name, owner and logo), for the project's owners
/// to check one they name before they let it start reviews.
pub async fn bot(ctx: &Ctx, id: &str, name: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    p.require_owner()?;
    match reviews::find_bot(ctx, &p.row, name).await? {
        Some(bot) => http::ok(&bot),
        None => Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::NotFound,
            "GitHub shows no app by that name. It shows no other account's private apps.",
        ))),
    }
}

/// `GET /api/v1/projects/{id}/reviews`: newest first, a page at a time
/// (`?before=`, `?limit=`); with `?session=`, that session's alone.
pub async fn list(ctx: &Ctx, req: &Request, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    if let Some(session) = http::query(req, "session") {
        let row = reviews::of_session(&ctx.db, &session)
            .await?
            .filter(|r| r.project_id == p.row.id);
        return http::ok(&ReviewList {
            reviews: row
                .iter()
                .filter_map(reviews::ReviewRow::to_review)
                .collect(),
            next_before: None,
        });
    }
    let limit = http::query(req, "limit")
        .and_then(|l| l.parse::<i64>().ok())
        .unwrap_or(30)
        .clamp(1, 100);
    let before = http::query(req, "before").and_then(|b| b.parse::<i64>().ok());
    let mut rows = reviews::list(&ctx.db, &p.row.id, before, limit + 1).await?;
    let more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    http::ok(&ReviewList {
        next_before: if more {
            rows.last().map(|r| r.created_at)
        } else {
            None
        },
        reviews: rows
            .iter()
            .filter_map(reviews::ReviewRow::to_review)
            .collect(),
    })
}

/// `POST /api/v1/projects/{id}/reviews`: people who start sessions in the
/// project ask for a review of a pull request's head now, on the
/// subscription its review agents run on, in place of a review of it still
/// under way.
pub async fn start(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: StartReviewRequest = http::body(req).await?;
    let p = access::project(ctx, id).await?;
    p.require_editor()?;
    if p.row.archived_at.is_some() {
        return Err(conflict("This project is archived"));
    }
    let on = reviews::settings(&ctx.db, &p.row.id)
        .await?
        .is_some_and(|c| c.enabled);
    if !on {
        return Err(conflict(
            "Review agents are off in this project. Its owners turn them on under Reviews.",
        ));
    }
    let (repo, number) = rv::pull_request_ref(&body.pull_request, p.row.repo().as_ref())?;
    let inst = reviews::reach(ctx, &p.row, &repo).await?;
    let gh = crate::github::from_config(&ctx.cfg, ctx.now)?;
    let answer = gh.lookup(inst, rv::lookup(&repo, number)).await?;
    let pr = rv::looked_up(&answer).ok_or_else(|| {
        validation(
            "pull_request",
            &format!(
                "{}#{number} isn't a pull request GitHub shows fctl",
                repo.full_name()
            ),
        )
    })?;
    if !pr.open {
        return Err(conflict(
            "That pull request is merged or closed: reviews are for open ones.",
        ));
    }
    let me = ctx.user()?.clone();
    let session = match reviews::start(ctx, &p.row, &pr, inst, Some(&me)).await? {
        Started::Review(session) => session,
        Started::Already => return Err(conflict("A review of this commit just started.")),
        Started::Skipped(why) => {
            return Err(AppError(ApiError::new(
                fctl_proto::ErrorCode::NotConfigured,
                why,
            )));
        }
    };
    let review = reviews::of_session(&ctx.db, &session)
        .await?
        .and_then(|r| r.to_review())
        .ok_or_else(|| crate::error::internal("the review vanished"))?;
    http::created(&review)
}
