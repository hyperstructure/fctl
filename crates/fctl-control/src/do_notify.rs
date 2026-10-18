//! NotifyObject: one Durable Object per person, for their notifications.
//!
//! Sessions they follow send it what happened (`/event`). It holds that for
//! a moment, so that a burst goes out as one digest and at most one
//! notification goes out a minute (`fctl_domain::notify::due`), then sends
//! it to the person's browsers when its alarm fires. A session that changed
//! again meanwhile (someone replied, it resumed) is left out. Once nothing
//! waits and the minute is over, it deletes everything it stored.

use crate::config::Config;
use crate::ctx::now_ms;
use crate::db::{q, s};
use crate::error::AppResult;
use crate::internal::{self, Notify};
use crate::push;
use fctl_domain::notify::{self, Event};
use fctl_proto::session::SessionStatus;
use fctl_proto::{ApiError, ErrorCode};
use serde::Deserialize;
use worker::{
    DurableObject, Env, Request, Response, Result, SqlStorage, SqlStorageValue, State,
    durable_object,
};

#[durable_object]
pub struct NotifyObject {
    state: State,
    env: Env,
    sql: SqlStorage,
}

#[derive(Deserialize)]
struct PendingRow {
    data: String,
}

#[derive(Deserialize)]
struct MetaRow {
    value: String,
}

fn sv(s: &str) -> SqlStorageValue {
    SqlStorageValue::from(s)
}

fn iv(n: i64) -> SqlStorageValue {
    SqlStorageValue::from(n)
}

fn create_schema(sql: &SqlStorage) {
    for stmt in [
        // One event per session, the latest.
        "CREATE TABLE IF NOT EXISTS pending (session_id TEXT PRIMARY KEY, at INTEGER NOT NULL, data TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    ] {
        if let Err(e) = sql.exec(stmt, None) {
            worker::console_error!("notify schema: {e}");
        }
    }
}

impl DurableObject for NotifyObject {
    fn new(state: State, env: Env) -> Self {
        let sql = state.storage().sql();
        create_schema(&sql);
        Self { state, env, sql }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        let result = match (req.method(), path.as_str()) {
            (worker::Method::Post, "/event") => match req.json::<Notify>().await {
                Ok(b) => self.event(b).await,
                Err(e) => Err(ApiError::new(ErrorCode::BadRequest, e.to_string())),
            },
            _ => Err(ApiError::not_found("Notify endpoint")),
        };
        match result {
            Ok(v) => internal::do_json(&v),
            Err(e) => internal::do_error(&e),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        // Never fail: a retried alarm would send the same notifications again.
        self.flush(now_ms()).await;
        Response::ok("ok")
    }
}

impl NotifyObject {
    fn meta(&self, key: &str) -> Option<String> {
        self.sql
            .exec("SELECT value FROM meta WHERE key = ?", Some(vec![sv(key)]))
            .and_then(|c| c.to_array::<MetaRow>())
            .ok()?
            .into_iter()
            .next()
            .map(|r| r.value)
    }

    fn set_meta(&self, key: &str, value: &str) {
        if let Err(e) = self.sql.exec(
            "INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            Some(vec![sv(key), sv(value)]),
        ) {
            worker::console_error!("notify meta write: {e}");
        }
    }

    fn last_sent(&self) -> Option<i64> {
        self.meta("last_sent").and_then(|v| v.parse().ok())
    }

    /// What waits to go out, oldest first.
    fn pending(&self) -> Vec<Event> {
        self.sql
            .exec("SELECT data FROM pending ORDER BY at ASC", None)
            .and_then(|c| c.to_array::<PendingRow>())
            .unwrap_or_default()
            .iter()
            .filter_map(|r| serde_json::from_str(&r.data).ok())
            .collect()
    }

    async fn schedule(&self, at: i64, now: i64) {
        if let Err(e) = self.state.storage().set_alarm((at - now).max(0)).await {
            worker::console_error!("notify alarm: {e}");
        }
    }

    async fn event(&self, b: Notify) -> std::result::Result<serde_json::Value, ApiError> {
        let now = now_ms();
        let starts_burst = self.pending().is_empty();
        let data = serde_json::to_string(&b.event)
            .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?;
        self.set_meta("user", &b.user_id);
        self.sql
            .exec(
                "INSERT INTO pending (session_id, at, data) VALUES (?, ?, ?) \
                 ON CONFLICT (session_id) DO UPDATE SET at = excluded.at, data = excluded.data",
                Some(vec![
                    sv(b.event.session_id.as_str()),
                    iv(b.event.at),
                    sv(&data),
                ]),
            )
            .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?;
        // A new burst replaces the clean-up alarm; later events join it.
        let alarm = self.state.storage().get_alarm().await.ok().flatten();
        if starts_burst || alarm.is_none() {
            self.schedule(notify::due(now, self.last_sent()), now).await;
        }
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn flush(&self, now: i64) {
        let events = self.pending();
        if events.is_empty() {
            if self
                .last_sent()
                .is_none_or(|t| now >= t + notify::SPACING_MS)
            {
                if let Err(e) = self.state.storage().delete_all().await {
                    worker::console_error!("notify clean-up: {e}");
                }
                // Empty tables, as a new object has, for the next event.
                create_schema(&self.sql);
            }
            return;
        }
        let Some(user) = self.meta("user") else {
            let _ = self.sql.exec("DELETE FROM pending", None);
            return;
        };
        let current = match self.still_current(&user, &events).await {
            Ok(current) => current,
            Err(e) => {
                worker::console_error!("notify: checking sessions failed: {}", e.0.message);
                events.clone()
            }
        };
        if let Some(message) = notify::compose(&current) {
            let cfg = Config::from_env(&self.env);
            let delivered = async {
                let db = self.env.d1("DB")?;
                push::deliver(&cfg, &db, &user, &message).await
            }
            .await;
            match delivered {
                Ok(_) => self.set_meta("last_sent", &now.to_string()),
                Err(e) => worker::console_error!("notify: sending failed: {}", e.0.message),
            }
        }
        // Forget what went out, but keep what arrived while it did: a newer
        // event for a session has a later time.
        for e in &events {
            let _ = self.sql.exec(
                "DELETE FROM pending WHERE session_id = ? AND at = ?",
                Some(vec![sv(e.session_id.as_str()), iv(e.at)]),
            );
        }
        let next = match self.pending().first() {
            Some(_) => notify::due(now, self.last_sent()),
            None => now + notify::SPACING_MS,
        };
        self.schedule(next, now).await;
    }

    /// The events whose session is still in the state they announce, and
    /// that `user` still follows.
    async fn still_current(&self, user: &str, events: &[Event]) -> AppResult<Vec<Event>> {
        #[derive(Deserialize)]
        struct Row {
            id: String,
            status: String,
        }
        let ids = serde_json::to_string(
            &events
                .iter()
                .map(|e| e.session_id.as_str())
                .collect::<Vec<_>>(),
        )?;
        let db = self.env.d1("DB")?;
        let rows: Vec<Row> = q(
            &db,
            "SELECT s.id AS id, s.status AS status FROM sessions s \
             JOIN session_followers f ON f.session_id = s.id AND f.user_id = ? \
             WHERE s.id IN (SELECT value FROM json_each(?))",
            &[s(user), s(&ids)],
        )?
        .all()
        .await?;
        Ok(events
            .iter()
            .filter(|e| {
                rows.iter().any(|r| {
                    r.id == e.session_id.as_str()
                        && SessionStatus::parse(&r.status).is_some_and(|st| e.kind.still(st))
                })
            })
            .cloned()
            .collect())
    }
}
