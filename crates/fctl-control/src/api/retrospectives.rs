//! Retrospectives (`fctl retro`, docs/architecture.md#retrospectives): what
//! agents record as they finish a piece of their session's work, for the
//! project's people, and update as later turns carry on with it. Agents
//! record, list and update their session's through their host
//! ([`crate::api::calls`]); everyone who sees a session reads its
//! retrospectives, everyone who sees a project reads them all, and the
//! people who manage a session delete one.

use crate::access;
use crate::api::calls::RunnerSession;
use crate::api::memories::person;
use crate::ctx::Ctx;
use crate::db::{n, os, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, internal, not_found, validation};
use crate::http;
use fctl_domain::retrospectives::{self as retros, Parts};
use fctl_proto::api::{Ok, OutcomeCounts, RetrospectiveList};
use fctl_proto::session::{Retrospective, RetrospectiveOutcome, SessionRef};
use fctl_proto::{ApiError, ErrorCode, ProjectId, RetrospectiveId, SessionId};
use serde::Deserialize;
use worker::{D1Database, Request, Response};

/// Retrospectives with their sessions and who started them.
const SELECT: &str = "SELECT r.id, r.project_id, r.session_id, s.number AS session_number, s.title AS session_title, \
    s.created_by, u.name AS starter_name, u.avatar_url AS starter_avatar, \
    r.created_at, r.updated_at, r.outcome, r.worked_on, r.approach, r.challenges, r.insights \
    FROM retrospectives r JOIN sessions s ON s.id = r.session_id LEFT JOIN users u ON u.id = s.created_by";

#[derive(Deserialize)]
struct Row {
    id: String,
    project_id: String,
    session_id: String,
    #[serde(deserialize_with = "crate::db::int")]
    session_number: i64,
    session_title: String,
    created_by: Option<String>,
    starter_name: Option<String>,
    starter_avatar: Option<String>,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    #[serde(default, deserialize_with = "crate::db::opt_int")]
    updated_at: Option<i64>,
    outcome: String,
    worked_on: String,
    approach: Option<String>,
    challenges: Option<String>,
    insights: Option<String>,
}

impl Row {
    fn into_retrospective(self) -> Retrospective {
        Retrospective {
            id: RetrospectiveId(self.id),
            project_id: ProjectId(self.project_id),
            session: SessionRef {
                id: SessionId(self.session_id),
                number: self.session_number.max(0) as u32,
                title: self.session_title,
            },
            started_by: person(self.created_by, self.starter_name, self.starter_avatar),
            created_at: self.created_at,
            updated_at: self.updated_at,
            // Only these are stored; a row from a later version reads as done.
            outcome: retros::parse_outcome(&self.outcome)
                .unwrap_or(RetrospectiveOutcome::Completed),
            worked_on: self.worked_on,
            approach: self.approach,
            challenges: self.challenges,
            insights: self.insights,
        }
    }
}

async fn one(db: &D1Database, id: &str) -> AppResult<Option<Retrospective>> {
    if !RetrospectiveId::is_valid(id) {
        return Ok(None);
    }
    let row: Option<Row> = q(db, &format!("{SELECT} WHERE r.id = ?"), &[s(id)])?
        .first()
        .await?;
    Ok(row.map(Row::into_retrospective))
}

/// What an agent records (`fctl retro`), for its session.
pub async fn record(
    db: &D1Database,
    session: &RunnerSession,
    outcome: RetrospectiveOutcome,
    parts: Parts,
    now: i64,
) -> AppResult<Retrospective> {
    let id = fctl_domain::ids::retrospective_id(now);
    let added = q(
        db,
        "INSERT INTO retrospectives (id, project_id, session_id, created_at, outcome, worked_on, approach, challenges, insights) \
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9 WHERE (SELECT COUNT(*) FROM retrospectives WHERE session_id = ?3) < ?10",
        &[
            s(id.as_str()),
            s(&session.project_id),
            s(&session.id),
            n(now),
            s(retros::outcome_name(outcome)),
            s(&parts.worked_on),
            os(parts.approach.as_deref()),
            os(parts.challenges.as_deref()),
            os(parts.insights.as_deref()),
            n(i64::from(retros::MAX_PER_SESSION)),
        ],
    )?
    .run()
    .await?;
    if added == 0 {
        return Err(conflict(&format!(
            "A session records at most {} retrospectives",
            retros::MAX_PER_SESSION
        )));
    }
    one(db, id.as_str())
        .await?
        .ok_or_else(|| internal("a retrospective vanished as it was recorded"))
}

/// What an agent updates (`fctl retro update`): a retrospective its
/// session recorded, as a later turn carries on with that piece of work.
/// The parts the update leaves out stay.
pub async fn update(
    db: &D1Database,
    session: &RunnerSession,
    id: &RetrospectiveId,
    change: &retros::Change<'_>,
    now: i64,
) -> AppResult<Retrospective> {
    let not_recorded = || {
        AppError(ApiError::new(
            ErrorCode::NotFound,
            format!(
                "This session recorded no retrospective {id}: `fctl retro list` lists those it did"
            ),
        ))
    };
    let r = one(db, id.as_str())
        .await?
        .filter(|r| r.session.id.as_str() == session.id)
        .ok_or_else(not_recorded)?;
    let (outcome, parts) = retros::update(&r, change)?;
    let changed = q(
        db,
        "UPDATE retrospectives SET outcome = ?1, worked_on = ?2, approach = ?3, challenges = ?4, insights = ?5, updated_at = ?6 \
         WHERE id = ?7 AND session_id = ?8",
        &[
            s(retros::outcome_name(outcome)),
            s(&parts.worked_on),
            os(parts.approach.as_deref()),
            os(parts.challenges.as_deref()),
            os(parts.insights.as_deref()),
            n(now),
            s(id.as_str()),
            s(&session.id),
        ],
    )?
    .run()
    .await?;
    // Someone deleted it in the meantime.
    if changed == 0 {
        return Err(not_recorded());
    }
    one(db, id.as_str())
        .await?
        .ok_or_else(|| internal("a retrospective vanished as it was updated"))
}

