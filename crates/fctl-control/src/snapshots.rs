//! Session snapshots (docs/architecture.md#session-snapshots): a session's
//! conversation and uncommitted work, which its agent saves as its sandbox
//! stops, sealed with a key of the session's that D1 keeps sealed with the
//! master key, so that the session resumes on any runner as it was. The
//! sealed bytes are kept in the session's SessionDO, in parts; D1 says
//! which snapshot is the session's now. They go with the session, its
//! project and its workspace, after the days its workspace keeps them, and
//! once the workspace or the instance stops keeping them.

use crate::crypto::Keys;
use crate::db::{int_bool, n, opt_int, os, q, s};
use crate::error::{AppError, AppResult, internal};
use crate::internal::{self, SnapshotParts};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fctl_domain::snapshots as domain;
use fctl_proto::agent::SnapshotSaved;
use fctl_proto::api::InstanceSettings;
use fctl_proto::runner::{SnapshotRef, SnapshotSpec};
use fctl_proto::session::SessionSnapshot;
use serde::Deserialize;
use worker::D1Database;

/// What a session's key is sealed for.
fn aad(session_id: &str) -> String {
    format!("snapshot:{session_id}")
}

/// A session's snapshot, as D1 keeps it.
#[derive(Debug, Deserialize)]
pub struct Row {
    pub key_enc: String,
    pub snapshot_id: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub size: Option<i64>,
    pub sha256: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub parts: Option<i64>,
    pub claude_version: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub agent_seq: Option<i64>,
    #[serde(deserialize_with = "int_bool", default)]
    pub conversation: bool,
    #[serde(deserialize_with = "int_bool", default)]
    pub changes: bool,
    pub left_out: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub saved_at: Option<i64>,
    pub runner_name: Option<String>,
    /// Its workspace keeps snapshots, this many days (NULL: the default).
    #[serde(deserialize_with = "int_bool", default)]
    pub workspace_keeps: bool,
    #[serde(deserialize_with = "opt_int", default)]
    pub days: Option<i64>,
}

const ROW_SELECT: &str = "SELECT ss.key_enc, ss.snapshot_id, ss.size, ss.sha256, ss.parts, \
    ss.claude_version, ss.agent_seq, ss.conversation, ss.changes, ss.left_out, ss.saved_at, ss.runner_name, \
    w.snapshots AS workspace_keeps, w.snapshot_days AS days \
    FROM session_snapshots ss JOIN sessions se ON se.id = ss.session_id \
    JOIN projects p ON p.id = se.project_id JOIN workspaces w ON w.id = p.workspace_id";

/// A session's snapshot row, if it has one.
pub async fn row(db: &D1Database, session_id: &str) -> AppResult<Option<Row>> {
    q(
        db,
        &format!("{ROW_SELECT} WHERE ss.session_id = ?"),
        &[s(session_id)],
    )?
    .first()
    .await
}

impl Row {
    fn days(&self) -> Option<u32> {
        self.days.and_then(|d| u32::try_from(d).ok())
    }

    /// Its snapshot, while it has one that its workspace still keeps.
    fn current(&self, now: i64) -> Option<(&str, i64)> {
        let id = self.snapshot_id.as_deref()?;
        let at = self.saved_at?;
        (self.workspace_keeps && domain::expires_at(at, self.days()) > now).then_some((id, at))
    }

    fn left_out(&self) -> Vec<String> {
        self.left_out
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default()
    }

    /// What a resume restores.
    pub fn reference(&self, now: i64) -> Option<SnapshotRef> {
        let (id, saved_at) = self.current(now)?;
        Some(SnapshotRef {
            id: id.to_owned(),
            size: u64::try_from(self.size?).ok()?,
            sha256: self.sha256.clone()?,
            parts: u32::try_from(self.parts?).ok()?,
            claude_version: self.claude_version.clone(),
            agent_seq: u64::try_from(self.agent_seq.unwrap_or(0)).unwrap_or(0),
            saved_at,
            saved_on: self.runner_name.clone(),
            conversation: self.conversation,
            changes: self.changes,
        })
    }

    /// What people see of it.
    pub fn info(&self, now: i64) -> Option<SessionSnapshot> {
        let (_, saved_at) = self.current(now)?;
        Some(SessionSnapshot {
            saved_at,
            saved_on: self.runner_name.clone(),
            size: u64::try_from(self.size.unwrap_or(0)).unwrap_or(0),
            conversation: self.conversation,
            changes: self.changes,
            left_out: self.left_out(),
            expires_at: Some(domain::expires_at(saved_at, self.days())),
        })
    }
}

/// Whether a session's workspace keeps snapshots: `None` when the session
/// isn't there.
async fn workspace_keeps(db: &D1Database, session_id: &str) -> AppResult<Option<bool>> {
    #[derive(Deserialize)]
    struct W {
        #[serde(deserialize_with = "int_bool", default)]
        snapshots: bool,
    }
    let w: Option<W> = q(
        db,
        "SELECT w.snapshots FROM sessions se JOIN projects p ON p.id = se.project_id \
         JOIN workspaces w ON w.id = p.workspace_id WHERE se.id = ?",
        &[s(session_id)],
    )?
    .first()
    .await?;
    Ok(w.map(|w| w.snapshots))
}

