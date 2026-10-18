//! Your own data: export it, or delete your account (Settings → Danger zone).
//!
//! Deleting an account deletes its sign-in methods, tokens, Claude
//! connections, memberships and shares, and the projects only it is a member
//! of, with their sessions and transcripts. Sessions and projects it created
//! that others still use stay; the account then remains as a nameless,
//! disabled placeholder (`fctl_domain::account`). Admins deleting a user go
//! through the same `erase`.

use crate::auth;
use crate::ctx::Ctx;
use crate::db::{Count, Val, batch, n, q, s};
use crate::error::{AppResult, conflict, internal, validation};
use crate::http;
use crate::internal as dobj;
use crate::model::UserRow;
use fctl_domain::account::{self as acct, iso8601};
use fctl_proto::api::{AccountDeletion, DeleteAccountRequest, DeletedProject, Ok as Done};
use fctl_proto::session::InstanceRole;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Map, Value, json};
use std::collections::VecDeque;
use worker::{Env, Request, Response};

const ACTIVE: &str = "('queued','provisioning','preparing','running','idle','paused','stopping')";

/// Projects where `?1` is the only member.
const SOLE_PROJECTS: &str = "SELECT m.project_id FROM project_members m WHERE m.user_id = ?1 \
     AND NOT EXISTS (SELECT 1 FROM project_members o WHERE o.project_id = m.project_id AND o.user_id != ?1)";

// ───────────────────────────── deletion ─────────────────────────────

#[derive(Deserialize)]
struct SoleProject {
    id: String,
    slug: String,
    name: String,
    sessions: i64,
}

/// What deleting an account involves.
pub struct Plan {
    sole: Vec<SoleProject>,
    kept_sessions: i64,
    /// Sessions that must be stopped first: theirs, or in projects that go.
    running: i64,
    /// The account is the instance's last active owner.
    last_owner: bool,
}

impl Plan {
    fn self_blocker(&self) -> Option<String> {
        if self.last_owner {
            return Some(
                "You are this instance's only owner. Make someone else an owner before you delete your account."
                    .into(),
            );
        }
        if self.running > 0 {
            return Some(format!(
                "Stop your running sessions first ({} running).",
                self.running
            ));
        }
        None
    }

    /// Running sessions that must be stopped before the account can go.
    pub fn running(&self) -> i64 {
        self.running
    }

    fn preview(&self) -> AccountDeletion {
        AccountDeletion {
            blocked: self.self_blocker(),
            projects: self
                .sole
                .iter()
                .map(|p| DeletedProject {
                    slug: p.slug.clone(),
                    name: p.name.clone(),
                    sessions: p.sessions.max(0) as u32,
                })
                .collect(),
            kept_sessions: self.kept_sessions.max(0) as u32,
        }
    }
}

async fn count(ctx: &Ctx, sql: &str, params: &[Val]) -> AppResult<i64> {
    let c: Option<Count> = q(&ctx.db, sql, params)?.first().await?;
    Ok(c.map_or(0, |c| c.n))
}

