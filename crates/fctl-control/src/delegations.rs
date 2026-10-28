//! Tickets delegated to fctl in Linear (docs/architecture.md#linear). People
//! there assign an issue to fctl's agent, or mention it in a comment, which
//! makes an agent session on the issue, and Linear tells the control plane
//! (`api::linear::webhook`). This starts a session of the project that
//! takes the issue's team, as the person who asked, once fctl knows who
//! they are there (`linear_accounts`); a reply in the agent session reaches
//! the session's agent as that person's message; and what the session does
//! goes back there: a thought as it starts, a step now and then while the
//! agent works, its pull requests, and at each turn's end the agent's last
//! message, as Linear's response, or its question.
//!
//! Each agent session is a thread here (`linear_threads`): waiting for its
//! person to link their Linear account, or to choose the project, until
//! they do or an hour passes; started, with the fctl session it reaches; or
//! refused, saying why in Linear.

use crate::config::Config;
use crate::crypto;
use crate::db::{n, os, q, s};
use crate::error::{AppError, AppResult, internal};
use crate::internal::{Deliver, Delivered};
use crate::linear::{self, Linear};
use crate::model::{PROJECT_SELECT, ProjectRow, UserRow, user_columns};
use crate::schedules::IntlZone;
use fctl_domain::cron::Zone;
use fctl_domain::linear::{self as lin, Act, AgentSessionEvent, Candidate};
use fctl_domain::tickets as tk;
use fctl_proto::api::InstanceSettings;
use fctl_proto::error::ErrorCode;
use fctl_proto::event::{EventBody, MessageSource, Notice, NoticeLevel, UserMessage};
use fctl_proto::session::{
    LinearTickets, ProjectRole, PullRequestRef, SessionStatus, SessionTicket, Tracker, UserRef,
};
use fctl_proto::tickets::{TicketEvent, TicketLink, TicketMessage, TicketNotice, TicketPerson};
use fctl_proto::uplink::{Delivery, DeliveryKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use worker::{D1Database, Env};

/// What a session that started from a Linear ticket keeps of it
/// (`sessions.ticket`): the ticket, and what its agent is told.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TicketColumn {
    #[serde(flatten)]
    pub ticket: SessionTicket,
    /// The Linear workspace, by Linear's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    /// They asked for an exploration.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub explore: bool,
    /// The state fctl moved it to as the session started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<String>,
}

/// An agent session, as D1 keeps it.
#[derive(Clone, Debug, Deserialize)]
struct ThreadRow {
    id: String,
    organization_id: String,
    issue_key: String,
    session_id: Option<String>,
    linear_user_id: Option<String>,
    state: String,
    waiting_for: Option<String>,
    payload: Option<String>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    stepped_at: Option<i64>,
}

const THREAD_SELECT: &str = "SELECT id, organization_id, issue_key, session_id, linear_user_id, state, waiting_for, payload, stepped_at FROM linear_threads";

async fn thread(db: &D1Database, id: &str) -> AppResult<Option<ThreadRow>> {
    q(db, &format!("{THREAD_SELECT} WHERE id = ?"), &[s(id)])?
        .first()
        .await
}

/// The thread a session answers in now: the one that last reached it,
/// while its project's workspace still works in that Linear workspace.
async fn thread_of(db: &D1Database, session_id: &str) -> AppResult<Option<ThreadRow>> {
    q(
        db,
        &format!(
            "{THREAD_SELECT} t WHERE t.session_id = ?1 AND t.state = 'started' AND EXISTS (SELECT 1 FROM sessions s \
             JOIN projects p ON p.id = s.project_id JOIN workspace_linear wl ON wl.workspace_id = p.workspace_id \
             WHERE s.id = ?1 AND wl.organization_id = t.organization_id) ORDER BY t.updated_at DESC LIMIT 1"
        ),
        &[s(session_id)],
    )?
    .first()
    .await
}

/// Whether session `session_id`'s project is in an fctl workspace that
/// works in Linear workspace `organization`: one that disconnected it hears
/// nothing more from there, though fctl's app may stay installed for others.
async fn connected(db: &D1Database, session_id: &str, organization: &str) -> AppResult<bool> {
    let c: Option<crate::db::Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM sessions s JOIN projects p ON p.id = s.project_id \
         JOIN workspace_linear wl ON wl.workspace_id = p.workspace_id WHERE s.id = ? AND wl.organization_id = ?",
        &[s(session_id), s(organization)],
    )?
    .first()
    .await?;
    Ok(c.is_some_and(|c| c.n > 0))
}

async fn set_state(
    db: &D1Database,
    id: &str,
    state: &str,
    session_id: Option<&str>,
    now: i64,
) -> AppResult<()> {
    q(
        db,
        "UPDATE linear_threads SET state = ?, session_id = COALESCE(?, session_id), waiting_for = NULL, payload = NULL, \
         expires_at = NULL, updated_at = ? WHERE id = ?",
        &[s(state), os(session_id), n(now), s(id)],
    )?
    .run()
    .await?;
    Ok(())
}

