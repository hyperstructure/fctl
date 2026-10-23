//! Schedules (`fctl_domain::schedules`): tasks a project's agents start on
//! their own, once at a set time or again and again, each run a session on
//! the Claude subscription of the person the schedule runs for, its owner.
//! D1 keeps them (`schedules`) and their runs (`schedule_runs`);
//! ScheduleObject's alarm takes up the ones that are due (`run_due`), and
//! people and other tools run them when they choose (`api::schedules`).
//!
//! A run that is due starts a session as its owner, unless they can no
//! longer start one in the project (fctl pauses the schedule, saying why),
//! their plan is at its limit (the runs until it resets are skipped; one
//! that runs once waits for the reset) or the schedule's last session still
//! works on its task (this run is skipped).

use crate::access;
use crate::db::{int, int_bool, n, on, opt_int, os, q, s};
use crate::error::{AppError, AppResult, internal};
use crate::internal::{InitSession, Schedule as Dispatch};
use crate::model::{
    CredentialRow, PROJECT_SELECT, ProjectRow, USAGE_JSON, UserRow, plan_usage, user_columns,
};
use fctl_domain::cron::{self, Zone};
use fctl_domain::schedules::{self as sch, RunNote, StartedBy, When};
use fctl_proto::api::{PlanLimit, Schedule, ScheduleRun, ScheduleRunOutcome, ScheduleTrigger};
use fctl_proto::event::{MessageSource, UserMessage};
use fctl_proto::session::{GithubAccess, SessionRef, SessionStatus, UserRef};
use fctl_proto::{ApiError, ProjectId, ScheduleId, ScheduleRunId, SessionId, UserId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use wasm_bindgen::{JsCast, JsValue};
use worker::{D1Database, Env};

/// Schedules one alarm takes up at most; it rings again at once for more.
const DUE_PER_ALARM: i64 = 20;

// ───────────────────────────── time zones ─────────────────────────────

/// A time zone as JavaScript's `Intl` knows it: the platform's zone rules,
/// which the web app's browsers follow too.
pub struct IntlZone {
    format: JsValue,
    parts: js_sys::Function,
}

impl IntlZone {
    /// The zone `name`; `None` when `Intl` doesn't know it.
    pub fn new(name: &str) -> Option<IntlZone> {
        let intl = js_sys::Reflect::get(&js_sys::global(), &"Intl".into()).ok()?;
        let make: js_sys::Function = js_sys::Reflect::get(&intl, &"DateTimeFormat".into())
            .ok()?
            .dyn_into()
            .ok()?;
        let options = js_sys::Object::new();
        for (key, value) in [
            ("timeZone", name),
            ("hourCycle", "h23"),
            ("year", "numeric"),
            ("month", "numeric"),
            ("day", "numeric"),
            ("hour", "numeric"),
            ("minute", "numeric"),
            ("second", "numeric"),
        ] {
            js_sys::Reflect::set(&options, &key.into(), &value.into()).ok()?;
        }
        // A zone it doesn't know throws a RangeError.
        let format =
            js_sys::Reflect::construct(&make, &js_sys::Array::of2(&"en-US".into(), &options))
                .ok()?;
        let parts: js_sys::Function = js_sys::Reflect::get(&format, &"formatToParts".into())
            .ok()?
            .dyn_into()
            .ok()?;
        Some(IntlZone { format, parts })
    }

    /// Its clocks at `utc_ms`, in seconds since the local epoch.
    fn wall_seconds(&self, utc_ms: i64) -> Option<i64> {
        let date = js_sys::Date::new(&JsValue::from_f64(utc_ms as f64));
        let parts: js_sys::Array =
            js_sys::Reflect::apply(&self.parts, &self.format, &js_sys::Array::of1(&date))
                .ok()?
                .dyn_into()
                .ok()?;
        let mut v: HashMap<String, i64> = HashMap::new();
        for part in parts.iter() {
            let kind = js_sys::Reflect::get(&part, &"type".into())
                .ok()?
                .as_string()?;
            let value = js_sys::Reflect::get(&part, &"value".into())
                .ok()?
                .as_string();
            if let Some(num) = value.and_then(|t| t.parse::<i64>().ok()) {
                v.insert(kind, num);
            }
        }
        let num = |k: &str| v.get(k).copied();
        let days = cron::days_from_civil(
            num("year")?,
            u32::try_from(num("month")?).ok()?,
            u32::try_from(num("day")?).ok()?,
        );
        Some(days * 86_400 + num("hour")? % 24 * 3600 + num("minute")? * 60 + num("second")?)
    }
}

impl Zone for IntlZone {
    fn offset_ms(&self, utc_ms: i64) -> i64 {
        self.wall_seconds(utc_ms)
            .map_or(0, |wall| (wall - utc_ms.div_euclid(1000)) * 1000)
    }
}

/// The zone `name`, or a validation error when `Intl` doesn't know it.
pub fn known_zone(name: &str) -> AppResult<IntlZone> {
    IntlZone::new(name).ok_or_else(|| {
        AppError(ApiError::validation(
            "timezone",
            format!("fctl doesn't know the time zone {name}"),
        ))
    })
}

/// The zone a stored schedule's times are in: UTC for one `Intl` stopped
/// knowing.
pub fn zone(name: &str) -> Box<dyn Zone> {
    match IntlZone::new(name) {
        Some(z) => Box::new(z),
        None => Box::new(cron::Utc),
    }
}

// ───────────────────────────── rows ─────────────────────────────

/// A schedule, as D1 keeps it, with its people's names.
#[derive(Clone, Debug, Deserialize)]
pub struct ScheduleRow {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub prompt: String,
    pub cron: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub at: Option<i64>,
    pub timezone: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub base_branch: Option<String>,
    pub github_access: Option<String>,
    pub owner_id: Option<String>,
    pub owner_name: Option<String>,
    pub owner_avatar: Option<String>,
    #[serde(deserialize_with = "int_bool")]
    pub enabled: bool,
    pub paused_reason: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub next_run_at: Option<i64>,
    pub created_by: Option<String>,
    pub creator_name: Option<String>,
    pub creator_avatar: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    pub updated_by: Option<String>,
    pub updater_name: Option<String>,
    pub updater_avatar: Option<String>,
    #[serde(deserialize_with = "int")]
    pub updated_at: i64,
}

/// SELECT for [`ScheduleRow`]; `WHERE` and `ORDER BY` follow.
pub const SCHEDULE_SELECT: &str = "SELECT sc.id, sc.project_id, sc.name, sc.prompt, sc.cron, sc.at, sc.timezone, \
    sc.model, sc.effort, sc.base_branch, sc.github_access, sc.owner_id, ou.name AS owner_name, \
    ou.avatar_url AS owner_avatar, sc.enabled, sc.paused_reason, sc.next_run_at, \
    sc.created_by, cu.name AS creator_name, cu.avatar_url AS creator_avatar, sc.created_at, \
    sc.updated_by, uu.name AS updater_name, uu.avatar_url AS updater_avatar, sc.updated_at \
    FROM schedules sc LEFT JOIN users ou ON ou.id = sc.owner_id LEFT JOIN users cu ON cu.id = sc.created_by \
    LEFT JOIN users uu ON uu.id = sc.updated_by";

fn user_ref(
    id: &Option<String>,
    name: &Option<String>,
    avatar: &Option<String>,
) -> Option<UserRef> {
    Some(UserRef {
        id: UserId(id.clone()?),
        name: name.clone().unwrap_or_default(),
        avatar_url: avatar.clone(),
    })
}

impl ScheduleRow {
    /// When it runs; `None` for what this version can't read.
    pub fn when(&self) -> Option<When> {
        match (&self.cron, self.at) {
            (Some(c), _) => cron::Cron::parse(c).ok().map(When::Cron),
            (None, Some(at)) => Some(When::At(at)),
            (None, None) => Some(When::Asked),
        }
    }

    /// Whether it runs at a time of its own, rather than when asked.
    pub fn timed(&self) -> bool {
        self.cron.is_some() || self.at.is_some()
    }

    /// The person it runs for, as sentences name them.
    pub fn owner_name(&self) -> String {
        self.owner_name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Its owner".into())
    }

    /// What its sessions' agents may do on GitHub; `None`: as much as the
    /// project allows.
    pub fn github(&self) -> Option<GithubAccess> {
        self.github_access.as_deref().and_then(GithubAccess::parse)
    }

    pub fn to_api(&self, last_run: Option<ScheduleRun>, problem: Option<String>) -> Schedule {
        Schedule {
            id: ScheduleId(self.id.clone()),
            project_id: ProjectId(self.project_id.clone()),
            name: self.name.clone(),
            prompt: self.prompt.clone(),
            cron: self.cron.clone(),
            at: self.at,
            timezone: self.timezone.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            base_branch: self.base_branch.clone(),
            github_access: self.github(),
            enabled: self.enabled,
            state: sch::state(self.enabled, self.timed(), self.next_run_at),
            paused_reason: self.paused_reason.clone().filter(|_| !self.enabled),
            problem,
            next_run_at: self.next_run_at.filter(|_| self.enabled),
            last_run,
            owner: user_ref(&self.owner_id, &self.owner_name, &self.owner_avatar),
            created_by: user_ref(&self.created_by, &self.creator_name, &self.creator_avatar),
            created_at: self.created_at,
            updated_by: user_ref(&self.updated_by, &self.updater_name, &self.updater_avatar),
            updated_at: self.updated_at,
        }
    }
}

/// A run, with its person and session.
#[derive(Clone, Debug, Deserialize)]
pub struct RunRow {
    pub id: String,
    pub schedule_id: String,
    #[serde(deserialize_with = "int")]
    pub due_at: i64,
    #[serde(deserialize_with = "int")]
    pub at: i64,
    pub outcome: String,
    pub reason: Option<String>,
    pub trigger: String,
    pub user_id: Option<String>,
    pub user_name: Option<String>,
    pub user_avatar: Option<String>,
    pub session_id: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub session_number: Option<i64>,
    pub session_title: Option<String>,
    pub session_status: Option<String>,
}

/// SELECT for [`RunRow`]; `WHERE` and `ORDER BY` follow.
const RUN_SELECT: &str = "SELECT r.id, r.schedule_id, r.due_at, r.at, r.outcome, r.reason, r.trigger, r.user_id, \
    u.name AS user_name, u.avatar_url AS user_avatar, r.session_id, s.number AS session_number, \
    s.title AS session_title, s.status AS session_status \
    FROM schedule_runs r LEFT JOIN users u ON u.id = r.user_id LEFT JOIN sessions s ON s.id = r.session_id";

impl RunRow {
    pub fn to_api(&self) -> ScheduleRun {
        let session = match (&self.session_id, self.session_number) {
            (Some(id), Some(number)) => Some(SessionRef {
                id: SessionId(id.clone()),
                number: u32::try_from(number).unwrap_or_default(),
                title: self.session_title.clone().unwrap_or_default(),
            }),
            _ => None,
        };
        ScheduleRun {
            id: ScheduleRunId(self.id.clone()),
            schedule_id: ScheduleId(self.schedule_id.clone()),
            due_at: self.due_at,
            at: self.at,
            outcome: ScheduleRunOutcome::parse(&self.outcome),
            reason: self.reason.clone(),
            trigger: ScheduleTrigger::parse(&self.trigger),
            by: user_ref(&self.user_id, &self.user_name, &self.user_avatar),
            status: session
                .as_ref()
                .and(self.session_status.as_deref())
                .and_then(SessionStatus::parse),
            session,
        }
    }
}

/// A schedule by id.
pub async fn row(db: &D1Database, id: &str) -> AppResult<Option<ScheduleRow>> {
    q(db, &format!("{SCHEDULE_SELECT} WHERE sc.id = ?"), &[s(id)])?
        .first()
        .await
}

/// A schedule by id, with the caller's access to its project: not found
/// for someone who doesn't see the project.
pub async fn load(
    ctx: &crate::ctx::Ctx,
    id: &str,
) -> AppResult<(ScheduleRow, access::ProjectAccess)> {
    let row = row(&ctx.db, id)
        .await?
        .ok_or_else(|| crate::error::not_found("Schedule"))?;
    let project = access::project(ctx, &row.project_id)
        .await
        .map_err(|_| crate::error::not_found("Schedule"))?;
    Ok((row, project))
}

/// A project's schedules, in the order they were made.
pub async fn of_project(db: &D1Database, project_id: &str) -> AppResult<Vec<ScheduleRow>> {
    q(
        db,
        &format!("{SCHEDULE_SELECT} WHERE sc.project_id = ? ORDER BY sc.created_at, sc.id"),
        &[s(project_id)],
    )?
    .all()
    .await
}

/// The latest run of each of a project's schedules, by schedule.
pub async fn last_runs(
    db: &D1Database,
    project_id: &str,
) -> AppResult<HashMap<String, ScheduleRun>> {
    let rows: Vec<RunRow> = q(
        db,
        &format!(
            "{RUN_SELECT} WHERE r.id IN (SELECT (SELECT l.id FROM schedule_runs l WHERE l.schedule_id = sc.id \
             ORDER BY l.at DESC, l.id DESC LIMIT 1) FROM schedules sc WHERE sc.project_id = ?)"
        ),
        &[s(project_id)],
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.schedule_id.clone(), r.to_api()))
        .collect())
}

