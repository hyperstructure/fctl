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
use fctl_proto::session::{SessionRef, SessionSchedule, SessionStatus, WorkspaceRole};
use fctl_proto::uplink::{Delivery, DeliveryKind};
use fctl_proto::{ApiError, ProjectId, ScheduleId, SessionId};
use serde::Deserialize;
use serde_json::{Value, json};
use worker::{D1Database, Env};

/// Autopilots one alarm takes up at most; it rings again at once for more.
const DUE_PER_ALARM: i64 = 20;

/// The statuses of a session at work, as `fctl_domain::autopilot::at_work`
/// has them, for SQL.
const AT_WORK: &str = "'queued','provisioning','preparing','running','stopping'";

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
    #[serde(deserialize_with = "int_bool", default)]
    pub keep_owner: bool,
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
    /// Where a fresh session takes over from its own, in tokens of context.
    #[serde(deserialize_with = "opt_int", default)]
    pub handoff_at: Option<i64>,
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
    /// The Claude connection its own session runs on.
    #[serde(default)]
    pub session_credential_id: Option<String>,
    /// The tokens in its own session's context, as it last said.
    #[serde(deserialize_with = "opt_int", default)]
    pub session_context: Option<i64>,
}

pub const AUTOPILOT_SELECT: &str = "SELECT a.project_id, a.workspace_id, a.scope, a.state, a.paused_reason, \
    a.paused_by, a.paused_at, a.owner_id, a.keep_owner, a.direction, a.direction_by, a.direction_at, a.direction_new, a.plan, \
    a.plan_at, a.powers, a.check_in_minutes, a.crew_max, a.model, a.effort, a.handoff_at, a.session_id, a.wake_at, a.wake_reasons, \
    a.asked_wake_at, a.woken_at, a.check_ins, a.failures, a.created_at, a.updated_at, \
    ou.name AS owner_name, ou.avatar_url AS owner_avatar, pu.name AS paused_by_name, pu.avatar_url AS paused_by_avatar, \
    du.name AS direction_by_name, du.avatar_url AS direction_by_avatar, \
    ps.number AS session_number, ps.status AS session_status, ps.created_at AS session_created_at, \
    ps.credential_id AS session_credential_id, json_extract(ps.stats, '$.context_tokens') AS session_context \
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

    /// Where a fresh session takes over from its own: `None`, only after
    /// [`ap::SHIFT_MS`].
    pub fn handoff_at(&self) -> Option<u32> {
        self.handoff_at
            .and_then(|t| u32::try_from(t).ok())
            .filter(|&t| t > 0)
    }

    /// The tokens in its own session's context, as it last said.
    pub fn session_context(&self) -> Option<u64> {
        self.session_context.and_then(|t| u64::try_from(t).ok())
    }

    /// Its own session as people and the API see it.
    pub fn session(&self) -> Option<fctl_proto::autopilot::AutopilotSession> {
        match (
            &self.session_id,
            self.session_number,
            self.session_status(),
            self.session_created_at,
        ) {
            (Some(id), Some(number), Some(status), Some(started_at)) => {
                Some(fctl_proto::autopilot::AutopilotSession {
                    id: SessionId(id.clone()),
                    number: u32::try_from(number).unwrap_or(0),
                    status,
                    started_at,
                    context_tokens: self.session_context(),
                })
            }
            _ => None,
        }
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
    // Why it stops, kept in its log and pause, says "its owner" rather
    // than their name, which leaves with their account.
    let user: Option<UserRow> = q(
        db,
        &format!("SELECT {} FROM users u WHERE u.id = ?", user_columns("u")),
        &[s(id)],
    )?
    .first()
    .await?;
    let Some(user) = user.filter(UserRow::is_active) else {
        return Ok(Err(
            "It acts as its owner, whose account is gone or disabled: someone who manages the project takes it over."
                .into(),
        ));
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
            "It acts as its owner, who no longer manages {}: someone who does takes it over.",
            project.name
        )));
    }
    if row.scope() == AutopilotScope::Workspace {
        let admin = crate::workspaces::member(db, &row.workspace_id, &user.id)
            .await?
            .and_then(|m| WorkspaceRole::parse(&m.role))
            .is_some_and(|r| matches!(r, WorkspaceRole::Owner | WorkspaceRole::Admin));
        if !admin {
            return Ok(Err(
                "It runs the workspace as its owner, who is no longer one of its admins: an admin takes it over."
                    .into(),
            ));
        }
    }
    let credential =
        crate::api::sessions::credential_for(db, &user, project.workspace_id()).await?;
    let Some(credential) = credential else {
        return Ok(Err(
            "It runs on its owner's Claude subscription, and its owner has no Claude connection: they connect one in Settings → Claude, or someone who manages the project takes it over."
                .into(),
        ));
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
    // Once claimed, what woke it is gone from its row: should waking it
    // fail, it goes back for a retry soon ([`wake_failed`]), rather than
    // wait for the next check-in.
    let woke: AppResult<bool> = async {
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
        let status = row.session_status();
        // People asked for a fresh session while its own works: the wake
        // waits for that one's turn to end, which wakes it again at once
        // (`session_moved`; here, should it have ended since it was read),
        // and all it woke for goes to the fresh session.
        if ap::holds(&reasons, status) {
            q(
                db,
                &format!(
                    "UPDATE autopilots SET wake_reasons = CASE WHEN wake_reasons = '' THEN ?1 \
                     ELSE wake_reasons || ',' || ?1 END, \
                     wake_at = CASE WHEN (SELECT status FROM sessions WHERE id = autopilots.session_id) \
                       IN ({AT_WORK}) THEN wake_at ELSE ?2 END \
                     WHERE project_id = ?3 AND state = 'on'"
                ),
                &[
                    s(&reasons.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(",")),
                    n(now),
                    s(&row.project_id),
                ],
            )?
            .run()
            .await?;
            ring(env).await;
            return Ok(true);
        }
        // Its sessions that wait move to the subscription it runs on now,
        // should it have moved (`move_crew`).
        if let Err(e) = move_crew(env, db, row, &owner, now).await {
            worker::console_error!(
                "autopilot {}: moving its sessions to its owner's Claude connection: {}",
                row.project_id,
                e.0.message
            );
        }
        let news = News::gather(db, row, now).await?;
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
        // Where it goes: its session as it is, or a fresh one, and why
        // (`Failed` for any a message can't reach: none, or one that failed
        // or takes none).
        let fresh = match (row.session_id.as_deref(), status, row.session_created_at) {
            (Some(_), Some(st), Some(started)) => {
                let own = ap::OwnSession {
                    status: st,
                    started_at: started,
                    context_tokens: row.session_context(),
                };
                let asked = reasons.contains(&AutopilotWakeReason::Fresh);
                match ap::fresh(own, row.handoff_at(), asked, now) {
                    None if !(st.accepts_messages() || st.resumes_for_messages()) => {
                        Some(ap::Fresh::Failed)
                    }
                    f => f,
                }
            }
            _ => Some(ap::Fresh::Failed),
        };
        let carries_on = fresh.is_none();
        // Near where it hands off, it hears so, to write down what its next
        // session needs while its cache is warm.
        let near = if carries_on {
            ap::handoff_near(row.session_context(), row.handoff_at())
        } else {
            None
        };
        let text = news.message(&reasons, near, now);
        let session = match (carries_on, row.session_id.as_deref()) {
            (true, Some(id)) => {
                // On another Claude connection than its owner's now (someone
                // took it over, or its owner chose another): unless at work,
                // it moves there, and Claude continues the same conversation.
                if let Some(st) = status
                    && !ap::at_work(st)
                    && row.session_credential_id.as_deref() != Some(owner.credential.id.as_str())
                    && let Ok(way) = fctl_domain::claude::moving(st)
                {
                    // Best effort, as for its crew: the wake goes on, and the
                    // next one moves it.
                    let moved = crate::api::sessions::move_to(
                        env,
                        db,
                        &SessionId(id.to_owned()),
                        way,
                        &owner.credential,
                        &owner.user.to_ref(),
                        false,
                        now,
                    )
                    .await;
                    if let Err(e) = moved {
                        worker::console_error!(
                            "autopilot {}: moving its session to its owner's Claude connection: {}",
                            row.project_id,
                            e.0.message
                        );
                    }
                }
                deliver(env, id, &text, wake.clone(), now).await?;
                id.to_owned()
            }
            _ => {
                let briefing = briefing(db, row, &project, &owner.user).await?;
                // A fresh session that takes over from one that did its
                // work hears why, and where that work is.
                let from = row
                    .session_number
                    .map(|number| format!("{}#{number}", project.slug));
                let over = fresh.zip(from.as_deref()).and_then(|(why, from)| {
                    ap::takes_over_from(why, from, row.handoff_at())
                });
                let first = match over {
                    Some(over) => format!("{briefing}\n---\n\n{over}\n\n{text}"),
                    None => format!("{briefing}\n---\n\n{text}"),
                };
                let (id, _) =
                    start_pilot(env, db, row, &project, &owner, first, wake.clone(), now).await?;
                // The session it leaves stops, rather than keep its sandbox
                // while it waits for a message that won't come.
                if let (Some(old), Some(st)) = (row.session_id.as_deref(), status)
                    && !st.is_terminal()
                {
                    stop_session(env, old).await;
                }
                id.0
            }
        };
        news.mark_seen(db, row, now).await?;
        // A direction changed since it was read says so at the next wake.
        q(
            db,
            "UPDATE autopilots SET woken_at = ?1, check_ins = check_ins + 1, \
             direction_new = CASE WHEN direction_at = ?2 THEN 0 ELSE direction_new END, updated_at = ?1 \
             WHERE project_id = ?3",
            &[n(now), n(row.direction_at), s(&row.project_id)],
        )?
        .run()
        .await?;
        let mut line = woke_line(&reasons, &wake);
        if let Some(ap::Fresh::Full(tokens)) = fresh.filter(|_| row.session_id.is_some()) {
            line.push_str(&format!(
                ", in a fresh session: the last one's context held {} tokens",
                fctl_domain::context::short(u32::try_from(tokens).unwrap_or(u32::MAX))
            ));
        }
        log(
            db,
            &row.project_id,
            Entry::new(AutopilotEntryKind::Woke, line).about(Some(&session)),
            now,
        )
        .await;
        Ok(true)
    }
    .await;
    if let Err(e) = &woke
        && let Err(again) = wake_failed(env, db, row, &e.0.message, now).await
    {
        worker::console_error!("autopilot {}: {}", row.project_id, again.0.message);
    }
    woke
}

