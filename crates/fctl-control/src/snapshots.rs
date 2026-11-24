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
    /// Where it is when its runner kept it in a bucket of its own.
    pub location: Option<String>,
    /// Its workspace keeps snapshots, this many days (NULL: the default).
    #[serde(deserialize_with = "int_bool", default)]
    pub workspace_keeps: bool,
    #[serde(deserialize_with = "opt_int", default)]
    pub days: Option<i64>,
}

const ROW_SELECT: &str = "SELECT ss.key_enc, ss.snapshot_id, ss.size, ss.sha256, ss.parts, \
    ss.claude_version, ss.agent_seq, ss.conversation, ss.changes, ss.left_out, ss.saved_at, ss.runner_name, \
    ss.location, w.snapshots AS workspace_keeps, w.snapshot_days AS days \
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
            location: self.location.clone(),
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
            location: self.location.clone(),
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
         agent_seq = ?, conversation = ?, changes = ?, left_out = ?, saved_at = ?, runner_id = ?, runner_name = ?, \
         location = ?, expire_tried_at = NULL, uploading_since = NULL WHERE session_id = ?",
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
            os(commit.location.as_deref()),
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

/// Notes that the session's runner started uploading a snapshot at `now`,
/// which [`abandon`] lets go of a day on, should it never commit.
pub async fn uploading(db: &D1Database, session_id: &str, now: i64) -> AppResult<()> {
    q(
        db,
        "UPDATE session_snapshots SET uploading_since = ? WHERE session_id = ?",
        &[n(now), s(session_id)],
    )?
    .run()
    .await?;
    Ok(())
}

/// The sessions whose runner started uploading a snapshot over a day
/// before `now` and never committed it: at most 200.
pub async fn abandoned(db: &D1Database, now: i64) -> AppResult<Vec<String>> {
    #[derive(Deserialize)]
    struct Abandoned {
        session_id: String,
    }
    let rows: Vec<Abandoned> = q(
        db,
        "SELECT session_id FROM session_snapshots WHERE uploading_since IS NOT NULL \
         AND uploading_since <= ? ORDER BY uploading_since LIMIT 200",
        &[n(now - 86_400_000)],
    )?
    .all()
    .await?;
    Ok(rows.into_iter().map(|r| r.session_id).collect())
}

/// Lets go of the uploads that never committed ([`abandoned`]): their
/// parts go from their SessionDO, whatever their workspace keeps. How
/// many sessions had one.
pub async fn abandon(env: &worker::Env, db: &D1Database, now: i64) -> AppResult<usize> {
    let mut gone = 0;
    for session_id in abandoned(db, now).await? {
        let stub = internal::session_stub(env, &session_id)?;
        let dropped: AppResult<serde_json::Value> = internal::post(
            &stub,
            "/snapshot/abandoned",
            &SnapshotParts {
                id: String::new(),
                parts: 0,
                size: 0,
            },
        )
        .await;
        if let Err(e) = dropped {
            worker::console_error!("letting go of {session_id}'s upload: {}", e.0.message);
            continue;
        }
        let cleared = q(
            db,
            "UPDATE session_snapshots SET uploading_since = NULL WHERE session_id = ?",
            &[s(&session_id)],
        );
        if let Ok(stmt) = cleared {
            let _ = stmt.run().await;
        }
        gone += 1;
    }
    Ok(gone)
}

/// The snapshots their sessions' workspaces no longer keep, at `now`, as
/// `(session, snapshot)`: older than their days, or kept by a workspace,
/// or an instance (`instance_keeps`), that stopped keeping them. At most
/// 200, the oldest first, those whose deletion failed after the others.
pub async fn due(
    db: &D1Database,
    instance_keeps: bool,
    now: i64,
) -> AppResult<Vec<(String, String)>> {
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
         ORDER BY COALESCE(ss.expire_tried_at, 0), ss.saved_at LIMIT 200",
        &[
            crate::db::b(instance_keeps),
            n(now),
            n(i64::from(domain::DEFAULT_DAYS)),
        ],
    )?
    .all()
    .await?;
    Ok(due
        .into_iter()
        .map(|d| (d.session_id, d.snapshot_id))
        .collect())
}

