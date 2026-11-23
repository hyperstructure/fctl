//! Runner pools (docs/architecture.md#runner-pools): what D1 has of them,
//! for FleetDO, which places sessions on their runners and counts what
//! they need (`fctl_domain::pools`), and what FleetDO counts, for the
//! enrollment token's holder, the workspace's admins and service accounts,
//! and the Prometheus endpoint.

use crate::ctx::Ctx;
use crate::db::{int, int_bool, n, opt_int, q, s};
use crate::error::AppResult;
use crate::internal;
use fctl_proto::api::{PoolShape, WorkspaceDemand};
use serde::Deserialize;
use worker::D1Database;

/// What a workspace's pools need now: every one of them, or those of the
/// enrollment tokens `pools` names.
pub async fn workspace_demand(
    ctx: &Ctx,
    workspace_id: &str,
    pools: Option<&[String]>,
) -> AppResult<WorkspaceDemand> {
    demand_of(&ctx.env, workspace_id, pools).await
}

/// [`workspace_demand`], with the Worker's environment alone.
pub async fn demand_of(
    env: &worker::Env,
    workspace_id: &str,
    pools: Option<&[String]>,
) -> AppResult<WorkspaceDemand> {
    let stub = internal::fleet_stub(env)?;
    internal::post(
        &stub,
        "/demand",
        &internal::DemandOf {
            workspace_id: workspace_id.to_owned(),
            pools: pools.map(<[String]>::to_vec),
        },
    )
    .await
}

/// A pool that sessions whose project asks for its labels name as they
/// wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedPool {
    pub workspace_id: String,
    pub name: String,
    pub labels: Vec<String>,
    /// For a token its maker made for their own sessions: them.
    pub owner: Option<String>,
    pub max_runners: Option<u32>,
    pub created_at: i64,
    /// Its runners that don't wait for approval, and of those the ones
    /// that never connected.
    pub runners: u32,
    pub starting: u32,
}

/// The pools with labels that may still run sessions at `now`: those
/// whose token still enrolls, or that have runners. A revoked or expired
/// token without runners is no pool any more.
pub async fn named(db: &D1Database, now: i64) -> AppResult<Vec<NamedPool>> {
    #[derive(Deserialize)]
    struct P {
        workspace_id: String,
        name: String,
        labels: Option<String>,
        owner: Option<String>,
        #[serde(deserialize_with = "opt_int", default)]
        max_runners: Option<i64>,
        #[serde(deserialize_with = "int")]
        created_at: i64,
        #[serde(deserialize_with = "int_bool", default)]
        open: bool,
        #[serde(deserialize_with = "int", default)]
        runners: i64,
        #[serde(deserialize_with = "int", default)]
        starting: i64,
    }
    let rows: Vec<P> = q(
        db,
        "SELECT t.workspace_id, t.name, t.labels, t.max_runners, t.created_at, \
         CASE WHEN t.personal = 1 THEN t.created_by END AS owner, \
         (t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > ?1)) AS open, \
         (SELECT COUNT(*) FROM runners x WHERE x.enrollment_id = t.id AND x.pending_until IS NULL) AS runners, \
         (SELECT COUNT(*) FROM runners x WHERE x.enrollment_id = t.id AND x.pending_until IS NULL \
           AND x.last_seen_at IS NULL) AS starting \
         FROM enrollment_tokens t WHERE t.labels IS NOT NULL",
        &[n(now)],
    )?
    .all()
    .await?;
    let count = |n: i64| u32::try_from(n).unwrap_or(0);
    Ok(rows
        .into_iter()
        .filter(|p| p.open || p.runners > 0)
        .map(|p| NamedPool {
            workspace_id: p.workspace_id,
            name: p.name,
            labels: crate::model::pool_labels(p.labels.as_deref()),
            owner: p.owner,
            max_runners: p.max_runners.and_then(|m| u32::try_from(m).ok()),
            created_at: p.created_at,
            runners: count(p.runners),
            starting: count(p.starting),
        })
        .collect())
}

