//! Plan usage (docs/architecture.md#plan-usage): the latest reading of each
//! Claude plan limit per credential, from the `rate_limit_event`s sessions
//! report (`fctl_domain::usage`), and telling the credential's owner.

use crate::db::{batch, n, of, on, os, q, s};
use crate::error::AppResult;
use crate::internal::{self, ClaudeUsage};
use crate::model::{USAGE_JSON, plan_usage};
use fctl_proto::CredentialId;
use fctl_proto::api::{LimitStatus, PlanLimit};
use serde::Deserialize;
use worker::Env;

/// Stores readings with the Claude credential the session runs on now (a
/// resumed session runs on the resumer's), each unless the stored reading
/// of its limit is newer, then sends the credential's usage to its owner's
/// open apps.
pub async fn record(env: &Env, session_id: &str, readings: &[PlanLimit]) -> AppResult<()> {
    let db = env.d1("DB")?;
    let mut statements = Vec::new();
    for r in readings {
        statements.push(q(
            &db,
            "INSERT INTO claude_usage (credential_id, window, utilization, resets_at, status, observed_at) \
             SELECT c.id, ?, ?, ?, ?, ? FROM sessions JOIN claude_credentials c ON c.id = sessions.credential_id \
             WHERE sessions.id = ? \
             ON CONFLICT (credential_id, window) DO UPDATE SET utilization = excluded.utilization, \
             resets_at = excluded.resets_at, status = excluded.status, observed_at = excluded.observed_at \
             WHERE excluded.observed_at >= claude_usage.observed_at",
            &[
                s(&r.window),
                of(r.utilization),
                on(r.resets_at),
                os(r.status.map(LimitStatus::as_str)),
                n(r.observed_at),
                s(session_id),
            ],
        )?);
    }
    batch(&db, statements).await?;

    #[derive(Deserialize)]
    struct Owner {
        id: String,
        user_id: String,
        usage: Option<String>,
    }
    let owner: Option<Owner> = q(
        &db,
        &format!(
            "SELECT c.id, c.user_id, {USAGE_JSON} AS usage FROM sessions \
             JOIN claude_credentials c ON c.id = sessions.credential_id WHERE sessions.id = ?"
        ),
        &[s(session_id)],
    )?
    .first()
    .await?;
    let Some(owner) = owner else {
        return Ok(());
    };
    let _: serde_json::Value = internal::post(
        &internal::fleet_stub(env)?,
        "/usage",
        &ClaudeUsage {
            user_id: owner.user_id,
            credential_id: CredentialId(owner.id),
            usage: plan_usage(owner.usage.as_deref()),
        },
    )
    .await?;
    Ok(())
}