/// A schedule's runs, newest first: before `before`, at most `limit`.
pub async fn runs(
    db: &D1Database,
    schedule_id: &str,
    before: Option<i64>,
    limit: i64,
) -> AppResult<Vec<RunRow>> {
    q(
        db,
        &format!(
            "{RUN_SELECT} WHERE r.schedule_id = ?1 AND (?2 IS NULL OR r.at < ?2) ORDER BY r.at DESC, r.id DESC LIMIT ?3"
        ),
        &[s(schedule_id), on(before), n(limit)],
    )?
    .all()
    .await
}

/// One run, by id.
pub async fn run_row(db: &D1Database, id: &str) -> AppResult<Option<RunRow>> {
    q(db, &format!("{RUN_SELECT} WHERE r.id = ?"), &[s(id)])?
        .first()
        .await
}

/// The latest session a schedule started: its number, when, and its status
/// now.
pub struct Previous {
    pub number: u32,
    pub at: i64,
    pub status: SessionStatus,
}

pub async fn previous(db: &D1Database, schedule_id: &str) -> AppResult<Option<Previous>> {
    #[derive(Deserialize)]
    struct Row {
        #[serde(deserialize_with = "int")]
        number: i64,
        #[serde(deserialize_with = "int")]
        at: i64,
        status: String,
    }
    let row: Option<Row> = q(
        db,
        "SELECT s.number, r.at, s.status FROM schedule_runs r JOIN sessions s ON s.id = r.session_id \
         WHERE r.schedule_id = ? ORDER BY r.at DESC, r.id DESC LIMIT 1",
        &[s(schedule_id)],
    )?
    .first()
    .await?;
    Ok(row.map(|r| Previous {
        number: u32::try_from(r.number).unwrap_or_default(),
        at: r.at,
        status: SessionStatus::parse(&r.status).unwrap_or(SessionStatus::Failed),
    }))
}