/// Keeps a delivery for a thread until its person may act, for an hour.
async fn wait(
    db: &D1Database,
    id: &str,
    state: &str,
    waiting_for: &str,
    linear_user: Option<&str>,
    payload: &Value,
    now: i64,
) -> AppResult<()> {
    q(
        db,
        "UPDATE linear_threads SET state = ?, waiting_for = ?, linear_user_id = COALESCE(?, linear_user_id), payload = ?, \
         expires_at = ?, updated_at = ? WHERE id = ?",
        &[
            s(state),
            s(waiting_for),
            os(linear_user),
            s(&payload.to_string()),
            n(now + lin::WAIT_MS),
            n(now),
            s(id),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// The person fctl knows as `linear_user` in Linear workspace
/// `organization`, while their account is active.
pub async fn person(
    db: &D1Database,
    organization: &str,
    linear_user: &str,
) -> AppResult<Option<UserRow>> {
    let user: Option<UserRow> = q(
        db,
        &format!(
            "SELECT {} FROM linear_accounts a JOIN users u ON u.id = a.user_id WHERE a.organization_id = ? AND a.linear_user_id = ?",
            user_columns("u")
        ),
        &[s(organization), s(linear_user)],
    )?
    .first()
    .await?;
    Ok(user.filter(UserRow::is_active))
}

/// Posts `act` in agent session `id`, logging a failure: Linear's side of
/// a session never fails fctl's.
async fn post(linear: &Linear<'_>, id: &str, act: &Act<'_>, ephemeral: bool) {
    if let Err(e) = linear.activity(lin::activity(id, act, ephemeral)).await {
        worker::console_error!("posting to Linear's agent session {id}: {}", e.0.message);
    }
}

/// What fctl's link to the session it started looks like in Linear.
fn session_link(cfg: &Config, session_id: &str) -> Value {
    json!({ "label": "fctl session", "url": cfg.url(&format!("/s/{session_id}")) })
}

// ───────────────────────────── from Linear ─────────────────────────────

/// Someone delegated an issue to fctl, or mentioned it: a new agent session,
/// which this answers at once, then takes to a session.
pub async fn created(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    e: &AgentSessionEvent,
    raw: &Value,
    now: i64,
) -> AppResult<()> {
    let keys = crypto::keys(cfg)?;
    let linear = linear::client(db, cfg, &keys, &e.organization_id, now).await?;
    let session = &e.agent_session;
    let Some(issue) = session.issue.as_ref().filter(|i| !i.id.is_empty()) else {
        post(
            &linear,
            &session.id,
            &Act::Error("fctl works on issues: delegate an issue to it, or mention it in an issue's comments."),
            false,
        )
        .await;
        return Ok(());
    };
    let inserted = q(
        db,
        "INSERT INTO linear_threads (id, organization_id, issue_id, issue_key, linear_user_id, state, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, 'new', ?, ?) ON CONFLICT (id) DO NOTHING",
        &[
            s(&session.id),
            s(&e.organization_id),
            s(&issue.id),
            s(&issue.identifier),
            os(session.creator_id.as_deref()),
            n(now),
            n(now),
        ],
    )?
    .run()
    .await?;
    if inserted == 0 {
        // Linear sent it again.
        return Ok(());
    }
    // Linear calls a session that hears nothing for ten seconds unresponsive.
    post(
        &linear,
        &session.id,
        &Act::Thought(&format!("Looking at {}…", issue.identifier)),
        true,
    )
    .await;
    take(env, cfg, db, &linear, e, raw, None, now).await
}

/// Someone replied in an agent session, or pressed stop.
pub async fn prompted(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    e: &AgentSessionEvent,
    raw: &Value,
    now: i64,
) -> AppResult<()> {
    let keys = crypto::keys(cfg)?;
    let linear = linear::client(db, cfg, &keys, &e.organization_id, now).await?;
    let session = &e.agent_session;
    let activity = e.agent_activity.clone().unwrap_or_default();
    let replier = activity
        .user_id
        .as_deref()
        .or(activity.user.as_ref().map(|u| u.id.as_str()));
    let creator = session
        .creator_id
        .as_deref()
        .or(session.creator.as_ref().map(|c| c.id.as_str()));
    let Some(t) = thread(db, &session.id).await? else {
        // A session fctl never heard begin (its webhook was off, or it went
        // with its fctl session): as new, when whoever made it says so.
        if session.issue.is_some() && replier.is_some() && replier == creator {
            return created(env, cfg, db, e, raw, now).await;
        }
        return Ok(());
    };
    if t.state == "started" {
        let Some(session_id) = t.session_id.as_deref() else {
            return Ok(());
        };
        return reply(env, cfg, db, &linear, &t, session_id, &activity, raw, now).await;
    }
    if activity.stops() {
        // Anyone may stop what hasn't started: it starts nothing.
        set_state(db, &t.id, "refused", None, now).await?;
        post(
            &linear,
            &session.id,
            &Act::Response("Stopped: fctl started nothing."),
            false,
        )
        .await;
        return Ok(());
    }
    // Before its session starts, only whoever made the agent session moves
    // it on: anyone else's reply would start it, or choose its project, as
    // them, on their subscription.
    let theirs = t.linear_user_id.as_deref().or(creator);
    if replier.is_none() || replier != theirs {
        let name = session
            .creator
            .as_ref()
            .and_then(|c| c.name.as_deref())
            .unwrap_or("the person who asked");
        post(
            &linear,
            &session.id,
            &Act::Response(&format!(
                "Only {name} can start this one. To have fctl work on it as you, delegate the issue to fctl, or mention fctl in a new comment."
            )),
            false,
        )
        .await;
        return Ok(());
    }
    match t.waiting_for.as_deref() {
        Some("project") if t.state == "waiting" => {
            // The answer to which project: one of the choices, by slug or name.
            let Some(kept) = t
                .payload
                .as_deref()
                .and_then(|p| serde_json::from_str::<Value>(p).ok())
            else {
                return Ok(());
            };
            let Some(first) = event_of(&kept) else {
                return Ok(());
            };
            take(
                env,
                cfg,
                db,
                &linear,
                &first,
                &kept,
                Some(activity.body().trim()),
                now,
            )
            .await
        }
        // Waiting for them to link their account, refused, or failed to
        // start: once more, from the start, as things may have changed.
        _ => {
            let kept = t
                .payload
                .as_deref()
                .and_then(|p| serde_json::from_str::<Value>(p).ok())
                .unwrap_or_else(|| raw.clone());
            let first = event_of(&kept).unwrap_or_else(|| e.clone());
            take(env, cfg, db, &linear, &first, &kept, None, now).await
        }
    }
}

/// The agent session event a kept delivery holds.
fn event_of(raw: &Value) -> Option<AgentSessionEvent> {
    match lin::delivery(raw) {
        lin::Delivery::Created(e) | lin::Delivery::Prompted(e) => Some(*e),
        _ => None,
    }
}

/// A project that takes tickets from a Linear team, with its choice.
struct Taker {
    row: ProjectRow,
    linear: LinearTickets,
}

/// The projects that take tickets from Linear workspace `organization`:
/// of fctl workspaces connected to it, not archived, whose tracker is
/// Linear there.
async fn takers(db: &D1Database, organization: &str) -> AppResult<Vec<Taker>> {
    let rows: Vec<ProjectRow> = q(
        db,
        &format!(
            "{PROJECT_SELECT} WHERE p.archived_at IS NULL AND p.workspace_id IN (SELECT workspace_id FROM workspace_linear WHERE organization_id = ?2) \
             AND json_extract(p.settings, '$.tickets.tracker') = 'linear' \
             AND json_extract(p.settings, '$.tickets.linear.organization') = ?2 ORDER BY p.created_at"
        ),
        &[s(""), s(organization)],
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let linear = row.settings().tickets.linear?;
            Some(Taker { row, linear })
        })
        .collect())
}

/// `user`'s role in `project`: `None` when they can't see it.
async fn role_in(
    db: &D1Database,
    user: &UserRow,
    settings: &InstanceSettings,
    project: &ProjectRow,
) -> AppResult<Option<ProjectRole>> {
    let theirs: Option<ProjectRow> = q(
        db,
        &format!("{PROJECT_SELECT} WHERE p.id = ?2"),
        &[s(&user.id), s(&project.id)],
    )?
    .first()
    .await?;
    Ok(theirs.and_then(|r| crate::access::project_role(user, settings, None, &r)))
}

/// Whether `user` may message session `session_id`.
async fn may_message(db: &D1Database, user: &UserRow, session_id: &str) -> AppResult<bool> {
    let settings = crate::instance::settings(db).await?;
    let role = crate::access::session_role_now(db, user, &settings, None, session_id).await?;
    Ok(role.is_some_and(|r| r.can_message()))
}

/// The link that links someone's Linear account, for Linear's button.
fn link_url(cfg: &Config, thread: &str) -> String {
    cfg.url(&format!("/api/v1/linear/link?thread={thread}"))
}

/// Takes a new agent session (or one that waited) up ([`take_up`]). When
/// that fails, it says so in the agent session rather than leave it hanging
/// on "Looking at…", and keeps the delivery for an hour, so that a reply
/// there tries again (`prompted`). Failing after the session started, or
/// while it waits for its person, changes nothing there.
#[allow(clippy::too_many_arguments)]
async fn take(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    linear: &Linear<'_>,
    e: &AgentSessionEvent,
    raw: &Value,
    chosen: Option<&str>,
    now: i64,
) -> AppResult<()> {
    let Err(err) = take_up(env, cfg, db, linear, e, raw, chosen, now).await else {
        return Ok(());
    };
    let id = e.agent_session.id.as_str();
    if thread(db, id)
        .await?
        .is_some_and(|t| t.state == "started" || t.state == "waiting")
    {
        return Err(err);
    }
    let key = e
        .agent_session
        .issue
        .as_ref()
        .map_or("this issue", |i| i.identifier.as_str());
    // fctl's own errors say nothing people can act on.
    let why = (err.0.code != ErrorCode::Internal).then_some(err.0.message.as_str());
    wait(db, id, "refused", "retry", None, raw, now).await?;
    post(
        linear,
        id,
        &Act::Error(&lin::couldnt_start(key, why)),
        false,
    )
    .await;
    Err(err)
}

/// Takes a new agent session (or one that waited) to a session: as its
/// person, in the project that takes its issue, a new one or the one that
/// works on the issue already. `chosen` answers which project, when it
/// asked.
#[allow(clippy::too_many_arguments)]
async fn take_up(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    linear: &Linear<'_>,
    e: &AgentSessionEvent,
    raw: &Value,
    chosen: Option<&str>,
    now: i64,
) -> AppResult<()> {
    let session = &e.agent_session;
    let id = session.id.as_str();
    let Some(issue_ref) = session.issue.as_ref() else {
        return Ok(());
    };
    let key = issue_ref.identifier.as_str();
    let creator = session
        .creator_id
        .as_deref()
        .or(session.creator.as_ref().map(|c| c.id.as_str()));
    let Some(user) = (match creator {
        Some(c) => person(db, &e.organization_id, c).await?,
        None => None,
    }) else {
        wait(db, id, "waiting", "link", creator, raw, now).await?;
        let input = lin::link_account(
            id,
            &format!(
                "fctl doesn't know who you are in Linear yet. Connect your Linear account to fctl once, and fctl starts on {key} at once, as you, on your Claude subscription."
            ),
            &link_url(cfg, id),
            creator,
        );
        if let Err(e) = linear.activity(input).await {
            worker::console_error!("asking to link a Linear account: {}", e.0.message);
        }
        return Ok(());
    };
    let refuse = |why: String| async move {
        set_state(db, id, "refused", None, now).await?;
        post(linear, id, &Act::Error(&why), false).await;
        Ok::<(), AppError>(())
    };
    let data = linear
        .graphql(&lin::issue_query(), json!({ "id": issue_ref.id }))
        .await?;
    let issue = data.get("issue").cloned().unwrap_or(Value::Null);
    let ticket = lin::ticket(&issue).ok_or_else(|| internal("Linear's issue is missing"))?;
    let (team, linear_project) = lin::place(&issue);
    let Some(team) = team else {
        return refuse(format!("fctl couldn't read {key}'s team.")).await;
    };
    let all = takers(db, &e.organization_id).await?;
    // What they may do in each: a project they can't start sessions in
    // hides none they can, and isn't named to them unless they see it, as
    // projects of other fctl workspaces connected to this Linear workspace
    // aren't (`fctl_domain::linear::takers`).
    let settings = crate::instance::settings(db).await?;
    let mut candidates = Vec::with_capacity(all.len());
    for t in &all {
        let role = role_in(db, &user, &settings, &t.row).await?;
        candidates.push(Candidate {
            project_id: t.row.id.clone(),
            team: t.linear.team.clone(),
            project: t.linear.project.clone(),
            repo: t.row.repo().map(|r| r.full_name()),
            may_start: fctl_domain::perm::can_start_sessions(role),
            may_see: role.is_some(),
        });
    }
    let team_name = || {
        ticket
            .place
            .as_deref()
            .and_then(|p| p.split(" · ").next())
            .unwrap_or("its team")
            .to_owned()
    };
    let name_of = |c: &Candidate| {
        all.iter()
            .find(|t| t.row.id == c.project_id)
            .map(|t| t.row.name.clone())
    };
    let matching = match lin::takers(&candidates, &team, linear_project.as_deref()) {
        lin::Takers::These(these) => these,
        lin::Takers::Nobody => {
            let team_name = team_name();
            return refuse(format!(
                "No fctl project takes {team_name}'s tickets yet. In fctl, choose Linear as a project's ticket tracker, with {team_name}, in its Tickets tab: {}",
                cfg.url("/")
            ))
            .await;
        }
        lin::Takers::NotTheirs(seen) => {
            let names: Vec<String> = seen.into_iter().filter_map(name_of).collect();
            return refuse(if names.is_empty() {
                format!(
                    "{}, you can't start sessions in the fctl project that takes {}'s tickets: its owners make you an editor of it.",
                    user.name,
                    team_name()
                )
            } else {
                format!(
                    "{}, you can't start sessions in {}, which takes these tickets in fctl: its owners make you an editor of it.",
                    user.name,
                    names.join(" or ")
                )
            })
            .await;
        }
    };
    let allowed: Vec<&Taker> = matching
        .iter()
        .filter_map(|c| all.iter().find(|t| t.row.id == c.project_id))
        .collect();
    let explore_of = |t: &Taker| tk::explores(&ticket.labels, t.linear.explore_label());
    let comment = session
        .comment
        .as_ref()
        .and_then(|c| c.body.as_deref())
        .map(str::trim)
        .filter(|c| !c.is_empty());
    // A comment with words in it mentioned fctl; without one, the issue was
    // delegated to it. Only an issue whose delegate is fctl's app user moves
    // as its session starts, not one delegated to another agent.
    let delegated = comment.is_none();
    let delegated_to_fctl = match e.app_user_id.as_deref() {
        Some(app) => issue.pointer("/delegate/id").and_then(Value::as_str) == Some(app),
        None => ticket.delegate.as_ref().is_some_and(|d| d.app),
    };
    // The session that works on the issue already, if they may message it.
    if let Some((session_id, number, slug)) = working_on(db, &ticket.id, &allowed).await?
        && may_message(db, &user, &session_id).await?
    {
        set_state(db, id, "started", Some(&session_id), now).await?;
        let st = ticket.session_ticket();
        let text = lin::task(&lin::Asked {
            ticket: &st,
            by: &user.name,
            comment,
            prompt_context: e.prompt_context.as_deref(),
            description: ticket.description.as_deref(),
            explore: false,
        });
        let message = TicketMessage {
            ticket: st.clone(),
            event: if comment.is_some() {
                TicketEvent::Mentioned
            } else {
                TicketEvent::Delegated
            },
            by: session.creator.as_ref().map(lin::Person::to_ticket_person),
            description: ticket.description.clone(),
            body: comment.map(str::to_owned),
            url: None,
            state: ticket.state.clone(),
            labels: ticket.labels.clone(),
            explore: false,
        };
        deliver(
            env,
            &session_id,
            text,
            user.to_ref(),
            message,
            Some(format!("linear:{id}")),
        )
        .await?;
        if let Err(err) = linear
            .update_session(
                id,
                json!({ "externalUrls": [session_link(cfg, &session_id)] }),
            )
            .await
        {
            worker::console_error!("linking Linear's agent session: {}", err.0.message);
        }
        post(
            linear,
            id,
            &Act::Thought(&format!(
                "Carrying on in fctl session {slug}#{number}, which works on {key} already."
            )),
            false,
        )
        .await;
        return Ok(());
    }
    let taker = match allowed.as_slice() {
        [only] => *only,
        _ => {
            let by_answer = chosen.and_then(|c| {
                allowed.iter().copied().find(|t| {
                    t.row.slug.eq_ignore_ascii_case(c) || t.row.name.eq_ignore_ascii_case(c)
                })
            });
            match by_answer {
                Some(t) => t,
                None => match suggested(linear, &issue_ref.id, id, &allowed).await {
                    Some(t) => t,
                    None => {
                        wait(db, id, "waiting", "project", None, raw, now).await?;
                        let options: Vec<(String, String)> = allowed
                            .iter()
                            .map(|t| (t.row.name.clone(), t.row.slug.clone()))
                            .collect();
                        let input = lin::choose_project(
                            id,
                            &format!(
                                "Several fctl projects take {key}'s team: which should work on it?"
                            ),
                            &options,
                        );
                        if let Err(e) = linear.activity(input).await {
                            worker::console_error!("asking for a project: {}", e.0.message);
                        }
                        return Ok(());
                    }
                },
            }
        }
    };
    let explore = explore_of(taker);
    let project = &taker.row;
    let Some(credential) =
        crate::api::sessions::credential_for(db, &user, project.workspace_id()).await?
    else {
        return refuse(format!(
            "{}, connect your Claude subscription in fctl first (Settings → Claude): sessions run on it. Then delegate {key} again.",
            user.name
        ))
        .await;
    };
    let settings = project.settings();
    let github_access = fctl_domain::limits::session_github(None, settings.github_access, None)
        .map_err(AppError)?;
    let st = ticket.session_ticket();
    let text = lin::task(&lin::Asked {
        ticket: &st,
        by: &user.name,
        comment,
        prompt_context: e.prompt_context.as_deref(),
        description: ticket.description.as_deref(),
        explore,
    });
    let title = fctl_domain::validate::session_title(&ticket.title)
        .unwrap_or_else(|_| fctl_domain::naming::title_from_prompt(&ticket.title));
    // Linear asks agents to start what is delegated to them: before the
    // session, so that its agent hears of it.
    let moved = if delegated && delegated_to_fctl && taker.linear.moves_started() {
        move_started(linear, &ticket, &team).await
    } else {
        None
    };
    let column = TicketColumn {
        ticket: st.clone(),
        organization: Some(e.organization_id.clone()),
        explore,
        moved_to: moved.as_ref().map(|m| m.name.clone()),
    };
    let (session_id, number) = crate::starts::start(
        env,
        db,
        crate::starts::NewSession {
            project,
            title,
            branch: crate::starts::Branch::Ticket(&ticket.key),
            base_branch: project.default_branch.clone(),
            model: None,
            effort: None,
            user: &user,
            credential: &credential,
            github_access,
            prompt: text,
            author: Some(user.to_ref()),
            source: MessageSource::Api,
            ticket_message: Some(TicketMessage {
                ticket: st.clone(),
                event: if comment.is_some() {
                    TicketEvent::Mentioned
                } else {
                    TicketEvent::Delegated
                },
                by: session.creator.as_ref().map(lin::Person::to_ticket_person),
                description: ticket.description.clone(),
                body: comment.map(str::to_owned),
                url: None,
                state: ticket.state.clone(),
                labels: ticket.labels.clone(),
                explore,
            }),
            schedule_id: None,
            ticket: Some(serde_json::to_value(&column)?),
            follow: true,
            now,
        },
    )
    .await?;
    set_state(db, id, "started", Some(session_id.as_str()), now).await?;
    // Handing fctl a ticket is starting a session themselves.
    if let Err(e) = crate::auth::mark_active(db, &user.id, now).await {
        worker::console_error!("noting when a person was last active: {}", e.0.message);
    }
    audit(
        db,
        project.workspace_id(),
        &user.id,
        &session_id,
        json!({
            "project": project.slug,
            "number": number,
            "github_access": github_access,
            "ticket": ticket.key,
            "linear_session": id,
        }),
        now,
    )
    .await;
    if let Err(err) = linear
        .update_session(
            id,
            json!({ "externalUrls": [session_link(cfg, session_id.as_str())] }),
        )
        .await
    {
        worker::console_error!("linking Linear's agent session: {}", err.0.message);
    }
    if let Some(state) = &moved {
        notice(
            env,
            session_id.as_str(),
            "ticket_moved",
            format!("Moved {key} to {} in Linear", state.name),
            TicketNotice {
                tracker: Tracker::Linear,
                ticket: TicketLink {
                    state: Some(state.clone()),
                    ..ticket.link()
                },
                body: None,
                url: None,
                state: Some(state.clone()),
            },
        )
        .await;
    }
    let what = if explore {
        "Exploring this"
    } else {
        "Working on this"
    };
    post(
        linear,
        id,
        &Act::Thought(&format!(
            "{what} in fctl session {}#{number}, as {}.",
            project.slug, user.name
        )),
        false,
    )
    .await;
    Ok(())
}

/// The session that works on Linear issue `issue_id` in one of `projects`:
/// the newest that isn't archived or failed. Its id, number and project.
async fn working_on(
    db: &D1Database,
    issue_id: &str,
    projects: &[&Taker],
) -> AppResult<Option<(String, i64, String)>> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
        #[serde(deserialize_with = "crate::db::int")]
        number: i64,
        slug: String,
    }
    let ids: Vec<&str> = projects.iter().map(|t| t.row.id.as_str()).collect();
    let row: Option<Row> = q(
        db,
        "SELECT s.id AS id, s.number AS number, p.slug AS slug FROM sessions s JOIN projects p ON p.id = s.project_id \
         WHERE json_extract(s.ticket, '$.tracker') = 'linear' AND json_extract(s.ticket, '$.id') = ? \
         AND s.project_id IN (SELECT value FROM json_each(?)) AND s.archived = 0 AND s.status != 'failed' \
         ORDER BY s.created_at DESC LIMIT 1",
        &[s(issue_id), s(&serde_json::to_string(&ids)?)],
    )?
    .first()
    .await?;
    Ok(row.map(|r| (r.id, r.number, r.slug)))
}

