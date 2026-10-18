//! What admins decide for the whole instance ([`InstanceSettings`]), kept
//! as JSON in D1's `settings` table.

use crate::db::{q, s};
use crate::error::AppResult;
use fctl_proto::api::InstanceSettings;
use serde::Deserialize;
use worker::D1Database;

const KEY: &str = "instance_settings";

#[derive(Deserialize)]
struct Row {
    value: String,
}

/// The instance's settings; their defaults until an admin changes one.
pub async fn settings(db: &D1Database) -> AppResult<InstanceSettings> {
    let row: Option<Row> = q(db, "SELECT value FROM settings WHERE key = ?", &[s(KEY)])?
        .first()
        .await?;
    Ok(row
        .and_then(|r| serde_json::from_str(&r.value).ok())
        .unwrap_or_default())
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
