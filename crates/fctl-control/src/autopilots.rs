//! Autopilots (experimental; docs/architecture.md#autopilots): an agent
//! that runs a project, or its whole workspace, on its own.
//!
//! D1 keeps each autopilot (`autopilots`, one per project, its home), the
//! notes people steer it with, what it asks them and its log. It works in
//! sessions of its own in its home project (`sessions.autopilot_role =
//! 'pilot'`), and through sessions it starts (`'crew'`), all on its owner's
//! Claude subscription. ScheduleObject's alarm wakes it ([`run_due`]): at
//! its check-in, at the time it asked for, and soon after something needs
//! it ([`nudge`]): people steered or answered it, one of its sessions ended
//! a turn, asks something or failed ([`session_moved`]). A wake is a
//! message from fctl to its session ([`take_up`]), which a session that
//! started less than [`fctl_domain::autopilot::SHIFT_MS`] ago takes as it
//! is; otherwise fctl starts it a fresh one, briefed.
//!
//! Its sandbox's `fctl` reaches the API through fctl-agent and its host
//! ([`relay`]): the control plane answers each request as the autopilot's
//! owner, held to its places and a maintainer's role (`Limits`), and to
//! what `fctl_domain::autopilot::allows` lets an autopilot do ([`gate`]).

use crate::access;
use crate::ctx::{Authed, AutopilotCaller, Ctx};
use crate::db::{int, int_bool, n, opt_int, os, q, s};
use crate::error::{AppError, AppResult, forbidden, not_found};
use crate::model::{CredentialRow, PROJECT_SELECT, ProjectRow, UserRow, user_columns};
use fctl_domain::autopilot::{self as ap, Gate};
use fctl_domain::limits::{Limits, Places};
use fctl_proto::api::{AgentAnswer, TokenAccess, TokenKind};
use fctl_proto::autopilot::{
    AutopilotEntryKind, AutopilotPower, AutopilotRole, AutopilotScope, AutopilotSpec,
    AutopilotState, AutopilotWake, AutopilotWakeReason,
};
use fctl_proto::event::{EventBody, MessageSource, UserMessage};
use fctl_proto::session::{SessionRef, SessionStatus, WorkspaceRole};
use fctl_proto::uplink::{Delivery, DeliveryKind};
use fctl_proto::{ApiError, ProjectId, SessionId};
use serde::Deserialize;
use serde_json::{Value, json};
use worker::{D1Database, Env};

/// Autopilots one alarm takes up at most; it rings again at once for more.
const DUE_PER_ALARM: i64 = 20;

// ───────────────────────────── rows ─────────────────────────────

/// An autopilot as D1 keeps it, with its people's names and its own
/// session.
#[derive(Clone, Debug, Deserialize)]
pub struct AutopilotRow {
    pub project_id: String,
    pub workspace_id: String,
    pub scope: String,
    pub state: String,
    pub paused_reason: Option<String>,
    pub paused_by: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub paused_at: Option<i64>,
    pub owner_id: Option<String>,
    pub direction: String,
    pub direction_by: Option<String>,
    #[serde(deserialize_with = "int")]
    pub direction_at: i64,
    #[serde(deserialize_with = "int_bool", default)]
    pub direction_new: bool,
    pub plan: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub plan_at: Option<i64>,
    pub powers: String,
    #[serde(deserialize_with = "int")]
    pub check_in_minutes: i64,
    #[serde(deserialize_with = "int")]
    pub crew_max: i64,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub session_id: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub wake_at: Option<i64>,
    #[serde(default)]
    pub wake_reasons: String,
    #[serde(deserialize_with = "opt_int", default)]
    pub asked_wake_at: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    pub woken_at: Option<i64>,
    #[serde(deserialize_with = "int")]
    pub check_ins: i64,
    #[serde(deserialize_with = "int")]
    pub failures: i64,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "int")]
    pub updated_at: i64,
    // Joined.
    pub owner_name: Option<String>,
    pub owner_avatar: Option<String>,
    pub paused_by_name: Option<String>,
    pub paused_by_avatar: Option<String>,
    pub direction_by_name: Option<String>,
    pub direction_by_avatar: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub session_number: Option<i64>,
    pub session_status: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub session_created_at: Option<i64>,
}

pub const AUTOPILOT_SELECT: &str = "SELECT a.project_id, a.workspace_id, a.scope, a.state, a.paused_reason, \
    a.paused_by, a.paused_at, a.owner_id, a.direction, a.direction_by, a.direction_at, a.direction_new, a.plan, \
    a.plan_at, a.powers, a.check_in_minutes, a.crew_max, a.model, a.effort, a.session_id, a.wake_at, a.wake_reasons, \
    a.asked_wake_at, a.woken_at, a.check_ins, a.failures, a.created_at, a.updated_at, \
    ou.name AS owner_name, ou.avatar_url AS owner_avatar, pu.name AS paused_by_name, pu.avatar_url AS paused_by_avatar, \
    du.name AS direction_by_name, du.avatar_url AS direction_by_avatar, \
    ps.number AS session_number, ps.status AS session_status, ps.created_at AS session_created_at \
    FROM autopilots a \
    LEFT JOIN users ou ON ou.id = a.owner_id \
    LEFT JOIN users pu ON pu.id = a.paused_by \
    LEFT JOIN users du ON du.id = a.direction_by \
    LEFT JOIN sessions ps ON ps.id = a.session_id";

impl AutopilotRow {
    pub fn scope(&self) -> AutopilotScope {
        AutopilotScope::parse(&self.scope).unwrap_or_default()
    }

    pub fn state(&self) -> AutopilotState {
        AutopilotState::parse(&self.state).unwrap_or(AutopilotState::Paused)
    }

    pub fn powers(&self) -> Vec<AutopilotPower> {
        ap::parse_powers(&self.powers)
    }

    pub fn check_in(&self) -> u32 {
        u32::try_from(self.check_in_minutes).unwrap_or(ap::DEFAULT_CHECK_IN)
    }

    pub fn crew_max(&self) -> u32 {
        u32::try_from(self.crew_max).unwrap_or(ap::DEFAULT_CREW)
    }