/// The sessions it started, that wait or rest, which [`move_crew`] moves
/// to the Claude connection it runs on: those paused on a usage limit, about
/// to start, and those that stopped in the last week. Those at work, or
/// waiting for a message, finish their run where they are. `?1`: its home
/// project, `?2`: the connection, `?3`: a week ago.
const CREW_TO_MOVE: &str = "autopilot_id = ?1 AND autopilot_role = 'crew' AND credential_id IS NOT ?2 \
    AND (status IN ('paused', 'queued', 'stopping') OR (status = 'stopped' AND updated_at > ?3))";

/// How many of its sessions one wake moves at most; the next wake moves more.
const CREW_MOVES_PER_WAKE: i64 = 25;

/// Moves the sessions it started that wait or rest ([`CREW_TO_MOVE`]) to
/// its owner's Claude connection, as it runs on that now: someone took it
/// over, or its owner chose another connection. One paused on its old
/// subscription's limit starts again on the new one; the others start on
/// it when they next start.
async fn move_crew(
    env: &Env,
    db: &D1Database,
    row: &AutopilotRow,
    owner: &Owner,
    now: i64,
) -> AppResult<()> {
    #[derive(Deserialize)]
    struct Crew {
        id: String,
        status: String,
    }
    let crew: Vec<Crew> = q(
        db,
        &format!(
            "SELECT id, status FROM sessions WHERE {CREW_TO_MOVE} ORDER BY updated_at DESC LIMIT {CREW_MOVES_PER_WAKE}"
        ),
        &[s(&row.project_id), s(&owner.credential.id), n(now - 7 * 86_400_000)],
    )?
    .all()
    .await?;
    let user = owner.user.to_ref();
    for c in crew {
        let Some(way) =
            SessionStatus::parse(&c.status).and_then(|st| fctl_domain::claude::moving(st).ok())
        else {
            continue;
        };
        let moved = crate::api::sessions::move_to(
            env,
            db,
            &SessionId(c.id.clone()),
            way,
            &owner.credential,
            &user,
            false,
            now,
        )
        .await;
        if let Err(e) = moved {
            worker::console_error!(
                "moving {} to its autopilot's connection: {}",
                c.id,
                e.0.message
            );
        }
    }
    Ok(())
}

