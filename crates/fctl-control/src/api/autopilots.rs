//! A project's autopilot (experimental), `/api/v1/projects/{id}/autopilot`
//! (`crate::autopilots`): putting a project on autopilot and changing,
//! pausing, resuming, waking and taking it off; the notes people steer it
//! with and their answers to what it asks; its log, plan and asks, which
//! its own session writes; and what waits for the caller across their
//! workspaces (`/api/v1/autopilots/asks`).
//!
//! Everyone who sees the project sees its autopilot, except that what a
//! workspace's autopilot does (its plan, notes, asks, sessions, log, brief
//! and own session) reaches projects some of them don't see: it is its
//! workspace's admins' ([`AutopilotCan::read`]), and others see only that
//! it runs, and how; it stays a workspace's autopilot. Those who may steer
//! it ([`AutopilotCan::steer`]: the project's maintainers and owners, and
//! for a workspace's autopilot its workspace's admins) steer it, answer
//! it, wake, pause and resume it, change and take it off. It runs on its
//! owner's Claude subscription: changing how much it works (its check-in,
//! how many sessions, its model), resuming it or taking it over runs it on
//! the changer's from then on, as for schedules. Its own session, acting as
//! its owner, writes its log and plan, asks people, says when to wake and
//! pauses itself (`fctl_domain::autopilot::allows`).

use crate::access::{self, ProjectAccess};
use crate::autopilots::{self as engine, AutopilotRow, Entry};
use crate::ctx::Ctx;
use crate::db::{int, n, on, opt_int, os, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, not_found, validation};
use crate::http;
use crate::model::{PROJECT_SELECT, PROJECT_VISIBLE, ProjectRow};
use fctl_domain::autopilot as ap;
use fctl_domain::verify::Sensitive;
use fctl_proto::autopilot::*;
use fctl_proto::session::{SessionRef, WorkspaceRole};
use fctl_proto::{ProjectId, SessionId};
use serde::Deserialize;
use worker::{Request, Response};

/// How long answered asks stay in the autopilot's list.
const ASKS_KEPT_MS: i64 = 7 * 86_400_000;

/// Why a change or a resume that started from whose it was didn't happen:
/// someone took it over, or its owner changed whether they keep it,
/// meanwhile.
const MOVED_ON: &str =
    "Whose subscription it runs on changed meanwhile: look again, then try again.";

/// The project and its autopilot, if it has one, with what the caller may
/// do.
struct Loaded {
    p: ProjectAccess,
    row: Option<AutopilotRow>,
    can: AutopilotCan,
}

async fn load(ctx: &Ctx, id: &str) -> AppResult<Loaded> {
    let p = access::project(ctx, id).await?;
    let row = engine::row(&ctx.db, &p.row.id).await?;
    let can = can(ctx, &p, row.as_ref()).await?;
    Ok(Loaded { p, row, can })
}

/// What the caller may do with the autopilot of `p` (one it would put on
/// when it has none).
async fn can(ctx: &Ctx, p: &ProjectAccess, row: Option<&AutopilotRow>) -> AppResult<AutopilotCan> {
    let manages = fctl_domain::perm::can_manage_project(p.role);
    let steer = match row.map(AutopilotRow::scope) {
        Some(AutopilotScope::Workspace) => manages && is_admin(ctx, p.workspace_id()).await?,
        _ => manages,
    };
    // A workspace's autopilot works across the workspace: what it does is
    // for its admins, who steer it. The autopilot sees its own.
    let read = match row.map(AutopilotRow::scope) {
        Some(AutopilotScope::Workspace) => ctx.autopilot().is_some() || steer,
        _ => true,
    };
    Ok(AutopilotCan {
        steer: steer && ctx.autopilot().is_none(),
        secrets: manages && ctx.autopilot().is_none(),
        read,
    })
}

async fn is_admin(ctx: &Ctx, workspace_id: &str) -> AppResult<bool> {
    let user = ctx.user()?;
    Ok(crate::workspaces::member(&ctx.db, workspace_id, &user.id)
        .await?
        .and_then(|m| WorkspaceRole::parse(&m.role))
        .is_some_and(|r| matches!(r, WorkspaceRole::Owner | WorkspaceRole::Admin)))
}

fn require(row: Option<AutopilotRow>) -> AppResult<AutopilotRow> {
    row.filter(|r| r.state() != AutopilotState::Off)
        .ok_or_else(|| not_found("Autopilot"))
}

fn require_steer(l: &Loaded) -> AppResult<()> {
    if l.can.steer {
        Ok(())
    } else if ctx_is_autopilot(l) {
        Err(forbidden("An autopilot doesn't steer itself: people do."))
    } else {
        Err(forbidden(
            "Only those who manage the project steer its autopilot: its maintainers and owners, and for a workspace's autopilot the workspace's admins.",
        ))
    }
}

fn ctx_is_autopilot(l: &Loaded) -> bool {
    !l.can.secrets && !l.can.steer && fctl_domain::perm::can_manage_project(l.p.role)
}

/// The caller is the autopilot's own session.
fn require_pilot(ctx: &Ctx, l: &Loaded) -> AppResult<()> {
    match ctx.autopilot() {
        Some(a) if a.project_id == l.p.row.id => Ok(()),
        _ => Err(forbidden(
            "The autopilot writes this from its own session: people steer it with notes.",
        )),
    }
}

// ───────────────────────────── reading ─────────────────────────────

/// `GET /api/v1/projects/{id}/autopilot`.
pub async fn get(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let l = load(ctx, id).await?;
    // One that is off too: it keeps its direction, notes, plan, log and
    // asks for when the project is put on autopilot again.
    let row = l.row.clone().ok_or_else(|| not_found("Autopilot"))?;
    http::ok(&view(ctx, &l.p, &row, l.can).await?)
}

/// The autopilot as people see it.
async fn view(
    ctx: &Ctx,
    p: &ProjectAccess,
    row: &AutopilotRow,
    can: AutopilotCan,
) -> AppResult<Autopilot> {
    let db = &ctx.db;
    let notes = notes(ctx, &row.project_id).await?;
    let asks = asks(ctx, &row.project_id, ctx.now - ASKS_KEPT_MS).await?;
    let crew: Vec<engine::CrewRow> = q(
        db,
        &format!(
            "{} WHERE s.autopilot_id = ? AND s.autopilot_role = 'crew' \
             ORDER BY CASE WHEN s.status IN ('queued','provisioning','preparing','running','idle','paused','stopping') THEN 0 ELSE 1 END, \
             s.created_at DESC LIMIT 20",
            engine::CREW_SELECT
        ),
        &[s(&row.project_id)],
    )?
    .all()
    .await?;
    #[derive(Deserialize)]
    struct Count {
        #[serde(deserialize_with = "int", default)]
        n: i64,
    }
    let total: Option<Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM sessions WHERE autopilot_id = ? AND autopilot_role = 'crew'",
        &[s(&row.project_id)],
    )?
    .first()
    .await?;
    let problem = engine::problem(db, &p.row, row, ctx.now).await?;
    let a = Autopilot {
        project_id: ProjectId(row.project_id.clone()),
        project_slug: p.row.slug.clone(),
        scope: row.scope(),
        state: row.state(),
        paused_reason: row.paused_reason.clone(),
        paused_by: row.paused_by_ref(),
        paused_at: row.paused_at,
        owner: row.owner_ref(),
        keep_owner: row.keep_owner,
        direction: row.direction.clone(),
        direction_by: row.direction_by_ref(),
        direction_at: row.direction_at,
        plan: row.plan.clone(),
        plan_at: row.plan_at,
        powers: row.powers(),
        check_in_minutes: row.check_in(),
        crew_max: row.crew_max(),
        model: row.model.clone(),
        effort: row.effort.clone(),
        session: match (
            &row.session_id,
            row.session_number,
            row.session_status(),
            row.session_created_at,
        ) {
            (Some(id), Some(number), Some(status), Some(started_at)) => Some(AutopilotSession {
                id: SessionId(id.clone()),
                number: u32::try_from(number).unwrap_or(0),
                status,
                started_at,
            }),
            _ => None,
        },
        wake_at: row.wake_at.filter(|_| row.state() == AutopilotState::On),
        wake_reason: row
            .wake_reason()
            .filter(|_| row.state() == AutopilotState::On),
        woken_at: row.woken_at,
        check_ins: u32::try_from(row.check_ins).unwrap_or(0),
        problem,
        notes,
        asks,
        crew: crew.iter().map(crew_of).collect(),
        crew_total: total.map_or(0, |t| u32::try_from(t.n).unwrap_or(0)),
        can,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };
    if can.read {
        return Ok(a);
    }
    // Who doesn't see what it does sees that it runs, and how: not why it
    // paused, which can tell of its work.
    Ok(Autopilot {
        paused_reason: None,
        direction: String::new(),
        direction_by: None,
        plan: None,
        plan_at: None,
        session: None,
        notes: Vec::new(),
        asks: Vec::new(),
        crew: Vec::new(),
        crew_total: 0,
        ..a
    })
}