// ───────────────────────────── who runs them ─────────────────────────────

/// Who a schedule runs for, and their Claude connection.
pub struct Owner {
    pub user: UserRow,
    pub credential: CredentialRow,
}

/// The person a schedule runs for and their Claude connection for the
/// project's workspace, or why its runs can't start on their subscription:
/// their account is gone or disabled, they can no longer start sessions in
/// the project (they left it or its workspace), or they have no connection.
pub async fn owner(
    db: &D1Database,
    project: &ProjectRow,
    row: &ScheduleRow,
) -> AppResult<Result<Owner, String>> {
    let name = row.owner_name();
    let Some(id) = row.owner_id.as_deref() else {
        return Ok(Err(
            "The account it ran for is gone. Resuming it runs it on your Claude subscription."
                .into(),
        ));
    };
    let user: Option<UserRow> = q(
        db,
        &format!("SELECT {} FROM users u WHERE u.id = ?", user_columns("u")),
        &[s(id)],
    )?
    .first()
    .await?;
    let Some(user) = user.filter(UserRow::is_active) else {
        return Ok(Err(format!(
            "It runs on {name}'s Claude subscription, and {name}'s account is gone or disabled."
        )));
    };
    let theirs: Option<ProjectRow> = q(
        db,
        &format!("{PROJECT_SELECT} WHERE p.id = ?2"),
        &[s(&user.id), s(&project.id)],
    )?
    .first()
    .await?;
    let settings = crate::instance::settings(db).await?;
    let role = theirs.and_then(|r| access::project_role(&user, &settings, None, &r));
    if !fctl_domain::perm::can_start_sessions(role) {
        return Ok(Err(format!(
            "It runs on {name}'s Claude subscription, and {name} can no longer start sessions in this project."
        )));
    }
    let credential =
        crate::api::sessions::credential_for(db, &user, project.workspace_id()).await?;
    let Some(credential) = credential else {
        return Ok(Err(format!(
            "It runs on {name}'s Claude subscription, and {name} has no Claude connection to run it on."
        )));
    };
    Ok(Ok(Owner { user, credential }))
}

