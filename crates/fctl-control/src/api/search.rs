//! `GET /api/v1/search`: the sessions the caller sees whose title,
//! description, people's messages or agent's replies contain what they
//! typed (`fctl_domain::search`).
//!
//! Each workspace's SearchObject keeps the index of its sessions. The
//! Worker asks those of the caller's workspaces to look among the projects
//! the caller sees there and the sessions shared with them, then reads the
//! sessions it found from D1, as the caller sees them now.

use crate::access;
use crate::ctx::Ctx;
use crate::db::{q, s};
use crate::error::{AppError, AppResult};
use crate::http;
use crate::internal::{self, IndexHit, IndexHits, SearchIndex};
use crate::model::{PROJECT_VISIBLE, SESSION_SELECT, SessionRow, WorkspaceCols};
use fctl_domain::search;
use fctl_proto::api::{SearchResults, SessionMatches};
use fctl_proto::session::ProjectRole;
use std::collections::{BTreeMap, HashMap};
use worker::{Request, Response};

/// Sessions a search returns, unless it asks for another number.
const DEFAULT_LIMIT: u32 = 20;
/// The most matches a result counts in its session.
const MAX_COUNT: u32 = 100;

/// A project, with what decides whether the caller sees it.
#[derive(serde::Deserialize)]
struct PlaceRow {
    id: String,
    member_role: Option<String>,
    #[serde(flatten)]
    workspace: WorkspaceCols,
}

/// What the caller may search in a workspace: its projects they see, and
/// the sessions of its other projects shared with them.
#[derive(Default)]
struct Scope {
    projects: Vec<String>,
    sessions: Vec<String>,
}

pub async fn search(ctx: &Ctx, req: &Request) -> AppResult<Response> {
    let user = ctx.user()?;
    let settings = ctx.settings();
    let q_text = search::query(&http::query(req, "q").unwrap_or_default()).map_err(AppError)?;
    let limit = http::query(req, "limit")
        .and_then(|l| l.parse::<u32>().ok())
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, search::MAX_RESULTS);
    let workspace = http::query(req, "workspace");
    // A project they can't see is one that doesn't exist.
    let project = match http::query(req, "project") {
        Some(p) => Some(access::project(ctx, &p).await?.row.id),
        None => None,
    };

    let mut scopes: BTreeMap<String, Scope> = BTreeMap::new();
    let mut sql = format!(
        "SELECT p.id, m.role AS member_role, w.id AS workspace_id, w.base_role AS workspace_base_role, \
         w.require_two_factor AS workspace_two_factor, wm.role AS workspace_role \
         FROM projects p JOIN workspaces w ON w.id = p.workspace_id \
         LEFT JOIN workspace_members wm ON wm.workspace_id = p.workspace_id AND wm.user_id = ?1 \
         LEFT JOIN project_members m ON m.project_id = p.id AND m.user_id = ?1 \
         WHERE {PROJECT_VISIBLE}"
    );
    let mut params = vec![s(&user.id)];
    if let Some(w) = &workspace {
        params.push(s(w));
        sql.push_str(&format!(
            " AND (w.id = ?{n} OR w.slug = ?{n})",
            n = params.len()
        ));
    }
    if let Some(p) = &project {
        params.push(s(p));
        sql.push_str(&format!(" AND p.id = ?{}", params.len()));
    }
    sql.push_str(&access::places_sql(ctx.limits(), &mut params));
    let places: Vec<PlaceRow> = q(&ctx.db, &sql, &params)?.all().await?;
    for p in places {
        let role = p.member_role.as_deref().and_then(ProjectRole::parse);
        if access::project_role_in(user, &settings, ctx.limits(), &p.workspace, &p.id, role)
            .is_some()
            && let Some(w) = p.workspace.workspace_id
        {
            scopes.entry(w).or_default().projects.push(p.id);
        }
    }
    if project.is_none() {
        let mut sql = format!(
            "{SESSION_SELECT} WHERE sh.role IS NOT NULL AND wm.user_id IS NOT NULL AND rv.session_id IS NULL"
        );
        let mut params = vec![s(&user.id)];
        if let Some(w) = &workspace {
            params.push(s(w));
            sql.push_str(&format!(
                " AND (w.id = ?{n} OR w.slug = ?{n})",
                n = params.len()
            ));
        }
        sql.push_str(&access::places_sql(ctx.limits(), &mut params));
        let shared: Vec<SessionRow> = q(&ctx.db, &sql, &params)?.all().await?;
        for r in shared {
            let Some(w) = r.workspace.workspace_id.clone() else {
                continue;
            };
            let scope = scopes.entry(w).or_default();
            if !scope.projects.contains(&r.project_id)
                && access::session_role(user, &settings, ctx.limits(), &r).is_some()
            {
                scope.sessions.push(r.id);
            }
        }
    }

    // Each workspace's index, at once.
    let asks = scopes.into_iter().map(|(workspace, scope)| {
        let body = SearchIndex {
            q: q_text.clone(),
            projects: scope.projects,
            sessions: scope.sessions,
            limit,
        };
        async move {
            let stub = internal::search_stub(&ctx.env, &workspace)?;
            internal::post::<_, IndexHits>(&stub, "/search", &body).await
        }
    });
    let answers = futures::future::join_all(asks).await;
    let asked = answers.len();
    let mut hits: Vec<IndexHit> = Vec::new();
    let mut failures = Vec::new();
    for a in answers {
        match a {
            Ok(h) => hits.extend(h.sessions),
            Err(e) => {
                worker::console_error!("searching a workspace's index: {}", e.0.message);
                failures.push(e);
            }
        }
    }
    // Some workspaces' results are better than none; none at all is an error.
    if failures.len() == asked
        && let Some(e) = failures.pop()
    {
        return Err(e);
    }
    hits.sort_by(|a, b| b.titled.cmp(&a.titled).then(b.last.cmp(&a.last)));
    let more = hits.len() > limit as usize;
    hits.truncate(limit as usize);

    // The sessions as the caller sees them now.
    let ids: Vec<&str> = hits.iter().map(|h| h.session_id.as_str()).collect();
    let rows: Vec<SessionRow> = if ids.is_empty() {
        Vec::new()
    } else {
        q(
            &ctx.db,
            &format!(
                "{SESSION_SELECT} WHERE s.id IN (SELECT value FROM json_each(?2)) AND rv.session_id IS NULL"
            ),
            &[s(&user.id), s(&serde_json::to_string(&ids)?)],
        )?
        .all()
        .await?
    };
    let mut by_id: HashMap<String, SessionRow> =
        rows.into_iter().map(|r| (r.id.clone(), r)).collect();
    let sessions = hits
        .into_iter()
        .filter_map(|h| {
            let row = by_id.remove(&h.session_id)?;
            let role = access::session_role(user, &settings, ctx.limits(), &row)?;
            Some(SessionMatches {
                session: row.to_session(Some(role)),
                matches: h.matches,
                count: h.count.min(MAX_COUNT),
            })
        })
        .collect();
    http::ok(&SearchResults { sessions, more })
}