pub async fn plan(ctx: &Ctx, user: &UserRow) -> AppResult<Plan> {
    let sole: Vec<SoleProject> = q(
        &ctx.db,
        &format!(
            "SELECT p.id AS id, p.slug AS slug, p.name AS name, \
             (SELECT COUNT(*) FROM sessions x WHERE x.project_id = p.id) AS sessions \
             FROM projects p WHERE p.id IN ({SOLE_PROJECTS}) ORDER BY p.name"
        ),
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let kept_sessions = count(
        ctx,
        &format!(
            "SELECT COUNT(*) AS n FROM sessions WHERE created_by = ?1 AND project_id NOT IN ({SOLE_PROJECTS})"
        ),
        &[s(&user.id)],
    )
    .await?;
    let running = count(
        ctx,
        &format!(
            "SELECT COUNT(*) AS n FROM sessions WHERE (created_by = ?1 OR project_id IN ({SOLE_PROJECTS})) AND status IN {ACTIVE}"
        ),
        &[s(&user.id)],
    )
    .await?;
    let last_owner = user.role() == InstanceRole::Owner
        && count(
            ctx,
            "SELECT COUNT(*) AS n FROM users WHERE role = 'owner' AND status = 'active' AND id != ?",
            &[s(&user.id)],
        )
        .await?
            == 0;
    Ok(Plan {
        sole,
        kept_sessions,
        running,
        last_owner,
    })
}

/// Deletes `user`'s account as `plan` describes, in one transaction, then
/// purges the deleted sessions' transcripts.
pub async fn erase(ctx: &mut Ctx, user: &UserRow, plan: &Plan) -> AppResult<()> {
    let db = &ctx.db;
    let id = user.id.as_str();
    let mut statements = Vec::new();
    for p in &plan.sole {
        statements.push(q(
            db,
            "INSERT OR IGNORE INTO session_purges (session_id, queued_at) SELECT id, ? FROM sessions WHERE project_id = ?",
            &[n(ctx.now), s(&p.id)],
        )?);
        statements.push(q(
            db,
            "INSERT OR IGNORE INTO file_purges (project_id, queued_at) VALUES (?, ?)",
            &[s(&p.id), n(ctx.now)],
        )?);
        statements.push(q(db, "DELETE FROM projects WHERE id = ?", &[s(&p.id)])?);
    }
    for table in [
        "identities",
        "tokens",
        "claude_credentials",
        "claude_login_jobs",
        "password_resets",
        "device_codes",
        "project_members",
        "session_shares",
    ] {
        statements.push(q(
            db,
            &format!("DELETE FROM {table} WHERE user_id = ?"),
            &[s(id)],
        )?);
    }
    statements.push(q(
        db,
        "DELETE FROM allowlist WHERE pattern = ?",
        &[s(&user.email)],
    )?);
    statements.push(q(
        db,
        "DELETE FROM invites WHERE email = ?",
        &[s(&user.email)],
    )?);
    // A placeholder if what they created stays, otherwise nothing at all.
    statements.push(q(
        db,
        "UPDATE users SET email = ?, name = ?, given_name = NULL, family_name = NULL, avatar_url = NULL, password_hash = NULL, role = 'member', \
         status = 'disabled', default_credential_id = NULL, last_login_at = NULL, updated_at = ? \
         WHERE id = ? AND (EXISTS (SELECT 1 FROM sessions WHERE created_by = ?) OR EXISTS (SELECT 1 FROM projects WHERE created_by = ?))",
        &[
            s(&acct::tombstone_email(id)),
            s(acct::DELETED_NAME),
            n(ctx.now),
            s(id),
            s(id),
            s(id),
        ],
    )?);
    statements.push(q(
        db,
        "DELETE FROM users WHERE id = ? AND NOT EXISTS (SELECT 1 FROM sessions WHERE created_by = ?) \
         AND NOT EXISTS (SELECT 1 FROM projects WHERE created_by = ?)",
        &[s(id), s(id), s(id)],
    )?);
    batch(db, statements).await?;
    if !plan.sole.is_empty() {
        let env = ctx.env.clone();
        let db = crate::db::handle(&ctx.db);
        ctx.defer(async move { crate::housekeeping::purge_deleted(&env, &db).await });
    }
    Ok(())
}

/// `GET /me/deletion`.
pub async fn deletion(ctx: &mut Ctx) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    http::ok(&plan(ctx, &user).await?.preview())
}

/// `POST /me/delete`.
pub async fn delete(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: DeleteAccountRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    if !acct::confirms_deletion(&body.email, &user.email) {
        return Err(validation(
            "email",
            "Type your email address exactly as shown to confirm",
        ));
    }
    let plan = plan(ctx, &user).await?;
    if let Some(reason) = plan.self_blocker() {
        return Err(conflict(&reason));
    }
    erase(ctx, &user, &plan).await?;
    auth::clear_session_cookie(ctx);
    ctx.audit(
        "user.deleted",
        Some(&user.id),
        Some(json!({ "self": true, "projects_deleted": plan.sole.len() })),
    );
    http::ok(&Done { ok: true })
}

