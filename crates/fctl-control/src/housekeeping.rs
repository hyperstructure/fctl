//! Deleting what is no longer needed: the event logs of deleted sessions,
//! the files of deleted projects, expired sign-in records, invitations and
//! what GitHub sign-ins offered to connect,
//! early access requests that are done or old, the links of expired
//! published screenshots and old audit entries. Runs
//! from the Worker's hourly cron trigger; deleting a project or a workspace
//! also purges its sessions and files right away. It also looks up the
//! pull requests sessions list that GitHub hasn't told about for a while
//! (`pull_requests::sweep`), and sets the alarm of schedules again
//! (`schedules::wake`), in case a change never reached it. It forgets the
//! delegations in Linear that waited an hour for their person, and gives
//! up fctl's access to Linear workspaces no fctl workspace works in. And it
//! has sessions put what the search index lacks of them there
//! (`index_sessions`). It removes the runners that waited a week for an
//! admin to approve them, and those that stayed offline longer than their
//! enrollment token lets them (`runners`).

use crate::db::{n, q, s};
use crate::error::AppResult;
use crate::internal;
use serde::Deserialize;
use worker::{D1Database, Env};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Audit entries older than this are deleted.
pub const AUDIT_RETENTION_MS: i64 = 365 * DAY_MS;
/// Claude login jobs are minutes long; anything older is abandoned.
const LOGIN_JOB_RETENTION_MS: i64 = DAY_MS;
/// Early access requests are deleted a year after they last asked.
pub const ACCESS_REQUEST_RETENTION_MS: i64 = 365 * DAY_MS;
/// Event logs purged per run (each is one Durable Object request).
pub const PURGES_PER_RUN: i64 = 200;
/// Sessions asked to index their transcript for search per run (each is
/// one Durable Object request), and how many at once.
pub const INDEXES_PER_RUN: i64 = 200;
const INDEXES_AT_ONCE: usize = 10;

#[derive(Deserialize)]
struct Queued {
    session_id: String,
}

/// Deletes the event logs of queued sessions (see `session_purges`).
/// Returns how many were purged; failures stay queued for the next run.
pub async fn purge_sessions(env: &Env, db: &D1Database, limit: i64) -> AppResult<usize> {
    // Only sessions whose rows are gone: the queue is filled in the same
    // batch that deletes them, so this is a safety net.
    q(
        db,
        "DELETE FROM session_purges WHERE session_id IN (SELECT id FROM sessions)",
        &[],
    )?
    .run()
    .await?;
    let queued: Vec<Queued> = q(
        db,
        "SELECT session_id FROM session_purges ORDER BY attempts, queued_at LIMIT ?",
        &[n(limit)],
    )?
    .all()
    .await?;
    let mut purged = 0;
    for row in queued {
        let result: AppResult<serde_json::Value> = async {
            let stub = internal::session_stub(env, &row.session_id)?;
            internal::post(&stub, "/purge", &serde_json::json!({})).await
        }
        .await;
        let next = match result {
            Ok(_) => {
                purged += 1;
                "DELETE FROM session_purges WHERE session_id = ?"
            }
            Err(e) => {
                worker::console_error!("purging session {}: {}", row.session_id, e.0.message);
                "UPDATE session_purges SET attempts = attempts + 1 WHERE session_id = ?"
            }
        };
        q(db, next, &[s(&row.session_id)])?.run().await?;
    }
    Ok(purged)
}

#[derive(Deserialize)]
struct QueuedFiles {
    project_id: String,
}