/// The project Linear thinks an issue is about, among several that take its
/// team: by their primary repositories.
async fn suggested<'t>(
    linear: &Linear<'_>,
    issue: &str,
    session: &str,
    takers: &[&'t Taker],
) -> Option<&'t Taker> {
    let candidates: Vec<Value> = takers
        .iter()
        .filter_map(|t| t.row.repo())
        .map(|r| json!({ "hostname": "github.com", "repositoryFullName": r.full_name() }))
        .collect();
    if candidates.len() < 2 {
        return None;
    }
    let answer = linear
        .graphql(
            lin::REPOSITORY_SUGGESTIONS,
            json!({ "issueId": issue, "agentSessionId": session, "candidates": candidates }),
        )
        .await
        .ok()?;
    let repo = lin::suggested(&answer)?;
    takers.iter().copied().find(|t| {
        t.row
            .repo()
            .is_some_and(|r| r.full_name().eq_ignore_ascii_case(&repo))
    })
}

/// Moves a ticket to its team's first started state, unless it started
/// already: the state it moved to.
async fn move_started(
    linear: &Linear<'_>,
    ticket: &fctl_proto::tickets::Ticket,
    team: &str,
) -> Option<fctl_proto::tickets::TicketState> {
    let data = linear
        .graphql(lin::TEAM_STATES, json!({ "id": team }))
        .await
        .ok()?;
    let states = lin::states(data.get("team")?);
    let ids: Vec<(String, fctl_proto::tickets::StateKind)> = states
        .iter()
        .filter_map(|s| Some((s.id.clone()?, s.kind)))
        .collect();
    let target = tk::start_state(ticket.state.as_ref().map(|s| s.kind), &ids)?;
    if let Err(e) = linear
        .graphql(
            &lin::issue_update(),
            json!({ "id": ticket.id, "input": { "stateId": target } }),
        )
        .await
    {
        worker::console_error!("moving {} to started: {}", ticket.key, e.0.message);
        return None;
    }
    states.into_iter().find(|s| s.id.as_deref() == Some(target))
}