/// What it does, for those who see it ([`AutopilotCan::read`]).
fn require_read(l: &Loaded) -> AppResult<()> {
    if l.can.read {
        Ok(())
    } else {
        Err(forbidden(
            "What a workspace's autopilot does reaches all the workspace's projects: its admins see it.",
        ))
    }
}

fn crew_of(c: &engine::CrewRow) -> AutopilotCrew {
    AutopilotCrew {
        id: SessionId(c.id.clone()),
        project_id: ProjectId(c.project_id.clone()),
        project_slug: c.project_slug.clone(),
        number: u32::try_from(c.number).unwrap_or(0),
        title: c.title.clone(),
        status: c.status(),
        question: c.question_text(),
        pull_requests: c.pull_requests(),
        created_at: c.created_at,
        last_activity_at: c.last_activity_at,
    }
}

#[derive(Deserialize)]
struct NoteRow {
    id: String,
    text: String,
    author_id: Option<String>,
    author_name: Option<String>,
    author_avatar: Option<String>,
    #[serde(deserialize_with = "int")]
    created_at: i64,
    #[serde(deserialize_with = "opt_int", default)]
    seen_at: Option<i64>,
}

async fn notes(ctx: &Ctx, project_id: &str) -> AppResult<Vec<AutopilotNote>> {
    let rows: Vec<NoteRow> = q(
        &ctx.db,
        "SELECT n.id, n.text, n.author_id, u.name AS author_name, u.avatar_url AS author_avatar, n.created_at, n.seen_at \
         FROM autopilot_notes n LEFT JOIN users u ON u.id = n.author_id WHERE n.project_id = ? \
         ORDER BY n.created_at DESC, n.id DESC",
        &[s(project_id)],
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| AutopilotNote {
            author: engine::user_ref(
                r.author_id.as_deref(),
                r.author_name.as_deref(),
                &r.author_avatar,
            ),
            id: r.id,
            text: r.text,
            created_at: r.created_at,
            seen_at: r.seen_at,
        })
        .collect())
}

/// Its open asks, then those closed since `since`, newest first.
async fn asks(ctx: &Ctx, project_id: &str, since: i64) -> AppResult<Vec<AutopilotAsk>> {
    let rows: Vec<engine::AskRow> = q(
        &ctx.db,
        &format!(
            "{} WHERE k.project_id = ? AND (k.status = 'open' OR COALESCE(k.answered_at, k.created_at) > ?) \
             ORDER BY CASE WHEN k.status = 'open' THEN 0 ELSE 1 END, k.created_at DESC LIMIT 40",
            engine::ASK_SELECT
        ),
        &[s(project_id), n(since)],
    )?
    .all()
    .await?;
    let mut out = Vec::new();
    for r in rows {
        out.push(ask_of(ctx, &r).await?);
    }
    Ok(out)
}

async fn ask_of(ctx: &Ctx, r: &engine::AskRow) -> AppResult<AutopilotAsk> {
    let secret = match (&r.secret_name, &r.secret_project_id) {
        (Some(name), Some(project)) => {
            #[derive(Deserialize)]
            struct Found {
                #[allow(dead_code)]
                name: String,
            }
            let exists: Option<Found> = q(
                &ctx.db,
                "SELECT name FROM project_secrets WHERE project_id = ? AND name = ?",
                &[s(project), s(name)],
            )?
            .first()
            .await?;
            Some(AskedSecret {
                name: name.clone(),
                project_id: ProjectId(project.clone()),
                project_slug: r.secret_project_slug.clone().unwrap_or_default(),
                exists: exists.is_some(),
            })
        }
        _ => None,
    };
    Ok(AutopilotAsk {
        id: r.id.clone(),
        kind: r.kind(),
        text: r.text.clone(),
        detail: r.detail.clone(),
        options: r.options(),
        secret,
        status: r.status(),
        answer: r.answer.clone(),
        answered_by: engine::user_ref(
            r.answered_by.as_deref(),
            r.answered_by_name.as_deref(),
            &r.answered_by_avatar,
        ),
        answered_at: r.answered_at,
        session: match (&r.session_id, r.session_number) {
            (Some(id), Some(number)) => Some(SessionRef {
                id: SessionId(id.clone()),
                number: u32::try_from(number).unwrap_or(0),
                title: r.session_title.clone().unwrap_or_default(),
            }),
            _ => None,
        },
        created_at: r.created_at,
    })
}