/// Deletes the files of deleted projects (see `file_purges`), like
/// [`purge_sessions`].
pub async fn purge_files(env: &Env, db: &D1Database, limit: i64) -> AppResult<usize> {
    q(
        db,
        "DELETE FROM file_purges WHERE project_id IN (SELECT id FROM projects)",
        &[],
    )?
    .run()
    .await?;
    let queued: Vec<QueuedFiles> = q(
        db,
        "SELECT project_id FROM file_purges ORDER BY attempts, queued_at LIMIT ?",
        &[n(limit)],
    )?
    .all()
    .await?;
    let mut purged = 0;
    for row in queued {
        let result: AppResult<serde_json::Value> = async {
            let stub = internal::files_stub(env, &row.project_id)?;
            internal::post(&stub, "/purge", &serde_json::json!({})).await
        }
        .await;
        let next = match result {
            Ok(_) => {
                purged += 1;
                "DELETE FROM file_purges WHERE project_id = ?"
            }
            Err(e) => {
                worker::console_error!("purging files of {}: {}", row.project_id, e.0.message);
                "UPDATE file_purges SET attempts = attempts + 1 WHERE project_id = ?"
            }
        };
        q(db, next, &[s(&row.project_id)])?.run().await?;
    }
    Ok(purged)
}

/// Has the SessionObjects of sessions that the search index lacks some of
/// put it there (`sessions.search_indexed`), the most recently active
/// first: the sessions from before search, and those whose index didn't
/// take what came. A long transcript goes on on its SessionObject's alarm.
/// Review agents' sessions aren't searched. Returns how many are done.
pub async fn index_sessions(env: &Env, db: &D1Database, limit: i64) -> AppResult<usize> {
    let queued: Vec<Queued> = q(
        db,
        "SELECT s.id AS session_id FROM sessions s WHERE s.search_indexed = 0 \
         AND NOT EXISTS (SELECT 1 FROM reviews rv WHERE rv.session_id = s.id) \
         ORDER BY s.last_activity_at DESC LIMIT ?",
        &[n(limit)],
    )?
    .all()
    .await?;
    let mut done = 0;
    for group in queued.chunks(INDEXES_AT_ONCE) {
        let asked = group.iter().map(|row| async move {
            let result: AppResult<internal::Indexed> = async {
                let stub = internal::session_stub(env, &row.session_id)?;
                internal::post(&stub, "/index", &serde_json::json!({})).await
            }
            .await;
            (row, result)
        });
        for (row, result) in futures::future::join_all(asked).await {
            match result {
                Ok(i) if i.done => {
                    done += 1;
                    q(
                        db,
                        "UPDATE sessions SET search_indexed = 1 WHERE id = ?",
                        &[s(&row.session_id)],
                    )?
                    .run()
                    .await?;
                }
                Ok(_) => {}
                Err(e) => worker::console_error!(
                    "indexing session {} for search: {}",
                    row.session_id,
                    e.0.message
                ),
            }
        }
    }
    Ok(done)
}

/// Purges what deleting projects left behind: session event logs and files.
pub async fn purge_deleted(env: &Env, db: &D1Database) -> AppResult<()> {
    purge_sessions(env, db, PURGES_PER_RUN).await?;
    purge_files(env, db, PURGES_PER_RUN).await?;
    Ok(())
}