/// Replies kept for people fctl doesn't know yet, by their Linear id,
/// until they link their account (`linked`) or an hour passes.
fn pending(t: &ThreadRow) -> serde_json::Map<String, Value> {
    t.payload
        .as_deref()
        .filter(|_| t.waiting_for.as_deref() == Some("reply"))
        .and_then(|p| serde_json::from_str::<Value>(p).ok())
        .and_then(|v| v.get("replies").and_then(Value::as_object).cloned())
        .unwrap_or_default()
}

/// Keeps the replies still waiting in a started thread, or none.
async fn keep_pending(
    db: &D1Database,
    id: &str,
    replies: serde_json::Map<String, Value>,
    now: i64,
) -> AppResult<()> {
    if replies.is_empty() {
        q(
            db,
            "UPDATE linear_threads SET waiting_for = NULL, payload = NULL, expires_at = NULL, updated_at = ? WHERE id = ?",
            &[n(now), s(id)],
        )?
        .run()
        .await?;
    } else {
        q(
            db,
            "UPDATE linear_threads SET waiting_for = 'reply', payload = ?, expires_at = ?, updated_at = ? WHERE id = ?",
            &[
                s(&json!({ "replies": replies }).to_string()),
                n(now + lin::WAIT_MS),
                n(now),
                s(id),
            ],
        )?
        .run()
        .await?;
    }
    Ok(())
}