    /// Why it wakes next, gathered since it last woke.
    pub fn reasons(&self) -> Vec<AutopilotWakeReason> {
        let mut out: Vec<AutopilotWakeReason> = self
            .wake_reasons
            .split(',')
            .filter_map(|r| AutopilotWakeReason::parse(r.trim()))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Why it wakes at `wake_at`: what gathered, else the time it asked
    /// for, else its check-in.
    pub fn wake_reason(&self) -> Option<AutopilotWakeReason> {
        self.wake_at?;
        Some(match self.reasons().first() {
            Some(r) => *r,
            None if self.asked_wake_at.is_some() && self.asked_wake_at == self.wake_at => {
                AutopilotWakeReason::Asked
            }
            None => AutopilotWakeReason::CheckIn,
        })
    }

    pub fn session_status(&self) -> Option<SessionStatus> {
        self.session_status
            .as_deref()
            .and_then(SessionStatus::parse)
    }

    pub fn owner_ref(&self) -> Option<fctl_proto::session::UserRef> {
        user_ref(
            self.owner_id.as_deref(),
            self.owner_name.as_deref(),
            &self.owner_avatar,
        )
    }

    pub fn paused_by_ref(&self) -> Option<fctl_proto::session::UserRef> {
        user_ref(
            self.paused_by.as_deref(),
            self.paused_by_name.as_deref(),
            &self.paused_by_avatar,
        )
    }

    pub fn direction_by_ref(&self) -> Option<fctl_proto::session::UserRef> {
        user_ref(
            self.direction_by.as_deref(),
            self.direction_by_name.as_deref(),
            &self.direction_by_avatar,
        )
    }
}

pub fn user_ref(
    id: Option<&str>,
    name: Option<&str>,
    avatar: &Option<String>,
) -> Option<fctl_proto::session::UserRef> {
    Some(fctl_proto::session::UserRef {
        id: fctl_proto::UserId(id?.to_owned()),
        name: name?.to_owned(),
        avatar_url: avatar.clone(),
    })
}

/// The autopilot of project `project_id`, if it has one.
pub async fn row(db: &D1Database, project_id: &str) -> AppResult<Option<AutopilotRow>> {
    q(
        db,
        &format!("{AUTOPILOT_SELECT} WHERE a.project_id = ?"),
        &[s(project_id)],
    )?
    .first()
    .await
}

/// The workspace's own autopilot, if it has one.
pub async fn of_workspace(db: &D1Database, workspace_id: &str) -> AppResult<Option<AutopilotRow>> {
    q(
        db,
        &format!(
            "{AUTOPILOT_SELECT} WHERE a.workspace_id = ? AND a.scope = 'workspace' AND a.state != 'off'"
        ),
        &[s(workspace_id)],
    )?
    .first()
    .await
}

// ───────────────────────────── its log ─────────────────────────────

/// An entry for an autopilot's log.
pub struct Entry<'a> {
    pub kind: AutopilotEntryKind,
    pub text: String,
    /// The person who did it.
    pub user: Option<&'a str>,
    /// The session it concerns.
    pub session: Option<&'a str>,
    /// The project it concerns, when another than its home.
    pub other_project: Option<&'a str>,
    /// Its own session that did it.
    pub from: Option<&'a str>,
}

impl<'a> Entry<'a> {
    pub fn new(kind: AutopilotEntryKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            user: None,
            session: None,
            other_project: None,
            from: None,
        }
    }

    pub fn by(mut self, user: &'a str) -> Self {
        self.user = Some(user);
        self
    }

    pub fn about(mut self, session: Option<&'a str>) -> Self {
        self.session = session;
        self
    }
}

/// Adds `entry` to the log of the autopilot of `project_id`, keeping its
/// newest [`ap::LOG_KEPT`]. A failure is logged: nothing hangs on it.
pub async fn log(db: &D1Database, project_id: &str, entry: Entry<'_>, now: i64) {
    let id = fctl_domain::ids::new_id("apl", now);
    let written: AppResult<()> = async {
        q(
            db,
            "INSERT INTO autopilot_log (id, project_id, at, kind, text, user_id, session_id, other_project_id, from_session_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                s(&id),
                s(project_id),
                n(now),
                s(entry.kind.as_str()),
                s(&entry.text),
                os(entry.user),
                os(entry.session),
                os(entry.other_project.filter(|p| *p != project_id)),
                os(entry.from),
            ],
        )?
        .run()
        .await?;
        // Now and then, rather than at every entry.
        if id.ends_with('0') {
            q(
                db,
                "DELETE FROM autopilot_log WHERE project_id = ?1 AND id NOT IN \
                 (SELECT id FROM autopilot_log WHERE project_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2)",
                &[s(project_id), n(ap::LOG_KEPT)],
            )?
            .run()
            .await?;
        }
        Ok(())
    }
    .await;
    if let Err(e) = written {
        worker::console_error!("autopilot log: {}", e.0.message);
    }
}

/// An entry in the workspace's audit log for what fctl did for an
/// autopilot, which nobody asked for then.
async fn audit(
    db: &D1Database,
    workspace_id: &str,
    action: &str,
    target: &str,
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
                s(target),
                s(&data.to_string()),
                os(Some(workspace_id).filter(|w| !w.is_empty())),
            ],
        )?
        .run()
        .await
    }
    .await;
    if let Err(e) = written {
        worker::console_error!("audit for an autopilot: {}", e.0.message);
    }
}

// ───────────────────────────── its owner ─────────────────────────────

/// The person an autopilot runs for, and their Claude connection for its
/// workspace.
pub struct Owner {
    pub user: UserRow,
    pub credential: CredentialRow,
}

