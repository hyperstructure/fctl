//! Sessions that start without a person at fctl's door: a schedule's runs
//! (`crate::schedules`) and tickets delegated to fctl in a tracker
//! (`crate::delegations`). Each is a session of its project, as one a
//! person starts there, on the Claude subscription of the person it runs
//! for, whom it names as its starter.

use crate::db::{n, os, q, s};
use crate::error::{AppResult, internal};
use crate::internal::{InitSession, Schedule as Dispatch};
use crate::model::{CredentialRow, ProjectRow, UserRow};
use fctl_proto::SessionId;
use fctl_proto::event::{MessageSource, UserMessage};
use fctl_proto::session::{GithubAccess, UserRef};
use fctl_proto::tickets::TicketMessage;
use serde::Deserialize;
use serde_json::Value;
use worker::{D1Database, Env};

/// How a new session names its branch.
pub enum Branch<'a> {
    /// `fctl/<number>-<title>`.
    Titled,
    /// `fctl/<number>-<key>-<title>`, after the ticket it starts from, so
    /// that the tracker links the branch's pull request to the ticket.
    Ticket(&'a str),
}

/// What a new session starts with.
pub struct NewSession<'a> {
    pub project: &'a ProjectRow,
    pub title: String,
    pub branch: Branch<'a>,
    pub base_branch: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Who it runs as, on which Claude connection.
    pub user: &'a UserRow,
    pub credential: &'a CredentialRow,
    pub github_access: GithubAccess,
    /// Its first message, and who sent it: nobody (fctl) for a schedule's
    /// run at its time.
    pub prompt: String,
    pub author: Option<UserRef>,
    pub source: MessageSource,
    /// The ticket the first message comes from, for the transcript.
    pub ticket_message: Option<TicketMessage>,
    /// The schedule it runs for (`sessions.schedule_id`).
    pub schedule_id: Option<&'a str>,
    /// The ticket it starts from (`sessions.ticket`, JSON).
    pub ticket: Option<Value>,
    /// The autopilot it belongs to (`sessions.autopilot_id`).
    pub autopilot: Option<AutopilotStart<'a>>,
    /// Its person follows it, as starting a session does.
    pub follow: bool,
    pub now: i64,
}

/// What a new session is to an autopilot.
pub struct AutopilotStart<'a> {
    /// The autopilot's home project.
    pub project_id: &'a str,
    pub role: fctl_proto::autopilot::AutopilotRole,
    /// What woke the autopilot, for its own session's first message.
    pub wake: Option<fctl_proto::autopilot::AutopilotWake>,
    /// For a session it started: the autopilot, as the first message's
    /// author shows it.
    pub author: Option<fctl_proto::autopilot::AutopilotAuthor>,
}

/// Starts a session: its row, its SessionObject with the first message,
/// and its place in FleetObject's queue. Returns it and its number.
pub async fn start(env: &Env, db: &D1Database, st: NewSession<'_>) -> AppResult<(SessionId, u32)> {
    #[derive(Deserialize)]
    struct Counter {
        #[serde(deserialize_with = "crate::db::int")]
        session_counter: i64,
    }
    let id = fctl_domain::ids::session_id(st.now);
    let counter: Option<Counter> = q(
        db,
        "UPDATE projects SET session_counter = session_counter + 1 WHERE id = ? RETURNING session_counter",
        &[s(&st.project.id)],
    )?
    .first()
    .await?;
    let number = u32::try_from(counter.map_or(1, |c| c.session_counter)).unwrap_or(1);
    let branch = match st.branch {
        Branch::Titled => fctl_domain::naming::session_branch(number, &st.title),
        Branch::Ticket(key) => fctl_domain::tickets::branch(number, key, &st.title),
    };
    let ticket = st.ticket.as_ref().map(Value::to_string);
    q(
        db,
        "INSERT INTO sessions (id, project_id, number, title, prompt, status, created_by, credential_id, branch, base_branch, \
         model, effort, claude_session_id, stats, created_at, updated_at, last_activity_at, github_access, schedule_id, ticket, \
         autopilot_id, autopilot_role) \
         VALUES (?, ?, ?, ?, ?, 'queued', ?, ?, ?, ?, ?, ?, ?, '{}', ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            s(id.as_str()),
            s(&st.project.id),
            n(i64::from(number)),
            s(&st.title),
            s(&st.prompt),
            s(&st.user.id),
            s(&st.credential.id),
            s(&branch),
            s(&st.base_branch),
            os(st.model.as_deref()),
            os(st.effort.as_deref()),
            s(&fctl_domain::ids::uuid_v4()),
            n(st.now),
            n(st.now),
            n(st.now),
            s(st.github_access.as_str()),
            os(st.schedule_id),
            os(ticket.as_deref()),
            os(st.autopilot.as_ref().map(|a| a.project_id)),
            os(st.autopilot.as_ref().map(|a| a.role.as_str())),
        ],
    )?
    .run()
    .await?;
    if st.follow
        && let Err(e) = crate::notify::follow_own(db, id.as_str(), &st.user.id, st.now).await
    {
        worker::console_error!("following a new session: {}", e.0.message);
    }
    let session = crate::spec::session_row(db, id.as_str())
        .await?
        .ok_or_else(|| internal("the new session vanished"))?
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
                follow_up: None,
                review: None,
                ticket: st.ticket_message,
                autopilot: st.autopilot.as_ref().and_then(|a| a.author.clone()),
                wake: st.autopilot.and_then(|a| a.wake),
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