/// A reply in a thread that reaches a session: the agent's next message,
/// as the person who wrote it, once fctl knows them and they may message
/// the session; or their stop, which interrupts the agent's turn.
#[allow(clippy::too_many_arguments)]
async fn reply(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    linear: &Linear<'_>,
    t: &ThreadRow,
    session_id: &str,
    activity: &lin::Activity,
    raw: &Value,
    now: i64,
) -> AppResult<()> {
    if !connected(db, session_id, &t.organization_id).await? {
        post(
            linear,
            &t.id,
            &Act::Error("The fctl workspace this session belongs to no longer works in this Linear workspace, so replies here don't reach it."),
            false,
        )
        .await;
        return Ok(());
    }
    let author = activity
        .user_id
        .as_deref()
        .or(activity.user.as_ref().map(|u| u.id.as_str()));
    let user = match author {
        Some(a) => person(db, &t.organization_id, a).await?,
        None => None,
    };
    let Some(user) = user else {
        // Kept for when they link their account, beside others' kept so.
        if let Some(a) = author {
            let mut replies = pending(t);
            replies.insert(a.to_owned(), raw.clone());
            keep_pending(db, &t.id, replies, now).await?;
        }
        let input = lin::link_account(
            &t.id,
            "fctl doesn't know who you are in Linear yet, so your reply can't reach its agent. Connect your Linear account to fctl once, and it passes your reply on.",
            &link_url(cfg, &t.id),
            author,
        );
        if let Err(e) = linear.activity(input).await {
            worker::console_error!("asking to link a Linear account: {}", e.0.message);
        }
        return Ok(());
    };
    if !may_message(db, &user, session_id).await? {
        post(
            linear,
            &t.id,
            &Act::Error(&format!(
                "{}, you can't message this session in fctl, so your reply didn't reach its agent.",
                user.name
            )),
            false,
        )
        .await;
        return Ok(());
    }
    if activity.stops() {
        interrupt(env, session_id, user.to_ref()).await?;
        post(
            linear,
            &t.id,
            &Act::Response("Stopped. Reply here to carry on."),
            false,
        )
        .await;
        return Ok(());
    }
    let body = activity.body().trim();
    if body.is_empty() {
        return Ok(());
    }
    let row = crate::spec::session_row(db, session_id).await?;
    if row.as_ref().is_some_and(|r| {
        let status = r.status();
        !status.accepts_messages() && !status.resumes_for_messages()
    }) {
        post(
            linear,
            &t.id,
            &Act::Error(&format!(
                "The fctl session failed, so your reply didn't reach its agent: resume it in fctl to carry on, at {}",
                cfg.url(&format!("/s/{session_id}"))
            )),
            false,
        )
        .await;
        return Ok(());
    }
    let text = lin::replied(&user.name, &t.issue_key, body);
    let st = row
        .as_ref()
        .and_then(|r| r.ticket.as_deref())
        .and_then(|j| serde_json::from_str::<TicketColumn>(j).ok())
        .map(|c| c.ticket)
        .unwrap_or_else(|| SessionTicket {
            tracker: Tracker::Linear,
            id: String::new(),
            key: t.issue_key.clone(),
            title: None,
            url: String::new(),
        });
    let message = TicketMessage {
        ticket: st,
        event: TicketEvent::Replied,
        by: activity
            .user
            .as_ref()
            .map(lin::Person::to_ticket_person)
            .or_else(|| {
                Some(TicketPerson {
                    name: user.name.clone(),
                    handle: None,
                    avatar_url: user.avatar_url.clone(),
                    app: false,
                })
            }),
        description: None,
        body: Some(body.to_owned()),
        url: None,
        state: None,
        labels: Vec::new(),
        explore: false,
    };
    match deliver(
        env,
        session_id,
        text,
        user.to_ref(),
        message,
        Some(format!("linear:{}", activity.id)),
    )
    .await
    {
        Ok(()) => {
            // Theirs no longer waits; others' replies still do.
            let mut replies = pending(t);
            if author.is_some_and(|a| replies.remove(a).is_some()) {
                keep_pending(db, &t.id, replies, now).await?;
            }
            post(
                linear,
                &t.id,
                &Act::Thought("Passed your reply on to the agent."),
                true,
            )
            .await;
        }
        Err(e) => {
            post(
                linear,
                &t.id,
                &Act::Error(&format!(
                    "fctl couldn't pass your reply on: {}",
                    e.0.message
                )),
                false,
            )
            .await;
        }
    }
    Ok(())
}