/// What a run of `session_id` does with snapshots, for its spec: save one
/// as its sandbox stops, and restore its last; `None` for a review agent's
/// session, and where the instance or the workspace keeps none. The
/// session's key is made the first time.
pub async fn spec(
    db: &D1Database,
    keys: &Keys,
    settings: &InstanceSettings,
    session_id: &str,
    review: bool,
    now: i64,
) -> AppResult<Option<SnapshotSpec>> {
    if review || !settings.snapshots || workspace_keeps(db, session_id).await? != Some(true) {
        return Ok(None);
    }
    let found = match row(db, session_id).await? {
        Some(r) => r,
        None => {
            let key: [u8; 32] = fctl_domain::random_bytes();
            let sealed = keys.seal(&aad(session_id), &key)?;
            // Two runs that start at once make one key.
            q(
                db,
                "INSERT OR IGNORE INTO session_snapshots (session_id, key_enc) VALUES (?, ?)",
                &[s(session_id), s(&sealed)],
            )?
            .run()
            .await?;
            row(db, session_id)
                .await?
                .ok_or_else(|| internal("the session's snapshot key vanished"))?
        }
    };
    let key = keys.open(&aad(session_id), &found.key_enc)?;
    Ok(Some(SnapshotSpec {
        key: STANDARD.encode(key),
        save: true,
        max_bytes: u64::from(settings.max_snapshot_mb) * 1024 * 1024,
        restore: found.reference(now),
    }))
}

/// What GET /api/v1/sessions/{id} says of the session's snapshot.
pub async fn info(
    db: &D1Database,
    session_id: &str,
    now: i64,
) -> AppResult<Option<SessionSnapshot>> {
    Ok(row(db, session_id).await?.and_then(|r| r.info(now)))
}

/// Why a runner's snapshot isn't kept, if it isn't: the instance or the
/// workspace keeps none, or it is larger than the instance allows.
pub async fn refusal(
    db: &D1Database,
    settings: &InstanceSettings,
    session_id: &str,
    size: u64,
) -> AppResult<Option<String>> {
    if !settings.snapshots {
        return Ok(Some("This instance keeps no session snapshots.".into()));
    }
    if workspace_keeps(db, session_id).await? != Some(true) {
        return Ok(Some("This session's workspace keeps no snapshots.".into()));
    }
    let max = u64::from(settings.max_snapshot_mb) * 1024 * 1024;
    if size > max {
        return Ok(Some(format!(
            "A snapshot may take at most {} MiB on this instance.",
            settings.max_snapshot_mb
        )));
    }
    Ok(None)
}

/// Makes the snapshot `snapshot_id`, whose parts SessionDO has, the
/// session's in D1, saved now on `runner`.
pub async fn keep(
    db: &D1Database,
    session_id: &str,
    snapshot_id: &str,
    commit: &fctl_proto::api::SnapshotCommit,
    runner: (&str, &str),
    now: i64,
) -> AppResult<()> {
    let saved: &SnapshotSaved = &commit.saved;
    let changed = q(
        db,
        "UPDATE session_snapshots SET snapshot_id = ?, size = ?, sha256 = ?, parts = ?, claude_version = ?, \
         agent_seq = ?, conversation = ?, changes = ?, left_out = ?, saved_at = ?, runner_id = ?, runner_name = ? \
         WHERE session_id = ?",
        &[
            s(snapshot_id),
            n(i64::try_from(saved.size).unwrap_or(i64::MAX)),
            s(&saved.sha256),
            n(i64::from(commit.parts)),
            os(saved.claude_version.as_deref()),
            n(i64::try_from(saved.agent_seq).unwrap_or(0)),
            crate::db::b(saved.conversation),
            crate::db::b(saved.changes),
            os((!saved.left_out.is_empty())
                .then(|| serde_json::to_string(&saved.left_out).unwrap_or_default())
                .as_deref()),
            n(now),
            s(runner.0),
            s(runner.1),
            s(session_id),
        ],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(AppError(fctl_proto::ApiError::not_found(
            "This session's snapshot key",
        )));
    }
    Ok(())
}

/// Deletes the snapshots that their sessions' workspaces no longer keep:
/// older than their days, or kept by a workspace, or an instance, that
/// stopped keeping them. Their parts go from their SessionDO first. At
/// most 200 a run; how many went.
pub async fn expire(env: &worker::Env, db: &D1Database, now: i64) -> AppResult<usize> {
    let settings = crate::instance::settings(db).await?;
    #[derive(Deserialize)]
    struct Due {
        session_id: String,
        snapshot_id: String,
    }
    let due: Vec<Due> = q(
        db,
        "SELECT ss.session_id, ss.snapshot_id FROM session_snapshots ss \
         JOIN sessions se ON se.id = ss.session_id JOIN projects p ON p.id = se.project_id \
         JOIN workspaces w ON w.id = p.workspace_id WHERE ss.snapshot_id IS NOT NULL \
         AND (?1 = 0 OR w.snapshots = 0 OR ss.saved_at <= ?2 - COALESCE(w.snapshot_days, ?3) * 86400000) \
         LIMIT 200",
        &[
            crate::db::b(settings.snapshots),
            n(now),
            n(i64::from(domain::DEFAULT_DAYS)),
        ],
    )?
    .all()
    .await?;
    let mut gone = 0;
    for d in due {
        let stub = internal::session_stub(env, &d.session_id)?;
        let deleted: AppResult<serde_json::Value> = internal::post(
            &stub,
            "/snapshot/delete",
            &SnapshotParts {
                id: d.snapshot_id.clone(),
                parts: 0,
                size: 0,
            },
        )
        .await;
        if let Err(e) = deleted {
            worker::console_error!("deleting {}'s snapshot: {}", d.session_id, e.0.message);
            continue;
        }
        q(
            db,
            "UPDATE session_snapshots SET snapshot_id = NULL, size = NULL, sha256 = NULL, parts = NULL, \
             left_out = NULL WHERE session_id = ? AND snapshot_id = ?",
            &[s(&d.session_id), s(&d.snapshot_id)],
        )?
        .run()
        .await?;
        gone += 1;
    }
    Ok(gone)
}
