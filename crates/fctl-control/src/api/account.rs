//! Your own data: export it, or delete your account (Settings → Danger zone).
//!
//! Deleting an account deletes its sign-in methods, addresses, tokens,
//! Claude connections, memberships, shares, follows and notification
//! settings and browsers, and the workspaces only it is a member of, with
//! their projects, sessions and transcripts. In the workspaces it was the
//! primary admin of with other people in them, the role passes to the
//! longest-standing admin, or member. Sessions and projects it created that
//! others still use stay; the account then remains as a nameless, disabled
//! placeholder (`fctl_domain::account`). Admins deleting a user go through
//! the same `erase`.

use crate::auth;
use crate::ctx::Ctx;
use crate::db::{Count, Val, batch, n, q, s};
use crate::error::{AppResult, conflict, internal, validation};
use crate::http;
use crate::internal as dobj;
use crate::model::{UserRow, WORKSPACE_ORDER, workspace_ref};
use fctl_domain::account::{self as acct, iso8601};
use fctl_domain::verify::Sensitive;
use fctl_proto::api::{
    AccountDeletion, DeleteAccountRequest, DeletedProject, Handover, Ok as Done,
};
use fctl_proto::session::{InstanceRole, UserRef};
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Map, Value, json};
use std::collections::VecDeque;
use worker::{Env, Request, Response};

const ACTIVE: &str = "('queued','provisioning','preparing','running','idle','paused','stopping')";

/// Workspaces where `?1` is the only member.
const SOLE_WORKSPACES: &str = "SELECT m.workspace_id FROM workspace_members m WHERE m.user_id = ?1 \
     AND NOT EXISTS (SELECT 1 FROM workspace_members o WHERE o.workspace_id = m.workspace_id AND o.user_id != ?1)";

// ───────────────────────────── deletion ─────────────────────────────

#[derive(Deserialize)]
struct SoleWorkspace {
    id: String,
    slug: String,
    name: String,
    color: Option<String>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    avatar_updated_at: Option<i64>,
}

#[derive(Deserialize)]
struct SoleProject {
    slug: String,
    name: String,
    sessions: i64,
}

/// A workspace whose primary admin role passes to someone else.
#[derive(Deserialize)]
struct Successor {
    id: String,
    slug: String,
    name: String,
    color: Option<String>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    avatar_updated_at: Option<i64>,
    successor_id: String,
    successor_name: String,
    successor_avatar: Option<String>,
}

/// What deleting an account involves.
pub struct Plan {
    sole: Vec<SoleWorkspace>,
    projects: Vec<SoleProject>,
    successors: Vec<Successor>,
    kept_sessions: i64,
    /// Sessions that must be stopped first: theirs, or in workspaces that go.
    running: i64,
    /// The account is the primary instance owner.
    primary: bool,
}