/// Hands a message from a tracker to a session's agent, resuming the
/// session if it stopped, as a person's message does.
async fn deliver(
    env: &Env,
    session_id: &str,
    text: String,
    author: UserRef,
    ticket: TicketMessage,
    client_key: Option<String>,
) -> AppResult<()> {
    let id = fctl_domain::ids::delivery_id(crate::ctx::now_ms());
    let stub = crate::internal::session_stub(env, session_id)?;
    let _: Delivered = crate::internal::post(
        &stub,
        "/deliver",
        &Deliver {
            delivery: Delivery {
                id: id.clone(),
                kind: DeliveryKind::Message {
                    text: text.clone(),
                    author: Some(author.clone()),
                    attachments: Vec::new(),
                },
            },
            client_key,
            event: EventBody::UserMessage(UserMessage {
                id,
                author: Some(author),
                text,
                source: MessageSource::Api,
                attachments: Vec::new(),
                follow_up: None,
                review: None,
                ticket: Some(ticket),
            }),
        },
    )
    .await?;
    Ok(())
}

/// Interrupts a session's turn, as its person's stop in Linear.
async fn interrupt(env: &Env, session_id: &str, by: UserRef) -> AppResult<()> {
    let id = fctl_domain::ids::delivery_id(crate::ctx::now_ms());
    let stub = crate::internal::session_stub(env, session_id)?;
    let _: Delivered = crate::internal::post(
        &stub,
        "/deliver",
        &Deliver {
            delivery: Delivery {
                id: id.clone(),
                kind: DeliveryKind::Interrupt {
                    by: Some(by.clone()),
                },
            },
            client_key: None,
            event: EventBody::Interrupt(fctl_proto::event::Interrupt { id, by: Some(by) }),
        },
    )
    .await?;
    Ok(())
}

