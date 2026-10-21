//! A project's memory (`fctl memory`, docs/architecture.md#project-memory):
//! notes that agents and people keep for the agents of later sessions.
//! Everyone who sees a project reads them; its editors add notes, change
//! them, mark them stale and delete them (`fctl_domain::perm`). Agents read
//! and keep them through their host ([`crate::api::calls`]), and each run
//! of a session starts with the current ones ([`for_spec`]).

use crate::access::{self, ProjectAccess};
use crate::api::calls::RunnerSession;
use crate::ctx::Ctx;
use crate::db::{Query, batch, n, os, q, s};
use crate::error::{AppResult, conflict, forbidden, not_found};
use crate::http;
use fctl_domain::{memory, perm};
use fctl_proto::api::{AddMemoryRequest, MemoryList, Ok, UpdateMemoryRequest};
use fctl_proto::runner::MemorySpec;
use fctl_proto::session::{Memory, SessionRef, Staleness, UserRef};
use fctl_proto::{MemoryId, ProjectId, SessionId, UserId};
use serde::Deserialize;
use worker::{D1Database, Request, Response};

/// Notes with the sessions and people they name.
const SELECT: &str = "SELECT m.id, m.project_id, m.text, m.created_at, m.edited_at, m.stale_at, m.stale_reason, m.replaced_by, \
    m.session_id, cs.number AS session_number, cs.title AS session_title, \
    m.created_by, cu.name AS creator_name, cu.avatar_url AS creator_avatar, \
    m.edited_by, eu.name AS editor_name, eu.avatar_url AS editor_avatar, \
    m.stale_session_id, ss.number AS stale_session_number, ss.title AS stale_session_title, \
    m.stale_by, su.name AS stale_by_name, su.avatar_url AS stale_by_avatar \
    FROM memories m \
    LEFT JOIN sessions cs ON cs.id = m.session_id \
    LEFT JOIN users cu ON cu.id = m.created_by \
    LEFT JOIN users eu ON eu.id = m.edited_by \
    LEFT JOIN sessions ss ON ss.id = m.stale_session_id \
    LEFT JOIN users su ON su.id = m.stale_by";

#[derive(Deserialize)]
struct Row {
    id: String,
    project_id: String,
    text: String,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    #[serde(default, deserialize_with = "crate::db::opt_int")]
    edited_at: Option<i64>,
    #[serde(default, deserialize_with = "crate::db::opt_int")]
    stale_at: Option<i64>,
    stale_reason: Option<String>,
    replaced_by: Option<String>,
    session_id: Option<String>,
    #[serde(default, deserialize_with = "crate::db::opt_int")]
    session_number: Option<i64>,
    session_title: Option<String>,
    created_by: Option<String>,
    creator_name: Option<String>,
    creator_avatar: Option<String>,
    edited_by: Option<String>,
    editor_name: Option<String>,
    editor_avatar: Option<String>,
    stale_session_id: Option<String>,
    #[serde(default, deserialize_with = "crate::db::opt_int")]
    stale_session_number: Option<i64>,
    stale_session_title: Option<String>,
    stale_by: Option<String>,
    stale_by_name: Option<String>,
    stale_by_avatar: Option<String>,
}

/// A person a row names, while their account (or its placeholder) is there.
pub(crate) fn person(
    id: Option<String>,
    name: Option<String>,
    avatar_url: Option<String>,
) -> Option<UserRef> {
    match (id, name) {
        (Some(id), Some(name)) => Some(UserRef {
            id: UserId(id),
            name,
            avatar_url,
        }),
        _ => None,
    }
}

/// A session a row names.
pub(crate) fn session_ref(
    id: Option<String>,
    number: Option<i64>,
    title: Option<String>,
) -> Option<SessionRef> {
    match (id, number, title) {
        (Some(id), Some(number), Some(title)) => Some(SessionRef {
            id: SessionId(id),
            number: number.max(0) as u32,
            title,
        }),
        _ => None,
    }
}