/// Wakes the autopilots `user_id` runs, now, whose sessions run on another
/// Claude connection than theirs for its workspace: they took one over, or
/// chose or connected another connection. The wake moves them
/// ([`move_crew`], [`take_up`]). Best effort: a failure is logged.
pub async fn owner_moved(env: &Env, db: &D1Database, user_id: &str, now: i64) {
    let moved: AppResult<()> = async {
        let rows: Vec<AutopilotRow> = q(
            db,
            &format!("{AUTOPILOT_SELECT} WHERE a.owner_id = ? AND a.state = 'on'"),
            &[s(user_id)],
        )?
        .all()
        .await?;
        for row in rows {
            let Some(project) = crate::spec::project_row(db, &row.project_id).await? else {
                continue;
            };
            let Ok(owner) = owner(db, &project, &row).await? else {
                continue;
            };
            // As the wake moves it: not while it works, nor once it failed.
            let own = row.session_id.is_some()
                && row
                    .session_status()
                    .is_some_and(|st| !ap::at_work(st) && fctl_domain::claude::moving(st).is_ok())
                && row.session_credential_id.as_deref() != Some(owner.credential.id.as_str());
            let crew: Option<crate::db::Count> = q(
                db,
                &format!("SELECT COUNT(*) AS n FROM sessions WHERE {CREW_TO_MOVE}"),
                &[
                    s(&row.project_id),
                    s(&owner.credential.id),
                    n(now - 7 * 86_400_000),
                ],
            )?
            .first()
            .await?;
            if own || crew.is_some_and(|c| c.n > 0) {
                nudge(env, db, &row.project_id, AutopilotWakeReason::Moved, now).await?;
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = moved {
        worker::console_error!(
            "waking {user_id}'s autopilots for their new Claude connection: {}",
            e.0.message
        );
    }
}

/// After waking it failed once claimed (`why`): what woke it goes back
/// for a retry, 5, 15 and 60 minutes on, as for its session failing, each
/// in its log; then it pauses, saying why.
async fn wake_failed(
    env: &Env,
    db: &D1Database,
    row: &AutopilotRow,
    why: &str,
    now: i64,
) -> AppResult<()> {
    let failures = u32::try_from(row.failures + 1).unwrap_or(u32::MAX);
    let Some(after) = ap::retry_in(failures) else {
        let why = format!(
            "fctl couldn't wake it {} times in a row, the last because: {why} Resume it once that's fixed.",
            ap::RETRY_MS.len() + 1
        );
        return stop_for(db, row, &why, now).await;
    };
    q(
        db,
        "UPDATE autopilots SET failures = ?4, wake_reasons = CASE WHEN wake_reasons = '' THEN ?1 \
         WHEN ?1 = '' THEN wake_reasons ELSE wake_reasons || ',' || ?1 END, \
         wake_at = MIN(COALESCE(wake_at, ?2), ?2) WHERE project_id = ?3 AND state = 'on'",
        &[
            s(&row.wake_reasons),
            n(now + after),
            s(&row.project_id),
            n(i64::from(failures)),
        ],
    )?
    .run()
    .await?;
    log(
        db,
        &row.project_id,
        Entry::new(
            AutopilotEntryKind::Problem,
            format!(
                "fctl couldn't wake it: {why} It tries again in {} minutes.",
                after / 60_000
            ),
        ),
        now,
    )
    .await;
    ring(env).await;
    Ok(())
}

/// What [`woke_line`] says of [`AutopilotWakeReason::Fresh`], alone.
const FRESH_ASKED: &str = "someone asked for a fresh session";

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
            AutopilotWakeReason::Moved => "it was moved to another Claude subscription".into(),
            AutopilotWakeReason::Fresh => FRESH_ASKED.into(),
            AutopilotWakeReason::Other => continue,
        });
    }
    match parts.as_slice() {
        [] => "Woke".into(),
        // "Woke as it was put on", alone; "being put on" among others.
        [one] if one.starts_with("it was ") || one == "someone woke it" || one == FRESH_ASKED => {
            format!("Woke as {one}")
        }
        [one] => format!("Woke for {one}"),
        _ => format!(
            "Woke for {}",
            parts
                .iter()
                .map(|p| match p.strip_prefix("it was ") {
                    Some(was) => format!("being {was}"),
                    None if p == "someone woke it" => "someone waking it".to_owned(),
                    None if p == FRESH_ASKED => "someone asking for a fresh session".to_owned(),
                    None => p.clone(),
                })
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

/// Stops a session of its own that rests and that a fresh one replaced, as
/// its idle timeout would; a failure is logged, as nothing hangs on it.
async fn stop_session(env: &Env, id: &str) {
    let stopped: AppResult<Value> = async {
        let fleet = crate::internal::fleet_stub(env)?;
        crate::internal::post(
            &fleet,
            "/stop",
            &crate::internal::Stop {
                session_id: SessionId(id.to_owned()),
                reason: fctl_proto::runner::StopReason::Idle,
                destroy: false,
            },
        )
        .await
    }
    .await;
    if let Err(e) = stopped {
        worker::console_error!("stopping the autopilot's session {id}: {}", e.0.message);
    }
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
            handoff_at: row.handoff_at(),
        },
        &row.direction,
    ))
}