/// Its owner, or why it can't run on their subscription and act as them:
/// their account is gone or disabled, they can no longer manage its home
/// project (or, for a workspace's autopilot, its workspace), or they have
/// no Claude connection.
pub async fn owner(
    db: &D1Database,
    project: &ProjectRow,
    row: &AutopilotRow,
) -> AppResult<Result<Owner, String>> {
    let Some(id) = row.owner_id.as_deref() else {
        return Ok(Err(
            "The account it ran for is gone: someone who manages the project takes it over.".into(),
        ));
    };
    let name = row.owner_name.clone().unwrap_or_else(|| "its owner".into());
    let user: Option<UserRow> = q(
        db,
        &format!("SELECT {} FROM users u WHERE u.id = ?", user_columns("u")),
        &[s(id)],
    )?
    .first()
    .await?;
    let Some(user) = user.filter(UserRow::is_active) else {
        return Ok(Err(format!(
            "It acts as {name}, whose account is gone or disabled: someone who manages the project takes it over."
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
    if !fctl_domain::perm::can_manage_project(role) {
        return Ok(Err(format!(
            "It acts as {name}, who no longer manages {}: someone who does takes it over.",
            project.name
        )));
    }
    if row.scope() == AutopilotScope::Workspace {
        let admin = crate::workspaces::member(db, &row.workspace_id, &user.id)
            .await?
            .and_then(|m| WorkspaceRole::parse(&m.role))
            .is_some_and(|r| matches!(r, WorkspaceRole::Owner | WorkspaceRole::Admin));
        if !admin {
            return Ok(Err(format!(
                "It runs the workspace as {name}, who is no longer one of its admins: an admin takes it over."
            )));
        }
    }
    let credential =
        crate::api::sessions::credential_for(db, &user, project.workspace_id()).await?;
    let Some(credential) = credential else {
        return Ok(Err(format!(
            "It runs on {name}'s Claude subscription, and {name} has no Claude connection."
        )));
    };
    Ok(Ok(Owner { user, credential }))
}

/// Why it wouldn't wake now, while it is on, if anything: what pauses it
/// at its next wake, or its owner's plan at its limit.
pub async fn problem(
    db: &D1Database,
    project: &ProjectRow,
    row: &AutopilotRow,
    now: i64,
) -> AppResult<Option<String>> {
    if row.state() != AutopilotState::On {
        return Ok(None);
    }
    if project.archived_at.is_some() {
        return Ok(Some(
            "Its project is archived, so it pauses when it next wakes.".into(),
        ));
    }
    if !workspace_allows(db, &row.workspace_id).await? {
        return Ok(Some(
            "Its workspace's admins turned autopilots off, so it pauses when it next wakes.".into(),
        ));
    }
    Ok(match owner(db, project, row).await? {
        Err(why) => Some(format!("{why} It pauses when it next wakes.")),
        Ok(o) => {
            let limits = crate::schedules::usage_of(db, &o.credential.id).await?;
            fctl_domain::schedules::at_limit(&limits, now).map(|limit| {
                let zone = fctl_domain::cron::Utc;
                format!(
                    "{} It wakes once the limit resets.",
                    fctl_domain::schedules::limit_reason(&o.user.name, limit, &zone, true)
                )
            })
        }
    })
}

/// Whether the workspace lets its projects have autopilots.
pub async fn workspace_allows(db: &D1Database, workspace_id: &str) -> AppResult<bool> {
    #[derive(Deserialize)]
    struct Row {
        #[serde(deserialize_with = "int_bool", default)]
        autopilots: bool,
    }
    let row: Option<Row> = q(
        db,
        "SELECT autopilots FROM workspaces WHERE id = ?",
        &[s(workspace_id)],
    )?
    .first()
    .await?;
    Ok(row.is_some_and(|r| r.autopilots))
}

// ───────────────────────────── waking ─────────────────────────────

/// The ScheduleObject, whose alarm wakes autopilots as it starts schedules.
async fn ring(env: &Env) {
    crate::schedules::wake(env).await;
}

/// Wakes the autopilot of `project_id` at `at` at the latest, for `reason`,
/// while it is on: what wakes it sooner stays.
pub async fn nudge(
    env: &Env,
    db: &D1Database,
    project_id: &str,
    reason: AutopilotWakeReason,
    at: i64,
) -> AppResult<()> {
    let moved = q(
        db,
        "UPDATE autopilots SET wake_at = CASE WHEN wake_at IS NULL OR wake_at > ?2 THEN ?2 ELSE wake_at END, \
         wake_reasons = CASE WHEN instr(',' || wake_reasons || ',', ',' || ?3 || ',') > 0 THEN wake_reasons \
           WHEN wake_reasons = '' THEN ?3 ELSE wake_reasons || ',' || ?3 END \
         WHERE project_id = ?1 AND state = 'on'",
        &[s(project_id), n(at), s(reason.as_str())],
    )?
    .run()
    .await?;
    if moved > 0 {
        ring(env).await;
    }
    Ok(())
}

/// When the earliest autopilot that is on wakes.
pub async fn earliest(db: &D1Database) -> AppResult<Option<i64>> {
    #[derive(Deserialize)]
    struct Row {
        #[serde(deserialize_with = "opt_int", default)]
        at: Option<i64>,
    }
    let row: Option<Row> = q(
        db,
        "SELECT MIN(wake_at) AS at FROM autopilots WHERE state = 'on' AND wake_at IS NOT NULL",
        &[],
    )?
    .first()
    .await?;
    Ok(row.and_then(|r| r.at))
}

/// Wakes the autopilots due at `now`, the longest due first: returns how
/// many it woke. One that fails is logged, and the others go on.
pub async fn run_due(env: &Env, now: i64) -> AppResult<usize> {
    let db = env.d1("DB")?;
    let due: Vec<AutopilotRow> = q(
        &db,
        &format!(
            "{AUTOPILOT_SELECT} WHERE a.state = 'on' AND a.wake_at IS NOT NULL AND a.wake_at <= ? \
             ORDER BY a.wake_at LIMIT ?"
        ),
        &[n(now), n(DUE_PER_ALARM)],
    )?
    .all()
    .await?;
    let mut woke = 0;
    for row in &due {
        match take_up(env, &db, row, now).await {
            Ok(true) => woke += 1,
            Ok(false) => {}
            Err(e) => worker::console_error!("autopilot {}: {}", row.project_id, e.0.message),
        }
    }
    Ok(woke)
}

/// Pauses an autopilot fctl can't run, or that paused itself, saying why;
/// `by` the person who did.
pub async fn pause(
    db: &D1Database,
    project_id: &str,
    why: Option<&str>,
    by: Option<&str>,
    now: i64,
) -> AppResult<bool> {
    let paused = q(
        db,
        "UPDATE autopilots SET state = 'paused', paused_reason = ?, paused_by = ?, paused_at = ?, wake_at = NULL, \
         wake_reasons = '', asked_wake_at = NULL, updated_at = ? WHERE project_id = ? AND state = 'on'",
        &[os(why), os(by), n(now), n(now), s(project_id)],
    )?
    .run()
    .await?;
    Ok(paused > 0)
}

/// Pauses the autopilots of a workspace whose admins turned them off,
/// saying so in each one's log.
pub async fn pause_workspace(
    db: &D1Database,
    workspace_id: &str,
    by: &str,
    now: i64,
) -> AppResult<()> {
    #[derive(Deserialize)]
    struct Id {
        project_id: String,
    }
    let on: Vec<Id> = q(
        db,
        "SELECT project_id FROM autopilots WHERE workspace_id = ? AND state = 'on'",
        &[s(workspace_id)],
    )?
    .all()
    .await?;
    let why = "Its workspace's admins turned autopilots off.";
    for a in on {
        if pause(db, &a.project_id, Some(why), Some(by), now).await? {
            log(
                db,
                &a.project_id,
                Entry::new(AutopilotEntryKind::State, format!("Paused: {why}")).by(by),
                now,
            )
            .await;
        }
    }
    Ok(())
}

/// Wakes one autopilot that is due: claims the wake (moving it to its next
/// check-in), then gives its session what is new, or starts it a fresh one.
/// `false` when another alarm took it, or it changed meanwhile.
async fn take_up(env: &Env, db: &D1Database, row: &AutopilotRow, now: i64) -> AppResult<bool> {
    let Some(due) = row.wake_at else {
        return Ok(false);
    };
    let next = ap::next_check_in(now, row.check_in());
    // Only as it was read: what moved it meanwhile waits for the next alarm.
    let took = q(
        db,
        "UPDATE autopilots SET wake_at = ?, wake_reasons = '', asked_wake_at = NULL WHERE project_id = ? \
         AND state = 'on' AND wake_at = ? AND wake_reasons = ?",
        &[n(next), s(&row.project_id), n(due), s(&row.wake_reasons)],
    )?
    .run()
    .await?;
    if took == 0 {
        return Ok(false);
    }
    let mut reasons = row.reasons();
    if reasons.is_empty() {
        reasons.push(row.wake_reason().unwrap_or(AutopilotWakeReason::CheckIn));
    }
    let Some(project) = crate::spec::project_row(db, &row.project_id).await? else {
        return Ok(false);
    };
    if project.archived_at.is_some() {
        stop_for(db, row, "Its project is archived.", now).await?;
        return Ok(true);
    }
    if !workspace_allows(db, &row.workspace_id).await? {
        stop_for(
            db,
            row,
            "Its workspace's admins turned autopilots off.",
            now,
        )
        .await?;
        return Ok(true);
    }
    let owner = match owner(db, &project, row).await? {
        Ok(o) => o,
        Err(why) => {
            stop_for(db, row, &why, now).await?;
            return Ok(true);
        }
    };
    // Its owner's plan at its limit: it wakes once the limit resets, with
    // what it would have woken for.
    let limits = crate::schedules::usage_of(db, &owner.credential.id).await?;
    if let Some(limit) = fctl_domain::schedules::at_limit(&limits, now) {
        let resets = limit
            .resets_at
            .filter(|r| *r > now)
            .map_or(now + 3_600_000, |r| r.min(now + 8 * 86_400_000))
            + 60_000;
        q(
            db,
            "UPDATE autopilots SET wake_at = ?, wake_reasons = ? WHERE project_id = ? AND state = 'on'",
            &[
                n(resets),
                s(&reasons.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(",")),
                s(&row.project_id),
            ],
        )?
        .run()
        .await?;
        return Ok(true);
    }
    let news = News::gather(db, row, now).await?;
    let status = row.session_status();
    let working = status.is_some_and(ap::at_work);
    // At work already: a check-in can wait; news reaches it at its next
    // step.
    let only_time = reasons
        .iter()
        .all(|r| matches!(r, AutopilotWakeReason::CheckIn | AutopilotWakeReason::Asked));
    if working && only_time && news.is_empty() {
        return Ok(true);
    }
    let wake = news.wake(&reasons);
    let text = news.message(&reasons, now);
    let carries_on = match (row.session_id.as_deref(), status, row.session_created_at) {
        (Some(_), Some(st), Some(started)) => {
            ap::carries_on(st, started, now) && (st.accepts_messages() || st.resumes_for_messages())
        }
        _ => false,
    };
    let session = match (carries_on, row.session_id.as_deref()) {
        (true, Some(id)) => {
            deliver(env, id, &text, wake.clone(), now).await?;
            id.to_owned()
        }
        _ => {
            let briefing = briefing(db, row, &project, &owner.user).await?;
            let first = format!("{briefing}\n---\n\n{text}");
            let (id, _) =
                start_pilot(env, db, row, &project, &owner, first, wake.clone(), now).await?;
            id.0
        }
    };
    news.mark_seen(db, row, now).await?;
    q(
        db,
        "UPDATE autopilots SET woken_at = ?, check_ins = check_ins + 1, direction_new = 0, updated_at = ? WHERE project_id = ?",
        &[n(now), n(now), s(&row.project_id)],
    )?
    .run()
    .await?;
    log(
        db,
        &row.project_id,
        Entry::new(AutopilotEntryKind::Woke, woke_line(&reasons, &wake)).about(Some(&session)),
        now,
    )
    .await;
    Ok(true)
}

/// What a wake says in the log: "Woke for its check-in", "Woke: Ada's
/// note, shop#41".
fn woke_line(reasons: &[AutopilotWakeReason], wake: &AutopilotWake) -> String {
    let mut parts: Vec<String> = Vec::new();
    for r in reasons {
        parts.push(match r {
            AutopilotWakeReason::Started => "it was put on".into(),
            AutopilotWakeReason::Resumed => "it was resumed".into(),
            AutopilotWakeReason::CheckIn => "its check-in".into(),
            AutopilotWakeReason::Asked => "the time it asked for".into(),
            AutopilotWakeReason::Woken => "someone woke it".into(),
            AutopilotWakeReason::Steered => match wake.notes {
                0 if wake.direction => "a new direction".into(),
                0 => "people steered it".into(),
                1 => "a new note".into(),
                n => format!("{n} new notes"),
            },
            AutopilotWakeReason::Answered => match wake.answers {
                0 | 1 => "an answer".into(),
                n => format!("{n} answers"),
            },
            AutopilotWakeReason::Crew => match wake.crew.len() {
                0 => "its sessions".into(),
                1 => wake.crew.first().map_or_else(
                    || "its sessions".into(),
                    |c| format!("session #{}", c.number),
                ),
                n => format!("{n} of its sessions"),
            },
            AutopilotWakeReason::Retry => "a retry after its session failed".into(),
            AutopilotWakeReason::Other => continue,
        });
    }
    match parts.as_slice() {
        [] => "Woke".into(),
        // "Woke as it was put on", alone; "being put on" among others.
        [one] if one.starts_with("it was ") => format!("Woke as {one}"),
        [one] => format!("Woke for {one}"),
        _ => format!(
            "Woke for {}",
            parts
                .iter()
                .map(|p| p
                    .strip_prefix("it was ")
                    .map_or_else(|| p.clone(), |p| format!("being {p}")))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Pauses an autopilot fctl can no longer run, saying why, in its log and
/// the workspace's audit log.
async fn stop_for(db: &D1Database, row: &AutopilotRow, why: &str, now: i64) -> AppResult<()> {
    if pause(db, &row.project_id, Some(why), None, now).await? {
        log(
            db,
            &row.project_id,
            Entry::new(AutopilotEntryKind::State, format!("fctl paused it: {why}")),
            now,
        )
        .await;
        audit(
            db,
            &row.workspace_id,
            "autopilot.paused",
            &row.project_id,
            json!({ "reason": why }),
            now,
        )
        .await;
    }
    Ok(())
}

/// Hands its session a message from fctl: it takes it at its next step, or
/// resumes for it.
async fn deliver(
    env: &Env,
    session_id: &str,
    text: &str,
    wake: AutopilotWake,
    now: i64,
) -> AppResult<()> {
    let delivery_id = fctl_domain::ids::delivery_id(now);
    let stub = crate::internal::session_stub(env, session_id)?;
    let _: crate::internal::Delivered = crate::internal::post(
        &stub,
        "/deliver",
        &crate::internal::Deliver {
            delivery: Delivery {
                id: delivery_id.clone(),
                kind: DeliveryKind::Message {
                    text: text.to_owned(),
                    author: None,
                    author_called: None,
                    attachments: Vec::new(),
                },
            },
            client_key: None,
            event: EventBody::UserMessage(UserMessage {
                id: delivery_id,
                author: None,
                text: text.to_owned(),
                source: MessageSource::System,
                attachments: Vec::new(),
                follow_up: None,
                review: None,
                ticket: None,
                autopilot: None,
                wake: Some(wake),
            }),
        },
    )
    .await?;
    Ok(())
}

/// Starts an autopilot's own session, briefed, and makes it the one fctl
/// wakes it in.
#[allow(clippy::too_many_arguments)]
async fn start_pilot(
    env: &Env,
    db: &D1Database,
    row: &AutopilotRow,
    project: &ProjectRow,
    owner: &Owner,
    prompt: String,
    wake: AutopilotWake,
    now: i64,
) -> AppResult<(SessionId, u32)> {
    let started = crate::starts::start(
        env,
        db,
        crate::starts::NewSession {
            project,
            title: format!("{} autopilot", project.name),
            branch: crate::starts::Branch::Titled,
            base_branch: project.default_branch.clone(),
            model: row.model.clone(),
            effort: row.effort.clone(),
            user: &owner.user,
            credential: &owner.credential,
            github_access: project.settings().github_access.unwrap_or_default(),
            prompt,
            author: None,
            source: MessageSource::System,
            ticket_message: None,
            schedule_id: None,
            ticket: None,
            autopilot: Some(crate::starts::AutopilotStart {
                project_id: &row.project_id,
                role: AutopilotRole::Pilot,
                wake: Some(wake),
                author: None,
            }),
            follow: true,
            now,
        },
    )
    .await?;
    q(
        db,
        "UPDATE autopilots SET session_id = ?, updated_at = ? WHERE project_id = ?",
        &[s(started.0.as_str()), n(now), s(&row.project_id)],
    )?
    .run()
    .await?;
    audit(
        db,
        &row.workspace_id,
        "session.created",
        started.0.as_str(),
        json!({
            "project": project.slug,
            "number": started.1,
            "autopilot": row.project_id,
            "for": owner.user.id,
        }),
        now,
    )
    .await;
    Ok(started)
}

/// The briefing its own sessions start with.
async fn briefing(
    db: &D1Database,
    row: &AutopilotRow,
    project: &ProjectRow,
    owner: &UserRow,
) -> AppResult<String> {
    let called = owner.called().unwrap_or_else(|| owner.name.clone());
    let workspace = project.workspace.workspace_name.clone().unwrap_or_default();
    let powers = row.powers();
    let _ = db;
    Ok(ap::briefing(
        ap::Place {
            project: &project.name,
            project_slug: &project.slug,
            workspace: &workspace,
            scope: row.scope(),
            owner: &called,
        },
        ap::Rules {
            powers: &powers,
            crew_max: row.crew_max(),
            check_in_minutes: row.check_in(),
        },
        &row.direction,
    ))
}

// ───────────────────────────── news ─────────────────────────────

#[derive(Deserialize)]
struct NoteRow {
    text: String,
    author_name: Option<String>,
    #[serde(deserialize_with = "int")]
    created_at: i64,
    #[serde(deserialize_with = "opt_int", default)]
    seen_at: Option<i64>,
}

#[derive(Clone, Deserialize)]
pub struct AskRow {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub detail: Option<String>,
    pub options: Option<String>,
    pub secret_name: Option<String>,
    pub secret_project_id: Option<String>,
    pub secret_project_slug: Option<String>,
    pub status: String,
    pub answer: Option<String>,
    pub answered_by: Option<String>,
    pub answered_by_name: Option<String>,
    pub answered_by_avatar: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub answered_at: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    pub seen_at: Option<i64>,
    pub session_id: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub session_number: Option<i64>,
    pub session_title: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
}

pub const ASK_SELECT: &str = "SELECT k.id, k.kind, k.text, k.detail, k.options, k.secret_name, k.secret_project_id, \
    sp.slug AS secret_project_slug, k.status, k.answer, k.answered_by, au.name AS answered_by_name, \
    au.avatar_url AS answered_by_avatar, k.answered_at, k.seen_at, k.session_id, ks.number AS session_number, \
    ks.title AS session_title, k.created_at \
    FROM autopilot_asks k \
    LEFT JOIN projects sp ON sp.id = k.secret_project_id \
    LEFT JOIN users au ON au.id = k.answered_by \
    LEFT JOIN sessions ks ON ks.id = k.session_id";

impl AskRow {
    pub fn options(&self) -> Vec<String> {
        self.options
            .as_deref()
            .and_then(|o| serde_json::from_str(o).ok())
            .unwrap_or_default()
    }

    pub fn kind(&self) -> fctl_proto::autopilot::AutopilotAskKind {
        match self.kind.as_str() {
            "question" => fctl_proto::autopilot::AutopilotAskKind::Question,
            "secret" => fctl_proto::autopilot::AutopilotAskKind::Secret,
            _ => fctl_proto::autopilot::AutopilotAskKind::Other,
        }
    }

    pub fn status(&self) -> fctl_proto::autopilot::AutopilotAskStatus {
        fctl_proto::autopilot::AutopilotAskStatus::parse(&self.status)
    }
}

#[derive(Deserialize)]
pub struct CrewRow {
    pub id: String,
    pub project_id: String,
    pub project_slug: String,
    #[serde(deserialize_with = "int")]
    pub number: i64,
    pub title: String,
    pub status: String,
    pub status_reason: Option<String>,
    pub question: Option<String>,
    pub pull_requests: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "opt_int", default)]
    pub last_activity_at: Option<i64>,
}

pub const CREW_SELECT: &str = "SELECT s.id, s.project_id, p.slug AS project_slug, s.number, s.title, s.status, \
    s.status_reason, s.question, s.pull_requests, s.created_at, s.last_activity_at \
    FROM sessions s JOIN projects p ON p.id = s.project_id";

impl CrewRow {
    pub fn status(&self) -> SessionStatus {
        SessionStatus::parse(&self.status).unwrap_or(SessionStatus::Failed)
    }

    pub fn question_text(&self) -> Option<String> {
        self.question
            .as_deref()
            .and_then(|q| serde_json::from_str::<fctl_proto::session::SessionQuestion>(q).ok())
            .map(|q| q.text)
    }

    pub fn pull_requests(&self) -> Vec<fctl_proto::session::PullRequestRef> {
        self.pull_requests
            .as_deref()
            .and_then(|p| serde_json::from_str(p).ok())
            .unwrap_or_default()
    }

    pub fn session_ref(&self) -> SessionRef {
        SessionRef {
            id: SessionId(self.id.clone()),
            number: u32::try_from(self.number).unwrap_or(0),
            title: self.title.clone(),
        }
    }

    /// "#41 merged".
    pub fn pr_lines(&self) -> Vec<String> {
        self.pull_requests()
            .iter()
            .map(|p| match &p.state {
                Some(state) => format!("#{} {state}", p.number),
                None => format!("#{}", p.number),
            })
            .collect()
    }
}

/// What is new for an autopilot since it last woke.
struct News {
    notes: Vec<NoteRow>,
    direction: Option<(String, Option<String>)>,
    answers: Vec<AskRow>,
    crew: Vec<CrewRow>,
}

impl News {
    async fn gather(db: &D1Database, row: &AutopilotRow, now: i64) -> AppResult<Self> {
        let notes: Vec<NoteRow> = q(
            db,
            "SELECT n.text, u.name AS author_name, n.created_at, n.seen_at FROM autopilot_notes n \
             LEFT JOIN users u ON u.id = n.author_id WHERE n.project_id = ? AND n.seen_at IS NULL ORDER BY n.created_at",
            &[s(&row.project_id)],
        )?
        .all()
        .await?;
        let answers: Vec<AskRow> = q(
            db,
            &format!(
                "{ASK_SELECT} WHERE k.project_id = ? AND k.status IN ('answered', 'declined') AND k.seen_at IS NULL \
                 ORDER BY k.answered_at"
            ),
            &[s(&row.project_id)],
        )?
        .all()
        .await?;
        // Its sessions that moved on since it last woke: those that wait,
        // ask, stopped or failed now.
        let since = row.woken_at.unwrap_or(row.created_at);
        let crew: Vec<CrewRow> = q(
            db,
            &format!(
                "{CREW_SELECT} WHERE s.autopilot_id = ? AND s.autopilot_role = 'crew' AND s.updated_at > ? \
                 AND s.status IN ('idle', 'stopped', 'failed') ORDER BY s.updated_at DESC LIMIT 10"
            ),
            &[s(&row.project_id), n(since)],
        )?
        .all()
        .await?;
        let _ = now;
        Ok(Self {
            notes,
            direction: row
                .direction_new
                .then(|| (row.direction.clone(), row.direction_by_name.clone())),
            answers,
            crew,
        })
    }

    fn is_empty(&self) -> bool {
        self.notes.is_empty()
            && self.direction.is_none()
            && self.answers.is_empty()
            && self.crew.is_empty()
    }

    fn wake(&self, reasons: &[AutopilotWakeReason]) -> AutopilotWake {
        AutopilotWake {
            reasons: reasons.to_vec(),
            notes: u32::try_from(self.notes.len()).unwrap_or(0),
            answers: u32::try_from(self.answers.len()).unwrap_or(0),
            crew: self.crew.iter().map(CrewRow::session_ref).collect(),
            direction: self.direction.is_some(),
        }
    }

    fn message(&self, reasons: &[AutopilotWakeReason], now: i64) -> String {
        let notes: Vec<ap::NoteView> = self
            .notes
            .iter()
            .map(|n| ap::NoteView {
                text: &n.text,
                author: n.author_name.as_deref().unwrap_or("Someone"),
                at: n.created_at,
                new: n.seen_at.is_none(),
            })
            .collect();
        let options: Vec<Vec<String>> = self.answers.iter().map(AskRow::options).collect();
        let answers: Vec<ap::AskView> = self
            .answers
            .iter()
            .zip(options.iter())
            .map(|(a, options)| ask_view(a, options))
            .collect();
        let crew: Vec<ap::CrewView> = self.crew.iter().map(crew_view).collect();
        let questions: Vec<Option<String>> = self.crew.iter().map(CrewRow::question_text).collect();
        let crew: Vec<ap::CrewView> = crew
            .into_iter()
            .zip(questions.iter())
            .map(|(mut c, q)| {
                c.question = q.as_deref();
                c
            })
            .collect();
        ap::wake_message(&ap::News {
            reasons: reasons.to_vec(),
            notes,
            direction: self
                .direction
                .as_ref()
                .map(|(d, by)| (d.as_str(), by.as_deref())),
            answers,
            crew,
            now,
        })
    }

    /// It has heard of these now.
    async fn mark_seen(&self, db: &D1Database, row: &AutopilotRow, now: i64) -> AppResult<()> {
        mark_seen(db, &row.project_id, now).await
    }
}

/// Marks what people told it seen: its notes, and the answers to its asks.
pub async fn mark_seen(db: &D1Database, project_id: &str, now: i64) -> AppResult<()> {
    q(
        db,
        "UPDATE autopilot_notes SET seen_at = ? WHERE project_id = ? AND seen_at IS NULL",
        &[n(now), s(project_id)],
    )?
    .run()
    .await?;
    q(
        db,
        "UPDATE autopilot_asks SET seen_at = ? WHERE project_id = ? AND seen_at IS NULL AND status != 'open'",
        &[n(now), s(project_id)],
    )?
    .run()
    .await?;
    Ok(())
}

pub fn ask_view<'a>(a: &'a AskRow, options: &'a [String]) -> ap::AskView<'a> {
    ap::AskView {
        id: &a.id,
        kind: a.kind(),
        text: &a.text,
        options,
        secret: a.secret_name.as_deref().map(|name| {
            (
                name,
                a.secret_project_slug.as_deref().unwrap_or("its project"),
            )
        }),
        status: a.status(),
        answer: a.answer.as_deref(),
        by: a.answered_by_name.as_deref(),
        at: a.answered_at.unwrap_or(a.created_at),
        new: a.seen_at.is_none(),
    }
}

pub fn crew_view(c: &CrewRow) -> ap::CrewView<'_> {
    ap::CrewView {
        reference: format!("{}#{}", c.project_slug, c.number),
        title: &c.title,
        status: c.status(),
        status_reason: c.status_reason.as_deref(),
        question: None,
        pull_requests: c.pr_lines(),
    }
}

// ───────────────────────────── its sessions ─────────────────────────────

/// What a session that moved means to its autopilot (from SessionObject):
/// its own session's turn ended (it checks in next at its time, or the
/// time it asked for) or failed (fctl tries again, then pauses it); one of
/// those it started ended a turn, asks something or failed (fctl wakes it
/// soon).
pub async fn session_moved(
    env: &Env,
    autopilot: &str,
    role: AutopilotRole,
    session_id: &str,
    status: SessionStatus,
    reason: Option<&str>,
    now: i64,
) -> AppResult<()> {
    let db = env.d1("DB")?;
    match role {
        AutopilotRole::Crew => {
            if matches!(status, SessionStatus::Idle | SessionStatus::Failed) {
                nudge(
                    env,
                    &db,
                    autopilot,
                    AutopilotWakeReason::Crew,
                    now + ap::CREW_GATHER_MS,
                )
                .await?;
            }
        }
        AutopilotRole::Pilot => {
            let Some(row) = row(&db, autopilot).await? else {
                return Ok(());
            };
            // An earlier session of its own says nothing any more.
            if row.session_id.as_deref() != Some(session_id) || row.state() != AutopilotState::On {
                return Ok(());
            }
            match status {
                SessionStatus::Idle => {
                    // Its turn ended: its check-in counts from now, unless it
                    // asked for a time, or something waits to wake it.
                    q(
                        &db,
                        "UPDATE autopilots SET failures = 0, wake_at = CASE \
                           WHEN wake_reasons != '' THEN wake_at \
                           WHEN asked_wake_at IS NOT NULL THEN asked_wake_at \
                           ELSE ? END, updated_at = ? WHERE project_id = ? AND state = 'on'",
                        &[
                            n(ap::next_check_in(now, row.check_in())),
                            n(now),
                            s(autopilot),
                        ],
                    )?
                    .run()
                    .await?;
                    ring(env).await;
                }
                SessionStatus::Failed => {
                    let failures = u32::try_from(row.failures + 1).unwrap_or(u32::MAX);
                    q(
                        &db,
                        "UPDATE autopilots SET failures = ?, updated_at = ? WHERE project_id = ?",
                        &[n(i64::from(failures)), n(now), s(autopilot)],
                    )?
                    .run()
                    .await?;
                    let why = reason.unwrap_or("it failed");
                    log(
                        &db,
                        autopilot,
                        Entry::new(
                            AutopilotEntryKind::Problem,
                            format!("Its session failed: {why}"),
                        )
                        .about(Some(session_id)),
                        now,
                    )
                    .await;
                    match ap::retry_in(failures) {
                        Some(after) => {
                            nudge(env, &db, autopilot, AutopilotWakeReason::Retry, now + after)
                                .await?;
                        }
                        None => {
                            let why = format!(
                                "Its last {} sessions failed, the last because {why}. Resume it once that's fixed.",
                                ap::RETRY_MS.len()
                            );
                            stop_for(&db, &row, &why, now).await?;
                        }
                    }
                }
                _ => {}
            }
        }
        AutopilotRole::Other => {}
    }
    Ok(())
}

/// Whether an autopilot may start another session now: fewer than its
/// people allow work at once, and fewer than [`ap::CREW_PER_HOUR`] started
/// in the last hour.
pub async fn crew_room(db: &D1Database, caller: &AutopilotCaller, now: i64) -> AppResult<()> {
    let Some(row) = row(db, &caller.project_id).await? else {
        return Err(forbidden("This autopilot is gone."));
    };
    #[derive(Deserialize)]
    struct Counts {
        #[serde(deserialize_with = "int", default)]
        working: i64,
        #[serde(deserialize_with = "int", default)]
        lately: i64,
    }
    let counts: Option<Counts> = q(
        db,
        "SELECT \
           (SELECT COUNT(*) FROM sessions WHERE autopilot_id = ?1 AND autopilot_role = 'crew' \
             AND status IN ('queued','provisioning','preparing','running','paused','stopping')) AS working, \
           (SELECT COUNT(*) FROM sessions WHERE autopilot_id = ?1 AND autopilot_role = 'crew' AND created_at > ?2) AS lately",
        &[s(&caller.project_id), n(now - 3_600_000)],
    )?
    .first()
    .await?;
    let counts = counts.unwrap_or(Counts {
        working: 0,
        lately: 0,
    });
    if counts.working >= row.crew_max {
        return Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::Conflict,
            format!(
                "{} of this autopilot's sessions work already, as many as its people allow at once: wait for one to end its turn, or stop one (fctl stop).",
                counts.working
            ),
        )));
    }
    if counts.lately >= i64::from(ap::CREW_PER_HOUR) {
        return Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::RateLimited,
            format!(
                "This autopilot started {} sessions in the last hour, as many as an autopilot may: start the next later.",
                counts.lately
            ),
        )));
    }
    Ok(())
}