/// The hourly job.
pub async fn run(env: &Env, now: i64) -> AppResult<()> {
    let db = env.d1("DB")?;
    let purged = purge_sessions(env, &db, PURGES_PER_RUN).await?;
    let purged_files = purge_files(env, &db, PURGES_PER_RUN).await?;
    let mut deleted = 0;
    for (sql, cutoff) in [
        // Expired tokens are already refused and hidden from Settings.
        (
            "DELETE FROM tokens WHERE expires_at IS NOT NULL AND expires_at < ?",
            now,
        ),
        ("DELETE FROM password_resets WHERE expires_at < ?", now),
        ("DELETE FROM sign_in_challenges WHERE expires_at < ?", now),
        ("DELETE FROM passkey_challenges WHERE expires_at < ?", now),
        ("DELETE FROM device_codes WHERE expires_at < ?", now),
        ("DELETE FROM github_offers WHERE expires_at < ?", now),
        // An invitation's admin role lasts as long as an admin's invitation
        // to the address: once none is left and no account has it, its
        // entry gives a member's, as revoking the invitation makes it, so
        // that a Google sign-up, which needs no invitation, makes no admin.
        (
            "UPDATE allowlist SET role = 'member' WHERE note = 'Invited' AND role = 'admin' \
             AND NOT EXISTS (SELECT 1 FROM users WHERE email = allowlist.pattern) \
             AND NOT EXISTS (SELECT 1 FROM user_emails WHERE email = allowlist.pattern) \
             AND NOT EXISTS (SELECT 1 FROM invites WHERE email = allowlist.pattern AND role = 'admin' \
               AND accepted_at IS NULL AND expires_at > ?)",
            now,
        ),
        ("DELETE FROM invites WHERE expires_at < ?", now),
        (
            "DELETE FROM workspace_invitations WHERE expires_at < ?",
            now,
        ),
        (
            "DELETE FROM access_requests WHERE asked_at < ?",
            now - ACCESS_REQUEST_RETENTION_MS,
        ),
        // Their links stopped working at expiry; their FilesObject deletes
        // the bytes on its own alarm.
        ("DELETE FROM published_files WHERE expires_at < ?", now),
        // What autopilots asked that people answered, declined or that
        // they took back, a quarter on: their logs say it.
        (
            "DELETE FROM autopilot_asks WHERE status != 'open' AND COALESCE(answered_at, created_at) < ?",
            now - 90 * 86_400_000,
        ),
        (
            "DELETE FROM claude_login_jobs WHERE updated_at < ?",
            now - LOGIN_JOB_RETENTION_MS,
        ),
        (
            "DELETE FROM audit_log WHERE ts < ?",
            now - AUDIT_RETENTION_MS,
        ),
    ] {
        deleted += q(&db, sql, &[n(cutoff)])?.run().await?;
    }
    // Early access requests whose address has an account now are done.
    deleted += q(
        &db,
        "DELETE FROM access_requests WHERE email IN (SELECT email FROM user_emails) \
         OR email IN (SELECT email FROM users)",
        &[],
    )?
    .run()
    .await?;
    deleted += settle(&db).await?;
    match runners(env, &db, now).await {
        Ok(0) => {}
        Ok(removed) => worker::console_log!("housekeeping: removed {removed} runners"),
        Err(e) => worker::console_error!("removing runners: {}", e.0.message),
    }
    // Delegations in Linear that waited an hour for their person.
    if let Err(e) = crate::delegations::expire(&db, now).await {
        worker::console_error!("forgetting delegations that waited: {}", e.0.message);
    }
    if let Err(e) = crate::delegations::uninstall_unused(env, &db).await {
        worker::console_error!("giving up unused Linear workspaces: {}", e.0.message);
    }
    // A failed lookup leaves the rest to do.
    match crate::pull_requests::sweep(env, &db, now).await {
        Ok(0) => {}
        Ok(told) => {
            worker::console_log!("housekeeping: told {told} sessions of their pull requests")
        }
        Err(e) => worker::console_error!("looking up pull requests: {}", e.0.message),
    }
    match crate::reviews::sweep(env, &db).await {
        Ok(0) => {}
        Ok(ended) => worker::console_log!("housekeeping: ended {ended} reviews"),
        Err(e) => worker::console_error!("ending reviews: {}", e.0.message),
    }
    // In case a change of schedules never reached the alarm.
    crate::schedules::wake(env).await;
    match index_sessions(env, &db, INDEXES_PER_RUN).await {
        Ok(0) => {}
        Ok(indexed) => worker::console_log!("housekeeping: indexed {indexed} sessions for search"),
        Err(e) => worker::console_error!("indexing sessions for search: {}", e.0.message),
    }
    if purged > 0 || purged_files > 0 || deleted > 0 {
        worker::console_log!(
            "housekeeping: purged {purged} sessions and the files of {purged_files} projects, deleted {deleted} rows"
        );
    }
    Ok(())
}