// ───────────────────────────── news ─────────────────────────────

#[derive(Deserialize)]
struct NoteRow {
    id: String,
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
    /// When it last changed, and who started it: for people's sessions
    /// ([`OTHERS_SELECT`]) alone.
    #[serde(deserialize_with = "opt_int", default)]
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub creator_name: Option<String>,
    /// What its agent said as its last turn ended, and what people wrote
    /// to it, for its autopilot ([`ap::CrewView`]): for its sessions
    /// ([`CREW_SELECT`]) alone.
    #[serde(default)]
    pub said: Option<String>,
    #[serde(default, deserialize_with = "heard_json")]
    pub heard: Vec<ap::Heard>,
}

/// People's messages a session's row keeps ([`ap::heard`]), newest first.
fn heard_json<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<ap::Heard>, D::Error> {
    let stored = Option::<String>::deserialize(d)?;
    Ok(ap::heard(stored.as_deref()))
}

pub const CREW_SELECT: &str = "SELECT s.id, s.project_id, p.slug AS project_slug, s.number, s.title, s.status, \
    s.status_reason, s.question, s.pull_requests, s.created_at, s.last_activity_at, s.said, s.heard \
    FROM sessions s JOIN projects p ON p.id = s.project_id";

/// The sessions people started in an autopilot's places, rather than the
/// autopilot: with [`others_where`], neither its own nor those it started,
/// nor review agents' or archived ones.
const OTHERS_SELECT: &str = "SELECT s.id, s.project_id, p.slug AS project_slug, s.number, s.title, s.status, \
    s.status_reason, s.question, s.pull_requests, s.created_at, s.last_activity_at, s.updated_at, \
    cu.name AS creator_name FROM sessions s JOIN projects p ON p.id = s.project_id \
    LEFT JOIN users cu ON cu.id = s.created_by";