/// A session's task as its agent gets it when an autopilot starts it: the
/// task, then fctl's note of who started it and how to reach them.
pub async fn crew_prompt(
    db: &D1Database,
    caller: &AutopilotCaller,
    task: &str,
    owner_called: &str,
) -> AppResult<String> {
    let name: Option<NameRow> = q(
        db,
        "SELECT name FROM projects WHERE id = ?",
        &[s(&caller.project_id)],
    )?
    .first()
    .await?;
    let project = name.map_or_else(|| caller.project_slug.clone(), |n| n.name);
    let what = match caller.scope {
        AutopilotScope::Project => "the project",
        AutopilotScope::Workspace => "its workspace's projects",
    };
    Ok(format!(
        "{}\n\n---\nThe autopilot of {project} started this session: an agent that runs {what} on its own (an experimental feature of fctl), for {owner_called}, as whom it acts, so its messages come to you as {owner_called}'s. It follows this session and hears when your turn ends. When you're done, or need a decision to go on, end your turn saying so plainly; ask what you need answered with `fctl ask`, and the autopilot answers, or asks people.",
        task.trim_end()
    ))
}

#[derive(Deserialize)]
struct NameRow {
    name: String,
}

// ───────────────────────────── requests from its sandbox ─────────────────────────────

/// The autopilot whose own session `session_id` is now.
pub async fn by_pilot(db: &D1Database, session_id: &str) -> AppResult<Option<AutopilotRow>> {
    q(
        db,
        &format!("{AUTOPILOT_SELECT} WHERE a.session_id = ?"),
        &[s(session_id)],
    )?
    .first()
    .await
}

