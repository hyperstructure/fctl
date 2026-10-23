//! `/api/v1/projects/{id}/schedules` and `/api/v1/schedules/*`: a project's
//! schedules (`crate::schedules`). Everyone who sees the project sees them
//! and their runs; people who start sessions there set them up, on their
//! own Claude subscription, and run them now, on theirs; a schedule's owner
//! and the project's maintainers and owners change, pause and delete it.

use crate::ctx::Ctx;
use crate::db::{Count, b, n, on, os, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, internal, validation};
use crate::http;
use crate::schedules::{self, Outcome, Problems, Start};
use fctl_domain::schedules::{self as sch, RunNote, StartedBy, When};
use fctl_domain::validate;
use fctl_proto::api::{
    CreateScheduleRequest, RunScheduleRequest, ScheduleList, ScheduleRunList, ScheduleTrigger,
    TokenKind, UpdateScheduleRequest,
};
use fctl_proto::session::GithubAccess;
use fctl_proto::{ApiError, ErrorCode, ScheduleId};
use serde_json::json;
use worker::{Request, Response};

/// `GET /api/v1/projects/{id}/schedules`.
pub async fn list(ctx: &Ctx, project_id: &str) -> AppResult<Response> {
    let p = crate::access::project(ctx, project_id).await?;
    let rows = schedules::of_project(&ctx.db, &p.row.id).await?;
    let mut last = schedules::last_runs(&ctx.db, &p.row.id).await?;
    let mut problems = Problems::default();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let problem = problems.of(&ctx.db, &p.row, &row, ctx.now).await?;
        out.push(row.to_api(last.remove(&row.id), problem));
    }
    http::ok(&ScheduleList { schedules: out })
}

/// A schedule as the API shows it now: with its latest run and what keeps
/// its next one from starting.
async fn shown(ctx: &Ctx, id: &str, project: &crate::model::ProjectRow) -> AppResult<Response> {
    let row = schedules::row(&ctx.db, id)
        .await?
        .ok_or_else(|| internal("a schedule vanished"))?;
    let last = schedules::runs(&ctx.db, id, None, 1)
        .await?
        .into_iter()
        .next()
        .map(|r| r.to_api());
    let problem = Problems::default()
        .of(&ctx.db, project, &row, ctx.now)
        .await?;
    http::ok(&row.to_api(last, problem))
}

/// `GET /api/v1/schedules/{id}`.
pub async fn get(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let (row, p) = schedules::load(ctx, id).await?;
    shown(ctx, &row.id, &p.row).await
}

/// A model, effort level or branch from a request: `None` for none, or
/// for the project's.
fn optional(
    input: Option<&str>,
    check: impl Fn(&str) -> Result<String, ApiError>,
) -> AppResult<Option<String>> {
    match input.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => Ok(Some(check(v)?)),
        None => Ok(None),
    }
}

/// What a schedule's agents may do on GitHub, as asked: at most what the
/// project allows, and the caller's access token. Asked for nothing, the
/// project's as each run starts, within the token's cap.
fn github(
    ctx: &Ctx,
    asked: Option<GithubAccess>,
    project: &crate::model::ProjectRow,
) -> AppResult<Option<GithubAccess>> {
    let token = token_github(ctx);
    let Some(asked) = asked else {
        return Ok(sch::github_within(None, token));
    };
    fctl_domain::limits::session_github(Some(asked), project.settings().github_access, token)
        .map(Some)
        .map_err(|e| {
            AppError(ApiError {
                field: Some("github_access".into()),
                ..e
            })
        })
}

/// How far the caller's access token lets agents go on GitHub, if it caps them.
fn token_github(ctx: &Ctx) -> Option<GithubAccess> {
    ctx.limits().and_then(|l| l.github_access)
}