impl Row {
    fn into_memory(self) -> Memory {
        let stale = self.stale_at.map(|at| Staleness {
            at,
            reason: self.stale_reason,
            session: session_ref(
                self.stale_session_id,
                self.stale_session_number,
                self.stale_session_title,
            ),
            by: person(self.stale_by, self.stale_by_name, self.stale_by_avatar),
            replaced_by: self.replaced_by.map(MemoryId),
        });
        Memory {
            id: MemoryId(self.id),
            project_id: ProjectId(self.project_id),
            text: self.text,
            session: session_ref(self.session_id, self.session_number, self.session_title),
            created_by: person(self.created_by, self.creator_name, self.creator_avatar),
            created_at: self.created_at,
            edited_at: self.edited_at,
            edited_by: person(self.edited_by, self.editor_name, self.editor_avatar),
            stale,
        }
    }
}

/// A project's memory, as lists order it: its current notes, and with
/// `stale` the stale ones after them.
pub async fn list(db: &D1Database, project_id: &str, stale: bool) -> AppResult<Vec<Memory>> {
    let only_current = if stale { "" } else { " AND m.stale_at IS NULL" };
    let sql = format!("{SELECT} WHERE m.project_id = ?{only_current}");
    let rows: Vec<Row> = q(db, &sql, &[s(project_id)])?.all().await?;
    let mut notes: Vec<Memory> = rows.into_iter().map(Row::into_memory).collect();
    memory::sort(&mut notes);
    Ok(notes)
}

/// One note of a project, or of any project without `project_id`.
async fn one(db: &D1Database, id: &str, project_id: Option<&str>) -> AppResult<Option<Memory>> {
    if !MemoryId::is_valid(id) {
        return Ok(None);
    }
    let row: Option<Row> = match project_id {
        Some(p) => {
            q(
                db,
                &format!("{SELECT} WHERE m.id = ? AND m.project_id = ?"),
                &[s(id), s(p)],
            )?
            .first()
            .await?
        }
        None => {
            q(db, &format!("{SELECT} WHERE m.id = ?"), &[s(id)])?
                .first()
                .await?
        }
    };
    Ok(row.map(Row::into_memory))
}

/// The notes a session's agent starts with (`SessionSpec::memory`).
pub async fn for_spec(db: &D1Database, project_id: &str) -> AppResult<MemorySpec> {
    Ok(memory::for_prompt(list(db, project_id, false).await?))
}

/// Adds a note to a project's memory unless it is full: from the agent of
/// `session` (whose starter it names too), or from `person`.
fn insert(
    db: &D1Database,
    id: &str,
    project_id: &str,
    text: &str,
    session: Option<&str>,
    person: Option<&str>,
    now: i64,
) -> AppResult<Query> {
    q(
        db,
        "INSERT INTO memories (id, project_id, text, session_id, created_by, created_at) \
         SELECT ?1, ?2, ?3, ?4, ?5, ?6 WHERE (SELECT COUNT(*) FROM memories WHERE project_id = ?2) < ?7",
        &[
            s(id),
            s(project_id),
            s(text),
            os(session),
            os(person),
            n(now),
            n(i64::from(memory::MAX_PER_PROJECT)),
        ],
    )
}

fn full() -> crate::error::AppError {
    conflict(&format!(
        "This project's memory is full: it keeps at most {} notes, stale ones included. People who can edit it delete the ones it no longer needs under Memory.",
        memory::MAX_PER_PROJECT
    ))
}

/// Who writes a note: the agent of a session, which names who started it
/// too, or a person.
pub enum Writer<'a> {
    Agent {
        session: &'a str,
        started_by: &'a str,
    },
    Person(&'a str),
}

impl Writer<'_> {
    fn session(&self) -> Option<&str> {
        match self {
            Self::Agent { session, .. } => Some(session),
            Self::Person(_) => None,
        }
    }

    fn person(&self) -> &str {
        match self {
            Self::Agent { started_by, .. } => started_by,
            Self::Person(id) => id,
        }
    }
}