// ───────────────────────────── export ─────────────────────────────

/// Times (`ts`, `*_at`) become RFC 3339; `data` columns holding JSON are
/// inlined.
fn readable(mut row: Value) -> Value {
    if let Value::Object(map) = &mut row {
        for (k, v) in map.iter_mut() {
            if (k == "ts" || k.ends_with("_at"))
                && let Some(ms) = v.as_f64()
            {
                *v = Value::String(iso8601(ms as i64));
            }
        }
        let parsed = match map.get("data") {
            Some(Value::String(d)) => serde_json::from_str::<Value>(d).ok(),
            _ => None,
        };
        if let Some(p) = parsed {
            map.insert("data".into(), p);
        }
    }
    row
}

async fn rows(ctx: &Ctx, sql: &str, params: &[Val]) -> AppResult<Value> {
    let list: Vec<Value> = q(&ctx.db, sql, params)?.all().await?;
    Ok(Value::Array(list.into_iter().map(readable).collect()))
}

#[derive(Deserialize)]
struct Page {
    events: Vec<Box<RawValue>>,
    has_more: bool,
}

#[derive(Deserialize)]
struct Seq {
    seq: i64,
}

/// The export, as a stream: the document head, then each session with its
/// transcript paged out of its Durable Object, so memory stays flat.
struct Export {
    env: Env,
    head: Option<String>,
    /// (session id, its object without the closing brace)
    sessions: VecDeque<(String, String)>,
    /// The session being written and the last event sequence sent.
    current: Option<(String, i64)>,
    first_session: bool,
    first_event: bool,
    done: bool,
}

async fn page(env: &Env, session_id: &str, after: i64) -> AppResult<Page> {
    let stub = dobj::session_stub(env, session_id)?;
    dobj::get(&stub, &format!("/events?after={after}&limit=1000")).await
}

async fn next_chunk(mut st: Export) -> Option<(worker::Result<Vec<u8>>, Export)> {
    if let Some(head) = st.head.take() {
        return Some((Ok(head.into_bytes()), st));
    }
    if let Some((id, after)) = st.current.take() {
        return Some(match page(&st.env, &id, after).await {
            Ok(p) => {
                let last = p
                    .events
                    .last()
                    .and_then(|e| serde_json::from_str::<Seq>(e.get()).ok())
                    .map(|e| e.seq);
                let mut out = String::new();
                for e in &p.events {
                    if !st.first_event {
                        out.push(',');
                    }
                    st.first_event = false;
                    out.push_str(e.get());
                }
                match last {
                    Some(seq) if p.has_more && seq > after => st.current = Some((id, seq)),
                    _ => out.push_str("]}"),
                }
                (Ok(out.into_bytes()), st)
            }
            Err(e) => {
                st.sessions.clear();
                st.done = true;
                (Err(worker::Error::RustError(e.0.message)), st)
            }
        });
    }
    if let Some((id, open)) = st.sessions.pop_front() {
        let mut out = String::new();
        if !st.first_session {
            out.push(',');
        }
        st.first_session = false;
        out.push_str(&open);
        out.push_str(",\"transcript\":[");
        st.current = Some((id, 0));
        st.first_event = true;
        return Some((Ok(out.into_bytes()), st));
    }
    if !st.done {
        st.done = true;
        return Some((Ok(b"]}".to_vec()), st));
    }
    None
}

fn opened(json: &str) -> AppResult<String> {
    acct::open_object(json)
        .map(str::to_owned)
        .ok_or_else(|| internal("export: not a JSON object"))
}