impl Plan {
    fn self_blocker(&self) -> Option<String> {
        if self.primary {
            return Some(
                "You are the primary instance owner. Hand the role to an admin (Admin → People) before you delete your account."
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
                .projects
                .iter()
                .map(|p| DeletedProject {
                    slug: p.slug.clone(),
                    name: p.name.clone(),
                    sessions: p.sessions.max(0) as u32,
                })
                .collect(),
            kept_sessions: self.kept_sessions.max(0) as u32,
            workspaces: self
                .sole
                .iter()
                .map(|w| {
                    workspace_ref(
                        &w.id,
                        &w.slug,
                        &w.name,
                        w.color.as_deref(),
                        w.avatar_updated_at,
                    )
                })
                .collect(),
            handovers: self
                .successors
                .iter()
                .map(|w| Handover {
                    workspace: workspace_ref(
                        &w.id,
                        &w.slug,
                        &w.name,
                        w.color.as_deref(),
                        w.avatar_updated_at,
                    ),
                    to: UserRef {
                        id: fctl_proto::UserId(w.successor_id.clone()),
                        name: w.successor_name.clone(),
                        avatar_url: w.successor_avatar.clone(),
                    },
                })
                .collect(),
        }
    }
}

async fn count(ctx: &Ctx, sql: &str, params: &[Val]) -> AppResult<i64> {
    let c: Option<Count> = q(&ctx.db, sql, params)?.first().await?;
    Ok(c.map_or(0, |c| c.n))
}

pub async fn plan(ctx: &Ctx, user: &UserRow) -> AppResult<Plan> {
    let sole: Vec<SoleWorkspace> = q(
        &ctx.db,
        &format!(
            "SELECT w.id, w.slug, w.name, w.color, w.avatar_updated_at FROM workspaces w \
             WHERE w.id IN ({SOLE_WORKSPACES}) ORDER BY w.name"
        ),
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let projects: Vec<SoleProject> = q(
        &ctx.db,
        &format!(
            "SELECT p.slug AS slug, p.name AS name, \
             (SELECT COUNT(*) FROM sessions x WHERE x.project_id = p.id) AS sessions \
             FROM projects p WHERE p.workspace_id IN ({SOLE_WORKSPACES}) ORDER BY p.name"
        ),
        &[s(&user.id)],
    )?
    .all()
    .await?;
    // Deleted by an instance admin, the role doesn't pass to them unless
    // they are one of its admins already (`successor!`).
    let actor = ctx.user()?.id.clone();
    let successors: Vec<Successor> = q(
        &ctx.db,
        &format!(
            "SELECT w.id, w.slug, w.name, w.color, w.avatar_updated_at, su.id AS successor_id, \
             su.name AS successor_name, su.avatar_url AS successor_avatar FROM workspaces w \
             JOIN workspace_members m ON m.workspace_id = w.id AND m.user_id = ?1 AND m.role = 'owner' \
             JOIN users su ON su.id = {} ORDER BY w.name",
            crate::successor!("?2")
        ),
        &[s(&user.id), s(&actor)],
    )?
    .all()
    .await?;
    let kept_sessions = count(
        ctx,
        &format!(
            "SELECT COUNT(*) AS n FROM sessions s JOIN projects p ON p.id = s.project_id \
             WHERE s.created_by = ?1 AND (p.workspace_id IS NULL OR p.workspace_id NOT IN ({SOLE_WORKSPACES}))"
        ),
        &[s(&user.id)],
    )
    .await?;
    let running = count(
        ctx,
        &format!(
            "SELECT COUNT(*) AS n FROM sessions s JOIN projects p ON p.id = s.project_id \
             WHERE (s.created_by = ?1 OR p.workspace_id IN ({SOLE_WORKSPACES})) AND s.status IN {ACTIVE}"
        ),
        &[s(&user.id)],
    )
    .await?;
    Ok(Plan {
        sole,
        projects,
        successors,
        kept_sessions,
        running,
        primary: user.role() == InstanceRole::Owner,
    })
}

/// Deletes `user`'s account as `plan` describes, in one transaction, then
/// purges the deleted sessions' transcripts.
pub async fn erase(ctx: &mut Ctx, user: &UserRow, plan: &Plan) -> AppResult<()> {
    let db = &ctx.db;
    let id = user.id.as_str();
    let mut statements = Vec::new();
    for w in &plan.sole {
        statements.push(q(
            db,
            "INSERT OR IGNORE INTO session_purges (session_id, queued_at) \
             SELECT s.id, ? FROM sessions s JOIN projects p ON p.id = s.project_id WHERE p.workspace_id = ?",
            &[n(ctx.now), s(&w.id)],
        )?);
        statements.push(q(
            db,
            "INSERT OR IGNORE INTO file_purges (project_id, queued_at) SELECT id, ? FROM projects WHERE workspace_id = ?",
            &[n(ctx.now), s(&w.id)],
        )?);
        statements.push(q(
            db,
            "DELETE FROM audit_log WHERE workspace_id = ?",
            &[s(&w.id)],
        )?);
        statements.push(q(db, "DELETE FROM workspaces WHERE id = ?", &[s(&w.id)])?);
    }
    // Their addresses' invitations and allowlist entries go before the
    // addresses do.
    for sql in [
        "DELETE FROM workspace_invitations WHERE email IN (SELECT email FROM user_emails WHERE user_id = ?1) OR email = ?2",
        "DELETE FROM allowlist WHERE pattern IN (SELECT email FROM user_emails WHERE user_id = ?1) OR pattern = ?2",
        "DELETE FROM invites WHERE email IN (SELECT email FROM user_emails WHERE user_id = ?1) OR email = ?2",
        "DELETE FROM access_requests WHERE email IN (SELECT email FROM user_emails WHERE user_id = ?1) OR email = ?2",
    ] {
        statements.push(q(db, sql, &[s(id), s(&user.email)])?);
    }
    // Their runners leave every workspace and their tokens stop working:
    // nobody answers for those machines any more.
    #[derive(Deserialize)]
    struct Runner {
        id: String,
    }
    let runners: Vec<Runner> = q(db, "SELECT id FROM runners WHERE created_by = ?", &[s(id)])?
        .all()
        .await?;
    statements.push(q(
        db,
        "DELETE FROM runner_workspaces WHERE runner_id IN (SELECT id FROM runners WHERE created_by = ?)",
        &[s(id)],
    )?);
    statements.push(q(
        db,
        "UPDATE runners SET enabled = 0 WHERE created_by = ?",
        &[s(id)],
    )?);
    for table in [
        "identities",
        "tokens",
        "claude_credentials",
        "claude_login_jobs",
        "password_resets",
        "device_codes",
        "project_members",
        "project_choices",
        "session_shares",
        "session_followers",
        "notification_settings",
        "push_subscriptions",
        "authenticator_apps",
        "recovery_codes",
        "sign_in_challenges",
        "passkeys",
        "passkey_challenges",
        "workspace_members",
        "user_emails",
    ] {
        statements.push(q(
            db,
            &format!("DELETE FROM {table} WHERE user_id = ?"),
            &[s(id)],
        )?);
    }
    // The primary admin role of their other workspaces passes on, once
    // their membership is gone (one per workspace).
    for w in &plan.successors {
        statements.push(q(
            db,
            "UPDATE workspace_members SET role = 'owner' WHERE workspace_id = ? AND user_id = ?",
            &[s(&w.id), s(&w.successor_id)],
        )?);
    }
    // A placeholder if what they created stays, otherwise nothing at all.
    statements.push(q(
        db,
        "UPDATE users SET email = ?, name = ?, given_name = NULL, family_name = NULL, avatar_url = NULL, commit_email = NULL, password_hash = NULL, role = 'member', \
         status = 'disabled', default_credential_id = NULL, last_login_at = NULL, two_factor_google = 0, updated_at = ? \
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
    let members: Vec<String> = plan
        .successors
        .iter()
        .map(|w| w.successor_id.clone())
        .collect();
    batch(db, statements).await?;
    if !runners.is_empty() {
        let fleet = crate::internal::fleet_stub(&ctx.env)?;
        for r in &runners {
            let _: serde_json::Value = crate::internal::post(
                &fleet,
                "/runner-disabled",
                &serde_json::json!({ "runner_id": r.id }),
            )
            .await?;
        }
    }
    if !plan.sole.is_empty() {
        let env = ctx.env.clone();
        let db = crate::db::handle(&ctx.db);
        ctx.defer(async move { crate::housekeeping::purge_deleted(&env, &db).await });
    }
    for w in &plan.successors {
        ctx.audit_in(
            Some(&w.id),
            "workspace.transferred",
            Some(&w.successor_id),
            Some(json!({ "from": id, "deleted": true })),
        );
    }
    crate::workspaces::changed(ctx, members);
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
    ctx.verified(Sensitive::DeleteAccount)?;
    erase(ctx, &user, &plan).await?;
    auth::clear_session_cookie(ctx);
    ctx.audit(
        "user.deleted",
        Some(&user.id),
        Some(json!({ "self": true, "workspaces_deleted": plan.sole.len(), "projects_deleted": plan.projects.len() })),
    );
    http::ok(&Done { ok: true })
}

// ───────────────────────────── export ─────────────────────────────

/// Times (`ts`, `*_at`) become RFC 3339; `data` and `pull_requests`
/// columns holding JSON are inlined.
fn readable(mut row: Value) -> Value {
    if let Value::Object(map) = &mut row {
        for (k, v) in map.iter_mut() {
            if (k == "ts" || k.ends_with("_at"))
                && let Some(ms) = v.as_f64()
            {
                *v = Value::String(iso8601(ms as i64));
            }
        }
        for key in ["data", "pull_requests", "project_order"] {
            let parsed = match map.get(key) {
                Some(Value::String(d)) => serde_json::from_str::<Value>(d).ok(),
                _ => None,
            };
            if let Some(p) = parsed {
                map.insert(key.into(), p);
            }
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
    ctx.verified(Sensitive::Export)?;
    ctx.rate_limit(&format!("export:{}", user.id)).await?;
    let me = || [s(&user.id)];
    // What they made in workspaces is theirs to export while those are
    // open to them, as it is theirs to see: not once they left one, or
    // while its two-factor requirement closes it to them.
    let settings = ctx.settings();
    let open: Vec<String> = crate::workspaces::mine(&ctx.db, &user.id)
        .await?
        .into_iter()
        .filter(|w| !user.blocked_by(&settings, w.require_two_factor))
        .map(|w| w.id)
        .collect();
    let mine_open =
        || -> AppResult<[Val; 2]> { Ok([s(&user.id), s(&serde_json::to_string(&open)?)]) };
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
        "two_factor".into(),
        json!({
            "ask_on_google_sign_in": user.two_factor_google,
            "authenticator_app": rows(
                ctx,
                "SELECT created_at, last_used_at, failures AS wrong_codes_in_a_row, locked_until FROM authenticator_apps WHERE user_id = ?",
                &me(),
            )
            .await?
            .as_array()
            .and_then(|a| a.first().cloned()),
            "recovery_codes": rows(
                ctx,
                "SELECT created_at, used_at FROM recovery_codes WHERE user_id = ? ORDER BY created_at, used_at",
                &me(),
            )
            .await?,
            "waiting_sign_ins": rows(
                ctx,
                "SELECT reason, created_at, expires_at FROM sign_in_challenges WHERE user_id = ? ORDER BY created_at",
                &me(),
            )
            .await?,
        }),
    );
    doc.insert(
        "passkeys".into(),
        rows(
            ctx,
            "SELECT name, aaguid AS authenticator_model, backed_up AS synced, transports, created_at, last_used_at FROM passkeys WHERE user_id = ? ORDER BY created_at",
            &me(),
        )
        .await?,
    );
    doc.insert(
        "devices_and_tokens".into(),
        rows(
            ctx,
            "SELECT kind, name, user_agent, ip, created_at, last_used_at, expires_at, scope, verified_at FROM tokens WHERE user_id = ? ORDER BY created_at",
            &me(),
        )
        .await?,
    );
    doc.insert(
        "cli_sign_in_requests".into(),
        rows(
            ctx,
            "SELECT client_name, CASE WHEN token_id IS NULL THEN 'sign_in' ELSE 'confirm' END AS purpose, status, ip, location, user_agent, created_at, expires_at \
             FROM device_codes WHERE user_id = ? ORDER BY created_at",
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
        "claude_plan_usage".into(),
        rows(
            ctx,
            "SELECT c.account_email, c.label, u.window, u.utilization, u.resets_at, u.status, u.observed_at \
             FROM claude_usage u JOIN claude_credentials c ON c.id = u.credential_id WHERE c.user_id = ? ORDER BY c.created_at, u.window",
            &me(),
        )
        .await?,
    );
    doc.insert(
        "notification_settings".into(),
        json!(crate::notify::settings(&ctx.db, &user.id).await?),
    );
    doc.insert(
        "notification_browsers".into(),
        rows(
            ctx,
            "SELECT name, endpoint, created_at, last_sent_at FROM push_subscriptions WHERE user_id = ? ORDER BY created_at",
            &me(),
        )
        .await?,
    );
    doc.insert(
        "followed_sessions".into(),
        rows(
            ctx,
            // Only the ones in their workspaces.
            "SELECT p.slug AS project, s.number, s.title, f.created_at AS followed_at FROM session_followers f \
             JOIN sessions s ON s.id = f.session_id JOIN projects p ON p.id = s.project_id \
             JOIN workspace_members wm ON wm.workspace_id = p.workspace_id AND wm.user_id = ?1 \
             WHERE f.user_id = ?1 ORDER BY f.created_at",
            &[s(&user.id)],
        )
        .await?,
    );
    doc.insert(
        "email_addresses".into(),
        rows(
            ctx,
            "SELECT email, email = (SELECT email FROM users WHERE id = ?1) AS \"primary\", commit_email, created_at AS added_at \
             FROM user_emails WHERE user_id = ?1 ORDER BY created_at",
            &me(),
        )
        .await?,
    );
    // Until the hourly cron trigger deletes them, once the address has an
    // account.
    doc.insert(
        "early_access_requests".into(),
        rows(
            ctx,
            "SELECT email, created_at AS first_asked_at, asked_at AS last_asked_at, invited_at FROM access_requests \
             WHERE email IN (SELECT email FROM user_emails WHERE user_id = ?1) ORDER BY created_at",
            &me(),
        )
        .await?,
    );
    doc.insert(
        "workspaces".into(),
        rows(
            ctx,
            // In their order, with the order of each one's projects they chose.
            &format!(
                "SELECT w.slug, w.name, wm.role, wm.email AS as_address, wm.created_at AS joined_at, \
                 CASE WHEN wm.project_order IS NOT NULL THEN (SELECT json_group_array(slug) FROM \
                 (SELECT p.slug FROM json_each(wm.project_order) o JOIN projects p ON p.id = o.value ORDER BY o.key)) \
                 END AS project_order \
                 FROM workspace_members wm JOIN workspaces w ON w.id = wm.workspace_id WHERE wm.user_id = ? \
                 ORDER BY {WORKSPACE_ORDER}"
            ),
            &me(),
        )
        .await?,
    );
    doc.insert(
        "projects".into(),
        rows(
            ctx,
            "SELECT p.slug, p.name, p.description, p.repo_owner, p.repo_name, m.role, m.created_at AS joined_at \
             FROM project_members m JOIN projects p ON p.id = m.project_id WHERE m.user_id = ?1 \
             AND p.workspace_id IN (SELECT value FROM json_each(?2)) ORDER BY p.name",
            &mine_open()?,
        )
        .await?,
    );
    doc.insert(
        "last_model_and_effort".into(),
        rows(
            ctx,
            // What they asked for when they last started a session in each
            // project (none: the project's), where its composers start.
            "SELECT p.slug AS project, c.model, c.effort, c.updated_at AS asked_at FROM project_choices c \
             JOIN projects p ON p.id = c.project_id WHERE c.user_id = ?1 \
             AND p.workspace_id IN (SELECT value FROM json_each(?2)) ORDER BY p.slug",
            &mine_open()?,
        )
        .await?,
    );
    doc.insert(
        "memories".into(),
        rows(
            ctx,
            // The notes they wrote in projects' memories, and those the
            // agents of their sessions wrote (`session`); the sessions'
            // retrospectives are in their transcripts.
            "SELECT p.slug AS project, s.number AS session, m.text, m.created_at, m.edited_at, m.stale_at, m.stale_reason \
             FROM memories m JOIN projects p ON p.id = m.project_id LEFT JOIN sessions s ON s.id = m.session_id \
             WHERE m.created_by = ?1 AND p.workspace_id IN (SELECT value FROM json_each(?2)) ORDER BY m.created_at",
            &mine_open()?,
        )
        .await?,
    );
    doc.insert(
        "schedules".into(),
        rows(
            ctx,
            // The schedules they set up, and those that run on their
            // subscription; what each run started is in the sessions.
            "SELECT p.slug AS project, sc.name, sc.prompt, sc.cron, sc.at, sc.timezone, sc.model, sc.effort, \
             sc.base_branch, sc.github_access, sc.enabled, sc.owner_id = ?1 AS runs_on_mine, sc.created_at, sc.updated_at \
             FROM schedules sc JOIN projects p ON p.id = sc.project_id WHERE (sc.created_by = ?1 OR sc.owner_id = ?1) \
             AND p.workspace_id IN (SELECT value FROM json_each(?2)) ORDER BY sc.created_at",
            &mine_open()?,
        )
        .await?,
    );
    doc.insert(
        "activity".into(),
        rows(
            ctx,
            // The addresses of their own requests, not of others' that
            // concerned them (an admin's, say); in workspaces, those open
            // to them, as the rest.
            "SELECT ts, action, target, data, CASE WHEN actor_id = ?1 THEN ip END AS ip FROM audit_log \
             WHERE (actor_id = ?1 OR target = ?1) \
             AND (workspace_id IS NULL OR workspace_id IN (SELECT value FROM json_each(?2))) ORDER BY ts",
            &mine_open()?,
        )
        .await?,
    );
    let sessions: Vec<Value> = q(
        &ctx.db,
        "SELECT s.id, p.slug AS project, s.number, s.title, s.description, s.prompt, s.status, s.branch, s.base_branch, \
         s.model, s.effort, s.claude_effort, s.pr_url AS pull_request, s.pull_requests, s.created_at, s.started_at, s.ended_at \
         FROM sessions s JOIN projects p ON p.id = s.project_id WHERE s.created_by = ?1 \
         AND p.workspace_id IN (SELECT value FROM json_each(?2)) ORDER BY s.created_at",
        &mine_open()?,
    )?
    .all()
    .await?;
    // Only those they can still see: starting a session counts while they
    // may start sessions in its project (`perm::session_role`).
    let visible: std::collections::HashSet<String> = q(
        &ctx.db,
        &format!("{} WHERE s.created_by = ?1", crate::model::SESSION_SELECT),
        &me(),
    )?
    .all::<crate::model::SessionRow>()
    .await?
    .into_iter()
    .filter(|r| crate::access::session_role(&user, &settings, None, r).is_some())
    .map(|r| r.id)
    .collect();
    let mut queue = VecDeque::new();
    for row in sessions {
        let id = row
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if !visible.contains(&id) {
            continue;
        }
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
