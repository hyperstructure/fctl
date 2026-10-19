//! What admins decide for the whole instance ([`InstanceSettings`]), kept
//! as JSON in D1's `settings` table.

use crate::config::Config;
use crate::db::{q, s};
use crate::error::AppResult;
use fctl_proto::api::InstanceSettings;
use serde::Deserialize;
use worker::D1Database;

/// Their key in the `settings` table.
pub const KEY: &str = "instance_settings";

#[derive(Deserialize)]
struct Row {
    value: String,
}

/// The instance's settings; their defaults until an admin changes one.
pub async fn settings(db: &D1Database) -> AppResult<InstanceSettings> {
    let row: Option<Row> = q(db, "SELECT value FROM settings WHERE key = ?", &[s(KEY)])?
        .first()
        .await?;
    Ok(parse(row.as_ref().map(|r| r.value.as_str())))
}

/// The settings stored as `json`, or their defaults.
pub fn parse(json: Option<&str>) -> InstanceSettings {
    json.and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default()
}

pub async fn save(db: &D1Database, settings: &InstanceSettings) -> AppResult<()> {
    q(
        db,
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        &[s(KEY), s(&serde_json::to_string(settings)?)],
    )?
    .run()
    .await?;
    Ok(())
}

/// The instance's name: what admins named it, or `FCTL_INSTANCE_NAME`.
pub async fn name(db: &D1Database, cfg: &Config) -> AppResult<String> {
    let row: Option<Row> = q(
        db,
        "SELECT value FROM settings WHERE key = 'instance_name'",
        &[],
    )?
    .first()
    .await?;
    Ok(row.map_or_else(|| cfg.instance_name.clone(), |r| r.value))
}