/// Removes the runners that enrolled in a workspace and either waited
/// longer than a week for one of its admins to approve them, or stayed
/// offline longer than their enrollment token lets them, and the revoked
/// and expired enrollment tokens that no runner is left of after 90 days.
/// The workspace's audit log records each. Returns how many runners went.
pub async fn runners(env: &Env, db: &D1Database, now: i64) -> AppResult<usize> {
    #[derive(Deserialize)]
    struct Gone {
        id: String,
        name: String,
        workspace_id: Option<String>,
    }
    let mut removed = 0;
    // Those that waited a week for approval.
    let expired: Vec<Gone> = q(
        db,
        "SELECT id, name, workspace_id FROM runners WHERE pending_until IS NOT NULL AND pending_until < ?",
        &[n(now)],
    )?
    .all()
    .await?;
    for g in &expired {
        if remove(
            env,
            db,
            now,
            Removal {
                id: &g.id,
                name: &g.name,
                workspace_id: g.workspace_id.as_deref(),
                still: "pending_until IS NOT NULL AND pending_until < ?2",
                action: "workspace.runner_expired",
                reason: "Its runner was never approved.",
            },
        )
        .await
        {
            removed += 1;
        }
    }
    // Those offline too long, but for the ones FleetObject has connected
    // now, whatever D1 last heard of them. FleetObject removes the ones it
    // saw go as it watches them, within a minute; these are those it never
    // saw connect, and any it missed. Without FleetObject's word on who is
    // online, none goes this hour.
    #[derive(Deserialize)]
    struct Offline {
        id: String,
        name: String,
        workspace_id: Option<String>,
        #[serde(deserialize_with = "crate::db::opt_int", default)]
        last_seen_at: Option<i64>,
        #[serde(deserialize_with = "crate::db::int")]
        created_at: i64,
        #[serde(deserialize_with = "crate::db::int")]
        minutes: i64,
    }
    let candidates: Vec<Offline> = q(
        db,
        "SELECT id, name, workspace_id, last_seen_at, created_at, remove_offline_after_minutes AS minutes \
         FROM runners WHERE remove_offline_after_minutes IS NOT NULL \
         AND COALESCE(last_seen_at, created_at) < ? - remove_offline_after_minutes * 60000",
        &[n(now)],
    )?
    .all()
    .await?;
    if !candidates.is_empty() {
        let live: AppResult<internal::LiveRunners> = async {
            let stub = internal::fleet_stub(env)?;
            internal::get(&stub, "/runners").await
        }
        .await;
        match live {
            Ok(live) => {
                for c in candidates.iter().filter(|c| {
                    fctl_domain::enrollment::offline_too_long(
                        u32::try_from(c.minutes).unwrap_or(u32::MAX),
                        c.last_seen_at,
                        c.created_at,
                        live.online.contains(&c.id),
                        now,
                    )
                }) {
                    if remove(
                        env,
                        db,
                        now,
                        Removal {
                            id: &c.id,
                            name: &c.name,
                            workspace_id: c.workspace_id.as_deref(),
                            still: "remove_offline_after_minutes IS NOT NULL \
                                    AND COALESCE(last_seen_at, created_at) < ?2 - remove_offline_after_minutes * 60000",
                            action: "workspace.runner_removed",
                            reason: "Its runner stayed offline too long and was removed.",
                        },
                    )
                    .await
                    {
                        removed += 1;
                    }
                }
            }
            Err(e) => worker::console_error!(
                "removing runners offline too long: FleetObject didn't say which are online: {}",
                e.0.message
            ),
        }
    }
    // What FleetObject still has of runners removed while it couldn't be
    // told, and while they were offline, goes too.
    let swept: AppResult<serde_json::Value> = async {
        let stub = internal::fleet_stub(env)?;
        internal::post(&stub, "/sweep", &serde_json::json!({})).await
    }
    .await;
    match swept {
        Ok(v) => {
            if let Some(n) = v
                .get("removed")
                .and_then(serde_json::Value::as_u64)
                .filter(|n| *n > 0)
            {
                worker::console_log!("housekeeping: FleetObject forgot {n} removed runners");
            }
        }
        Err(e) => worker::console_error!("sweeping FleetObject's runners: {}", e.0.message),
    }
    let ended = q(
        db,
        "DELETE FROM enrollment_tokens WHERE (revoked_at < ?1 OR expires_at < ?1) \
         AND NOT EXISTS (SELECT 1 FROM runners r WHERE r.enrollment_id = enrollment_tokens.id)",
        &[n(now - fctl_domain::enrollment::KEEP_ENDED_MS)],
    )?
    .run()
    .await?;
    if ended > 0 {
        worker::console_log!("housekeeping: deleted {ended} ended enrollment tokens");
    }
    Ok(removed)
}

