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
//! up fctl's access to Linear workspaces no fctl workspace works in.

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
    if purged > 0 || purged_files > 0 || deleted > 0 {
        worker::console_log!(
            "housekeeping: purged {purged} sessions and the files of {purged_files} projects, deleted {deleted} rows"
        );
    }
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