/// The latest plan usage of a Claude connection.
pub async fn usage_of(db: &D1Database, credential_id: &str) -> AppResult<Vec<PlanLimit>> {
    #[derive(Deserialize)]
    struct Row {
        usage: Option<String>,
    }
    let row: Option<Row> = q(
        db,
        &format!("SELECT {USAGE_JSON} AS usage FROM claude_credentials c WHERE c.id = ?"),
        &[s(credential_id)],
    )?
    .first()
    .await?;
    Ok(plan_usage(row.and_then(|r| r.usage).as_deref()))
}

/// Why schedules that are on wouldn't start their next run now, for a list
/// of them: each owner looked up once.
#[derive(Default)]
pub struct Problems {
    owners: HashMap<String, Result<(String, Vec<PlanLimit>), String>>,
}

impl Problems {
    pub async fn of(
        &mut self,
        db: &D1Database,
        project: &ProjectRow,
        row: &ScheduleRow,
        now: i64,
    ) -> AppResult<Option<String>> {
        let Some(next) = row.next_run_at.filter(|_| row.enabled) else {
            return Ok(None);
        };
        if project.archived_at.is_some() {
            return Ok(Some(
                "Its project is archived, so its next run pauses it.".into(),
            ));
        }
        let key = row.owner_id.clone().unwrap_or_default();
        if !self.owners.contains_key(&key) {
            let found = match owner(db, project, row).await? {
                Ok(o) => Ok((o.user.name.clone(), usage_of(db, &o.credential.id).await?)),
                Err(why) => Err(why),
            };
            self.owners.insert(key.clone(), found);
        }
        let zone = zone(&row.timezone);
        Ok(match self.owners.get(&key) {
            Some(Err(why)) => Some(format!("{why} Its next run pauses it.")),
            Some(Ok((name, limits))) => match (sch::at_limit(limits, next.max(now)), row.when()) {
                (Some(limit), Some(When::At(_))) => Some(format!(
                    "{} It starts once the limit resets.",
                    sch::limit_reason(name, limit, &*zone, true)
                )),
                (Some(limit), _) => Some(format!(
                    "{} Its runs until then are skipped.",
                    sch::limit_reason(name, limit, &*zone, true)
                )),
                // It ran once, at a time the plan was at its limit.
                (None, Some(When::At(at))) if next > at + cron::MINUTE_MS => Some(format!(
                    "{name}'s Claude plan was at its limit at its time, so it waits for the limit to reset: it starts on {}.",
                    sch::stamp(&*zone, next)
                )),
                (None, _) => None,
            },
            None => None,
        })
    }
}