/// A runner the hourly job removes, and why.
struct Removal<'a> {
    id: &'a str,
    name: &'a str,
    workspace_id: Option<&'a str>,
    /// What must still hold of its row for it to go (`?2` is now).
    still: &'a str,
    action: &'a str,
    /// What the sessions it ran say.
    reason: &'a str,
}

/// Removes one runner: its row, while `still` holds, then the audit log,
/// then FleetObject, which closes its connections and stops its sessions.
/// Its row decides: should FleetObject not answer, it lets the runner go at
/// its connection's next heartbeat, which finds the row gone, or, offline,
/// at this job's sweep within the hour, or when its sessions stop waiting
/// for it, whichever comes first. Returns whether it went.
async fn remove(env: &Env, db: &D1Database, now: i64, r: Removal<'_>) -> bool {
    let deleted = async {
        q(
            db,
            &format!("DELETE FROM runners WHERE id = ?1 AND {}", r.still),
            &[s(r.id), n(now)],
        )?
        .run()
        .await
    }
    .await;
    match deleted {
        Ok(0) => return false,
        Ok(_) => {}
        Err(e) => {
            worker::console_error!("removing runner {}: {}", r.id, e.0.message);
            return false;
        }
    }
    let data = serde_json::json!({
        "name": r.name,
        "by": "fctl",
        "offline": r.action == "workspace.runner_removed",
    });
    if let Err(e) = audit_runner(db, now, r.action, r.id, r.workspace_id, data).await {
        worker::console_error!("auditing runner {}: {}", r.id, e.0.message);
    }
    if let Err(e) = crate::api::runners::removed(env, r.id, r.reason).await {
        worker::console_error!(
            "telling FleetObject runner {} was removed (it lets it go at its next heartbeat, or its sweep): {}",
            r.id,
            e.0.message
        );
    }
    true
}

/// Records what fctl itself did to a runner in its workspace's audit log
/// (the instance's, without one): no one did it.
pub async fn audit_runner(
    db: &D1Database,
    now: i64,
    action: &str,
    runner_id: &str,
    workspace_id: Option<&str>,
    data: serde_json::Value,
) -> AppResult<()> {
    q(
        db,
        "INSERT INTO audit_log (id, ts, actor_id, action, target, data, ip, workspace_id) \
         VALUES (?, ?, NULL, ?, ?, ?, NULL, ?)",
        &[
            s(&fctl_domain::ids::new_id("aud", now)),
            n(now),
            s(action),
            s(runner_id),
            s(&data.to_string()),
            crate::db::os(workspace_id),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// What a Worker from before workspaces left while a deploy's migration had
/// already run: accounts without their address row, and projects without a
/// workspace, which go to the first one (as the migration put the others).
async fn settle(db: &D1Database) -> AppResult<usize> {
    let mut changed = q(
        db,
        "INSERT OR IGNORE INTO user_emails (email, user_id, created_at) SELECT u.email, u.id, u.created_at FROM users u \
         WHERE u.email NOT LIKE 'deleted-%@deleted.invalid' AND NOT EXISTS (SELECT 1 FROM user_emails e WHERE e.email = u.email)",
        &[],
    )?
    .run()
    .await?;
    changed += q(
        db,
        "UPDATE projects SET workspace_id = (SELECT id FROM workspaces ORDER BY created_at LIMIT 1) WHERE workspace_id IS NULL",
        &[],
    )?
    .run()
    .await?;
    Ok(changed)
}