/// Adds a note to a project's memory (`fctl memory add`), in place of
/// `replaces` when it names one, which goes stale and points at it, in one
/// batch. Returns the new note, and the one it replaced.
pub async fn remember(
    db: &D1Database,
    project_id: &str,
    writer: &Writer<'_>,
    text: &str,
    replaces: Option<&MemoryId>,
    now: i64,
) -> AppResult<(Memory, Option<Memory>)> {
    let text = memory::text(text)?;
    let replaced = match replaces {
        Some(old) => Some(
            one(db, old.as_str(), Some(project_id))
                .await?
                .ok_or_else(|| not_found("Memory"))?,
        ),
        None => None,
    };
    let id = fctl_domain::ids::memory_id(now);
    let mut statements = vec![insert(
        db,
        id.as_str(),
        project_id,
        &text,
        writer.session(),
        Some(writer.person()),
        now,
    )?];
    if let Some(old) = &replaced {
        // Only once the new note is in; one stale already keeps its reason.
        statements.push(q(
            db,
            "UPDATE memories SET stale_at = COALESCE(stale_at, ?1), \
             stale_reason = CASE WHEN stale_at IS NULL THEN ?2 ELSE stale_reason END, \
             stale_session_id = CASE WHEN stale_at IS NULL THEN ?3 ELSE stale_session_id END, \
             stale_by = CASE WHEN stale_at IS NULL THEN ?4 ELSE stale_by END, replaced_by = ?5 \
             WHERE id = ?6 AND EXISTS (SELECT 1 FROM memories WHERE id = ?5)",
            &[
                n(now),
                s(&format!("Replaced by {id}")),
                os(writer.session()),
                s(writer.person()),
                s(id.as_str()),
                s(old.id.as_str()),
            ],
        )?);
    }
    batch(db, statements).await?;
    let memory = one(db, id.as_str(), None).await?.ok_or_else(full)?;
    let replaced = match replaced {
        Some(old) => one(db, old.id.as_str(), None).await?,
        None => None,
    };
    Ok((memory, replaced))
}

/// An agent marks a note stale (`fctl memory stale`), saying why. One its
/// session marked stale already stays as it is (the host may ask again);
/// one marked by someone else is refused, with their reason.
pub async fn mark_stale(
    db: &D1Database,
    session: &RunnerSession,
    id: &MemoryId,
    reason: &str,
    now: i64,
) -> AppResult<Memory> {
    let reason = memory::reason(Some(reason))?
        .ok_or_else(|| crate::error::validation("reason", "Say why the note is stale"))?;
    let note = one(db, id.as_str(), Some(&session.project_id))
        .await?
        .ok_or_else(|| not_found("Memory"))?;
    if let Some(stale) = &note.stale {
        if stale
            .session
            .as_ref()
            .is_some_and(|s| s.id.as_str() == session.id)
        {
            return Ok(note);
        }
        let why = stale
            .reason
            .as_deref()
            .map(|r| format!(": {r}"))
            .unwrap_or_default();
        return Err(conflict(&format!("{id} is stale already{why}")));
    }
    q(
        db,
        "UPDATE memories SET stale_at = ?, stale_reason = ?, stale_session_id = ?, stale_by = ? \
         WHERE id = ? AND stale_at IS NULL",
        &[
            n(now),
            s(&reason),
            s(&session.id),
            s(&session.created_by),
            s(id.as_str()),
        ],
    )?
    .run()
    .await?;
    one(db, id.as_str(), None)
        .await?
        .ok_or_else(|| not_found("Memory"))
}

// ───────────────────────────── for people ─────────────────────────────

fn require_editor(p: &ProjectAccess) -> AppResult<()> {
    if perm::can_edit_memory(p.role) {
        Ok(())
    } else {
        Err(forbidden(
            "You need the editor role in this project to change its memory",
        ))
    }
}

/// A note the caller may change, with its project.
async fn editable(ctx: &Ctx, id: &str) -> AppResult<(Memory, ProjectAccess)> {
    let note = one(&ctx.db, id, None)
        .await?
        .ok_or_else(|| not_found("Memory"))?;
    let p = access::project(ctx, note.project_id.as_str())
        .await
        .map_err(|_| not_found("Memory"))?;
    require_editor(&p)?;
    Ok((note, p))
}

/// `GET /api/v1/projects/{id}/memories[?session=ses_…]`: for everyone who
/// sees the project, whether or not its agents keep the memory now; with
/// `session`, the notes that session's agent wrote or marked stale.
pub async fn project_list(ctx: &Ctx, req: &Request, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    let mut memories = list(&ctx.db, &p.row.id, true).await?;
    let current = memories.iter().filter(|m| m.stale.is_none()).count();
    if let Some(session) = http::query(req, "session") {
        memories.retain(|m| {
            m.session.as_ref().is_some_and(|s| s.id.as_str() == session)
                || m.stale
                    .as_ref()
                    .and_then(|st| st.session.as_ref())
                    .is_some_and(|s| s.id.as_str() == session)
        });
    }
    http::ok(&MemoryList {
        memories,
        current: Some(current.try_into().unwrap_or(u32::MAX)),
    })
}