// ───────────────────────────── the alarm ─────────────────────────────

/// The ScheduleObject, whose alarm takes up the schedules that are due.
fn stub(env: &Env) -> AppResult<worker::Stub> {
    Ok(env
        .durable_object("SCHEDULES")?
        .id_from_name("schedules")?
        .get_stub()?)
}

/// Tells the ScheduleObject that schedules changed, so that its alarm
/// rings for the earliest. A failure waits for the hourly job.
pub async fn wake(env: &Env) {
    let woken: AppResult<Value> =
        async { crate::internal::post(&stub(env)?, "/wake", &json!({})).await }.await;
    if let Err(e) = woken {
        worker::console_error!("waking the schedules: {}", e.0.message);
    }
}

/// When the earliest schedule that is on is due.
pub async fn earliest(db: &D1Database) -> AppResult<Option<i64>> {
    #[derive(Deserialize)]
    struct Row {
        #[serde(deserialize_with = "opt_int", default)]
        at: Option<i64>,
    }
    let row: Option<Row> = q(
        db,
        "SELECT MIN(next_run_at) AS at FROM schedules WHERE enabled = 1 AND next_run_at IS NOT NULL",
        &[],
    )?
    .first()
    .await?;
    Ok(row.and_then(|r| r.at))
}

/// Takes up the schedules due at `now`, the longest due first: returns how
/// many it took up. One that fails is logged, and the others go on.
pub async fn run_due(env: &Env, now: i64) -> AppResult<usize> {
    let db = env.d1("DB")?;
    let due: Vec<ScheduleRow> = q(
        &db,
        &format!(
            "{SCHEDULE_SELECT} WHERE sc.enabled = 1 AND sc.next_run_at IS NOT NULL AND sc.next_run_at <= ? \
             ORDER BY sc.next_run_at LIMIT ?"
        ),
        &[n(now), n(DUE_PER_ALARM)],
    )?
    .all()
    .await?;
    let mut taken = 0;
    for row in &due {
        match take_up(env, &db, row, now).await {
            Ok(true) => taken += 1,
            Ok(false) => {}
            Err(e) => worker::console_error!("schedule {}: {}", row.id, e.0.message),
        }
    }
    Ok(taken)
}