/// `AgentCall::Api`: a request to the API from an autopilot's own session,
/// answered as its owner within what the autopilot may do, and logged when
/// it changed something.
pub async fn relay(
    ctx: &mut Ctx,
    session_id: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> AppResult<AgentAnswer> {
    let refuse = |m: &str| Err(forbidden(m));
    let Some(row) = by_pilot(&ctx.db, session_id).await? else {
        return refuse(
            "Only an autopilot's own session reaches the API from its sandbox: this one isn't, or isn't its current one any more.",
        );
    };
    match row.state() {
        AutopilotState::On => {}
        AutopilotState::Paused => {
            // It may still say what it did, and when to wake: once resumed.
            return refuse("This autopilot is paused: it acts again once someone resumes it.");
        }
        AutopilotState::Off => return refuse("This project is no longer on autopilot."),
    }
    let method = method.to_ascii_uppercase();
    let worker_method = match method.as_str() {
        "GET" => worker::Method::Get,
        "POST" => worker::Method::Post,
        "PUT" => worker::Method::Put,
        "PATCH" => worker::Method::Patch,
        "DELETE" => worker::Method::Delete,
        _ => {
            return Err(AppError(ApiError::bad_request(
                "Use GET, POST, PUT, PATCH or DELETE",
            )));
        }
    };
    if !path.starts_with("/api/v1/")
        || path.len() > 2_000
        || path.contains("..")
        || path.contains("//")
    {
        return Err(AppError(ApiError::bad_request(
            "The path is one of the API's, like /api/v1/projects/my-app",
        )));
    }
    let Some(project) = crate::spec::project_row(&ctx.db, &row.project_id).await? else {
        return Err(not_found("Project"));
    };
    let owner = match owner(&ctx.db, &project, &row).await? {
        Ok(o) => o,
        Err(why) => return refuse(&why),
    };
    let scope = row.scope();
    let caller = AutopilotCaller {
        project_id: row.project_id.clone(),
        project_slug: project.slug.clone(),
        workspace_id: row.workspace_id.clone(),
        scope,
        powers: row.powers(),
        session_id: session_id.to_owned(),
    };
    let places = match scope {
        AutopilotScope::Project => Places {
            workspaces: Vec::new(),
            projects: vec![row.project_id.clone()],
        },
        AutopilotScope::Workspace => Places {
            workspaces: vec![row.workspace_id.clone()],
            projects: Vec::new(),
        },
    };
    let settings = crate::instance::settings(&ctx.db).await?;
    let authed = Authed {
        user: owner.user.clone(),
        token_id: format!("autopilot:{}", row.project_id),
        kind: TokenKind::Pat,
        via_cookie: false,
        signed_in_at: row.created_at,
        verified_at: None,
        two_factor_setup_required: false,
        settings,
        limits: Some(Limits {
            access: TokenAccess::Full,
            places: Some(places),
            github_access: None,
            most: Some(ap::ROLE_CAP),
        }),
        autopilot: Some(caller),
    };
    let (route_path, _) = path.split_once('?').unwrap_or((path, ""));
    let route_path = route_path.to_owned();
    // A project it creates without naming a workspace goes into its own,
    // rather than the owner's only one.
    let mut body = body;
    if method == "POST"
        && route_path.trim_end_matches('/') == "/api/v1/projects"
        && let Some(Value::Object(fields)) = body.as_mut()
        && fields.get("workspace").is_none_or(Value::is_null)
    {
        fields.insert(
            "workspace".to_owned(),
            Value::String(row.workspace_id.clone()),
        );
    }
    let raw = match &body {
        Some(b) => Some(serde_json::to_string(b)?),
        None => None,
    };
    let headers = worker::Headers::new();
    headers.set("Content-Type", "application/json")?;
    headers.set("Accept", "application/json")?;
    let mut init = worker::RequestInit::new();
    init.with_method(worker_method).with_headers(headers);
    if let Some(raw) = &raw {
        init.with_body(Some(wasm_bindgen::JsValue::from_str(raw)));
    }
    let mut req = worker::Request::new_with_init(&ctx.cfg.url(path), &init)?;
    let saved = ctx.auth.replace(authed);
    let answered = crate::route_autopilot(ctx, &mut req, &route_path, body.as_ref()).await;
    ctx.auth = saved;
    let (status, answer) = match answered {
        Ok(mut response) => {
            let status = response.status_code();
            let text = response.text().await.unwrap_or_default();
            let value = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
            (status, value)
        }
        Err(e) => (
            e.0.code.http_status(),
            serde_json::to_value(&e.0).unwrap_or(Value::Null),
        ),
    };
    let segs: Vec<&str> = route_path
        .trim_end_matches('/')
        .split('/')
        .skip(3)
        .collect();
    let empty = Value::Null;
    let sent = body.as_ref().unwrap_or(&empty);
    // Only what changed something gets a line, and the lookups for it.
    if (200..300).contains(&status) && ap::describe(&method, &segs, sent, &answer, None).is_some() {
        let about = about(&ctx.db, &segs, &answer).await;
        // Another project it changed, by its slug, which the line names.
        let elsewhere = match about.1.as_deref() {
            Some(other) if other != row.project_id => slug_of(&ctx.db, other).await,
            _ => None,
        };
        if let Some((kind, text)) =
            ap::describe(&method, &segs, sent, &answer, elsewhere.as_deref())
        {
            // Its own transcript shows what it did, the session it started
            // or steered with it.
            transcript(
                ctx,
                session_id,
                kind,
                &text,
                about.0.as_deref(),
                about.1.as_deref(),
            )
            .await;
            let mut entry = Entry::new(kind, text);
            entry.session = about.0.as_deref();
            entry.other_project = about.1.as_deref();
            entry.from = Some(session_id);
            log(&ctx.db, &row.project_id, entry, ctx.now).await;
        }
    }
    Ok(AgentAnswer::Api {
        status,
        body: answer,
    })
}

/// Adds what the autopilot did to its own session's transcript, as an
/// `autopilot_action` notice with the session it concerns, which the apps
/// show as that session, live. A failure is logged: nothing hangs on it.
async fn transcript(
    ctx: &Ctx,
    pilot: &str,
    kind: AutopilotEntryKind,
    text: &str,
    session: Option<&str>,
    project: Option<&str>,
) {
    #[derive(Deserialize)]
    struct Found {
        id: String,
        #[serde(deserialize_with = "int")]
        number: i64,
        title: String,
        slug: String,
    }
    let posted: AppResult<Value> = async {
        let found: Option<Found> = match session {
            Some(id) => {
                q(
                    &ctx.db,
                    "SELECT s.id, s.number, s.title, p.slug FROM sessions s JOIN projects p ON p.id = s.project_id \
                     WHERE s.id = ?",
                    &[s(id)],
                )?
                .first()
                .await?
            }
            None => None,
        };
        #[derive(Deserialize)]
        struct Slug {
            slug: String,
        }
        let project_slug = match (&found, project) {
            (Some(f), _) => Some(f.slug.clone()),
            (None, Some(p)) => q(&ctx.db, "SELECT slug FROM projects WHERE id = ?", &[s(p)])?
                .first::<Slug>()
                .await?
                .map(|s| s.slug),
            (None, None) => None,
        };
        let mut notice = fctl_proto::event::Notice::new(
            fctl_proto::event::NoticeLevel::Info,
            "autopilot_action",
            match &found {
                Some(f) => format!("{text} {}#{}: {}", f.slug, f.number, f.title),
                None => text.to_owned(),
            },
        );
        notice.autopilot = Some(fctl_proto::autopilot::AutopilotNotice {
            kind,
            session: found.map(|f| SessionRef {
                id: SessionId(f.id),
                number: u32::try_from(f.number).unwrap_or(0),
                title: f.title,
            }),
            project_slug,
        });
        let stub = crate::internal::session_stub(&ctx.env, pilot)?;
        crate::internal::post(&stub, "/notice", &notice).await
    }
    .await;
    if let Err(e) = posted {
        worker::console_error!("an autopilot's notice: {}", e.0.message);
    }
}

/// The session and project a request the autopilot made concerns, for its
/// log: the session it started or steered, the project it changed.
async fn about(db: &D1Database, segs: &[&str], answer: &Value) -> (Option<String>, Option<String>) {
    match segs {
        ["projects", _, "sessions"] => (
            answer.get("id").and_then(Value::as_str).map(str::to_owned),
            answer
                .get("project_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ),
        ["sessions", id, ..] => {
            #[derive(Deserialize)]
            struct S {
                project_id: String,
            }
            let project = q(db, "SELECT project_id FROM sessions WHERE id = ?", &[s(id)]).ok();
            let project = match project {
                Some(query) => query
                    .first::<S>()
                    .await
                    .ok()
                    .flatten()
                    .map(|s| s.project_id),
                None => None,
            };
            (Some((*id).to_owned()), project)
        }
        ["projects"] => (
            None,
            answer.get("id").and_then(Value::as_str).map(str::to_owned),
        ),
        ["projects", key, ..] => (None, project_of(db, key).await),
        ["schedules", id, ..] => {
            #[derive(Deserialize)]
            struct S {
                project_id: String,
            }
            let project = match q(
                db,
                "SELECT project_id FROM schedules WHERE id = ?",
                &[s(id)],
            )
            .ok()
            {
                Some(query) => query
                    .first::<S>()
                    .await
                    .ok()
                    .flatten()
                    .map(|s| s.project_id),
                None => None,
            };
            (
                None,
                project.or_else(|| {
                    answer
                        .get("project_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                }),
            )
        }
        _ => (None, None),
    }
}

/// The id of the project `key` (an id or a slug), if there is one.
async fn project_of(db: &D1Database, key: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Found {
        id: String,
    }
    let query = q(
        db,
        "SELECT id FROM projects WHERE id = ?1 OR slug = ?1",
        &[s(key)],
    )
    .ok()?;
    query.first::<Found>().await.ok().flatten().map(|f| f.id)
}

/// A project's slug, by its id.
async fn slug_of(db: &D1Database, id: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Found {
        slug: String,
    }
    let query = q(db, "SELECT slug FROM projects WHERE id = ?", &[s(id)]).ok()?;
    query.first::<Found>().await.ok().flatten().map(|f| f.slug)
}

/// What a request from an autopilot still needs checked, once
/// `fctl_domain::autopilot::allows` lets it try: that the session it
/// steers is one it started, that it reaches only its own autopilot, and
/// that a project it creates is in its workspace.
pub async fn gate(ctx: &Ctx, method: &str, segs: &[&str], body: Option<&Value>) -> AppResult<()> {
    let Some(caller) = ctx.autopilot() else {
        return Err(forbidden("Only an autopilot's own session asks this way."));
    };
    let gate = ap::allows(
        ap::Caller {
            scope: caller.scope,
            powers: &caller.powers,
        },
        method,
        segs,
    )
    .map_err(AppError)?;
    match gate {
        Gate::Open => Ok(()),
        Gate::Home => {
            let key = segs.get(1).copied().unwrap_or_default();
            if key == caller.project_id || key == caller.project_slug {
                Ok(())
            } else {
                Err(forbidden(
                    "An autopilot reaches its own autopilot only: its home project's.",
                ))
            }
        }
        Gate::Crew(session) => {
            #[derive(Deserialize)]
            struct Found {
                #[allow(dead_code)]
                id: String,
            }
            let found: Option<Found> = q(
                &ctx.db,
                "SELECT id FROM sessions WHERE id = ? AND autopilot_id = ? AND autopilot_role = 'crew'",
                &[s(&session), s(&caller.project_id)],
            )?
            .first()
            .await?;
            if found.is_some() {
                Ok(())
            } else {
                Err(forbidden(
                    "An autopilot steers only the sessions it started: people steer theirs. Read this one with fctl show or fctl logs.",
                ))
            }
        }
        Gate::NewProject => {
            let asked = body
                .and_then(|b| b.get("workspace"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            #[derive(Deserialize)]
            struct Found {
                id: String,
            }
            let found: Option<Found> = q(
                &ctx.db,
                "SELECT id FROM workspaces WHERE id = ?1 OR slug = ?1",
                &[s(asked)],
            )?
            .first()
            .await?;
            if found.is_some_and(|f| f.id == caller.workspace_id) {
                Ok(())
            } else {
                Err(forbidden(
                    "A workspace's autopilot creates projects in its own workspace only: say which with --workspace.",
                ))
            }
        }
    }
}

// ───────────────────────────── for its session's spec ─────────────────────────────

/// What the spec of an autopilot's own session says of it (`autopilot`:
/// the session's `autopilot_id`, its home project), so that its sandbox's
/// `fctl` reaches the API: while it is on or paused, and only its current
/// session's requests get answers (`relay`).
pub async fn spec(db: &D1Database, autopilot: &str) -> AppResult<Option<AutopilotSpec>> {
    // By the session's own column: FleetObject may build its spec before
    // the autopilot records it as its session.
    let Some(row) = row(db, autopilot)
        .await?
        .filter(|r| r.state() != AutopilotState::Off)
    else {
        return Ok(None);
    };
    #[derive(Deserialize)]
    struct Slug {
        slug: String,
    }
    let slug: Option<Slug> = q(
        db,
        "SELECT slug FROM projects WHERE id = ?",
        &[s(&row.project_id)],
    )?
    .first()
    .await?;
    Ok(Some(AutopilotSpec {
        project_id: ProjectId(row.project_id.clone()),
        project_slug: slug.map(|s| s.slug).unwrap_or_default(),
        scope: row.scope(),
    }))
}

/// Notifies the people who follow the autopilot's own session that it
/// asks them something (`crate::notify`).
pub async fn notify_ask(env: &Env, db: &D1Database, row: &AutopilotRow, text: &str, now: i64) {
    let Some(session) = row.session_id.clone() else {
        return;
    };
    let reference = match crate::spec::session_row(db, &session).await {
        Ok(Some(r)) => format!("{}#{}", r.project_slug, r.number),
        _ => return,
    };
    let title = match crate::spec::project_row(db, &row.project_id).await {
        Ok(Some(p)) => format!("{}'s autopilot", p.name),
        _ => "Autopilot".to_owned(),
    };
    let slug = reference.split('#').next().unwrap_or_default().to_owned();
    let event = fctl_domain::notify::Event {
        session_id: SessionId(session),
        reference,
        title,
        kind: fctl_domain::notify::Kind::Autopilot,
        detail: Some(text.to_owned()),
        at: now,
        url: Some(format!("/p/{slug}?tab=autopilot")),
    };
    if let Err(e) = crate::notify::session_changed(env, event, Vec::new()).await {
        worker::console_error!("notifying of an autopilot's ask: {}", e.0.message);
    }
}