/// `POST /api/v1/projects/{id}/schedules`: a schedule on the caller's
/// Claude subscription.
pub async fn create(ctx: &mut Ctx, req: &mut Request, project_id: &str) -> AppResult<Response> {
    let body: CreateScheduleRequest = http::body(req).await?;
    let p = crate::access::project(ctx, project_id).await?;
    p.require_editor()?;
    if p.row.archived_at.is_some() {
        return Err(conflict("This project is archived"));
    }
    let user = ctx.user()?.clone();
    let prompt = validate::message("prompt", &body.prompt)?;
    let name = sch::name(body.name.as_deref(), &prompt)?;
    let timezone = sch::timezone(body.timezone.as_deref())?;
    let zone = schedules::known_zone(&timezone)?;
    let when = When::parse(body.cron.as_deref(), body.at, ctx.now)?;
    let model = optional(body.model.as_deref(), validate::model)?;
    let effort = optional(body.effort.as_deref(), validate::effort)?;
    let base_branch = optional(body.base_branch.as_deref(), |v| {
        validate::branch("base_branch", v)
    })?;
    let github_access = github(ctx, body.github_access, &p.row)?;
    let count: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM schedules WHERE project_id = ?",
        &[s(&p.row.id)],
    )?
    .first()
    .await?;
    if count.is_some_and(|c| c.n >= sch::MAX_PER_PROJECT) {
        return Err(conflict(&format!(
            "A project keeps at most {} schedules: delete one first",
            sch::MAX_PER_PROJECT
        )));
    }
    // Its runs start on the caller's subscription, which they need.
    crate::api::sessions::default_credential(ctx, &user, p.workspace_id()).await?;
    let enabled = body.enabled.unwrap_or(true);
    let next = if enabled {
        when.next(&zone, ctx.now, false)
    } else {
        None
    };
    let id = ScheduleId(fctl_domain::ids::new_id(ScheduleId::PREFIX, ctx.now));
    q(
        &ctx.db,
        "INSERT INTO schedules (id, project_id, name, prompt, cron, at, timezone, model, effort, base_branch, github_access, \
         owner_id, enabled, next_run_at, created_by, created_at, updated_by, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?12, ?15, ?12, ?15)",
        &[
            s(id.as_str()),
            s(&p.row.id),
            s(&name),
            s(&prompt),
            os(when.cron()),
            on(when.at()),
            s(&timezone),
            os(model.as_deref()),
            os(effort.as_deref()),
            os(base_branch.as_deref()),
            os(github_access.map(GithubAccess::as_str)),
            s(&user.id),
            b(enabled),
            on(next),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    schedules::wake(&ctx.env).await;
    ctx.audit_in(
        Some(p.workspace_id()),
        "schedule.created",
        Some(id.as_str()),
        Some(json!({
            "project": p.row.slug,
            "name": name,
            "cron": when.cron(),
            "at": when.at(),
            "timezone": timezone,
        })),
    );
    let row = schedules::row(&ctx.db, id.as_str())
        .await?
        .ok_or_else(|| internal("a new schedule vanished"))?;
    http::created(&row.to_api(None, None))
}

/// `PATCH /api/v1/schedules/{id}`.
pub async fn update(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdateScheduleRequest = http::body(req).await?;
    let (row, p) = schedules::load(ctx, id).await?;
    let user = ctx.user()?.clone();
    let mine = row.owner_id.as_deref() == Some(user.id.as_str());
    let rights = sch::rights(p.role, mine);
    let timing = body.cron.is_some() || body.at.is_some() || body.timezone.is_some();
    let changes = timing
        || body.name.is_some()
        || body.prompt.is_some()
        || body.model.is_some()
        || body.effort.is_some()
        || body.base_branch.is_some()
        || body.github_access.is_some();
    let pauses = body.enabled == Some(false);
    let resumes = body.enabled == Some(true) && !row.enabled;
    let takes_over = body.run_as_me == Some(true) && !mine;
    if pauses && !changes && !takes_over {
        if !rights.manage {
            return Err(forbidden(
                "Its owner and the project's maintainers and owners pause it",
            ));
        }
    } else if !rights.edit {
        return Err(forbidden(if rights.run {
            "Its owner and the project's maintainers and owners change it: run it now instead, on your subscription"
        } else {
            "You need the editor role in this project"
        }));
    }
    if p.row.archived_at.is_some() && (resumes || takes_over || changes) {
        return Err(conflict("This project is archived"));
    }
    // A change by anyone else makes it theirs: nobody puts work on someone
    // else's subscription.
    let becomes_mine = !mine && (changes || resumes || takes_over);
    if becomes_mine {
        crate::api::sessions::default_credential(ctx, &user, p.workspace_id()).await?;
    }
    let prompt = match body.prompt.as_deref() {
        Some(t) => validate::message("prompt", t)?,
        None => row.prompt.clone(),
    };
    let name = match body.name.as_deref() {
        Some(t) => sch::name(Some(t), &prompt)?,
        None => row.name.clone(),
    };
    let timezone = match body.timezone.as_deref() {
        Some(t) => sch::timezone(Some(t))?,
        None => row.timezone.clone(),
    };
    let zone: Box<dyn fctl_domain::cron::Zone> = if body.timezone.is_some() {
        Box::new(schedules::known_zone(&timezone)?)
    } else {
        schedules::zone(&timezone)
    };
    let when = if body.cron.is_some() || body.at.is_some() {
        When::parse(body.cron.as_deref(), body.at, ctx.now)?
    } else {
        row.when()
            .ok_or_else(|| validation("cron", "Say again when it runs"))?
    };
    let keep = |v: &Option<String>| v.clone();
    let model = match body.model.as_deref() {
        Some(v) => optional(Some(v), validate::model)?,
        None => keep(&row.model),
    };
    let effort = match body.effort.as_deref() {
        Some(v) => optional(Some(v), validate::effort)?,
        None => keep(&row.effort),
    };
    let base_branch = match body.base_branch.as_deref() {
        Some(v) => optional(Some(v), |b| validate::branch("base_branch", b))?,
        None => keep(&row.base_branch),
    };
    let github_access = match body.github_access.as_deref().map(str::trim) {
        Some("") => None,
        Some(v) => {
            let asked = GithubAccess::parse(v).ok_or_else(|| {
                validation(
                    "github_access",
                    "Use push, comment or read, or \"\" for the project's",
                )
            })?;
            github(ctx, Some(asked), &p.row)?
        }
        None => row.github(),
    };
    // A change, a resume or a take-over with a token capped on GitHub keeps
    // the schedule within the cap: its runs start after the token is gone.
    let github_access = if changes || resumes || takes_over {
        sch::github_within(github_access, token_github(ctx))
    } else {
        github_access
    };
    let enabled = body.enabled.unwrap_or(row.enabled);
    let next = if !enabled {
        None
    } else if timing || resumes {
        if let When::At(at) = when
            && at < ctx.now - sch::LATE_MS
        {
            return Err(validation("at", "That time has passed: choose one ahead"));
        }
        when.next(&*zone, ctx.now, false)
    } else {
        row.next_run_at
    };
    let owner = if becomes_mine {
        Some(user.id.clone())
    } else {
        row.owner_id.clone()
    };
    // What the alarm changes too (it pauses schedules, and moves them on as
    // they run) is written only when this asks for it, so that a rename
    // never takes back what the alarm did meanwhile.
    let state = pauses || resumes;
    let retimed = timing || state;
    q(
        &ctx.db,
        "UPDATE schedules SET name = ?1, prompt = ?2, cron = ?3, at = ?4, timezone = ?5, model = ?6, effort = ?7, \
         base_branch = ?8, github_access = ?9, owner_id = ?10, \
         enabled = CASE WHEN ?11 THEN ?12 ELSE enabled END, \
         paused_reason = CASE WHEN ?11 THEN NULL ELSE paused_reason END, \
         next_run_at = CASE WHEN ?13 THEN ?14 ELSE next_run_at END, \
         updated_by = ?15, updated_at = ?16 WHERE id = ?17",
        &[
            s(&name),
            s(&prompt),
            os(when.cron()),
            on(when.at()),
            s(&timezone),
            os(model.as_deref()),
            os(effort.as_deref()),
            os(base_branch.as_deref()),
            os(github_access.map(GithubAccess::as_str)),
            os(owner.as_deref()),
            b(state),
            b(enabled),
            b(retimed),
            on(next),
            s(&user.id),
            n(ctx.now),
            s(&row.id),
        ],
    )?
    .run()
    .await?;
    schedules::wake(&ctx.env).await;
    let action = if pauses && row.enabled {
        "schedule.paused"
    } else if resumes {
        "schedule.resumed"
    } else {
        "schedule.updated"
    };
    ctx.audit_in(
        Some(p.workspace_id()),
        action,
        Some(&row.id),
        Some(json!({
            "project": p.row.slug,
            "name": name,
            "cron": when.cron(),
            "at": when.at(),
            "timezone": timezone,
            "taken_over_from": becomes_mine.then_some(row.owner_id.as_deref()).flatten(),
        })),
    );
    shown(ctx, &row.id, &p.row).await
}

/// `DELETE /api/v1/schedules/{id}`: its runs go with it; the sessions it
/// started stay, saying a schedule started them.
pub async fn delete(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let (row, p) = schedules::load(ctx, id).await?;
    let mine = ctx.user()?.id == row.owner_id.clone().unwrap_or_default();
    if !sch::rights(p.role, mine).manage {
        return Err(forbidden(
            "Its owner and the project's maintainers and owners delete it",
        ));
    }
    q(&ctx.db, "DELETE FROM schedules WHERE id = ?", &[s(&row.id)])?
        .run()
        .await?;
    schedules::wake(&ctx.env).await;
    ctx.audit_in(
        Some(p.workspace_id()),
        "schedule.deleted",
        Some(&row.id),
        Some(json!({ "project": p.row.slug, "name": row.name })),
    );
    http::ok(&json!({ "ok": true }))
}

/// `GET /api/v1/schedules/{id}/runs`: newest first.
pub async fn runs(ctx: &Ctx, req: &Request, id: &str) -> AppResult<Response> {
    let (row, _) = schedules::load(ctx, id).await?;
    let limit = http::query(req, "limit")
        .and_then(|l| l.parse::<i64>().ok())
        .unwrap_or(30)
        .clamp(1, 100);
    let before = http::query(req, "before").and_then(|b| b.parse::<i64>().ok());
    let mut rows = schedules::runs(&ctx.db, &row.id, before, limit + 1).await?;
    let more = rows.len() as i64 > limit;
    rows.truncate(usize::try_from(limit).unwrap_or(30));
    let next_before = if more {
        rows.last().map(|r| r.at)
    } else {
        None
    };
    http::ok(&ScheduleRunList {
        runs: rows.iter().map(|r| r.to_api()).collect(),
        next_before,
    })
}

/// `POST /api/v1/schedules/{id}/runs`: its task now, in a session on the
/// caller's subscription, with what they send with it.
pub async fn run(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: RunScheduleRequest = http::body(req).await?;
    let (row, p) = schedules::load(ctx, id).await?;
    p.require_editor()?;
    if p.row.archived_at.is_some() {
        return Err(conflict("This project is archived"));
    }
    // Pausing it stops what other tools start too.
    if !row.enabled {
        return Err(conflict(
            "It's paused, so it starts nothing: resume it to run it",
        ));
    }
    let context = sch::context(body.context.as_deref())?;
    let asked: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM schedule_runs WHERE schedule_id = ? AND trigger != 'time' AND at > ?",
        &[s(&row.id), n(ctx.now - fctl_domain::cron::HOUR_MS)],
    )?
    .first()
    .await?;
    if asked.is_some_and(|c| c.n >= sch::RUNS_PER_HOUR) {
        return Err(AppError(
            ApiError::new(
                ErrorCode::RateLimited,
                format!(
                    "People and tools ran this schedule {} times in the last hour: wait a while before you run it again",
                    sch::RUNS_PER_HOUR
                ),
            )
            .with_retry_after(600),
        ));
    }
    let user = ctx.user()?.clone();
    let credential = crate::api::sessions::default_credential(ctx, &user, p.workspace_id()).await?;
    let token = ctx.auth.as_ref().is_some_and(|a| a.kind == TokenKind::Pat);
    let (trigger, by) = if token {
        (ScheduleTrigger::Token, StartedBy::Token(&user.name))
    } else {
        (ScheduleTrigger::Person, StartedBy::Person(&user.name))
    };
    let github_access =
        fctl_domain::limits::effective(row.github(), p.row.settings().github_access)
            .min(token_github(ctx).unwrap_or_default());
    let zone = schedules::zone(&row.timezone);
    let previous = schedules::previous(&ctx.db, &row.id).await?;
    let previous_at = previous.as_ref().map(|prev| sch::stamp(&*zone, prev.at));
    let when = row
        .when()
        .map(|w| w.rhythm())
        .unwrap_or_else(|| "when someone runs it".into());
    let text = sch::prompt(
        &row.prompt,
        &RunNote {
            name: &row.name,
            when: &when,
            timezone: &row.timezone,
            owner: &row.owner_name(),
            by,
            now: &sch::stamp(&*zone, ctx.now),
            previous: previous
                .as_ref()
                .zip(previous_at.as_deref())
                .map(|(prev, at)| (prev.number, at)),
            context: context.as_deref(),
        },
    );
    let (session, number) = schedules::start_session(
        &ctx.env,
        &ctx.db,
        Start {
            project: &p.row,
            schedule: &row,
            user: &user,
            credential: &credential,
            github_access,
            prompt: text,
            author: (!token).then(|| user.to_ref()),
            source: crate::api::sessions::source_of(ctx),
            // Sessions a limited token starts notify nobody unless someone
            // follows them, as `api::sessions::create`'s.
            follow: ctx.limits().is_none(),
            now: ctx.now,
        },
    )
    .await?;
    let run_id = Outcome::Started(session.clone())
        .record(&ctx.db, &row, ctx.now, ctx.now, trigger, Some(&user.id))
        .await?;
    ctx.audit_in(
        Some(p.workspace_id()),
        "session.created",
        Some(session.as_str()),
        Some(json!({
            "project": p.row.slug,
            "number": number,
            "github_access": github_access,
            "schedule": row.id,
            "token": ctx.auth.as_ref().filter(|a| a.kind == TokenKind::Pat).map(|a| &a.token_id),
        })),
    );
    let run = schedules::run_row(&ctx.db, &run_id)
        .await?
        .ok_or_else(|| internal("a schedule's run vanished"))?;
    http::created(&run.to_api())
}
