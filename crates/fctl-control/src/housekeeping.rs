//! Deleting what is no longer needed: the event logs of deleted sessions,
//! expired sign-in records and old audit entries. Runs from the Worker's
//! hourly cron trigger; deleting a project also purges its sessions right away.

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

/// The hourly job.
pub async fn run(env: &Env, now: i64) -> AppResult<()> {
    let db = env.d1("DB")?;
    let purged = purge_sessions(env, &db, PURGES_PER_RUN).await?;
    let mut deleted = 0;
    for (sql, cutoff) in [
        // Expired tokens are already refused and hidden from Settings.
        (
            "DELETE FROM tokens WHERE expires_at IS NOT NULL AND expires_at < ?",
            now,
        ),
        ("DELETE FROM password_resets WHERE expires_at < ?", now),
        ("DELETE FROM device_codes WHERE expires_at < ?", now),
        ("DELETE FROM invites WHERE expires_at < ?", now),
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
    if purged > 0 || deleted > 0 {
        worker::console_log!("housekeeping: purged {purged} sessions, deleted {deleted} rows");
    }
    Ok(())
}