/// That a session (`s`) lists a pull request still open, as the pull
/// requests page counts it: one nobody looked up counts as open.
const PULL_OPEN: &str = "s.id IN (SELECT session_id FROM pull_request_sessions) AND EXISTS (SELECT 1 FROM \
    json_each(COALESCE(s.pull_requests, json_array(json_object('state', s.pr_state)))) j \
    WHERE COALESCE(json_extract(j.value, '$.state'), 'open') IN ('open','draft','queued'))";

/// Where [`OTHERS_SELECT`]'s sessions are: in the autopilot's places (its
/// project, or its workspace's projects that aren't archived), people's
/// own. Pushes what it binds onto `params`.
fn others_where(row: &AutopilotRow, params: &mut Vec<crate::db::Val>) -> String {
    params.push(s(&row.project_id));
    let home = params.len();
    let place = match row.scope() {
        AutopilotScope::Project => format!("s.project_id = ?{home}"),
        AutopilotScope::Workspace => {
            params.push(s(&row.workspace_id));
            format!(
                "p.workspace_id = ?{} AND p.archived_at IS NULL",
                params.len()
            )
        }
    };
    format!(
        "{place} AND s.archived = 0 AND (s.autopilot_role IS NULL OR (s.autopilot_role = 'crew' \
         AND s.autopilot_id != ?{home})) AND NOT EXISTS (SELECT 1 FROM reviews rv WHERE rv.session_id = s.id)"
    )
}

/// The sessions people started in its places that work now, or rest with
/// a pull request still open, the latest first: what its brief lists, as
/// it looks after their pull requests too.
pub async fn others(db: &D1Database, row: &AutopilotRow) -> AppResult<Vec<CrewRow>> {
    let mut params = Vec::new();
    let place = others_where(row, &mut params);
    q(
        db,
        &format!(
            "{OTHERS_SELECT} WHERE {place} AND (s.status IN ('queued','provisioning','preparing','running','paused','stopping') \
             OR (s.status IN ('idle','stopped','failed') AND {PULL_OPEN})) \
             ORDER BY COALESCE(s.last_activity_at, s.updated_at) DESC LIMIT 15"
        ),
        &params,
    )?
    .all()
    .await
}

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

    /// When its agent last worked, or else when it last changed.
    pub fn at(&self) -> i64 {
        self.last_activity_at
            .or(self.updated_at)
            .unwrap_or(self.created_at)
    }

    /// Its pull requests still open, with their checks.
    pub fn open_pulls(&self) -> Vec<String> {
        self.pull_requests()
            .iter()
            .filter_map(ap::open_pull)
            .collect()
    }

    pub fn session_ref(&self) -> SessionRef {
        SessionRef {
            id: SessionId(self.id.clone()),
            number: u32::try_from(self.number).unwrap_or(0),
            title: self.title.clone(),
        }
    }

    /// "#41 (open, checks running)", "#40 merged" ([`ap::pull_line`]).
    pub fn pr_lines(&self) -> Vec<String> {
        self.pull_requests().iter().map(ap::pull_line).collect()
    }
}

/// What is new for an autopilot since it last woke.
struct News {
    /// When it last woke.
    since: i64,
    notes: Vec<NoteRow>,
    direction: Option<(String, Option<String>)>,
    answers: Vec<AskRow>,
    crew: Vec<CrewRow>,
    /// Its other sessions that people wrote to since: they come with what
    /// wakes it, and wake it for nothing.
    heard: Vec<CrewRow>,
    /// People's sessions that ended a turn since, with a pull request
    /// open: they come with what wakes it, and wake it for nothing.
    others: Vec<CrewRow>,
}