/// `GET …/autopilot/log`: its log, newest first, a page at a time
/// (`?before=` the last entry's id, `?limit=` up to 200).
pub async fn log(ctx: &mut Ctx, req: &Request, id: &str) -> AppResult<Response> {
    let l = load(ctx, id).await?;
    require_read(&l)?;
    let row = l.row.ok_or_else(|| not_found("Autopilot"))?;
    let limit = http::query(req, "limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(50)
        .clamp(1, 200);
    #[derive(Deserialize)]
    struct EntryRow {
        id: String,
        #[serde(deserialize_with = "int")]
        at: i64,
        kind: String,
        text: String,
        user_id: Option<String>,
        user_name: Option<String>,
        user_avatar: Option<String>,
        session_id: Option<String>,
        #[serde(deserialize_with = "opt_int", default)]
        session_number: Option<i64>,
        session_title: Option<String>,
        project_slug: Option<String>,
        from_session_id: Option<String>,
    }
    let mut params = vec![s(&row.project_id)];
    let mut sql = "SELECT e.id, e.at, e.kind, e.text, e.user_id, u.name AS user_name, u.avatar_url AS user_avatar, \
         e.session_id, es.number AS session_number, es.title AS session_title, op.slug AS project_slug, e.from_session_id \
         FROM autopilot_log e LEFT JOIN users u ON u.id = e.user_id LEFT JOIN sessions es ON es.id = e.session_id \
         LEFT JOIN projects op ON op.id = e.other_project_id WHERE e.project_id = ?1"
        .to_owned();
    if let Some(before) = http::query(req, "before") {
        params.push(s(&before));
        sql.push_str(
            " AND (e.at, e.id) < (SELECT at, id FROM autopilot_log WHERE id = ?2 AND project_id = ?1)",
        );
    }
    params.push(n(limit + 1));
    sql.push_str(&format!(
        " ORDER BY e.at DESC, e.id DESC LIMIT ?{}",
        params.len()
    ));
    let mut rows: Vec<EntryRow> = q(&ctx.db, &sql, &params)?.all().await?;
    let has_more = rows.len() as i64 > limit;
    rows.truncate(usize::try_from(limit).unwrap_or(50));
    http::ok(&AutopilotLog {
        entries: rows
            .into_iter()
            .map(|r| AutopilotEntry {
                by: engine::user_ref(r.user_id.as_deref(), r.user_name.as_deref(), &r.user_avatar),
                session: match (r.session_id, r.session_number) {
                    (Some(id), Some(number)) => Some(SessionRef {
                        id: SessionId(id),
                        number: u32::try_from(number).unwrap_or(0),
                        title: r.session_title.unwrap_or_default(),
                    }),
                    _ => None,
                },
                id: r.id,
                at: r.at,
                kind: AutopilotEntryKind::parse(&r.kind),
                text: ap::said(&r.text, r.user_name.as_deref()),
                project_slug: r.project_slug,
                from: r.from_session_id.map(SessionId),
            })
            .collect(),
        has_more,
    })
}

/// `GET …/autopilot/brief`: where things stand, as Markdown for its own
/// session (`fctl autopilot brief`), which then has seen its notes and
/// answers. People read it too.
pub async fn brief(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let l = load(ctx, id).await?;
    let row = require(l.row.clone())?;
    require_read(&l)?;
    let (text, shown) = brief_of(ctx, &l.p.row, &row).await?;
    if ctx.autopilot().is_some() {
        engine::mark_seen(&ctx.db, &row.project_id, &shown.notes, &shown.asks, ctx.now).await?;
    }
    http::ok(&AutopilotBrief { brief: text })
}

/// What a brief showed of what people told the autopilot: its notes, and
/// the closed asks, whose answers it has then heard of.
struct Shown {
    notes: Vec<String>,
    asks: Vec<String>,
}

async fn brief_of(
    ctx: &Ctx,
    project: &ProjectRow,
    row: &AutopilotRow,
) -> AppResult<(String, Shown)> {
    let db = &ctx.db;
    let notes = notes(ctx, &row.project_id).await?;
    let ask_rows: Vec<engine::AskRow> = q(
        db,
        &format!(
            "{} WHERE k.project_id = ? AND (k.status = 'open' OR COALESCE(k.answered_at, k.created_at) > ?) \
             ORDER BY CASE WHEN k.status = 'open' THEN 0 ELSE 1 END, k.created_at DESC LIMIT 20",
            engine::ASK_SELECT
        ),
        &[s(&row.project_id), n(ctx.now - 2 * 86_400_000)],
    )?
    .all()
    .await?;
    let crew: Vec<engine::CrewRow> = q(
        db,
        &format!(
            "{} WHERE s.autopilot_id = ? AND s.autopilot_role = 'crew' AND (s.status IN \
             ('queued','provisioning','preparing','running','idle','paused','stopping') OR s.updated_at > ?) \
             ORDER BY s.created_at DESC LIMIT 15",
            engine::CREW_SELECT
        ),
        &[s(&row.project_id), n(ctx.now - 2 * 86_400_000)],
    )?
    .all()
    .await?;
    #[derive(Deserialize)]
    struct LogRow {
        #[serde(deserialize_with = "int")]
        at: i64,
        text: String,
        user_name: Option<String>,
        session_slug: Option<String>,
        #[serde(deserialize_with = "opt_int", default)]
        session_number: Option<i64>,
        session_title: Option<String>,
    }
    let entries: Vec<LogRow> = q(
        db,
        "SELECT e.at, e.text, u.name AS user_name, sp.slug AS session_slug, es.number AS session_number, \
         es.title AS session_title FROM autopilot_log e LEFT JOIN users u ON u.id = e.user_id \
         LEFT JOIN sessions es ON es.id = e.session_id LEFT JOIN projects sp ON sp.id = es.project_id \
         WHERE e.project_id = ? AND e.kind NOT IN ('woke', 'plan') ORDER BY e.at DESC, e.id DESC LIMIT 12",
        &[s(&row.project_id)],
    )?
    .all()
    .await?;
    #[derive(Deserialize)]
    struct Slug {
        slug: String,
    }
    let projects: Vec<String> = if row.scope() == AutopilotScope::Workspace {
        q(
            db,
            "SELECT slug FROM projects WHERE workspace_id = ? AND archived_at IS NULL ORDER BY created_at",
            &[s(&row.workspace_id)],
        )?
        .all::<Slug>()
        .await?
        .into_iter()
        .map(|p| p.slug)
        .collect()
    } else {
        Vec::new()
    };
    let owner = row.owner_name.clone().unwrap_or_else(|| "its owner".into());
    let workspace = project.workspace.workspace_name.clone().unwrap_or_default();
    let note_views: Vec<ap::NoteView> = notes
        .iter()
        .map(|n| ap::NoteView {
            text: &n.text,
            author: n.author.as_ref().map_or("Someone", |a| a.name.as_str()),
            at: n.created_at,
            new: n.seen_at.is_none(),
        })
        .collect();
    let options: Vec<Vec<String>> = ask_rows.iter().map(engine::AskRow::options).collect();
    let ask_views: Vec<ap::AskView> = ask_rows
        .iter()
        .zip(options.iter())
        .map(|(a, o)| engine::ask_view(a, o))
        .collect();
    let questions: Vec<Option<String>> = crew.iter().map(engine::CrewRow::question_text).collect();
    let crew_views: Vec<ap::CrewView> = crew
        .iter()
        .zip(questions.iter())
        .map(|(c, q)| {
            let mut v = engine::crew_view(c);
            v.question = q.as_deref();
            v
        })
        .collect();
    // People's lines name them as they read ([`ap::said`]); their notes,
    // which don't, say who wrote them beside the note.
    let lines: Vec<String> = entries
        .iter()
        .map(|e| ap::said(&e.text, e.user_name.as_deref()))
        .collect();
    let log_views: Vec<ap::EntryView> = entries
        .iter()
        .zip(lines.iter())
        .map(|(e, text)| ap::EntryView {
            at: e.at,
            text,
            by: e
                .user_name
                .as_deref()
                .filter(|_| !e.text.starts_with(ap::WHO)),
            session: match (&e.session_slug, e.session_number) {
                (Some(slug), Some(n)) => Some(format!(
                    "{slug}#{n} “{}”",
                    e.session_title.as_deref().unwrap_or_default()
                )),
                _ => None,
            },
        })
        .collect();
    let powers = row.powers();
    let direction_by = row.direction_by_name.clone();
    let shown = Shown {
        notes: notes.iter().map(|n| n.id.clone()).collect(),
        asks: ask_rows
            .iter()
            .filter(|a| a.status() != AutopilotAskStatus::Open)
            .map(|a| a.id.clone())
            .collect(),
    };
    let text = ap::brief(&ap::Brief {
        place: ap::Place {
            project: &project.name,
            project_slug: &project.slug,
            workspace: &workspace,
            scope: row.scope(),
            owner: &owner,
        },
        rules: ap::Rules {
            powers: &powers,
            crew_max: row.crew_max(),
            check_in_minutes: row.check_in(),
        },
        direction: &row.direction,
        direction_by: direction_by.as_deref(),
        direction_at: row.direction_at,
        plan: row.plan.as_deref(),
        plan_at: row.plan_at,
        notes: &note_views,
        asks: &ask_views,
        crew: &crew_views,
        log: &log_views,
        projects: &projects,
        wake_at: row.wake_at,
        now: ctx.now,
    });
    Ok((text, shown))
}

// ───────────────────────────── putting it on, and changing it ─────────────────────────────

/// `PUT /api/v1/projects/{id}/autopilot`: puts the project on autopilot,
/// on again once it was off, or changes its autopilot.
pub async fn put(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: PutAutopilotRequest = http::body(req).await?;
    let l = load(ctx, id).await?;
    if ctx.autopilot().is_some() {
        return Err(forbidden("An autopilot doesn't change itself: people do."));
    }
    let me = ctx.user()?.clone();
    if !fctl_domain::perm::can_manage_project(l.p.role) {
        return Err(forbidden(
            "Only the project's maintainers and owners put it on autopilot, or change its autopilot.",
        ));
    }
    let model = body
        .model
        .as_deref()
        .map(str::trim)
        .map(|m| {
            if m.is_empty() || m == "default" {
                Ok(None)
            } else {
                fctl_domain::validate::model(m).map(Some)
            }
        })
        .transpose()
        .map_err(AppError)?;
    let effort = body
        .effort
        .as_deref()
        .map(str::trim)
        .map(|e| {
            if e.is_empty() || e == "default" {
                Ok(None)
            } else {
                fctl_domain::validate::effort(e).map(Some)
            }
        })
        .transpose()
        .map_err(AppError)?;
    let check_in = body
        .check_in_minutes
        .map(ap::check_in)
        .transpose()
        .map_err(AppError)?;
    let crew = body
        .crew_max
        .map(ap::crew_max)
        .transpose()
        .map_err(AppError)?;
    let direction = body
        .direction
        .as_deref()
        .map(ap::direction)
        .transpose()
        .map_err(AppError)?;
    match l.row.clone().filter(|r| r.state() != AutopilotState::Off) {
        None => {
            // On, or on again: from what it had before, as far as asked.
            let before = l.row.clone();
            let scope = body
                .scope
                .or(before.as_ref().map(AutopilotRow::scope))
                .unwrap_or_default();
            // What a workspace's autopilot kept (its plan, notes, asks, log
            // and own sessions) reaches the whole workspace: it stays one,
            // its admins', rather than open up as a project's.
            if scope == AutopilotScope::Project
                && before.as_ref().map(AutopilotRow::scope) == Some(AutopilotScope::Workspace)
            {
                return Err(conflict(
                    "This project's autopilot ran the whole workspace, and what it kept reaches all its projects: it stays the workspace's. Its admins put it on again for the workspace.",
                ));
            }
            let direction = match (direction, &before) {
                (Some(d), _) => d,
                (None, Some(b)) => b.direction.clone(),
                (None, None) => ap::direction("").map_err(AppError)?,
            };
            let powers = match &body.powers {
                Some(p) => ap::powers(scope, p).map_err(AppError)?,
                None => before
                    .as_ref()
                    .filter(|b| b.scope() == scope)
                    .map_or_else(|| ap::default_powers(scope), AutopilotRow::powers),
            };
            if l.p.row.archived_at.is_some() {
                return Err(conflict("This project is archived"));
            }
            if !engine::workspace_allows(&ctx.db, l.p.workspace_id()).await? {
                return Err(forbidden(
                    "This workspace's admins haven't turned autopilots on: they do in its settings.",
                ));
            }
            if scope == AutopilotScope::Workspace {
                if !is_admin(ctx, l.p.workspace_id()).await? {
                    return Err(forbidden(
                        "Only a workspace's admins put it on autopilot as a whole.",
                    ));
                }
                if let Some(other) = engine::of_workspace(&ctx.db, l.p.workspace_id()).await?
                    && other.project_id != l.p.row.id
                {
                    return Err(conflict(
                        "This workspace has an autopilot already, which works in all its projects: take it off first.",
                    ));
                }
            }
            if crate::api::sessions::credential_for(&ctx.db, &me, l.p.workspace_id())
                .await?
                .is_none()
            {
                return Err(conflict(
                    "Connect your Claude subscription first (Settings → Claude): the autopilot runs on it.",
                ));
            }
            ctx.verified(Sensitive::PutOnAutopilot)?;
            let check_in = check_in
                .or(before.as_ref().map(AutopilotRow::check_in))
                .unwrap_or(ap::DEFAULT_CHECK_IN);
            let crew = crew
                .or(before.as_ref().map(AutopilotRow::crew_max))
                .unwrap_or(ap::DEFAULT_CREW);
            // Theirs to choose, who own it from now on: what its owner
            // before them chose isn't.
            let keep_owner = body.keep_owner.unwrap_or_else(|| {
                before
                    .as_ref()
                    .is_some_and(|b| b.keep_owner && b.owner_id.as_deref() == Some(me.id.as_str()))
            });
            let written = q(
                &ctx.db,
                "INSERT INTO autopilots (project_id, workspace_id, scope, state, owner_id, keep_owner, direction, direction_by, \
                 direction_at, direction_new, powers, check_in_minutes, crew_max, model, effort, wake_at, wake_reasons, failures, \
                 created_by, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 'on', ?4, ?12, ?5, ?4, ?6, 0, ?7, ?8, ?9, ?10, ?11, ?6, 'started', 0, ?4, ?6, ?6) \
                 ON CONFLICT (project_id) DO UPDATE SET workspace_id = excluded.workspace_id, scope = excluded.scope, \
                 state = 'on', paused_reason = NULL, paused_by = NULL, paused_at = NULL, owner_id = excluded.owner_id, \
                 keep_owner = excluded.keep_owner, \
                 direction = excluded.direction, direction_by = CASE WHEN autopilots.direction = excluded.direction \
                   THEN autopilots.direction_by ELSE excluded.direction_by END, \
                 direction_at = CASE WHEN autopilots.direction = excluded.direction THEN autopilots.direction_at \
                   ELSE excluded.direction_at END, direction_new = 0, \
                 powers = excluded.powers, check_in_minutes = excluded.check_in_minutes, crew_max = excluded.crew_max, \
                 model = excluded.model, effort = excluded.effort, session_id = NULL, wake_at = excluded.wake_at, \
                 wake_reasons = 'started', asked_wake_at = NULL, failures = 0, updated_at = excluded.updated_at",
                &[
                    s(&l.p.row.id),
                    s(l.p.workspace_id()),
                    s(scope.as_str()),
                    s(&me.id),
                    s(&direction),
                    n(ctx.now),
                    s(&ap::store_powers(&powers)),
                    n(i64::from(check_in)),
                    n(i64::from(crew)),
                    // Given, the one asked for (`default`: the project's);
                    // else the one it had.
                    os(match &model {
                        Some(m) => m.as_deref(),
                        None => before.as_ref().and_then(|b| b.model.as_deref()),
                    }),
                    os(match &effort {
                        Some(e) => e.as_deref(),
                        None => before.as_ref().and_then(|b| b.effort.as_deref()),
                    }),
                    crate::db::b(keep_owner),
                ],
            )?
            .run()
            .await;
            if let Err(e) = written {
                // Another project of the workspace was put on autopilot as a
                // whole meanwhile, which autopilots_one_per_workspace keeps
                // to one: asked again, as D1's errors don't say which.
                if scope == AutopilotScope::Workspace
                    && engine::of_workspace(&ctx.db, l.p.workspace_id())
                        .await?
                        .is_some_and(|other| other.project_id != l.p.row.id)
                {
                    return Err(conflict(
                        "This workspace has an autopilot already, which works in all its projects: take it off first.",
                    ));
                }
                return Err(e);
            }
            let again = if before.is_some() { " again" } else { "" };
            engine::log(
                &ctx.db,
                &l.p.row.id,
                Entry::new(
                    AutopilotEntryKind::State,
                    match scope {
                        AutopilotScope::Project => {
                            format!("{} put the project on autopilot{again}", ap::WHO)
                        }
                        AutopilotScope::Workspace => {
                            format!("{} put the workspace on autopilot{again}", ap::WHO)
                        }
                    },
                )
                .by(&me.id),
                ctx.now,
            )
            .await;
            ctx.audit_in(
                Some(l.p.workspace_id()),
                "autopilot.on",
                Some(&l.p.row.id),
                Some(serde_json::json!({
                    "scope": scope.as_str(),
                    "powers": ap::store_powers(&powers),
                    "check_in_minutes": check_in,
                    "crew_max": crew,
                    "keep_owner": keep_owner,
                })),
            );
            crate::schedules::wake(&ctx.env).await;
        }
        Some(row) => {
            if !can(ctx, &l.p, Some(&row)).await?.steer {
                return Err(forbidden(
                    "Only the workspace's admins change its autopilot, which works in all its projects.",
                ));
            }
            if body.scope.is_some_and(|sc| sc != row.scope()) {
                return Err(conflict(
                    "An autopilot keeps its scope while it's on. A project's autopilot can be put on again for the whole workspace once taken off; a workspace's stays the workspace's.",
                ));
            }
            let powers = body
                .powers
                .as_ref()
                .map(|p| ap::powers(row.scope(), p))
                .transpose()
                .map_err(AppError)?;
            // What spends its owner's subscription, changed by someone else,
            // runs it on theirs from now on, unless its owner keeps it.
            let costs = check_in.is_some_and(|c| c != row.check_in())
                || crew.is_some_and(|c| c != row.crew_max())
                || model
                    .as_ref()
                    .is_some_and(|m| m.as_deref() != row.model.as_deref())
                || effort
                    .as_ref()
                    .is_some_and(|e| e.as_deref() != row.effort.as_deref());
            let mine = row.owner_id.as_deref() == Some(me.id.as_str());
            let take_over = takes_over(ctx, &l.p, &row, mine, body.take_over, costs).await?;
            // Whether it stays on its owner's subscription is theirs to
            // choose: the new owner's, once someone takes it over, who
            // starts without it unless they ask.
            let keep_owner = if take_over {
                body.keep_owner.unwrap_or(false)
            } else if mine {
                body.keep_owner.unwrap_or(row.keep_owner)
            } else if body.keep_owner.is_some_and(|k| k != row.keep_owner) {
                return Err(forbidden(
                    "Only its owner chooses whether it stays on their subscription when others change it: take it over to choose for yours.",
                ));
            } else {
                row.keep_owner
            };
            if take_over {
                if crate::api::sessions::credential_for(&ctx.db, &me, l.p.workspace_id())
                    .await?
                    .is_none()
                {
                    return Err(conflict(
                        "Connect your Claude subscription first (Settings → Claude): from now on the autopilot runs on it.",
                    ));
                }
                ctx.verified(Sensitive::PutOnAutopilot)?;
            }
            // Keeping it lets others spend its owner's plan: theirs to
            // confirm, as putting it on is. What the log says of it: a new
            // owner's choice to keep it, or the owner's change.
            let keeps = if take_over {
                keep_owner.then_some(true)
            } else {
                (keep_owner != row.keep_owner).then_some(keep_owner)
            };
            if keeps == Some(true) {
                ctx.verified(Sensitive::KeepAutopilot)?;
            }
            let new_direction = direction.filter(|d| *d != row.direction);
            // Only while whose it is, and whether they keep it, are as read:
            // what was decided from them (taking it over or not, keeping it)
            // holds then, and nothing writes back what changed meanwhile.
            let written = q(
                &ctx.db,
                "UPDATE autopilots SET direction = COALESCE(?1, direction), \
                 direction_by = CASE WHEN ?1 IS NULL THEN direction_by ELSE ?2 END, \
                 direction_at = CASE WHEN ?1 IS NULL THEN direction_at ELSE ?3 END, \
                 direction_new = CASE WHEN ?1 IS NULL THEN direction_new ELSE 1 END, \
                 powers = COALESCE(?4, powers), check_in_minutes = COALESCE(?5, check_in_minutes), \
                 crew_max = COALESCE(?6, crew_max), \
                 model = CASE WHEN ?7 THEN ?8 ELSE model END, effort = CASE WHEN ?9 THEN ?10 ELSE effort END, \
                 owner_id = CASE WHEN ?11 THEN ?2 ELSE owner_id END, keep_owner = ?13, updated_at = ?3 \
                 WHERE project_id = ?12 AND owner_id IS ?14 AND keep_owner = ?15",
                &[
                    os(new_direction.as_deref()),
                    s(&me.id),
                    n(ctx.now),
                    os(powers.as_ref().map(|p| ap::store_powers(p)).as_deref()),
                    on(check_in.map(i64::from)),
                    on(crew.map(i64::from)),
                    crate::db::b(model.is_some()),
                    os(model.clone().flatten().as_deref()),
                    crate::db::b(effort.is_some()),
                    os(effort.clone().flatten().as_deref()),
                    crate::db::b(take_over),
                    s(&row.project_id),
                    crate::db::b(keep_owner),
                    os(row.owner_id.as_deref()),
                    crate::db::b(row.keep_owner),
                ],
            )?
            .run()
            .await?;
            if written == 0 {
                return Err(conflict(MOVED_ON));
            }
            // Its sessions run on the old owner's subscription: it wakes
            // now to move them to the new owner's, rather than wait there,
            // perhaps for a limit to reset.
            if take_over {
                engine::nudge(
                    &ctx.env,
                    &ctx.db,
                    &row.project_id,
                    AutopilotWakeReason::Moved,
                    ctx.now,
                )
                .await?;
            }
            if new_direction.is_some() {
                engine::log(
                    &ctx.db,
                    &row.project_id,
                    Entry::new(
                        AutopilotEntryKind::Direction,
                        format!("{} changed its direction", ap::WHO),
                    )
                    .by(&me.id),
                    ctx.now,
                )
                .await;
                engine::nudge(
                    &ctx.env,
                    &ctx.db,
                    &row.project_id,
                    AutopilotWakeReason::Steered,
                    ctx.now + ap::PEOPLE_GATHER_MS,
                )
                .await?;
            }
            let changed: Vec<&str> = [
                (
                    powers.as_ref().is_some_and(|p| *p != row.powers()),
                    "its powers",
                ),
                (
                    check_in.is_some_and(|c| c != row.check_in()),
                    "its check-in",
                ),
                (
                    crew.is_some_and(|c| c != row.crew_max()),
                    "how many sessions it runs at once",
                ),
                (
                    model
                        .as_ref()
                        .is_some_and(|m| m.as_deref() != row.model.as_deref())
                        || effort
                            .as_ref()
                            .is_some_and(|e| e.as_deref() != row.effort.as_deref()),
                    "its model",
                ),
            ]
            .into_iter()
            .filter_map(|(yes, what)| yes.then_some(what))
            .collect();
            if !changed.is_empty() || take_over {
                let mut text = if changed.is_empty() {
                    format!("{} took it over", ap::WHO)
                } else {
                    format!("{} changed {}", ap::WHO, join(&changed))
                };
                if take_over && !changed.is_empty() {
                    text.push_str(": it runs on their subscription from now on");
                } else if !mine && !take_over && costs {
                    text.push_str(": it stays on its owner's subscription, as they keep it");
                }
                engine::log(
                    &ctx.db,
                    &row.project_id,
                    Entry::new(AutopilotEntryKind::State, text).by(&me.id),
                    ctx.now,
                )
                .await;
                ctx.audit_in(
                    Some(l.p.workspace_id()),
                    "autopilot.changed",
                    Some(&row.project_id),
                    Some(serde_json::json!({
                        "powers": powers.as_ref().map(|p| ap::store_powers(p)),
                        "check_in_minutes": check_in,
                        "crew_max": crew,
                        "taken_over": take_over,
                        "keep_owner": keep_owner,
                    })),
                );
            } else if keeps.is_some() {
                ctx.audit_in(
                    Some(l.p.workspace_id()),
                    "autopilot.changed",
                    Some(&row.project_id),
                    Some(serde_json::json!({ "keep_owner": keep_owner })),
                );
            }
            if let Some(keeps) = keeps {
                engine::log(
                    &ctx.db,
                    &row.project_id,
                    Entry::new(
                        AutopilotEntryKind::State,
                        if keeps {
                            format!(
                                "{} chose to keep it on their subscription when others change or resume it",
                                ap::WHO
                            )
                        } else {
                            format!(
                                "{} no longer keeps it on their subscription: others who change or resume it take it over",
                                ap::WHO
                            )
                        },
                    )
                    .by(&me.id),
                    ctx.now,
                )
                .await;
            }
        }
    }
    get(ctx, &l.p.row.id).await
}

/// Whether what the caller does to `row` runs it on their subscription
/// from now on (`ap::takes_over`): asking to; or changing how much it
/// works, or resuming it, unless its owner keeps it and can still run it.
async fn takes_over(
    ctx: &Ctx,
    p: &ProjectAccess,
    row: &AutopilotRow,
    mine: bool,
    asks: bool,
    costs: bool,
) -> AppResult<bool> {
    // Whether its owner can run it matters only where they keep it.
    let owner_runs = !mine
        && !asks
        && costs
        && row.keep_owner
        && engine::owner(&ctx.db, &p.row, row).await?.is_ok();
    Ok(ap::takes_over(ap::Change {
        by_owner: mine,
        asks,
        costs,
        kept: row.keep_owner,
        owner_runs,
    }))
}

fn join(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// `POST …/autopilot/off`: takes the project off autopilot. It keeps its
/// direction, notes, plan, log and asks; its own session stops, and with
/// `stop_sessions` those it started that still work.
pub async fn off(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: AutopilotOffRequest = if http::header(req, "Content-Type").is_some() {
        http::body(req).await?
    } else {
        AutopilotOffRequest::default()
    };
    let l = load(ctx, id).await?;
    let row = require(l.row.clone())?;
    require_steer(&l)?;
    let me = ctx.user()?.clone();
    q(
        &ctx.db,
        "UPDATE autopilots SET state = 'off', paused_reason = NULL, paused_by = ?, paused_at = ?, wake_at = NULL, \
         wake_reasons = '', asked_wake_at = NULL, updated_at = ? WHERE project_id = ?",
        &[s(&me.id), n(ctx.now), n(ctx.now), s(&row.project_id)],
    )?
    .run()
    .await?;
    let stopped = stop_sessions(ctx, &row, body.stop_sessions).await?;
    engine::log(
        &ctx.db,
        &row.project_id,
        Entry::new(
            AutopilotEntryKind::State,
            match stopped {
                0 => format!("{} took it off autopilot", ap::WHO),
                n => format!(
                    "{} took it off autopilot, and stopped {n} of its sessions",
                    ap::WHO
                ),
            },
        )
        .by(&me.id),
        ctx.now,
    )
    .await;
    ctx.audit_in(
        Some(l.p.workspace_id()),
        "autopilot.off",
        Some(&row.project_id),
        Some(serde_json::json!({ "stopped_sessions": stopped })),
    );
    http::ok(&serde_json::json!({ "ok": true }))
}

/// Stops the autopilot's own session, and with `crew` the sessions it
/// started that still work: how many of those.
async fn stop_sessions(ctx: &Ctx, row: &AutopilotRow, crew: bool) -> AppResult<usize> {
    #[derive(Deserialize)]
    struct Id {
        id: String,
    }
    let mut ids: Vec<String> = Vec::new();
    if let Some(own) = row
        .session_id
        .clone()
        .filter(|_| row.session_status().is_some_and(|st| !st.is_terminal()))
    {
        ids.push(own);
    }
    let mut stopped = 0;
    if crew {
        let rows: Vec<Id> = q(
            &ctx.db,
            "SELECT id FROM sessions WHERE autopilot_id = ? AND autopilot_role = 'crew' \
             AND status IN ('queued','provisioning','preparing','running','idle','paused')",
            &[s(&row.project_id)],
        )?
        .all()
        .await?;
        stopped = rows.len();
        ids.extend(rows.into_iter().map(|r| r.id));
    }
    let fleet = crate::internal::fleet_stub(&ctx.env)?;
    for id in ids {
        let sent: AppResult<serde_json::Value> = crate::internal::post(
            &fleet,
            "/stop",
            &crate::internal::Stop {
                session_id: SessionId(id.clone()),
                reason: fctl_proto::runner::StopReason::User,
                destroy: false,
            },
        )
        .await;
        if let Err(e) = sent {
            worker::console_error!("stopping {id} for its autopilot: {}", e.0.message);
        }
    }
    Ok(stopped)
}

/// `POST …/autopilot/pause`: by a person, whose pause stops its own
/// session too, or by the autopilot itself once its direction is done.
pub async fn pause(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: PauseAutopilotRequest = if http::header(req, "Content-Type").is_some() {
        http::body(req).await?
    } else {
        PauseAutopilotRequest::default()
    };
    let l = load(ctx, id).await?;
    let row = require(l.row.clone())?;
    let reason = ap::reason(body.reason.as_deref()).map_err(AppError)?;
    if ctx.autopilot().is_some() {
        require_pilot(ctx, &l)?;
        engine::pause(&ctx.db, &row.project_id, reason.as_deref(), None, ctx.now).await?;
        engine::log(
            &ctx.db,
            &row.project_id,
            Entry::new(
                AutopilotEntryKind::State,
                match &reason {
                    Some(why) => format!("It paused itself: {why}"),
                    None => "It paused itself".to_owned(),
                },
            ),
            ctx.now,
        )
        .await;
        return http::ok(&serde_json::json!({ "ok": true }));
    }
    require_steer(&l)?;
    if row.state() != AutopilotState::On {
        return get(ctx, id).await;
    }
    let me = ctx.user()?.clone();
    engine::pause(
        &ctx.db,
        &row.project_id,
        reason.as_deref(),
        Some(&me.id),
        ctx.now,
    )
    .await?;
    stop_sessions(ctx, &row, false).await?;
    engine::log(
        &ctx.db,
        &row.project_id,
        Entry::new(
            AutopilotEntryKind::State,
            match &reason {
                Some(why) => format!("{} paused it: {why}", ap::WHO),
                None => format!("{} paused it", ap::WHO),
            },
        )
        .by(&me.id),
        ctx.now,
    )
    .await;
    ctx.audit_in(
        Some(l.p.workspace_id()),
        "autopilot.paused",
        Some(&row.project_id),
        None,
    );
    get(ctx, id).await
}

/// `POST …/autopilot/resume`: it wakes at once, on the resumer's
/// subscription from now on when it ran on someone else's.
pub async fn resume(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let l = load(ctx, id).await?;
    let row = require(l.row.clone())?;
    require_steer(&l)?;
    if row.state() == AutopilotState::On {
        return get(ctx, id).await;
    }
    if !engine::workspace_allows(&ctx.db, l.p.workspace_id()).await? {
        return Err(forbidden(
            "This workspace's admins turned autopilots off: they turn them on in its settings.",
        ));
    }
    let me = ctx.user()?.clone();
    let mine = row.owner_id.as_deref() == Some(me.id.as_str());
    let take_over = takes_over(ctx, &l.p, &row, mine, false, true).await?;
    if take_over {
        if crate::api::sessions::credential_for(&ctx.db, &me, l.p.workspace_id())
            .await?
            .is_none()
        {
            return Err(conflict(
                "Connect your Claude subscription first (Settings → Claude): from now on the autopilot runs on it.",
            ));
        }
        ctx.verified(Sensitive::PutOnAutopilot)?;
    }
    // Only one still paused: one taken off meanwhile stays off. Taken
    // over, it's no longer kept: its new owner chooses that for theirs.
    let resumed = q(
        &ctx.db,
        "UPDATE autopilots SET state = 'on', paused_reason = NULL, paused_by = NULL, paused_at = NULL, failures = 0, \
         wake_at = ?1, wake_reasons = 'resumed', asked_wake_at = NULL, owner_id = CASE WHEN ?2 THEN ?3 ELSE owner_id END, \
         keep_owner = CASE WHEN ?2 THEN 0 ELSE keep_owner END, updated_at = ?1 \
         WHERE project_id = ?4 AND state = 'paused' AND owner_id IS ?5 AND keep_owner = ?6",
        &[
            n(ctx.now),
            crate::db::b(take_over),
            s(&me.id),
            s(&row.project_id),
            os(row.owner_id.as_deref()),
            crate::db::b(row.keep_owner),
        ],
    )?
    .run()
    .await?;
    if resumed == 0 {
        let now = engine::row(&ctx.db, &row.project_id).await?;
        return Err(conflict(
            if now.is_some_and(|n| n.state() == AutopilotState::Paused) {
                MOVED_ON
            } else {
                "It isn't paused any more: someone resumed it, or took it off autopilot, meanwhile."
            },
        ));
    }
    engine::log(
        &ctx.db,
        &row.project_id,
        Entry::new(
            AutopilotEntryKind::State,
            if take_over {
                format!("{} resumed it, on their subscription from now on", ap::WHO)
            } else if !mine {
                format!(
                    "{} resumed it, on its owner's subscription, as they keep it",
                    ap::WHO
                )
            } else {
                format!("{} resumed it", ap::WHO)
            },
        )
        .by(&me.id),
        ctx.now,
    )
    .await;
    ctx.audit_in(
        Some(l.p.workspace_id()),
        "autopilot.resumed",
        Some(&row.project_id),
        Some(serde_json::json!({ "taken_over": take_over })),
    );
    crate::schedules::wake(&ctx.env).await;
    get(ctx, id).await
}

/// `POST …/autopilot/wake`: a person wakes it now; its own session says
/// when it wakes next unless something wakes it sooner.
pub async fn wake(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: WakeAutopilotRequest = if http::header(req, "Content-Type").is_some() {
        http::body(req).await?
    } else {
        WakeAutopilotRequest::default()
    };
    let l = load(ctx, id).await?;
    let row = require(l.row.clone())?;
    if row.state() != AutopilotState::On {
        return Err(conflict("It's paused: resume it to wake it."));
    }
    if ctx.autopilot().is_some() {
        require_pilot(ctx, &l)?;
        let at = ap::asked_wake(ctx.now, body.in_minutes, body.at).map_err(AppError)?;
        q(
            &ctx.db,
            "UPDATE autopilots SET asked_wake_at = ?1, wake_at = CASE WHEN wake_reasons != '' AND wake_at < ?1 \
             THEN wake_at ELSE ?1 END WHERE project_id = ?2 AND state = 'on'",
            &[n(at), s(&row.project_id)],
        )?
        .run()
        .await?;
        crate::schedules::wake(&ctx.env).await;
        return http::ok(&serde_json::json!({ "wake_at": at }));
    }
    require_steer(&l)?;
    engine::nudge(
        &ctx.env,
        &ctx.db,
        &row.project_id,
        AutopilotWakeReason::Woken,
        ctx.now,
    )
    .await?;
    get(ctx, id).await
}

// ───────────────────────────── what it writes ─────────────────────────────

/// `POST …/autopilot/log`: what the autopilot did, decided or plans.
pub async fn write_log(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: AutopilotLogRequest = http::body(req).await?;
    let l = load(ctx, id).await?;
    require_pilot(ctx, &l)?;
    let text = ap::log_text(&body.text).map_err(AppError)?;
    let from = ctx.autopilot().map(|a| a.session_id.clone());
    let mut entry = Entry::new(AutopilotEntryKind::Note, text.clone());
    entry.from = from.as_deref();
    engine::log(&ctx.db, &l.p.row.id, entry, ctx.now).await;
    http::created(&serde_json::json!({ "ok": true, "text": text }))
}

/// `PUT …/autopilot/plan`: its plan, whole.
pub async fn plan(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: AutopilotPlanRequest = http::body(req).await?;
    let l = load(ctx, id).await?;
    require_pilot(ctx, &l)?;
    let plan = ap::plan(&body.plan).map_err(AppError)?;
    q(
        &ctx.db,
        "UPDATE autopilots SET plan = ?, plan_at = ?, updated_at = ? WHERE project_id = ?",
        &[
            os(plan.as_deref()),
            on(plan.as_ref().map(|_| ctx.now)),
            n(ctx.now),
            s(&l.p.row.id),
        ],
    )?
    .run()
    .await?;
    let from = ctx.autopilot().map(|a| a.session_id.clone());
    let mut entry = Entry::new(
        AutopilotEntryKind::Plan,
        if plan.is_some() {
            "Updated its plan"
        } else {
            "Cleared its plan"
        },
    );
    entry.from = from.as_deref();
    engine::log(&ctx.db, &l.p.row.id, entry, ctx.now).await;
    http::ok(&serde_json::json!({ "ok": true }))
}

/// `POST …/autopilot/asks`: the autopilot asks people a question, or for a
/// secret, and goes on meanwhile.
pub async fn ask(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: CreateAutopilotAskRequest = http::body(req).await?;
    let l = load(ctx, id).await?;
    require_pilot(ctx, &l)?;
    let row = require(l.row.clone())?;
    let detail = ap::detail(body.detail.as_deref()).map_err(AppError)?;
    let (kind, text, options, secret) = match body.secret.as_deref() {
        Some(name) => {
            let name = fctl_domain::validate::env_name("secret", name)?;
            let text = fctl_domain::validate::question(&body.text, &[])?
                .map(|(t, _)| t)
                .ok_or_else(|| validation("text", "Say what the secret is for"))?;
            // The project it goes into: one the autopilot reaches.
            let target = match body.project.as_deref() {
                Some(p) => access::project(ctx, p).await?,
                None => access::project(ctx, &row.project_id).await?,
            };
            (
                "secret",
                text,
                Vec::new(),
                Some((name, target.row.id.clone())),
            )
        }
        None => {
            let (text, options) = fctl_domain::validate::question(&body.text, &body.options)?
                .ok_or_else(|| validation("text", "Ask something"))?;
            ("question", text, options, None)
        }
    };
    #[derive(Deserialize)]
    struct Count {
        #[serde(deserialize_with = "int", default)]
        n: i64,
    }
    let open: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM autopilot_asks WHERE project_id = ? AND status = 'open'",
        &[s(&row.project_id)],
    )?
    .first()
    .await?;
    if open.is_some_and(|c| c.n >= ap::MAX_OPEN_ASKS as i64) {
        return Err(conflict(&format!(
            "{} asks wait for people already: take one back (fctl autopilot withdraw) before you ask more.",
            ap::MAX_OPEN_ASKS
        )));
    }
    let ask_id = fctl_domain::ids::new_id("apa", ctx.now);
    let session = ctx.autopilot().map(|a| a.session_id.clone());
    q(
        &ctx.db,
        "INSERT INTO autopilot_asks (id, project_id, kind, text, detail, options, secret_name, secret_project_id, status, \
         session_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'open', ?, ?)",
        &[
            s(&ask_id),
            s(&row.project_id),
            s(kind),
            s(&text),
            os(detail.as_deref()),
            os((!options.is_empty())
                .then(|| serde_json::to_string(&options).unwrap_or_default())
                .as_deref()),
            os(secret.as_ref().map(|(name, _)| name.as_str())),
            os(secret.as_ref().map(|(_, project)| project.as_str())),
            os(session.as_deref()),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    let line = match &secret {
        Some((name, _)) => format!("Asked for the secret `{name}`: {text}"),
        None => format!("Asked: {text}"),
    };
    let mut entry = Entry::new(AutopilotEntryKind::Asked, line);
    entry.from = session.as_deref();
    engine::log(&ctx.db, &row.project_id, entry, ctx.now).await;
    let notified = match &secret {
        Some((name, _)) => format!("It needs the secret {name}: {text}"),
        None => fctl_domain::notify::question_detail(&text, &options),
    };
    engine::notify_ask(&ctx.env, &ctx.db, &row, &notified, ctx.now).await;
    let created: Option<engine::AskRow> = q(
        &ctx.db,
        &format!("{} WHERE k.id = ?", engine::ASK_SELECT),
        &[s(&ask_id)],
    )?
    .first()
    .await?;
    let created = created.ok_or_else(|| crate::error::internal("the new ask vanished"))?;
    http::created(&ask_of(ctx, &created).await?)
}

/// `DELETE …/autopilot/asks/{ask}`: the autopilot takes back what it
/// asked: it found out itself.
pub async fn withdraw(ctx: &mut Ctx, id: &str, ask: &str) -> AppResult<Response> {
    let l = load(ctx, id).await?;
    require_pilot(ctx, &l)?;
    let done = q(
        &ctx.db,
        "UPDATE autopilot_asks SET status = 'withdrawn', answered_at = ?, seen_at = ? WHERE id = ? AND project_id = ? \
         AND status = 'open'",
        &[n(ctx.now), n(ctx.now), s(ask), s(&l.p.row.id)],
    )?
    .run()
    .await?;
    if done == 0 {
        return Err(not_found("Open ask"));
    }
    let from = ctx.autopilot().map(|a| a.session_id.clone());
    let mut entry = Entry::new(AutopilotEntryKind::Asked, "Took back what it asked");
    entry.from = from.as_deref();
    engine::log(&ctx.db, &l.p.row.id, entry, ctx.now).await;
    http::ok(&serde_json::json!({ "ok": true }))
}

// ───────────────────────────── what people say ─────────────────────────────

/// `POST …/autopilot/notes`: steers it. It hears at once.
pub async fn add_note(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: AddAutopilotNoteRequest = http::body(req).await?;
    let l = load(ctx, id).await?;
    let row = require(l.row.clone())?;
    require_steer(&l)?;
    let text = ap::note(&body.text).map_err(AppError)?;
    #[derive(Deserialize)]
    struct Count {
        #[serde(deserialize_with = "int", default)]
        n: i64,
    }
    let count: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM autopilot_notes WHERE project_id = ?",
        &[s(&row.project_id)],
    )?
    .first()
    .await?;
    if count.is_some_and(|c| c.n >= ap::MAX_NOTES as i64) {
        return Err(conflict(&format!(
            "It keeps {} notes at most: remove those it no longer needs first.",
            ap::MAX_NOTES
        )));
    }
    let me = ctx.user()?.clone();
    let note_id = fctl_domain::ids::new_id("apn", ctx.now);
    q(
        &ctx.db,
        "INSERT INTO autopilot_notes (id, project_id, text, author_id, created_at) VALUES (?, ?, ?, ?, ?)",
        &[s(&note_id), s(&row.project_id), s(&text), s(&me.id), n(ctx.now)],
    )?
    .run()
    .await?;
    engine::log(
        &ctx.db,
        &row.project_id,
        Entry::new(AutopilotEntryKind::Direction, text).by(&me.id),
        ctx.now,
    )
    .await;
    if row.state() == AutopilotState::On {
        engine::nudge(
            &ctx.env,
            &ctx.db,
            &row.project_id,
            AutopilotWakeReason::Steered,
            ctx.now + ap::PEOPLE_GATHER_MS,
        )
        .await?;
    }
    get(ctx, id).await
}

/// `DELETE …/autopilot/notes/{note}`: it no longer needs it.
pub async fn remove_note(ctx: &mut Ctx, id: &str, note: &str) -> AppResult<Response> {
    let l = load(ctx, id).await?;
    let row = require(l.row.clone())?;
    require_steer(&l)?;
    let gone = q(
        &ctx.db,
        "DELETE FROM autopilot_notes WHERE id = ? AND project_id = ?",
        &[s(note), s(&row.project_id)],
    )?
    .run()
    .await?;
    if gone == 0 {
        return Err(not_found("Note"));
    }
    get(ctx, id).await
}

/// `POST …/autopilot/asks/{ask}/answer`: answers a question, or gives the
/// secret it asked for, which goes into the project's secrets: the
/// autopilot hears only that it's set.
pub async fn answer(ctx: &mut Ctx, req: &mut Request, id: &str, ask: &str) -> AppResult<Response> {
    let body: AnswerAutopilotAskRequest = http::body(req).await?;
    let l = load(ctx, id).await?;
    let row = require(l.row.clone())?;
    require_steer(&l)?;
    let found: Option<engine::AskRow> = q(
        &ctx.db,
        &format!("{} WHERE k.id = ? AND k.project_id = ?", engine::ASK_SELECT),
        &[s(ask), s(&row.project_id)],
    )?
    .first()
    .await?;
    let found = found.ok_or_else(|| not_found("Ask"))?;
    if found.status() != AutopilotAskStatus::Open {
        return Err(conflict(
            "Someone answered it already, or it was taken back.",
        ));
    }
    let me = ctx.user()?.clone();
    // A secret, and where it goes; else the answer.
    let (answer, secret) = match (found.kind(), &found.secret_name, &found.secret_project_id) {
        (AutopilotAskKind::Secret, Some(name), Some(project)) => {
            let value = body
                .secret
                .as_deref()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| validation("secret", "Type in the secret"))?;
            // Those who may set the project's secrets give them.
            let target = access::project(ctx, project).await?;
            target.require_maintainer()?;
            (None, Some((target, name.clone(), value.to_owned())))
        }
        _ => {
            let text = body.answer.as_deref().map(str::trim).unwrap_or_default();
            if text.is_empty() {
                return Err(validation("answer", "Write an answer, or pick one"));
            }
            (
                Some(fctl_domain::validate::text("answer", text, 4_000)?),
                None,
            )
        }
    };
    // It's theirs to answer once they claim it: two people at once, or one
    // after it was taken back, are told so, and store nothing.
    let claim = q(
        &ctx.db,
        "UPDATE autopilot_asks SET status = 'answered', answer = ?, answered_by = ?, answered_at = ? WHERE id = ? \
         AND status = 'open'",
        &[os(answer.as_deref()), s(&me.id), n(ctx.now), s(ask)],
    )?;
    let taken = || conflict("Someone answered it already, or it was taken back.");
    let line = match (secret, &answer) {
        // The secret goes in with the claim, in one transaction.
        (Some((target, name, value)), _) => {
            let stored =
                crate::api::projects::store_asked_secret(ctx, &target, &name, &value, ask, claim)
                    .await?
                    .ok_or_else(taken)?;
            format!("{} set the secret `{stored}` for it", ap::WHO)
        }
        (None, text) => {
            if claim.run().await? == 0 {
                return Err(taken());
            }
            format!(
                "{} answered “{}”",
                ap::WHO,
                short(text.as_deref().unwrap_or_default())
            )
        }
    };
    engine::log(
        &ctx.db,
        &row.project_id,
        Entry::new(AutopilotEntryKind::Answered, line).by(&me.id),
        ctx.now,
    )
    .await;
    if row.state() == AutopilotState::On {
        engine::nudge(
            &ctx.env,
            &ctx.db,
            &row.project_id,
            AutopilotWakeReason::Answered,
            ctx.now + ap::PEOPLE_GATHER_MS,
        )
        .await?;
    }
    get(ctx, id).await
}

/// `POST …/autopilot/asks/{ask}/decline`: no, maybe saying why.
pub async fn decline(ctx: &mut Ctx, req: &mut Request, id: &str, ask: &str) -> AppResult<Response> {
    let body: DeclineAutopilotAskRequest = if http::header(req, "Content-Type").is_some() {
        http::body(req).await?
    } else {
        DeclineAutopilotAskRequest::default()
    };
    let l = load(ctx, id).await?;
    let row = require(l.row.clone())?;
    require_steer(&l)?;
    let reason = ap::reason(body.reason.as_deref()).map_err(AppError)?;
    let me = ctx.user()?.clone();
    let done = q(
        &ctx.db,
        "UPDATE autopilot_asks SET status = 'declined', answer = ?, answered_by = ?, answered_at = ? WHERE id = ? \
         AND project_id = ? AND status = 'open'",
        &[os(reason.as_deref()), s(&me.id), n(ctx.now), s(ask), s(&row.project_id)],
    )?
    .run()
    .await?;
    if done == 0 {
        return Err(conflict(
            "Someone answered it already, or it was taken back.",
        ));
    }
    engine::log(
        &ctx.db,
        &row.project_id,
        Entry::new(
            AutopilotEntryKind::Answered,
            match &reason {
                Some(why) => format!("{} declined: “{}”", ap::WHO, short(why)),
                None => format!("{} declined what it asked", ap::WHO),
            },
        )
        .by(&me.id),
        ctx.now,
    )
    .await;
    if row.state() == AutopilotState::On {
        engine::nudge(
            &ctx.env,
            &ctx.db,
            &row.project_id,
            AutopilotWakeReason::Answered,
            ctx.now + ap::PEOPLE_GATHER_MS,
        )
        .await?;
    }
    get(ctx, id).await
}

/// At most a line of what someone wrote, for the log.
fn short(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    let mut out: String = line.chars().take(140).collect();
    if out.chars().count() < line.chars().count() || text.lines().count() > 1 {
        out.push('…');
    }
    out
}

/// `GET /api/v1/autopilots/asks`: what autopilots ask that the caller may
/// answer, across their workspaces, newest first.
pub async fn open_asks(ctx: &mut Ctx) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    if ctx.autopilot().is_some() {
        return Err(forbidden("Asks are for people to answer."));
    }
    let settings = ctx.settings();
    let projects: Vec<ProjectRow> = q(
        &ctx.db,
        &format!(
            "{PROJECT_SELECT} WHERE {PROJECT_VISIBLE} AND ap.state IN ('on', 'paused') AND EXISTS \
             (SELECT 1 FROM autopilot_asks k WHERE k.project_id = p.id AND k.status = 'open')"
        ),
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let mut out = Vec::new();
    for project in projects {
        let role = access::project_role(&user, &settings, ctx.limits(), &project);
        let p = ProjectAccess { row: project, role };
        let row = engine::row(&ctx.db, &p.row.id).await?;
        if !can(ctx, &p, row.as_ref()).await?.steer {
            continue;
        }
        let rows: Vec<engine::AskRow> = q(
            &ctx.db,
            &format!(
                "{} WHERE k.project_id = ? AND k.status = 'open' ORDER BY k.created_at DESC LIMIT 10",
                engine::ASK_SELECT
            ),
            &[s(&p.row.id)],
        )?
        .all()
        .await?;
        for r in &rows {
            out.push(OpenAutopilotAsk {
                project_id: ProjectId(p.row.id.clone()),
                project_slug: p.row.slug.clone(),
                project_name: p.row.name.clone(),
                ask: ask_of(ctx, r).await?,
            });
        }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.ask.created_at));
    http::ok(&AutopilotAsksList { asks: out })
}