/// Takes up one schedule that is due: moves it to its next time, then
/// starts its run's session, or records why it starts none. `false` when
/// another alarm took it up, or someone changed it meanwhile.
async fn take_up(env: &Env, db: &D1Database, row: &ScheduleRow, now: i64) -> AppResult<bool> {
    let Some(due) = row.next_run_at else {
        return Ok(false);
    };
    let zone = zone(&row.timezone);
    let Some(when) = row.when() else {
        pause(
            db,
            &row.id,
            "fctl can't read when it runs: set its time again.",
            now,
        )
        .await?;
        return Ok(false);
    };
    let next = when.next(&*zone, now, true);
    let took = q(
        db,
        "UPDATE schedules SET next_run_at = ?, last_run_at = ? WHERE id = ? AND enabled = 1 AND next_run_at = ?",
        &[on(next), n(now), s(&row.id), n(due)],
    )?
    .run()
    .await?;
    if took == 0 {
        return Ok(false);
    }
    let Some(project) = crate::spec::project_row(db, &row.project_id).await? else {
        return Ok(false);
    };
    // Once claimed, the run is recorded whatever happens: a schedule that
    // runs once has no other time.
    let claimed = Claimed {
        row,
        project: &project,
        when: &when,
        zone: &*zone,
        next,
        now,
    };
    let outcome = match run(env, db, claimed).await {
        Ok(Some(outcome)) => outcome,
        Ok(None) => return Ok(true),
        Err(e) => Outcome::Failed(e.0.message),
    };
    outcome
        .record(
            db,
            row,
            due,
            now,
            ScheduleTrigger::Time,
            row.owner_id.as_deref(),
        )
        .await?;
    Ok(true)
}

/// A run a schedule's time came for, which moved the schedule to `next`.
struct Claimed<'a> {
    row: &'a ScheduleRow,
    project: &'a ProjectRow,
    when: &'a When,
    zone: &'a dyn Zone,
    next: Option<i64>,
    now: i64,
}