/// Forgets snapshot `snapshot_id` as the session's, unless another is
/// the session's by now: all D1 said of it. The session's key stays, for
/// its next one.
pub async fn forget(db: &D1Database, session_id: &str, snapshot_id: &str) -> AppResult<()> {
    q(
        db,
        "UPDATE session_snapshots SET snapshot_id = NULL, size = NULL, sha256 = NULL, parts = NULL, \
         claude_version = NULL, agent_seq = NULL, conversation = NULL, changes = NULL, left_out = NULL, \
         saved_at = NULL, runner_id = NULL, runner_name = NULL, location = NULL, expire_tried_at = NULL \
         WHERE session_id = ? AND snapshot_id = ?",
        &[s(session_id), s(snapshot_id)],
    )?
    .run()
    .await?;
    Ok(())
}

/// Deletes the snapshots that their sessions' workspaces no longer keep
/// ([`due`]): their parts go from their SessionDO first. How many went.
pub async fn expire(env: &worker::Env, db: &D1Database, now: i64) -> AppResult<usize> {
    let settings = crate::instance::settings(db).await?;
    let mut gone = 0;
    for (session_id, snapshot_id) in due(db, settings.snapshots, now).await? {
        let stub = internal::session_stub(env, &session_id)?;
        let deleted: AppResult<serde_json::Value> = internal::post(
            &stub,
            "/snapshot/delete",
            &SnapshotParts {
                id: snapshot_id.clone(),
                parts: 0,
                size: 0,
            },
        )
        .await;
        if let Err(e) = deleted {
            worker::console_error!("deleting {session_id}'s snapshot: {}", e.0.message);
            // After the others next time, so that it holds none of them up.
            let tried = q(
                db,
                "UPDATE session_snapshots SET expire_tried_at = ? WHERE session_id = ?",
                &[n(now), s(&session_id)],
            );
            if let Ok(stmt) = tried {
                let _ = stmt.run().await;
            }
            continue;
        }
        match forget(db, &session_id, &snapshot_id).await {
            Ok(()) => gone += 1,
            Err(e) => worker::console_error!("forgetting {session_id}'s snapshot: {}", e.0.message),
        }
    }
    Ok(gone)
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::testing;
    use fctl_proto::api::SnapshotCommit;
    use wasm_bindgen_test::wasm_bindgen_test;

    const NOW: i64 = 1_790_000_000_000;
    const DAY: i64 = 86_400_000;

    async fn acme(db: &D1Database) -> AppResult<()> {
        for sql in [
            "INSERT INTO users (id, email, name, created_at, updated_at) VALUES ('usr_ada', 'ada@example.com', 'Ada', 0, 0)",
            "INSERT INTO workspaces (id, slug, name, created_at, updated_at) VALUES ('wsp_acme', 'acme', 'Acme', 0, 0)",
            "INSERT INTO workspaces (id, slug, name, created_at, updated_at, snapshots) VALUES ('wsp_off', 'off', 'Off', 0, 0, 0)",
            "INSERT INTO projects (id, slug, name, created_by, created_at, updated_at, workspace_id) VALUES ('prj_web', 'web', 'Web', 'usr_ada', 0, 0, 'wsp_acme')",
            "INSERT INTO projects (id, slug, name, created_by, created_at, updated_at, workspace_id) VALUES ('prj_off', 'off', 'Off', 'usr_ada', 0, 0, 'wsp_off')",
            "INSERT INTO sessions (id, project_id, number, title, prompt, status, created_by, claude_session_id, created_at, updated_at) \
             VALUES ('ses_1', 'prj_web', 1, 'One', 'Go', 'stopped', 'usr_ada', 'c1', 0, 0)",
            "INSERT INTO sessions (id, project_id, number, title, prompt, status, created_by, claude_session_id, created_at, updated_at) \
             VALUES ('ses_off', 'prj_off', 1, 'Off', 'Go', 'stopped', 'usr_ada', 'c2', 0, 0)",
        ] {
            q(db, sql, &[])?.run().await?;
        }
        Ok(())
    }

    /// Whether a resume of `ses_1` at `now` skips the wait for its busy
    /// runner, as FleetDO reads it.
    async fn skips_home_wait(db: &D1Database, now: i64) -> AppResult<bool> {
        Ok(crate::spec::session_wants(db, &["ses_1"], now)
            .await?
            .get("ses_1")
            .is_some_and(|w| w.snapshot))
    }

    fn commit(size: u64, parts: u32) -> SnapshotCommit {
        SnapshotCommit {
            saved: SnapshotSaved {
                size,
                sha256: "ab".repeat(32),
                claude_version: Some("2.1.140".into()),
                agent_seq: 42,
                conversation: true,
                changes: true,
                left_out: vec!["images".into()],
            },
            parts,
            location: None,
        }
    }

    /// A session's first spec makes its key, which later ones keep; once a
    /// snapshot is kept, specs restore it and the session's details show
    /// it, until its workspace's days are over, when it is due.
    #[wasm_bindgen_test]
    async fn a_sessions_snapshot_is_kept_restored_and_expires() -> AppResult<()> {
        let db = testing::database();
        acme(&db).await?;
        let keys = Keys::derive(&[5; 32]);
        let settings = InstanceSettings::default();
        let first = spec(&db, &keys, &settings, "ses_1", false, NOW)
            .await?
            .ok_or_else(|| internal("no spec"))?;
        assert!(first.save && first.restore.is_none());
        assert_eq!(first.max_bytes, 200 * 1024 * 1024);
        let again = spec(&db, &keys, &settings, "ses_1", false, NOW)
            .await?
            .ok_or_else(|| internal("no spec"))?;
        assert_eq!(first.key, again.key);
        // None for review agents, or where the workspace or instance keeps none.
        assert!(
            spec(&db, &keys, &settings, "ses_1", true, NOW)
                .await?
                .is_none()
        );
        assert!(
            spec(&db, &keys, &settings, "ses_off", false, NOW)
                .await?
                .is_none()
        );
        let off = InstanceSettings {
            snapshots: false,
            ..InstanceSettings::default()
        };
        assert!(spec(&db, &keys, &off, "ses_1", false, NOW).await?.is_none());
        assert!(refusal(&db, &off, "ses_1", 10).await?.is_some());
        assert!(refusal(&db, &settings, "ses_off", 10).await?.is_some());
        assert!(
            refusal(&db, &settings, "ses_1", 201 * 1024 * 1024)
                .await?
                .is_some()
        );
        assert_eq!(refusal(&db, &settings, "ses_1", 10).await?, None);
        assert!(!skips_home_wait(&db, NOW).await?);
        // Kept, saved on studio.
        keep(
            &db,
            "ses_1",
            "snp_one",
            &commit(12_000_000, 3),
            ("rnr_1", "studio"),
            NOW,
        )
        .await?;
        // A resume doesn't wait for its busy runner: it restores the
        // snapshot elsewhere, unless the instance keeps none.
        assert!(skips_home_wait(&db, NOW + DAY).await?);
        crate::instance::save(&db, &off).await?;
        assert!(!skips_home_wait(&db, NOW + DAY).await?);
        crate::instance::save(&db, &settings).await?;
        let with = spec(&db, &keys, &settings, "ses_1", false, NOW + DAY)
            .await?
            .ok_or_else(|| internal("no spec"))?;
        let restore = with.restore.ok_or_else(|| internal("nothing to restore"))?;
        assert_eq!(
            (
                restore.id.as_str(),
                restore.size,
                restore.parts,
                restore.agent_seq
            ),
            ("snp_one", 12_000_000, 3, 42)
        );
        assert_eq!(restore.saved_on.as_deref(), Some("studio"));
        let shown = info(&db, "ses_1", NOW + DAY)
            .await?
            .ok_or_else(|| internal("no info"))?;
        assert_eq!(shown.left_out, ["images"]);
        assert_eq!(shown.expires_at, Some(NOW + 30 * DAY));
        // Not due yet; due once the workspace's days are over, or it stops
        // keeping them, or the instance does.
        assert!(due(&db, true, NOW + DAY).await?.is_empty());
        assert_eq!(
            due(&db, true, NOW + 30 * DAY).await?,
            [("ses_1".to_owned(), "snp_one".to_owned())]
        );
        assert_eq!(due(&db, false, NOW + DAY).await?.len(), 1);
        // The oldest go first, and one whose deletion failed after the
        // others, so that it holds none of them up.
        q(
            &db,
            "INSERT INTO session_snapshots (session_id, key_enc) VALUES ('ses_off', 'sealed')",
            &[],
        )?
        .run()
        .await?;
        keep(
            &db,
            "ses_off",
            "snp_off",
            &commit(1_000, 1),
            ("rnr_1", "studio"),
            NOW - DAY,
        )
        .await?;
        let order = |d: Vec<(String, String)>| d.into_iter().map(|(s, _)| s).collect::<Vec<_>>();
        assert_eq!(
            order(due(&db, true, NOW + 30 * DAY).await?),
            ["ses_off", "ses_1"]
        );
        q(
            &db,
            "UPDATE session_snapshots SET expire_tried_at = ? WHERE session_id = 'ses_off'",
            &[n(NOW)],
        )?
        .run()
        .await?;
        assert_eq!(
            order(due(&db, true, NOW + 30 * DAY).await?),
            ["ses_1", "ses_off"]
        );
        forget(&db, "ses_off", "snp_off").await?;
        // An upload that never commits is let go of a day on.
        uploading(&db, "ses_1", NOW).await?;
        assert!(abandoned(&db, NOW + DAY - 1).await?.is_empty());
        assert_eq!(abandoned(&db, NOW + DAY).await?, ["ses_1"]);
        q(
            &db,
            "UPDATE workspaces SET snapshot_days = 3 WHERE id = 'wsp_acme'",
            &[],
        )?
        .run()
        .await?;
        assert_eq!(due(&db, true, NOW + 3 * DAY).await?.len(), 1);
        // Nor past its workspace's days, when it would restore nothing.
        assert!(skips_home_wait(&db, NOW + 2 * DAY).await?);
        assert!(!skips_home_wait(&db, NOW + 3 * DAY).await?);
        // Past its days, a spec restores nothing even before the job runs.
        let late = spec(&db, &keys, &settings, "ses_1", false, NOW + 3 * DAY)
            .await?
            .ok_or_else(|| internal("no spec"))?;
        assert!(late.restore.is_none());
        // A newer snapshot isn't forgotten for an older one.
        keep(
            &db,
            "ses_1",
            "snp_two",
            &commit(1_000, 1),
            ("rnr_2", "aws-7"),
            NOW + DAY,
        )
        .await?;
        forget(&db, "ses_1", "snp_one").await?;
        assert!(info(&db, "ses_1", NOW + DAY).await?.is_some());
        forget(&db, "ses_1", "snp_two").await?;
        assert!(info(&db, "ses_1", NOW + DAY).await?.is_none());
        // One its runner kept in a bucket of its own: only where it is,
        // which a resume's spec, the session's details and FleetDO all see.
        let at = "s3://acme-fctl/sessions/ses_1/snp_three";
        keep(
            &db,
            "ses_1",
            "snp_three",
            &SnapshotCommit {
                parts: 0,
                location: Some(at.into()),
                ..commit(900_000_000, 0)
            },
            ("rnr_3", "aws-8"),
            NOW + DAY,
        )
        .await?;
        let with = spec(&db, &keys, &settings, "ses_1", false, NOW + DAY)
            .await?
            .ok_or_else(|| internal("no spec"))?;
        let restore = with.restore.ok_or_else(|| internal("nothing to restore"))?;
        assert_eq!(restore.location.as_deref(), Some(at));
        let shown = info(&db, "ses_1", NOW + DAY)
            .await?
            .ok_or_else(|| internal("no info"))?;
        assert_eq!(shown.location.as_deref(), Some(at));
        let wants = crate::spec::session_wants(&db, &["ses_1"], NOW + DAY).await?;
        assert_eq!(
            wants
                .get("ses_1")
                .and_then(|w| w.snapshot_location.as_deref()),
            Some(at)
        );
        forget(&db, "ses_1", "snp_three").await?;
        assert!(info(&db, "ses_1", NOW + DAY).await?.is_none());
        // Of a snapshot forgotten, D1 keeps nothing but the session's key.
        let left = row(&db, "ses_1")
            .await?
            .ok_or_else(|| internal("the session's key went too"))?;
        assert!(!left.key_enc.is_empty());
        assert_eq!(
            (
                left.snapshot_id,
                left.size,
                left.sha256,
                left.parts,
                left.claude_version,
                left.agent_seq,
                left.conversation,
                left.changes,
                left.left_out,
                left.saved_at,
                left.runner_name,
                left.location,
            ),
            (
                None, None, None, None, None, None, false, false, None, None, None, None
            )
        );
        let runner: Option<serde_json::Value> = q(
            &db,
            "SELECT runner_id FROM session_snapshots WHERE session_id = 'ses_1' AND runner_id IS NOT NULL",
            &[],
        )?
        .first()
        .await?;
        assert!(runner.is_none());
        Ok(())
    }
}