/// Someone linked their Linear account in `organization`: what waited for
/// them goes on. Returns the sessions it started or reached.
pub async fn linked(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    organization: &str,
    linear_user: &str,
    now: i64,
) -> AppResult<Vec<String>> {
    let waiting: Vec<ThreadRow> = q(
        db,
        &format!(
            "{THREAD_SELECT} WHERE organization_id = ? AND expires_at > ? AND waiting_for IS NOT NULL ORDER BY created_at"
        ),
        &[s(organization), n(now)],
    )?
    .all()
    .await?;
    let keys = crypto::keys(cfg)?;
    let mut reached = Vec::new();
    for t in waiting {
        // Their delegation that waited for them, or their reply kept in a
        // started thread among others'.
        let raw = match t.waiting_for.as_deref() {
            Some("link") if t.linear_user_id.as_deref() == Some(linear_user) => t
                .payload
                .as_deref()
                .and_then(|p| serde_json::from_str::<Value>(p).ok()),
            Some("reply") => pending(&t).remove(linear_user),
            _ => None,
        };
        let Some(raw) = raw else { continue };
        let Some(e) = event_of(&raw) else { continue };
        let linear = match linear::client(db, cfg, &keys, organization, now).await {
            Ok(l) => l,
            Err(e) => {
                worker::console_error!("linking: no Linear client: {}", e.0.message);
                break;
            }
        };
        let result = if t.waiting_for.as_deref() == Some("reply") {
            match (t.session_id.as_deref(), e.agent_activity.as_ref()) {
                (Some(session_id), Some(a)) => {
                    reply(env, cfg, db, &linear, &t, session_id, a, &raw, now).await
                }
                _ => Ok(()),
            }
        } else {
            take(env, cfg, db, &linear, &e, &raw, None, now).await
        };
        if let Err(err) = result {
            worker::console_error!("taking up {} after a link: {}", t.id, err.0.message);
        }
        if let Some(after) = thread(db, &t.id).await?
            && after.state == "started"
            && let Some(sid) = after.session_id
        {
            reached.push(sid);
        }
    }
    Ok(reached)
}

/// fctl's app was uninstalled from a Linear workspace, or its access
/// revoked there.
pub async fn revoked(db: &D1Database, organization: &str, now: i64) -> AppResult<()> {
    linear::mark_revoked(db, organization, now).await
}

async fn audit(
    db: &D1Database,
    workspace_id: &str,
    actor: &str,
    session_id: &fctl_proto::SessionId,
    data: Value,
    now: i64,
) {
    let written = async {
        q(
            db,
            "INSERT INTO audit_log (id, ts, actor_id, action, target, data, ip, workspace_id) VALUES (?, ?, ?, 'session.created', ?, ?, NULL, ?)",
            &[
                s(&fctl_domain::ids::new_id("aud", now)),
                n(now),
                s(actor),
                s(session_id.as_str()),
                s(&data.to_string()),
                os(Some(workspace_id).filter(|w| !w.is_empty())),
            ],
        )?
        .run()
        .await
    }
    .await;
    if let Err(e) = written {
        worker::console_error!("audit for a delegated session: {}", e.0.message);
    }
}

// ───────────────────────────── to Linear ─────────────────────────────

/// A session that answers in a Linear thread: the thread, and fctl's app
/// there.
struct Answering<'a> {
    thread: ThreadRow,
    linear: Linear<'a>,
}

async fn answering<'a>(
    db: &D1Database,
    cfg: &'a Config,
    session_id: &str,
    now: i64,
) -> AppResult<Option<Answering<'a>>> {
    let Some(thread) = thread_of(db, session_id).await? else {
        return Ok(None);
    };
    let keys = crypto::keys(cfg)?;
    let linear = linear::client(db, cfg, &keys, &thread.organization_id, now).await?;
    Ok(Some(Answering { thread, linear }))
}

/// Adds a notice to a session's transcript, saying what fctl did in its
/// tracker for it.
pub async fn notice(
    env: &Env,
    session_id: &str,
    code: &str,
    message: String,
    ticket: TicketNotice,
) {
    let n = Notice {
        level: NoticeLevel::Info,
        code: code.to_owned(),
        message,
        until: None,
        published: None,
        memory: None,
        retrospective: None,
        review: None,
        follow_up: None,
        ticket: Some(ticket),
    };
    let sent = async {
        let stub = crate::internal::session_stub(env, session_id)?;
        let _: Value = crate::internal::post(&stub, "/notice", &n).await?;
        Ok::<(), AppError>(())
    }
    .await;
    if let Err(e) = sent {
        worker::console_error!("a ticket notice for {session_id}: {}", e.0.message);
    }
}

/// A turn of a session that answers in a Linear thread ended: the agent's
/// last message goes there, as Linear's response, or its question.
pub async fn turn_ended(env: &Env, session_id: &str, text: &str) -> AppResult<()> {
    let cfg = Config::from_env(env);
    let db = env.d1("DB")?;
    let now = crate::ctx::now_ms();
    let Some(a) = answering(&db, &cfg, session_id, now).await? else {
        return Ok(());
    };
    let url = cfg.url(&format!("/s/{session_id}"));
    let Some((kind, body)) = tk::reply(text, &url) else {
        return Ok(());
    };
    let act = match kind {
        tk::ReplyKind::Question => Act::Elicitation(&body),
        tk::ReplyKind::Response => Act::Response(&body),
    };
    a.linear
        .activity(lin::activity(&a.thread.id, &act, false))
        .await?;
    let row = crate::spec::session_row(&db, session_id).await?;
    let ticket = row
        .and_then(|r| r.ticket)
        .and_then(|j| serde_json::from_str::<TicketColumn>(&j).ok())
        .map(|c| c.ticket);
    if let Some(t) = ticket {
        notice(
            env,
            session_id,
            "ticket_replied",
            format!("Replied on {} in Linear", t.key),
            TicketNotice {
                tracker: Tracker::Linear,
                ticket: TicketLink {
                    key: t.key.clone(),
                    title: t.title.clone().unwrap_or_default(),
                    url: t.url.clone(),
                    state: None,
                },
                body: Some(body.clone()),
                url: None,
                state: None,
            },
        )
        .await;
    }
    Ok(())
}

/// A session that answers in a Linear thread failed, or paused on a usage
/// limit: Linear's session says so, and when a paused one carries on, in
/// the time zone of the person who made the agent session.
pub async fn halted(
    env: &Env,
    session_id: &str,
    status: SessionStatus,
    reason: Option<&str>,
    until: Option<i64>,
) -> AppResult<()> {
    let cfg = Config::from_env(env);
    let db = env.d1("DB")?;
    let now = crate::ctx::now_ms();
    let Some(a) = answering(&db, &cfg, session_id, now).await? else {
        return Ok(());
    };
    let text = match status {
        SessionStatus::Failed => format!(
            "The fctl session failed{}. People who manage it can resume it in fctl.",
            reason.map(|r| format!(": {r}")).unwrap_or_default()
        ),
        SessionStatus::Paused => {
            let zone = if until.is_some() {
                creator_zone(&a).await
            } else {
                None
            };
            lin::paused(until, zone.as_ref().map(|z| z as &dyn Zone))
        }
        _ => return Ok(()),
    };
    let act = if status == SessionStatus::Failed {
        Act::Error(&text)
    } else {
        Act::Thought(&text)
    };
    a.linear
        .activity(lin::activity(&a.thread.id, &act, false))
        .await
}