/// A claimed run: the session it starts, or why it starts none; `None` for
/// a schedule that runs once and waits for its owner's plan to reset, which
/// records no run.
async fn run(env: &Env, db: &D1Database, c: Claimed<'_>) -> AppResult<Option<Outcome>> {
    let Claimed {
        row,
        project,
        when,
        zone,
        next,
        now,
    } = c;
    let skip = |reason: String| Some(Outcome::Skipped(reason));
    Ok('run: {
        if project.archived_at.is_some() {
            let why = "Its project is archived.".to_owned();
            pause(db, &row.id, &why, now).await?;
            break 'run skip(why);
        }
        let owner = match owner(db, project, row).await? {
            Ok(o) => o,
            Err(why) => {
                pause(db, &row.id, &why, now).await?;
                break 'run skip(why);
            }
        };
        let limits = usage_of(db, &owner.credential.id).await?;
        if let Some(limit) = sch::at_limit(&limits, now) {
            let resets = limit.resets_at.filter(|r| *r > now);
            match &when {
                // It runs once: it waits for the limit to reset.
                When::At(_) => {
                    let at = resets.unwrap_or(now + cron::HOUR_MS) + cron::MINUTE_MS;
                    q(
                        db,
                        "UPDATE schedules SET next_run_at = ? WHERE id = ? AND enabled = 1 AND next_run_at IS NULL",
                        &[n(at), s(&row.id)],
                    )?
                    .run()
                    .await?;
                    return Ok(None);
                }
                // Again and again: the runs until it resets are skipped.
                // (One that runs when asked has no time to come to this.)
                When::Cron(_) | When::Asked => {
                    if let (When::Cron(c), Some(resets)) = (when, resets) {
                        q(
                            db,
                            "UPDATE schedules SET next_run_at = ? WHERE id = ? AND enabled = 1 AND next_run_at IS ?",
                            &[on(c.next_after(zone, resets)), s(&row.id), on(next)],
                        )?
                        .run()
                        .await?;
                    }
                    break 'run skip(format!(
                        "{} Its runs until then are skipped.",
                        sch::limit_reason(&owner.user.name, limit, zone, false)
                    ));
                }
            }
        }
        let previous = previous(db, &row.id).await?;
        if let Some(p) = previous.as_ref().filter(|p| sch::still_working(p.status)) {
            break 'run skip(format!(
                "Its last run, session #{}, was still working on it.",
                p.number
            ));
        }
        let github = fctl_domain::limits::effective(row.github(), project.settings().github_access);
        let previous_at = previous.as_ref().map(|p| sch::stamp(zone, p.at));
        let text = sch::prompt(
            &row.prompt,
            &RunNote {
                name: &row.name,
                when: &when.rhythm(),
                timezone: &row.timezone,
                owner: &owner.user.name,
                by: StartedBy::Time,
                now: &sch::stamp(zone, now),
                previous: previous
                    .as_ref()
                    .zip(previous_at.as_deref())
                    .map(|(p, at)| (p.number, at)),
                context: None,
            },
        );
        let started = start_session(
            env,
            db,
            Start {
                project,
                schedule: row,
                user: &owner.user,
                credential: &owner.credential,
                github_access: github,
                prompt: text,
                author: None,
                source: MessageSource::System,
                follow: true,
                now,
            },
        )
        .await;
        match started {
            Ok((session, number)) => {
                audit(
                    db,
                    project.workspace_id(),
                    "session.created",
                    Some(session.as_str()),
                    json!({
                        "project": project.slug,
                        "number": number,
                        "github_access": github,
                        "schedule": row.id,
                        "for": owner.user.id,
                    }),
                    now,
                )
                .await;
                Some(Outcome::Started(session))
            }
            Err(e) => Some(Outcome::Failed(e.0.message)),
        }
    })
}

/// What came of a run.
pub enum Outcome {
    Started(SessionId),
    Skipped(String),
    Failed(String),
}

impl Outcome {
    /// Records it as a run of `schedule`, due at `due`, for `user`.
    pub async fn record(
        &self,
        db: &D1Database,
        schedule: &ScheduleRow,
        due: i64,
        now: i64,
        trigger: ScheduleTrigger,
        user: Option<&str>,
    ) -> AppResult<String> {
        let (outcome, reason, session) = match self {
            Outcome::Started(id) => (ScheduleRunOutcome::Started, None, Some(id.as_str())),
            Outcome::Skipped(why) => (ScheduleRunOutcome::Skipped, Some(why.as_str()), None),
            Outcome::Failed(why) => (ScheduleRunOutcome::Failed, Some(why.as_str()), None),
        };
        let id = fctl_domain::ids::new_id(ScheduleRunId::PREFIX, now);
        q(
            db,
            "INSERT INTO schedule_runs (id, schedule_id, due_at, at, outcome, reason, trigger, user_id, session_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                s(&id),
                s(&schedule.id),
                n(due),
                n(now),
                s(outcome.as_str()),
                os(reason),
                s(trigger.as_str()),
                os(user),
                os(session),
            ],
        )?
        .run()
        .await?;
        // Only the newest are kept.
        q(
            db,
            "DELETE FROM schedule_runs WHERE schedule_id = ?1 AND id NOT IN \
             (SELECT id FROM schedule_runs WHERE schedule_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2)",
            &[s(&schedule.id), n(sch::RUNS_KEPT)],
        )?
        .run()
        .await?;
        Ok(id)
    }
}

/// Pauses a schedule that fctl can't run, saying why.
async fn pause(db: &D1Database, id: &str, why: &str, now: i64) -> AppResult<()> {
    q(
        db,
        "UPDATE schedules SET enabled = 0, paused_reason = ?, next_run_at = NULL, updated_at = ? WHERE id = ?",
        &[s(why), n(now), s(id)],
    )?
    .run()
    .await?;
    Ok(())
}