/// `GET /me/export`: a JSON file with the account's personal data.
pub async fn export(ctx: &mut Ctx) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    ctx.rate_limit(&format!("export:{}", user.id)).await?;
    let me = || [s(&user.id)];
    let mut doc = Map::new();
    doc.insert("format".into(), json!("fctl-export/1"));
    doc.insert("exported_at".into(), json!(iso8601(ctx.now)));
    doc.insert("instance".into(), json!(ctx.cfg.public_url));
    doc.insert(
        "account".into(),
        readable(json!({
            "id": user.id,
            "email": user.email,
            "name": user.name,
            "given_name": user.given_name,
            "family_name": user.family_name,
            "avatar_url": user.avatar_url,
            "role": user.role,
            "status": user.status,
            "has_password": user.password_hash.is_some(),
            "created_at": user.created_at,
            "last_login_at": user.last_login_at,
        })),
    );
    doc.insert(
        "sign_in_methods".into(),
        rows(
            ctx,
            "SELECT provider, email, name, avatar_url, created_at FROM identities WHERE user_id = ? ORDER BY created_at",
            &me(),
        )
        .await?,
    );
    doc.insert(
        "devices_and_tokens".into(),
        rows(
            ctx,
            "SELECT kind, name, user_agent, ip, created_at, last_used_at, expires_at FROM tokens WHERE user_id = ? ORDER BY created_at",
            &me(),
        )
        .await?,
    );
    doc.insert(
        "cli_sign_in_requests".into(),
        rows(
            ctx,
            "SELECT client_name, status, ip, location, user_agent, created_at, expires_at FROM device_codes WHERE user_id = ? ORDER BY created_at",
            &me(),
        )
        .await?,
    );
    doc.insert(
        "claude_connections".into(),
        rows(
            ctx,
            "SELECT kind, label, account_email, subscription_type, status, created_at, last_used_at, expires_at FROM claude_credentials WHERE user_id = ? ORDER BY created_at",
            &me(),
        )
        .await?,
    );
    doc.insert(
        "projects".into(),
        rows(
            ctx,
            "SELECT p.slug, p.name, p.description, p.repo_owner, p.repo_name, m.role, m.created_at AS joined_at \
             FROM project_members m JOIN projects p ON p.id = m.project_id WHERE m.user_id = ? ORDER BY p.name",
            &me(),
        )
        .await?,
    );
    doc.insert(
        "activity".into(),
        rows(
            ctx,
            "SELECT ts, action, target, data, ip FROM audit_log WHERE actor_id = ?1 OR target = ?1 ORDER BY ts",
            &me(),
        )
        .await?,
    );
    let sessions: Vec<Value> = q(
        &ctx.db,
        "SELECT s.id, p.slug AS project, s.number, s.title, s.prompt, s.status, s.branch, s.base_branch, \
         s.model, s.effort, s.pr_url AS pull_request, s.created_at, s.started_at, s.ended_at \
         FROM sessions s JOIN projects p ON p.id = s.project_id WHERE s.created_by = ? ORDER BY s.created_at",
        &me(),
    )?
    .all()
    .await?;
    let mut queue = VecDeque::new();
    for row in sessions {
        let id = row
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        queue.push_back((id, opened(&serde_json::to_string(&readable(row))?)?));
    }
    let head = format!(
        "{},\"sessions\":[",
        opened(&serde_json::to_string(&Value::Object(doc))?)?
    );
    let stream = futures::stream::unfold(
        Export {
            env: ctx.env.clone(),
            head: Some(head),
            sessions: queue,
            current: None,
            first_session: true,
            first_event: true,
            done: false,
        },
        next_chunk,
    );
    let mut res = Response::from_stream(stream)?;
    let h = res.headers_mut();
    h.set("Content-Type", "application/json; charset=utf-8")?;
    h.set(
        "Content-Disposition",
        &format!(
            "attachment; filename=\"{}\"",
            acct::export_filename(ctx.now)
        ),
    )?;
    h.set("Cache-Control", "no-store")?;
    ctx.audit("user.exported", Some(&user.id), None);
    Ok(res)
}
