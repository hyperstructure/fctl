//! Retrospectives (`fctl retro`, docs/architecture.md#retrospectives): what
//! agents record as they finish a piece of their session's work, for the
//! project's people. Agents record them through their host
//! ([`crate::api::calls`]); everyone who sees a session reads its
//! retrospectives, everyone who sees a project reads them all, and the
//! people who manage a session delete one.

use crate::access;
use crate::api::calls::RunnerSession;
use crate::api::memories::person;
use crate::ctx::Ctx;
use crate::db::{n, os, q, s};
use crate::error::{AppResult, conflict, forbidden, internal, not_found, validation};
use crate::http;
use fctl_domain::retrospectives::{self as retros, Parts};
use fctl_proto::api::{Ok, OutcomeCounts, RetrospectiveList};
use fctl_proto::session::{Retrospective, RetrospectiveOutcome, SessionRef};
use fctl_proto::{ProjectId, RetrospectiveId, SessionId};
use serde::Deserialize;
use worker::{D1Database, Request, Response};

/// Retrospectives with their sessions and who started them.
const SELECT: &str = "SELECT r.id, r.project_id, r.session_id, s.number AS session_number, s.title AS session_title, \
    s.created_by, u.name AS starter_name, u.avatar_url AS starter_avatar, \
    r.created_at, r.outcome, r.worked_on, r.approach, r.challenges, r.insights \
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

/// `GET /api/v1/sessions/{id}/retrospectives`: for everyone who sees the
/// session, newest first.
pub async fn session_list(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = access::session(ctx, id).await?;
    let rows: Vec<Row> = q(
        &ctx.db,
        &format!("{SELECT} WHERE r.session_id = ? ORDER BY r.created_at DESC, r.id DESC"),
        &[s(&a.row.id)],
    )?
    .all()
    .await?;
    http::ok(&RetrospectiveList {
        retrospectives: rows.into_iter().map(Row::into_retrospective).collect(),
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
/// left out, with how many ended each way.
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
    if let Some(before) = http::query(req, "before").and_then(|b| b.parse::<i64>().ok()) {
        params.push(n(before));
        sql.push_str(&format!(" AND r.created_at < ?{}", params.len()));
    }
    params.push(n(limit + 1));
    sql.push_str(&format!(
        " ORDER BY r.created_at DESC, r.id DESC LIMIT ?{}",
        params.len()
    ));
    let mut rows: Vec<Row> = q(&ctx.db, &sql, &params)?.all().await?;
    let more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let next_before = if more {
        rows.last().map(|r| r.created_at)
    } else {
        None
    };
    let counts: Vec<OutcomeRow> = q(
        &ctx.db,
        "SELECT r.outcome, COUNT(*) AS n FROM retrospectives r JOIN sessions s ON s.id = r.session_id \
         WHERE r.project_id = ? AND s.archived = 0 GROUP BY r.outcome",
        &[s(&p.row.id)],
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
    http::ok(&RetrospectiveList {
        retrospectives: rows.into_iter().map(Row::into_retrospective).collect(),
        next_before,
        outcomes: Some(outcomes),
    })
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