/// An entry in the workspace's audit log for what the alarm did, which no
/// one asked for then.
async fn audit(
    db: &D1Database,
    workspace_id: &str,
    action: &str,
    target: Option<&str>,
    data: Value,
    now: i64,
) {
    let id = fctl_domain::ids::new_id("aud", now);
    let written: AppResult<usize> = async {
        q(
            db,
            "INSERT INTO audit_log (id, ts, actor_id, action, target, data, ip, workspace_id) VALUES (?, ?, NULL, ?, ?, ?, NULL, ?)",
            &[
                s(&id),
                n(now),
                s(action),
                os(target),
                s(&data.to_string()),
                os(Some(workspace_id).filter(|w| !w.is_empty())),
            ],
        )?
        .run()
        .await
    }
    .await;
    if let Err(e) = written {
        worker::console_error!("audit for a schedule: {}", e.0.message);
    }
}

// ───────────────────────────── a run's session ─────────────────────────────

/// What a run's session starts with.
pub struct Start<'a> {
    pub project: &'a ProjectRow,
    pub schedule: &'a ScheduleRow,
    /// Who it runs as, on which Claude connection.
    pub user: &'a UserRow,
    pub credential: &'a CredentialRow,
    pub github_access: GithubAccess,
    /// Its first message: the task and what the run is.
    pub prompt: String,
    /// Who sent it: nobody (fctl) at the schedule's time.
    pub author: Option<UserRef>,
    pub source: MessageSource,
    /// Its person follows it, as starting a session does.
    pub follow: bool,
    pub now: i64,
}

/// Starts a run's session, as a session that a person starts in the
/// project, named after the schedule: returns it and its number.
pub async fn start_session(
    env: &Env,
    db: &D1Database,
    st: Start<'_>,
) -> AppResult<(SessionId, u32)> {
    #[derive(Deserialize)]
    struct Counter {
        #[serde(deserialize_with = "int")]
        session_counter: i64,
    }
    let id = fctl_domain::ids::session_id(st.now);
    let title = fctl_domain::validate::session_title(&st.schedule.name)
        .unwrap_or_else(|_| fctl_domain::naming::title_from_prompt(&st.schedule.prompt));
    let base_branch = st
        .schedule
        .base_branch
        .clone()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| st.project.default_branch.clone());
    let counter: Option<Counter> = q(
        db,
        "UPDATE projects SET session_counter = session_counter + 1 WHERE id = ? RETURNING session_counter",
        &[s(&st.project.id)],
    )?
    .first()
    .await?;
    let number = u32::try_from(counter.map_or(1, |c| c.session_counter)).unwrap_or(1);
    let branch = fctl_domain::naming::session_branch(number, &title);
    q(
        db,
        "INSERT INTO sessions (id, project_id, number, title, prompt, status, created_by, credential_id, branch, base_branch, \
         model, effort, claude_session_id, stats, created_at, updated_at, last_activity_at, github_access, schedule_id) \
         VALUES (?, ?, ?, ?, ?, 'queued', ?, ?, ?, ?, ?, ?, ?, '{}', ?, ?, ?, ?, ?)",
        &[
            s(id.as_str()),
            s(&st.project.id),
            n(i64::from(number)),
            s(&title),
            s(&st.prompt),
            s(&st.user.id),
            s(&st.credential.id),
            s(&branch),
            s(&base_branch),
            os(st.schedule.model.as_deref()),
            os(st.schedule.effort.as_deref()),
            s(&fctl_domain::ids::uuid_v4()),
            n(st.now),
            n(st.now),
            n(st.now),
            s(st.github_access.as_str()),
            s(&st.schedule.id),
        ],
    )?
    .run()
    .await?;
    if st.follow
        && let Err(e) = crate::notify::follow_own(db, id.as_str(), &st.user.id, st.now).await
    {
        worker::console_error!("following a schedule's session: {}", e.0.message);
    }
    let session = crate::spec::session_row(db, id.as_str())
        .await?
        .ok_or_else(|| internal("the schedule's session vanished"))?
        .to_session(None);
    let _: Value = crate::internal::post(
        &crate::internal::session_stub(env, id.as_str())?,
        "/init",
        &InitSession {
            session,
            prompt: UserMessage {
                id: fctl_domain::ids::delivery_id(st.now),
                author: st.author,
                text: st.prompt,
                source: st.source,
                attachments: Vec::new(),
            },
        },
    )
    .await?;
    let _: Value = crate::internal::post(
        &crate::internal::fleet_stub(env)?,
        "/schedule",
        &Dispatch {
            session_id: id.clone(),
        },
    )
    .await?;
    Ok((id, number))
}