impl News {
    async fn gather(db: &D1Database, row: &AutopilotRow, now: i64) -> AppResult<Self> {
        let notes: Vec<NoteRow> = q(
            db,
            "SELECT n.id, n.text, u.name AS author_name, n.created_at, n.seen_at FROM autopilot_notes n \
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
        // Its others that people wrote to since (`ap::hear`), besides those
        // above, which say so themselves.
        let listed: Vec<&str> = crew.iter().map(|c| c.id.as_str()).collect();
        let heard: Vec<CrewRow> = q(
            db,
            &format!(
                "{CREW_SELECT} WHERE s.autopilot_id = ? AND s.autopilot_role = 'crew' \
                 AND json_extract(s.heard, '$[0].at') > ? AND s.id NOT IN (SELECT value FROM json_each(?)) \
                 ORDER BY json_extract(s.heard, '$[0].at') DESC LIMIT 10"
            ),
            &[
                s(&row.project_id),
                n(since),
                s(&serde_json::to_string(&listed)?),
            ],
        )?
        .all()
        .await?;
        let mut params = Vec::new();
        let place = others_where(row, &mut params);
        params.push(n(since));
        let others: Vec<CrewRow> = q(
            db,
            &format!(
                "{OTHERS_SELECT} WHERE {place} AND COALESCE(s.last_activity_at, s.updated_at) > ?{} \
                 AND s.status IN ('idle', 'stopped') AND s.question IS NULL AND {PULL_OPEN} \
                 ORDER BY COALESCE(s.last_activity_at, s.updated_at) DESC LIMIT 10",
                params.len()
            ),
            &params,
        )?
        .all()
        .await?;
        let _ = now;
        Ok(Self {
            since,
            notes,
            direction: row
                .direction_new
                .then(|| (row.direction.clone(), row.direction_by_name.clone())),
            answers,
            crew,
            heard,
            others,
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

    fn message(
        &self,
        reasons: &[AutopilotWakeReason],
        handoff: Option<(u64, u32)>,
        now: i64,
    ) -> String {
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
        // Those that moved on, then those people wrote to, with what they
        // wrote since it last woke.
        let rows: Vec<&CrewRow> = self.crew.iter().chain(&self.heard).collect();
        let questions: Vec<Option<String>> = rows.iter().map(|c| c.question_text()).collect();
        let crew: Vec<ap::CrewView> = rows
            .iter()
            .zip(questions.iter())
            .map(|(c, q)| {
                let mut v = crew_view(c);
                v.question = q.as_deref();
                v.heard = ap::heard_since(&c.heard, self.since);
                v
            })
            .collect();
        let others: Vec<ap::OtherView> = self.others.iter().map(|o| other_view(o, None)).collect();
        ap::wake_message(&ap::News {
            reasons: reasons.to_vec(),
            notes,
            direction: self
                .direction
                .as_ref()
                .map(|(d, by)| (d.as_str(), by.as_deref())),
            answers,
            crew,
            others,
            handoff,
            now,
        })
    }

    /// It has heard of these now: what it was told, and no more, as people
    /// may have written meanwhile.
    async fn mark_seen(&self, db: &D1Database, row: &AutopilotRow, now: i64) -> AppResult<()> {
        let notes: Vec<String> = self.notes.iter().map(|n| n.id.clone()).collect();
        let answers: Vec<String> = self.answers.iter().map(|a| a.id.clone()).collect();
        mark_seen(db, &row.project_id, &notes, &answers, now).await
    }
}

/// Marks what people told it seen once it heard of them: the notes
/// `notes`, and the answers to its asks `asks`.
pub async fn mark_seen(
    db: &D1Database,
    project_id: &str,
    notes: &[String],
    asks: &[String],
    now: i64,
) -> AppResult<()> {
    if !notes.is_empty() {
        q(
            db,
            "UPDATE autopilot_notes SET seen_at = ? WHERE project_id = ? AND seen_at IS NULL \
             AND id IN (SELECT value FROM json_each(?))",
            &[n(now), s(project_id), s(&serde_json::to_string(notes)?)],
        )?
        .run()
        .await?;
    }
    if !asks.is_empty() {
        q(
            db,
            "UPDATE autopilot_asks SET seen_at = ? WHERE project_id = ? AND seen_at IS NULL AND status != 'open' \
             AND id IN (SELECT value FROM json_each(?))",
            &[n(now), s(project_id), s(&serde_json::to_string(asks)?)],
        )?
        .run()
        .await?;
    }
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

/// One of its sessions, for what fctl tells it, with all that people
/// wrote to it that its row keeps.
pub fn crew_view(c: &CrewRow) -> ap::CrewView<'_> {
    ap::CrewView {
        reference: format!("{}#{}", c.project_slug, c.number),
        title: &c.title,
        status: c.status(),
        status_reason: c.status_reason.as_deref(),
        question: None,
        pull_requests: c.pr_lines(),
        said: c.said.as_deref(),
        heard: &c.heard,
    }
}

/// One of people's sessions, for what fctl tells it, with what its agent
/// asks (`question`, from [`CrewRow::question_text`]).
pub fn other_view<'a>(c: &'a CrewRow, question: Option<&'a str>) -> ap::OtherView<'a> {
    ap::OtherView {
        reference: format!("{}#{}", c.project_slug, c.number),
        title: &c.title,
        by: c.creator_name.as_deref().unwrap_or("Someone"),
        status: c.status(),
        question,
        pull_requests: c.open_pulls(),
        at: c.at(),
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
                    // asked for a time, or something waits to wake it; a
                    // fresh session people asked for takes over now.
                    q(
                        &db,
                        "UPDATE autopilots SET failures = 0, wake_at = CASE \
                           WHEN instr(',' || wake_reasons || ',', ',fresh,') > 0 THEN MIN(COALESCE(wake_at, ?2), ?2) \
                           WHEN wake_reasons != '' THEN wake_at \
                           WHEN asked_wake_at IS NOT NULL THEN asked_wake_at \
                           ELSE ?1 END, updated_at = ?2 WHERE project_id = ?3 AND state = 'on'",
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
                // Stopped mid-turn by a person, or paused on a usage limit:
                // a fresh session people asked for needn't wait for its
                // check-in (one at its owner's limit waits for the reset, as
                // any wake does).
                SessionStatus::Stopped | SessionStatus::Paused
                    if row.reasons().contains(&AutopilotWakeReason::Fresh) =>
                {
                    nudge(env, &db, autopilot, AutopilotWakeReason::Fresh, now).await?;
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
                            // Its sessions' failures notify nobody: this
                            // one waits for people to resume it.
                            notify_people(
                                env,
                                &db,
                                &row,
                                fctl_domain::notify::Kind::AutopilotPaused,
                                &why,
                                now,
                            )
                            .await;
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

/// Its sessions (`sessions`, unaliased) that work, or that a message sent
/// them will set working: one that rests says so only once the runner
/// reports, and its last message came after it last changed.
const CREW_WORKING: &str = "(status IN ('queued','provisioning','preparing','running','paused','stopping') \
     OR (status = 'idle' AND messaged_at IS NOT NULL AND messaged_at >= updated_at))";

/// Whether an autopilot may start another session now: fewer than its
/// people allow work at once, and fewer than [`ap::CREW_PER_HOUR`] started
/// in the last hour.
pub async fn crew_room(db: &D1Database, caller: &AutopilotCaller, now: i64) -> AppResult<()> {
    crew_room_for(db, caller, now, true).await
}

/// [`crew_room`], for a session of its own that a message or a resume sets
/// working again (`new` false): only the room at once counts, as it starts
/// nothing new.
async fn crew_room_for(
    db: &D1Database,
    caller: &AutopilotCaller,
    now: i64,
    new: bool,
) -> AppResult<()> {
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
        &format!(
            "SELECT \
           (SELECT COUNT(*) FROM sessions WHERE autopilot_id = ?1 AND autopilot_role = 'crew' AND {CREW_WORKING}) AS working, \
           (SELECT COUNT(*) FROM sessions WHERE autopilot_id = ?1 AND autopilot_role = 'crew' AND created_at > ?2) AS lately"
        ),
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
    if new && counts.lately >= i64::from(ap::CREW_PER_HOUR) {
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
    Ok(format!(
        "{}{}",
        task.trim_end(),
        ap::crew_note(
            &project,
            caller.scope,
            owner_called,
            caller.powers.contains(&AutopilotPower::Merge),
        )
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
    if (200..300).contains(&status)
        && ap::describe(&method, &segs, sent, &answer, None, None).is_some()
    {
        let about = about(&ctx.db, &segs, &answer).await;
        // Another project it changed, by its slug, which the line names.
        let elsewhere = match about.project.as_deref() {
            Some(other) if other != row.project_id => slug_of(&ctx.db, other).await,
            _ => None,
        };
        if let Some((kind, text)) = ap::describe(
            &method,
            &segs,
            sent,
            &answer,
            elsewhere.as_deref(),
            about.schedule.as_deref(),
        ) {
            // Its own transcript shows what it did, the session it started
            // or steered with it.
            transcript(
                ctx,
                session_id,
                kind,
                &text,
                about.session.as_deref(),
                about.project.as_deref(),
            )
            .await;
            let mut entry = Entry::new(kind, text);
            entry.session = about.session.as_deref();
            entry.other_project = about.project.as_deref();
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
        schedule_id: Option<String>,
        schedule_name: Option<String>,
    }
    let posted: AppResult<Value> = async {
        let found: Option<Found> = match session {
            Some(id) => q(
                &ctx.db,
                "SELECT s.id, s.number, s.title, p.slug, s.schedule_id, sc.name AS schedule_name \
                     FROM sessions s JOIN projects p ON p.id = s.project_id \
                     LEFT JOIN schedules sc ON sc.id = s.schedule_id WHERE s.id = ?",
                &[s(id)],
            )?
            .first()
            .await?,
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
        // A schedule it ran goes with the session it started, which the
        // apps link to.
        let schedule = match (&found, kind) {
            (Some(f), AutopilotEntryKind::Schedule) => {
                f.schedule_id.clone().map(|id| SessionSchedule {
                    id: ScheduleId(id),
                    name: f.schedule_name.clone(),
                })
            }
            _ => None,
        };
        notice.autopilot = Some(fctl_proto::autopilot::AutopilotNotice {
            kind,
            schedule,
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

/// What a request the autopilot made concerns, for its log.
#[derive(Default)]
struct About {
    /// The session it started or steered, or that the schedule it ran
    /// started.
    session: Option<String>,
    /// The project it changed.
    project: Option<String>,
    /// The name of the schedule it ran.
    schedule: Option<String>,
}

/// What a request the autopilot made concerns, from its path and answer.
async fn about(db: &D1Database, segs: &[&str], answer: &Value) -> About {
    let str_at = |pointer: &str| {
        answer
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    match segs {
        ["projects", _, "sessions"] => About {
            session: str_at("/id"),
            project: str_at("/project_id"),
            schedule: None,
        },
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
            About {
                session: Some((*id).to_owned()),
                project,
                schedule: None,
            }
        }
        ["projects"] => About {
            project: str_at("/id"),
            ..About::default()
        },
        ["projects", key, ..] => About {
            project: project_of(db, key).await,
            ..About::default()
        },
        ["schedules", id, rest @ ..] => {
            #[derive(Deserialize)]
            struct S {
                project_id: String,
                name: String,
            }
            let found = match q(
                db,
                "SELECT project_id, name FROM schedules WHERE id = ?",
                &[s(id)],
            )
            .ok()
            {
                Some(query) => query.first::<S>().await.ok().flatten(),
                None => None,
            };
            // A run's answer is the run, with the session it started.
            let ran = rest == ["runs"];
            About {
                session: if ran { str_at("/session/id") } else { None },
                schedule: found.as_ref().filter(|_| ran).map(|f| f.name.clone()),
                project: found
                    .map(|f| f.project_id)
                    .or_else(|| str_at("/project_id")),
            }
        }
        _ => About::default(),
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
                #[serde(deserialize_with = "int_bool")]
                working: bool,
            }
            let found: Option<Found> = q(
                &ctx.db,
                &format!(
                    "SELECT {CREW_WORKING} AS working FROM sessions WHERE id = ? AND autopilot_id = ? \
                     AND autopilot_role = 'crew'"
                ),
                &[s(&session), s(&caller.project_id)],
            )?
            .first()
            .await?;
            let Some(found) = found else {
                return Err(forbidden(
                    "An autopilot steers only the sessions it started: people steer theirs. Read this one with fctl show or fctl logs.",
                ));
            };
            // A message or a resume sets one that rests working again:
            // only with room for it, as many at once as its people allow.
            let sets_working =
                method == "POST" && matches!(segs.get(2), Some(&("messages" | "resume")));
            if sets_working && !found.working {
                crew_room_for(&ctx.db, caller, ctx.now, false).await?;
            }
            Ok(())
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
    notify_people(
        env,
        db,
        row,
        fctl_domain::notify::Kind::Autopilot,
        text,
        now,
    )
    .await;
}

/// Tells the people who follow the autopilot's own session what of it
/// waits for them, `kind` with `text`, in a notification that opens its tab:
/// what it asks, or that fctl paused it. Its sessions keep the rest to
/// themselves (`fctl_domain::notify::Audience`).
async fn notify_people(
    env: &Env,
    db: &D1Database,
    row: &AutopilotRow,
    kind: fctl_domain::notify::Kind,
    text: &str,
    now: i64,
) {
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
        kind,
        detail: Some(text.to_owned()),
        at: now,
        url: Some(format!("/p/{slug}?tab=autopilot")),
    };
    if let Err(e) = crate::notify::session_changed(env, event, Vec::new()).await {
        worker::console_error!("notifying of an autopilot: {}", e.0.message);
    }
}
