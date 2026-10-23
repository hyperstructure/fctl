//! ScheduleObject: one Durable Object per instance, whose alarm starts the
//! sessions of schedules that are due (`crate::schedules`).
//!
//! D1 keeps the schedules; the object keeps nothing but its alarm, which
//! it sets for the earliest next run when schedules change (`/wake`, from
//! the API and the hourly job) and after each time it rings. Alarms ring
//! wherever the Worker runs: on Cloudflare, in the standalone server and
//! under `wrangler dev`, which runs no cron triggers of its own.

use crate::ctx::now_ms;
use crate::internal;
use fctl_proto::ApiError;
use worker::{DurableObject, Env, Request, Response, Result, State, durable_object};

/// After a failure to read the schedules, the alarm tries again this soon.
const RETRY_MS: i64 = 60_000;

#[durable_object]
pub struct ScheduleObject {
    state: State,
    env: Env,
}

impl DurableObject for ScheduleObject {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        match (req.method(), req.path().as_str()) {
            (worker::Method::Post, "/wake") => {
                let next = self.arm(false).await;
                internal::do_json(&serde_json::json!({ "next": next }))
            }
            _ => internal::do_error(&ApiError::not_found("Schedule endpoint")),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        // Never fail: each run is claimed in D1 before it starts, so a
        // retried alarm would start nothing twice, but it would start late.
        match crate::schedules::run_due(&self.env, now_ms()).await {
            Ok(0) => {}
            Ok(n) => worker::console_log!("schedules: took up {n}"),
            Err(e) => worker::console_error!("schedules: {}", e.0.message),
        }
        self.arm(true).await;
        Response::ok("ok")
    }
}

impl ScheduleObject {
    /// Sets the alarm for the earliest schedule that is on; returns when it
    /// rings. A wake only moves it earlier, since another wake that read the
    /// schedules first may come after it: an alarm that rings too soon finds
    /// nothing due and sets the next. After it rang (`spent`), it is set
    /// outright, and cleared once no schedule is on.
    async fn arm(&self, spent: bool) -> Option<i64> {
        let storage = self.state.storage();
        let now = now_ms();
        let earliest = match self.env.d1("DB") {
            Ok(db) => crate::schedules::earliest(&db).await,
            Err(e) => Err(e.into()),
        };
        let at = match earliest {
            Ok(Some(at)) => at.max(now),
            Ok(None) if !spent => return None,
            Ok(None) => {
                if let Err(e) = storage.delete_alarm().await {
                    worker::console_error!("schedules alarm: {e}");
                }
                return None;
            }
            Err(e) => {
                worker::console_error!("reading when schedules are due: {}", e.0.message);
                now + RETRY_MS
            }
        };
        if !spent
            && let Ok(Some(set)) = storage.get_alarm().await
            && set <= at
        {
            return Some(set);
        }
        if let Err(e) = storage.set_alarm(at - now).await {
            worker::console_error!("schedules alarm: {e}");
        }
        Some(at)
    }
}