/// A workspace's enrollment token, as its pool's demand counts it.
#[derive(Debug, Deserialize)]
pub struct TokenRow {
    pub id: String,
    pub name: String,
    pub labels: Option<String>,
    pub placement: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub max_runners: Option<i64>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    /// For a token its maker made for their own sessions: them.
    pub owner: Option<String>,
    pub owner_name: Option<String>,
    /// It still enrolls runners: neither revoked nor expired.
    #[serde(deserialize_with = "int_bool", default)]
    pub open: bool,
    /// What its runners announced last (JSON, [`PoolShape`]).
    pub shape: Option<String>,
}

impl TokenRow {
    pub fn shape(&self) -> Option<PoolShape> {
        self.shape
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
    }
}

/// A runner of one of a workspace's pools, as D1 has it.
#[derive(Debug, Deserialize)]
pub struct MemberRow {
    pub id: String,
    /// Its enrollment token.
    pub pool: String,
    #[serde(deserialize_with = "int_bool", default)]
    pub enabled: bool,
    #[serde(deserialize_with = "opt_int", default)]
    pub pending_until: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    pub last_seen_at: Option<i64>,
}

/// A workspace's enrollment tokens, oldest first, and the runners they
/// enrolled, as of `now`.
pub async fn rows(
    db: &D1Database,
    workspace_id: &str,
    now: i64,
) -> AppResult<(Vec<TokenRow>, Vec<MemberRow>)> {
    let tokens: Vec<TokenRow> = q(
        db,
        "SELECT t.id, t.name, t.labels, t.placement, t.max_runners, t.created_at, t.shape, \
         CASE WHEN t.personal = 1 THEN t.created_by END AS owner, \
         CASE WHEN t.personal = 1 THEN u.name END AS owner_name, \
         (t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > ?2)) AS open \
         FROM enrollment_tokens t LEFT JOIN users u ON u.id = t.created_by \
         WHERE t.workspace_id = ?1 ORDER BY t.created_at, t.id",
        &[s(workspace_id), n(now)],
    )?
    .all()
    .await?;
    let runners: Vec<MemberRow> = q(
        db,
        "SELECT r.id, r.enrollment_id AS pool, r.enabled, r.pending_until, r.last_seen_at \
         FROM runners r JOIN enrollment_tokens t ON t.id = r.enrollment_id WHERE t.workspace_id = ?",
        &[s(workspace_id)],
    )?
    .all()
    .await?;
    Ok((tokens, runners))
}

/// Keeps what runner `runner_id` announced as its pool's shape, which the
/// pool's demand counts with once none of its runners is online. Runners
/// that enrolled with no token change nothing.
pub async fn keep_shape(db: &D1Database, runner_id: &str, shape: &PoolShape) -> AppResult<()> {
    q(
        db,
        "UPDATE enrollment_tokens SET shape = ?1 \
         WHERE id = (SELECT enrollment_id FROM runners WHERE id = ?2)",
        &[s(&serde_json::to_string(shape)?), s(runner_id)],
    )?
    .run()
    .await?;
    Ok(())
}