/// The time zone of the person who made a thread's agent session, from
/// their Linear settings: `None` when they have none there, `Intl` doesn't
/// know it, or Linear doesn't answer.
async fn creator_zone(a: &Answering<'_>) -> Option<IntlZone> {
    let answer = a
        .linear
        .graphql(lin::CREATOR_TIMEZONE, json!({ "id": a.thread.id }))
        .await
        .inspect_err(|e| worker::console_error!("reading a time zone in Linear: {}", e.0.message))
        .ok()?;
    IntlZone::new(lin::creator_timezone(&answer)?)
}

/// A session that answers in a Linear thread found pull requests: Linear's
/// session links them, and the issue lists them.
pub async fn pull_requests(env: &Env, session_id: &str, found: &[PullRequestRef]) -> AppResult<()> {
    if found.is_empty() {
        return Ok(());
    }
    let cfg = Config::from_env(env);
    let db = env.d1("DB")?;
    let now = crate::ctx::now_ms();
    let Some(a) = answering(&db, &cfg, session_id, now).await? else {
        return Ok(());
    };
    let row = crate::spec::session_row(&db, session_id).await?;
    let all: Vec<PullRequestRef> = row
        .as_ref()
        .map(|r| r.to_session(None).pull_requests)
        .unwrap_or_default();
    let issue = row
        .and_then(|r| r.ticket)
        .and_then(|j| serde_json::from_str::<TicketColumn>(&j).ok())
        .map(|c| c.ticket.id)
        .filter(|i| !i.is_empty());
    let mut links = vec![session_link(&cfg, session_id)];
    for pr in all.iter().chain(found.iter()) {
        if !links.iter().any(|l| l.get("url") == Some(&json!(pr.url))) {
            links.push(json!({ "label": pull_label(pr), "url": pr.url }));
        }
    }
    if let Err(e) = a
        .linear
        .update_session(&a.thread.id, json!({ "externalUrls": links }))
        .await
    {
        worker::console_error!("linking pull requests in Linear: {}", e.0.message);
    }
    for pr in found {
        let label = pull_label(pr);
        a.linear
            .activity(lin::activity(
                &a.thread.id,
                &Act::Action {
                    action: "Opened a pull request",
                    parameter: &label,
                    result: Some(&pr.url),
                },
                false,
            ))
            .await?;
        if let Some(issue) = issue.as_deref()
            && let Err(e) = a
                .linear
                .graphql(
                    lin::ATTACHMENT_LINK_URL,
                    json!({ "issueId": issue, "url": pr.url, "title": pr.title.clone().unwrap_or_else(|| label.clone()) }),
                )
                .await
        {
            worker::console_error!("attaching a pull request in Linear: {}", e.0.message);
        }
    }
    Ok(())
}

fn pull_label(pr: &PullRequestRef) -> String {
    let repo = fctl_domain::repos::parse_remote(&pr.url).map(|r| r.full_name());
    match repo {
        Some(r) => format!("{r}#{}", pr.number),
        None => format!("#{}", pr.number),
    }
}

/// A step of the agent's (one of its tool calls) for Linear's live line,
/// at most every few minutes: it keeps Linear from calling the session
/// stale while the agent works, and shows what it is at.
pub async fn step(env: &Env, session_id: &str, action: &str, parameter: &str) -> AppResult<()> {
    let cfg = Config::from_env(env);
    let db = env.d1("DB")?;
    let now = crate::ctx::now_ms();
    let Some(a) = answering(&db, &cfg, session_id, now).await? else {
        return Ok(());
    };
    if a.thread
        .stepped_at
        .is_some_and(|at| now - at < lin::STEP_EVERY_MS)
    {
        return Ok(());
    }
    q(
        &db,
        "UPDATE linear_threads SET stepped_at = ? WHERE id = ?",
        &[n(now), s(&a.thread.id)],
    )?
    .run()
    .await?;
    a.linear
        .activity(lin::activity(
            &a.thread.id,
            &Act::Action {
                action,
                parameter,
                result: None,
            },
            true,
        ))
        .await
}

/// The agent's plan (Claude Code's todo list) for Linear's session.
pub async fn plan(env: &Env, session_id: &str, plan: Value) -> AppResult<()> {
    let cfg = Config::from_env(env);
    let db = env.d1("DB")?;
    let now = crate::ctx::now_ms();
    let Some(a) = answering(&db, &cfg, session_id, now).await? else {
        return Ok(());
    };
    a.linear
        .update_session(&a.thread.id, json!({ "plan": plan }))
        .await
}

/// Gives up fctl's access to the Linear workspaces no fctl workspace works
/// in any more: one whose last workspace was deleted (the hourly job).
pub async fn uninstall_unused(env: &Env, db: &D1Database) -> AppResult<()> {
    #[derive(Deserialize)]
    struct Row {
        organization_id: String,
    }
    let unused: Vec<Row> = q(
        db,
        "SELECT organization_id FROM linear_installations i WHERE NOT EXISTS \
         (SELECT 1 FROM workspace_linear wl WHERE wl.organization_id = i.organization_id) LIMIT 20",
        &[],
    )?
    .all()
    .await?;
    if unused.is_empty() {
        return Ok(());
    }
    let cfg = Config::from_env(env);
    let keys = crypto::keys(&cfg)?;
    for r in unused {
        linear::uninstall_if_unused(db, &cfg, &keys, &r.organization_id).await?;
    }
    Ok(())
}

/// Forgets what waited longer than an hour (the hourly job).
pub async fn expire(db: &D1Database, now: i64) -> AppResult<()> {
    q(
        db,
        "UPDATE linear_threads SET state = CASE WHEN state = 'waiting' THEN 'expired' ELSE state END, \
         waiting_for = NULL, payload = NULL, expires_at = NULL WHERE expires_at IS NOT NULL AND expires_at <= ?",
        &[n(now)],
    )?
    .run()
    .await?;
    q(
        db,
        "DELETE FROM linear_threads WHERE state IN ('expired', 'refused', 'new') AND updated_at < ?",
        &[n(now - 30 * 24 * 60 * 60 * 1000)],
    )?
    .run()
    .await?;
    q(
        db,
        "DELETE FROM ticket_writes WHERE at < ?",
        &[n(now - 60 * 60 * 1000)],
    )?
    .run()
    .await?;
    Ok(())
}