/// A session's retrospectives, newest first: for its agent, and for
/// everyone who sees it.
pub async fn of_session(db: &D1Database, session_id: &str) -> AppResult<Vec<Retrospective>> {
    let rows: Vec<Row> = q(
        db,
        &format!("{SELECT} WHERE r.session_id = ? ORDER BY r.id DESC"),
        &[s(session_id)],
    )?
    .all()
    .await?;
    Ok(rows.into_iter().map(Row::into_retrospective).collect())
}

/// `GET /api/v1/sessions/{id}/retrospectives`: for everyone who sees the
/// session, newest first.
pub async fn session_list(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = access::session(ctx, id).await?;
    http::ok(&RetrospectiveList {
        retrospectives: of_session(&ctx.db, &a.row.id).await?,
        next_before: None,
        outcomes: None,
    })
}

#[derive(Deserialize)]
struct OutcomeRow {
    outcome: String,
    #[serde(deserialize_with = "crate::db::int")]
    n: i64,
}

/// `GET /api/v1/projects/{id}/retrospectives?outcome=&before=&limit=`: for
/// everyone who sees the project, newest first, those of archived sessions
/// left out. `before` is the id of the last one of the page before: ids
/// sort as retrospectives were recorded, without ties. The first page says
/// how many ended each way.
pub async fn project_list(ctx: &Ctx, req: &Request, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    let limit = http::query(req, "limit")
        .and_then(|l| l.parse::<i64>().ok())
        .unwrap_or(30)
        .clamp(1, 100);
    let mut sql = format!("{SELECT} WHERE r.project_id = ?1 AND s.archived = 0");
    let mut params = vec![s(&p.row.id)];
    if let Some(outcome) = http::query(req, "outcome") {
        let outcome = retros::parse_outcome(&outcome)
            .ok_or_else(|| validation("outcome", "Choose completed, partial or blocked"))?;
        params.push(s(retros::outcome_name(outcome)));
        sql.push_str(&format!(" AND r.outcome = ?{}", params.len()));
    }
    let before = http::query(req, "before");
    if let Some(before) = &before {
        let before = RetrospectiveId::parse(before)
            .ok_or_else(|| validation("before", "Give the id of a retrospective"))?;
        params.push(s(before.as_str()));
        sql.push_str(&format!(" AND r.id < ?{}", params.len()));
    }
    params.push(n(limit + 1));
    sql.push_str(&format!(" ORDER BY r.id DESC LIMIT ?{}", params.len()));
    let mut rows: Vec<Row> = q(&ctx.db, &sql, &params)?.all().await?;
    let more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let next_before = if more {
        rows.last().map(|r| RetrospectiveId(r.id.clone()))
    } else {
        None
    };
    let outcomes = if before.is_none() {
        Some(outcome_counts(&ctx.db, &p.row.id).await?)
    } else {
        None
    };
    http::ok(&RetrospectiveList {
        retrospectives: rows.into_iter().map(Row::into_retrospective).collect(),
        next_before,
        outcomes,
    })
}

/// How many of a project's retrospectives ended each way, those of
/// archived sessions left out.
async fn outcome_counts(db: &D1Database, project_id: &str) -> AppResult<OutcomeCounts> {
    let counts: Vec<OutcomeRow> = q(
        db,
        "SELECT r.outcome, COUNT(*) AS n FROM retrospectives r JOIN sessions s ON s.id = r.session_id \
         WHERE r.project_id = ? AND s.archived = 0 GROUP BY r.outcome",
        &[s(project_id)],
    )?
    .all()
    .await?;
    let mut outcomes = OutcomeCounts::default();
    for c in counts {
        let count = c.n.max(0) as u32;
        match retros::parse_outcome(&c.outcome) {
            Some(RetrospectiveOutcome::Completed) => outcomes.completed += count,
            Some(RetrospectiveOutcome::Partial) => outcomes.partial += count,
            Some(RetrospectiveOutcome::Blocked) => outcomes.blocked += count,
            None => {}
        }
    }
    Ok(outcomes)
}

/// `DELETE /api/v1/retrospectives/{id}`: for people who manage its
/// session, as for its published screenshots.
pub async fn delete(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    ctx.user()?;
    let r = one(&ctx.db, id)
        .await?
        .ok_or_else(|| not_found("Retrospective"))?;
    let a = access::session(ctx, r.session.id.as_str())
        .await
        .map_err(|_| not_found("Retrospective"))?;
    if !a.role.can_manage() {
        return Err(forbidden(
            "Only people who manage this session can delete its retrospectives",
        ));
    }
    q(
        &ctx.db,
        "DELETE FROM retrospectives WHERE id = ?",
        &[s(r.id.as_str())],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(a.workspace_id()),
        "retrospective.deleted",
        Some(r.id.as_str()),
        Some(serde_json::json!({ "session_id": r.session.id, "project_id": r.project_id })),
    );
    http::ok(&Ok { ok: true })
}