/// The runners enrollment token `token_id` enrolled.
pub async fn runners_of(db: &D1Database, token_id: &str) -> AppResult<Vec<String>> {
    #[derive(Deserialize)]
    struct Id {
        id: String,
    }
    let rows: Vec<Id> = q(
        db,
        "SELECT id FROM runners WHERE enrollment_id = ?",
        &[s(token_id)],
    )?
    .all()
    .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;
    use wasm_bindgen_test::wasm_bindgen_test;

    const NOW: i64 = 1_790_000_000_000;

    async fn acme(db: &D1Database) -> AppResult<()> {
        for sql in [
            "INSERT INTO users (id, email, name, created_at, updated_at) VALUES ('usr_ada', 'ada@example.com', 'Ada', 0, 0)",
            "INSERT INTO workspaces (id, slug, name, created_at, updated_at) VALUES ('wsp_acme', 'acme', 'Acme', 0, 0)",
            "INSERT INTO workspaces (id, slug, name, created_at, updated_at) VALUES ('wsp_other', 'other', 'Other', 0, 0)",
            // A pool with labels, packed, with three runners: one online,
            // one starting, one waiting for approval.
            "INSERT INTO enrollment_tokens (id, workspace_id, name, token_hash, max_runners, created_by, created_at, labels, placement) \
             VALUES ('enr_large', 'wsp_acme', 'AWS large', 'h1', 20, 'usr_ada', 2, '[\"aws\",\"large\"]', 'packed')",
            "INSERT INTO runners (id, name, token_hash, enabled, created_at, workspace_id, enrollment_id, last_seen_at) \
             VALUES ('rnr_1', 'aws-1', 'r1', 1, 0, 'wsp_acme', 'enr_large', 5)",
            "INSERT INTO runners (id, name, token_hash, enabled, created_at, workspace_id, enrollment_id) \
             VALUES ('rnr_2', 'aws-2', 'r2', 1, 0, 'wsp_acme', 'enr_large')",
            "INSERT INTO runners (id, name, token_hash, enabled, created_at, workspace_id, enrollment_id, pending_until) \
             VALUES ('rnr_3', 'aws-3', 'r3', 1, 0, 'wsp_acme', 'enr_large', 99)",
            // Ada's own, without labels; a revoked one without runners;
            // another workspace's.
            "INSERT INTO enrollment_tokens (id, workspace_id, name, token_hash, personal, created_by, created_at) \
             VALUES ('enr_mine', 'wsp_acme', 'My cluster', 'h2', 1, 'usr_ada', 1)",
            "INSERT INTO enrollment_tokens (id, workspace_id, name, token_hash, created_by, created_at, labels, revoked_at) \
             VALUES ('enr_gone', 'wsp_acme', 'Old GPUs', 'h3', 'usr_ada', 3, '[\"gpu\"]', 4)",
            "INSERT INTO enrollment_tokens (id, workspace_id, name, token_hash, created_by, created_at, labels) \
             VALUES ('enr_theirs', 'wsp_other', 'Theirs', 'h4', 'usr_ada', 4, '[\"gpu\"]')",
            // A runner registered by a person: in no pool.
            "INSERT INTO runners (id, name, token_hash, enabled, created_by, created_at) \
             VALUES ('rnr_laptop', 'laptop', 'r4', 1, 'usr_ada', 0)",
        ] {
            q(db, sql, &[])?.run().await?;
        }
        Ok(())
    }

    /// Sessions that ask for labels name the pools that have them and may
    /// still run sessions, with their runners that don't wait for
    /// approval, of which the ones that never connected are starting.
    #[wasm_bindgen_test]
    async fn names_the_pools_that_may_still_run_sessions() -> AppResult<()> {
        let db = testing::database();
        acme(&db).await?;
        let mut pools = named(&db, NOW).await?;
        pools.sort_by_key(|p| p.created_at);
        assert_eq!(
            pools,
            [
                NamedPool {
                    workspace_id: "wsp_acme".into(),
                    name: "AWS large".into(),
                    labels: vec!["aws".into(), "large".into()],
                    owner: None,
                    max_runners: Some(20),
                    created_at: 2,
                    runners: 2,
                    starting: 1,
                },
                NamedPool {
                    workspace_id: "wsp_other".into(),
                    name: "Theirs".into(),
                    labels: vec!["gpu".into()],
                    owner: None,
                    max_runners: None,
                    created_at: 4,
                    runners: 0,
                    starting: 0,
                },
            ]
        );
        Ok(())
    }

    /// Demand reads a workspace's tokens, open or not, with their owner
    /// and shape, and the runners each enrolled; a runner's hello keeps
    /// its pool's shape.
    #[wasm_bindgen_test]
    async fn reads_a_workspaces_pools_and_keeps_their_shape() -> AppResult<()> {
        let db = testing::database();
        acme(&db).await?;
        let shape = PoolShape {
            capacity: 4,
            cpus: 16,
            memory_mb: 65_536,
            sandbox: None,
            image: None,
            allowed_images: None,
            allowed_builds: None,
            driver: "docker".into(),
            version: "0.38.0".into(),
            labels: vec![],
            reviews: true,
            keeps_token: false,
            seen_at: NOW,
        };
        keep_shape(&db, "rnr_1", &shape).await?;
        // A runner in no pool keeps nothing.
        keep_shape(&db, "rnr_laptop", &shape).await?;
        let (tokens, runners) = rows(&db, "wsp_acme", NOW).await?;
        let names: Vec<(&str, bool, Option<&str>)> = tokens
            .iter()
            .map(|t| (t.name.as_str(), t.open, t.owner_name.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                ("My cluster", true, Some("Ada")),
                ("AWS large", true, None),
                ("Old GPUs", false, None),
            ]
        );
        let large = tokens.iter().find(|t| t.id == "enr_large");
        assert_eq!(large.and_then(TokenRow::shape), Some(shape));
        assert_eq!(large.and_then(|t| t.placement.as_deref()), Some("packed"));
        assert!(
            tokens
                .iter()
                .filter(|t| t.id != "enr_large")
                .all(|t| t.shape.is_none())
        );
        let mut ids: Vec<&str> = runners.iter().map(|r| r.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["rnr_1", "rnr_2", "rnr_3"]);
        let waiting: Vec<&str> = runners
            .iter()
            .filter(|r| r.pending_until.is_some())
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(waiting, ["rnr_3"]);
        let mut of = runners_of(&db, "enr_large").await?;
        of.sort();
        assert_eq!(of, ["rnr_1", "rnr_2", "rnr_3"]);
        Ok(())
    }

    /// A runner of a pool carries its labels beside its host's, and names
    /// it, in the runners list.
    #[wasm_bindgen_test]
    async fn runners_carry_their_pools_labels() -> AppResult<()> {
        let db = testing::database();
        acme(&db).await?;
        let hello = serde_json::json!({
            "protocol": 1, "version": "0.38.0", "hostname": "aws-1", "os": "linux", "arch": "x86_64",
            "cpus": 16, "memory_mb": 65536, "drivers": [], "driver": "docker", "capacity": 4,
            "labels": ["large", "zone:eu"], "sessions": []
        });
        q(
            &db,
            "UPDATE runners SET info = ? WHERE id = 'rnr_1'",
            &[s(&hello.to_string())],
        )?
        .run()
        .await?;
        let rows: Vec<crate::model::RunnerRow> = q(
            &db,
            &format!(
                "{} WHERE r.id IN ('rnr_1', 'rnr_laptop') ORDER BY r.id",
                crate::model::RUNNER_SELECT
            ),
            &[],
        )?
        .all()
        .await?;
        let runners: Vec<fctl_proto::api::Runner> = rows
            .iter()
            .map(|r| {
                r.to_runner(
                    &internal::LiveRunners::default(),
                    &crate::model::RunnerView::default(),
                )
            })
            .collect();
        let labels: Vec<Vec<String>> = runners.iter().map(|r| r.labels.clone()).collect();
        assert_eq!(labels, [vec!["aws", "large", "zone:eu"], vec![]]);
        let pools: Vec<Option<(&str, &str)>> = runners
            .iter()
            .map(|r| r.pool.as_ref().map(|p| (p.id.as_str(), p.name.as_str())))
            .collect();
        assert_eq!(pools, [Some(("enr_large", "AWS large")), None]);
        Ok(())
    }
}