/// `GET /api/v1/memories/{id}`: one note, for everyone who sees its
/// project.
pub async fn get(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let note = one(&ctx.db, id, None)
        .await?
        .ok_or_else(|| not_found("Memory"))?;
    access::project(ctx, note.project_id.as_str())
        .await
        .map_err(|_| not_found("Memory"))?;
    http::ok(&note)
}

/// `POST /api/v1/projects/{id}/memories` (editors): a note from a person.
pub async fn add(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: AddMemoryRequest = http::body(req).await?;
    let p = access::project(ctx, id).await?;
    require_editor(&p)?;
    let user = ctx.user()?.id.clone();
    let (note, replaced) = remember(
        &ctx.db,
        &p.row.id,
        &Writer::Person(&user),
        &body.text,
        body.replaces.as_ref(),
        ctx.now,
    )
    .await?;
    ctx.audit_in(
        Some(p.workspace_id()),
        "memory.added",
        Some(note.id.as_str()),
        Some(serde_json::json!({
            "project_id": p.row.id,
            "text": memory::headline(&note.text, 80),
            "replaces": replaced.map(|m| m.id),
        })),
    );
    http::created(&note)
}

/// `PATCH /api/v1/memories/{id}` (editors): changes a note's text, marks
/// it stale or makes it current again.
pub async fn update(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdateMemoryRequest = http::body(req).await?;
    let (note, p) = editable(ctx, id).await?;
    let user = ctx.user()?.id.clone();
    let mut statements = Vec::new();
    let mut changed = Vec::new();
    if let Some(text) = &body.text {
        let text = memory::text(text)?;
        if text != note.text {
            statements.push(q(
                &ctx.db,
                "UPDATE memories SET text = ?, edited_at = ?, edited_by = ? WHERE id = ?",
                &[s(&text), n(ctx.now), s(&user), s(note.id.as_str())],
            )?);
            changed.push("text");
        }
    }
    match body.stale {
        // A stale note keeps when, why and who.
        Some(true) if note.stale.is_some() => {}
        Some(true) => {
            let reason = memory::reason(body.reason.as_deref())?;
            statements.push(q(
                &ctx.db,
                "UPDATE memories SET stale_at = ?, stale_reason = ?, stale_session_id = NULL, stale_by = ? WHERE id = ? AND stale_at IS NULL",
                &[n(ctx.now), os(reason.as_deref()), s(&user), s(note.id.as_str())],
            )?);
            changed.push("stale");
        }
        Some(false) if note.stale.is_some() => {
            statements.push(q(
                &ctx.db,
                "UPDATE memories SET stale_at = NULL, stale_reason = NULL, stale_session_id = NULL, stale_by = NULL, replaced_by = NULL WHERE id = ?",
                &[s(note.id.as_str())],
            )?);
            changed.push("current");
        }
        _ => {}
    }
    if !statements.is_empty() {
        batch(&ctx.db, statements).await?;
        ctx.audit_in(
            Some(p.workspace_id()),
            "memory.edited",
            Some(note.id.as_str()),
            Some(serde_json::json!({
                "project_id": p.row.id,
                "changed": changed,
                "text": memory::headline(&note.text, 80),
            })),
        );
    }
    let note = one(&ctx.db, note.id.as_str(), None)
        .await?
        .ok_or_else(|| not_found("Memory"))?;
    http::ok(&note)
}

/// `DELETE /api/v1/memories/{id}` (editors).
pub async fn delete(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let (note, p) = editable(ctx, id).await?;
    q(
        &ctx.db,
        "DELETE FROM memories WHERE id = ?",
        &[s(note.id.as_str())],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(p.workspace_id()),
        "memory.deleted",
        Some(note.id.as_str()),
        Some(serde_json::json!({
            "project_id": p.row.id,
            "text": memory::headline(&note.text, 80),
        })),
    );
    http::ok(&Ok { ok: true })
}
