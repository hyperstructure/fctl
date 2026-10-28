//! A project's ticket tracker (docs/architecture.md#ticket-trackers): what
//! its sessions' agents are told of it (`SessionSpec::tickets`), what they
//! read and write with `fctl ticket` (`AgentCall::Ticket`), and how it
//! works now, for its Tickets tab. The control plane holds the tracker's
//! credentials: fctl's Linear app in the project's Linear workspace
//! (`crate::linear`), or a token of the GitHub App's for one repository's
//! issues alone, which never leaves it and which it revokes at once. Every
//! call stays within what the project chose: its Linear team (and
//! project), or its repositories.

use crate::api::calls::RunnerSession;
use crate::config::Config;
use crate::crypto::{self, Keys};
use crate::ctx::Ctx;
use crate::db::{Count, n, q, s};
use crate::delegations::{self, TicketColumn};
use crate::error::{AppError, AppResult, forbidden, internal, not_found, validation};
use crate::github;
use crate::linear::{self, Linear};
use crate::model::{ProjectRow, SessionRow};
use fctl_domain::issues;
use fctl_domain::linear as lin;
use fctl_domain::repos;
use fctl_domain::tickets::{self as tk, GithubIssues};
use fctl_proto::runner::GithubCredential;
use fctl_proto::session::{
    GithubAccess, LinearTickets, ProjectSettings, RepoGrant, RepoRef, SessionTicket, Tickets,
    Tracker,
};
use fctl_proto::tickets::{
    LinearConnection, LinearStatus, LinearTeam, ProjectTickets, StateFilter, Ticket, TicketAnswer,
    TicketLink, TicketNotice, TicketRequest, TicketRun, TicketState, TicketsSpec, TrackerAbout,
};
use fctl_proto::{ApiError, ErrorCode};
use serde::Deserialize;
use serde_json::{Value, json};
use worker::{D1Database, Method};

// ───────────────────────────── what agents are told ─────────────────────────────

/// What a session's agent is told about its project's tracker, and the
/// ticket it started from. `None` for review agents, whose spec has none.
pub async fn spec(
    db: &D1Database,
    row: &SessionRow,
    project: &ProjectRow,
    settings: &ProjectSettings,
    grants: &[RepoGrant],
    access: GithubAccess,
) -> AppResult<TicketsSpec> {
    let t = &settings.tickets;
    let primary = project.repo();
    let mut via = tk::session_via(t, primary.as_ref(), grants, access, row.github_issues);
    let linear_workspace = match (&t.tracker, &t.linear) {
        (Tracker::Linear, Some(l)) => {
            // A workspace that disconnected it reaches none of its tickets.
            if !works_in(db, project.workspace_id(), &l.organization).await? {
                via = fctl_proto::tickets::TicketsVia::Nothing;
            }
            linear::installation(db, &l.organization)
                .await?
                .map(|i| i.name)
        }
        _ => None,
    };
    let column = row
        .ticket
        .as_deref()
        .and_then(|j| serde_json::from_str::<TicketColumn>(j).ok());
    let by = row.creator_name.clone().unwrap_or_else(|| "Someone".into());
    let started_from = column.as_ref().map(|c| tk::StartedFrom {
        ticket: &c.ticket,
        by: &by,
        mentioned: c.mentioned,
        explore: c.explore,
        moved_to: c.moved_to.as_deref(),
    });
    let instructions = tk::instructions(&tk::Told {
        tickets: t,
        via,
        primary: primary.as_ref(),
        grants,
        access,
        github_issues: row.github_issues,
        linear_workspace: linear_workspace.as_deref(),
        started_from,
    });
    Ok(TicketsSpec {
        tracker: t.tracker,
        via,
        ticket: column.map(|c| c.ticket),
        instructions,
    })
}

// ───────────────────────────── how it works now ─────────────────────────────

/// The Linear workspaces connected to an fctl workspace, as its tabs show
/// them, with the projects that take tickets from each.
pub async fn connections(db: &D1Database, workspace_id: &str) -> AppResult<Vec<LinearConnection>> {
    #[derive(Deserialize)]
    struct Row {
        organization_id: String,
        name: String,
        url_key: String,
        logo_url: Option<String>,
        app_user_id: Option<String>,
        app_user_name: Option<String>,
        app_user_handle: Option<String>,
        status: String,
        linear_user: Option<String>,
        connected_by: Option<String>,
        by_name: Option<String>,
        by_avatar: Option<String>,
        #[serde(deserialize_with = "crate::db::int")]
        created_at: i64,
    }
    let rows: Vec<Row> = q(
        db,
        "SELECT i.organization_id, i.name, i.url_key, i.logo_url, i.app_user_id, i.app_user_name, i.app_user_handle, \
         i.status, wl.linear_user, \
         wl.connected_by, u.name AS by_name, u.avatar_url AS by_avatar, wl.created_at \
         FROM workspace_linear wl JOIN linear_installations i ON i.organization_id = wl.organization_id \
         LEFT JOIN users u ON u.id = wl.connected_by WHERE wl.workspace_id = ? ORDER BY wl.created_at",
        &[s(workspace_id)],
    )?
    .all()
    .await?;
    #[derive(Deserialize)]
    struct Use {
        slug: String,
        name: String,
        settings: String,
    }
    let uses: Vec<Use> = q(
        db,
        "SELECT slug, name, settings FROM projects WHERE workspace_id = ? AND archived_at IS NULL \
         AND json_extract(settings, '$.tickets.tracker') = 'linear' ORDER BY created_at",
        &[s(workspace_id)],
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let projects = uses
                .iter()
                .filter_map(|u| {
                    let t = serde_json::from_str::<ProjectSettings>(&u.settings)
                        .ok()?
                        .tickets
                        .linear?;
                    (t.organization == r.organization_id).then(|| {
                        fctl_proto::tickets::LinearProjectUse {
                            slug: u.slug.clone(),
                            name: u.name.clone(),
                            team_key: t.team_key.clone(),
                        }
                    })
                })
                .collect();
            LinearConnection {
                organization: r.organization_id,
                name: r.name,
                url_key: r.url_key,
                logo_url: r.logo_url,
                app_user: r.app_user_id,
                app_name: r.app_user_name,
                app_handle: r.app_user_handle,
                status: if r.status == "active" {
                    LinearStatus::Active
                } else {
                    LinearStatus::Revoked
                },
                connected_by: r.connected_by.map(|id| fctl_proto::session::UserRef {
                    id: fctl_proto::UserId(id),
                    name: r.by_name.unwrap_or_default(),
                    avatar_url: r.by_avatar,
                }),
                linear_user: r.linear_user,
                created_at: r.created_at,
                projects,
            }
        })
        .collect())
}

/// `GET /api/v1/projects/{id}/tickets`: the project's tracker as it works
/// now, for everyone who sees the project.
pub async fn status(ctx: &Ctx, project: &ProjectRow) -> AppResult<ProjectTickets> {
    let settings = project.settings();
    let t = &settings.tickets;
    let grants = crate::spec::grants(&ctx.db, &project.id).await?;
    let primary = project.repo();
    let via = tk::via(
        t,
        primary.as_ref(),
        &grants,
        settings.github_access.unwrap_or_default(),
    );
    let workspace_id = project.workspace_id().to_owned();
    let connections = connections(&ctx.db, &workspace_id).await?;
    // Linear's connection, for a project that takes its tickets.
    let connection = t
        .linear
        .as_ref()
        .filter(|_| t.tracker == Tracker::Linear)
        .and_then(|l| {
            connections
                .iter()
                .find(|c| c.organization == l.organization)
        })
        .cloned();
    let linked = match (
        t.linear.as_ref().filter(|_| t.tracker == Tracker::Linear),
        ctx.auth.as_ref(),
    ) {
        (Some(l), Some(a)) => {
            let c: Option<Count> = q(
                &ctx.db,
                "SELECT COUNT(*) AS n FROM linear_accounts WHERE user_id = ? AND organization_id = ?",
                &[s(&a.user.id), s(&l.organization)],
            )?
            .first()
            .await?;
            Some(c.is_some_and(|c| c.n > 0))
        }
        _ => None,
    };
    let problem = match t.tracker {
        Tracker::None => None,
        Tracker::Github => match tk::github_issues(t, primary.as_ref(), &grants) {
            GithubIssues::None if t.repo.is_some() => Some(format!(
                "Agents no longer write to {}, so they reach no issues: choose another repository.",
                t.repo.as_ref().map(RepoRef::full_name).unwrap_or_default()
            )),
            GithubIssues::None => Some(
                "The project has no repository, so its agents reach no issues: link one under Repositories."
                    .to_owned(),
            ),
            _ => None,
        },
        Tracker::Linear => match (&t.linear, &connection) {
            (None, _) => Some("Choose the Linear team whose tickets this project works on.".to_owned()),
            (Some(_), None) => Some(
                "The Linear workspace this project chose isn't connected to its workspace any more: an admin connects it again."
                    .to_owned(),
            ),
            (Some(_), Some(c)) if c.status == LinearStatus::Revoked => Some(format!(
                "Linear no longer takes fctl's access to {}: an admin of the workspace connects it again.",
                c.name
            )),
            _ if ctx.cfg.linear.as_ref().is_none_or(|l| l.webhook_secret.is_none()) => Some(
                "This instance takes no Linear webhooks yet, so delegating tickets starts nothing: its admins set LINEAR_WEBHOOK_SECRET."
                    .to_owned(),
            ),
            _ => None,
        },
    };
    let writable: Vec<RepoRef> = tk::writable(primary.as_ref(), &grants);
    let recent = recent(&ctx.db, &project.id).await?;
    Ok(ProjectTickets {
        tracker: t.tracker,
        scope: tk::scope(
            t,
            primary.as_ref(),
            &grants,
            connection.as_ref().map(|c| c.name.as_str()),
        ),
        via,
        problem,
        workspace: fctl_proto::WorkspaceId(workspace_id),
        linear: connection,
        linked,
        writable,
        recent,
    })
}

/// The sessions tickets started in a project lately, newest first.
async fn recent(db: &D1Database, project_id: &str) -> AppResult<Vec<TicketRun>> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
        #[serde(deserialize_with = "crate::db::int")]
        number: i64,
        title: String,
        status: String,
        ticket: String,
        created_by: String,
        creator_name: Option<String>,
        creator_avatar: Option<String>,
        #[serde(deserialize_with = "crate::db::int")]
        created_at: i64,
    }
    let rows: Vec<Row> = q(
        db,
        "SELECT s.id, s.number, s.title, s.status, s.ticket, s.created_by, u.name AS creator_name, \
         u.avatar_url AS creator_avatar, s.created_at FROM sessions s LEFT JOIN users u ON u.id = s.created_by \
         WHERE s.project_id = ? AND s.ticket IS NOT NULL AND s.archived = 0 ORDER BY s.created_at DESC LIMIT 10",
        &[s(project_id)],
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let ticket = serde_json::from_str::<TicketColumn>(&r.ticket).ok()?.ticket;
            Some(TicketRun {
                ticket,
                session: fctl_proto::session::SessionRef {
                    id: fctl_proto::SessionId(r.id),
                    number: u32::try_from(r.number).unwrap_or_default(),
                    title: r.title,
                },
                status: fctl_proto::session::SessionStatus::parse(&r.status)
                    .unwrap_or(fctl_proto::session::SessionStatus::Stopped),
                by: Some(fctl_proto::session::UserRef {
                    id: fctl_proto::UserId(r.created_by),
                    name: r.creator_name.unwrap_or_default(),
                    avatar_url: r.creator_avatar,
                }),
                at: r.created_at,
            })
        })
        .collect())
}

// ───────────────────────────── choosing Linear ─────────────────────────────

/// The teams fctl's app reaches in a Linear workspace, each with its
/// workflow, the state its tickets start in and its open projects. Linear
/// prices a query by how much it asks for rather than by what it finds, so
/// fctl counts the teams first and then asks for as many as there are.
pub async fn teams(linear: &Linear<'_>) -> AppResult<Vec<LinearTeam>> {
    let ids = linear.graphql(lin::TEAM_IDS, json!({})).await?;
    let mut teams = Vec::new();
    let mut after: Option<String> = None;
    for first in lin::team_pages(&ids) {
        let d = linear
            .graphql(lin::TEAMS, json!({ "first": first, "after": after }))
            .await?;
        let (page, next) = lin::teams_page(&d);
        teams.extend(page);
        let Some(next) = next else { break };
        after = Some(next);
    }
    Ok(teams)
}

/// Checks a project's Linear choice as it is saved: its Linear workspace is
/// connected to the project's workspace, and fctl's app reaches its team
/// (and project) there, whose key and names it keeps for display.
pub async fn check_linear(
    db: &D1Database,
    cfg: &Config,
    keys: &Keys,
    workspace_id: &str,
    l: &LinearTickets,
    now: i64,
) -> AppResult<LinearTickets> {
    let connected: Option<Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM workspace_linear WHERE workspace_id = ? AND organization_id = ?",
        &[s(workspace_id), s(&l.organization)],
    )?
    .first()
    .await?;
    if connected.is_none_or(|c| c.n == 0) {
        return Err(validation(
            "settings.tickets.linear",
            "This Linear workspace isn't connected to the project's workspace: an admin connects it first",
        ));
    }
    let client = linear::client(db, cfg, keys, &l.organization, now).await?;
    // Looked up by their ids, which a Linear workspace with more teams and
    // projects than the lists show has too.
    let found = |query: &'static str, id: &str, field: &'static str| {
        let client = &client;
        let id = id.to_owned();
        async move {
            match client.graphql(query, json!({ "id": id })).await {
                Ok(d) => Ok(d.get(field).filter(|v| !v.is_null()).cloned()),
                Err(e) if e.0.code == ErrorCode::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }
    };
    let team = found(lin::TEAM, &l.team, "team").await?.ok_or_else(|| {
        validation(
            "settings.tickets.linear.team",
            "fctl's app doesn't reach that team in Linear: choose another, or give the app the team in Linear's settings",
        )
    })?;
    let st = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let mut out = l.clone();
    out.team_key = st(&team, "key");
    out.team_name = st(&team, "name");
    out.project_name = match &l.project {
        Some(p) => Some(
            found(lin::PROJECT, p, "project")
                .await?
                .filter(|v| lin::open_project_of(v, &l.team))
                .and_then(|v| st(&v, "name"))
                .ok_or_else(|| {
                    validation(
                        "settings.tickets.linear.project",
                        "That Linear project isn't one of the team's open projects",
                    )
                })?,
        ),
        None => None,
    };
    Ok(out)
}

// ───────────────────────────── what agents ask ─────────────────────────────

/// The tracker a session's agent reaches, and how much of it.
enum Backend {
    /// GitHub's issues of these repositories, the first where new ones go.
    Github(Vec<RepoRef>),
    Linear(LinearTickets),
}

/// What a session's `fctl ticket` call asks, answered: a
/// [`TicketAnswer`] as JSON, for hosts to pass on as it is.
pub async fn call(ctx: &Ctx, session: &RunnerSession, request: Value) -> AppResult<Value> {
    let op = request
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let request: TicketRequest = serde_json::from_value(request).map_err(|_| {
        AppError(ApiError::new(
            ErrorCode::BadRequest,
            format!("This control plane doesn't know `fctl ticket {op}`"),
        ))
    })?;
    if session.review {
        return Err(forbidden(
            "Review agents don't reach the project's ticket tracker.",
        ));
    }
    let grants = crate::spec::grants(&ctx.db, &session.project_id).await?;
    let t = &session.settings.tickets;
    let backend = match t.tracker {
        Tracker::None => {
            return Err(forbidden(
                "This project uses no ticket tracker: its agents read and write no tickets.",
            ));
        }
        Tracker::Github => match tk::github_issues(t, session.repo.as_ref(), &grants) {
            GithubIssues::None => {
                return Err(forbidden(
                    "This project's agents reach no GitHub issues now: its ticket tracker's repository is no longer one they write to.",
                ));
            }
            GithubIssues::Writable => Backend::Github(tk::writable(session.repo.as_ref(), &grants)),
            GithubIssues::One(r) => Backend::Github(vec![r]),
        },
        Tracker::Linear => match &t.linear {
            Some(l) if works_in(&ctx.db, &session.workspace_id, &l.organization).await? => {
                Backend::Linear(l.clone())
            }
            Some(_) => {
                return Err(forbidden(
                    "This project's workspace no longer works in its Linear workspace: an admin connects it again under the workspace's Linear tab.",
                ));
            }
            None => {
                return Err(forbidden(
                    "This project hasn't chosen its Linear team yet: its maintainers choose it in the project's Tickets tab.",
                ));
            }
        },
    };
    if !request.reads() && !tk::writes(session.github_access) {
        return Err(forbidden(
            "This session's agent only reads: it can't write tickets.",
        ));
    }
    // The call's place within its limit, taken in one statement, so that
    // calls at once can't go past it together.
    let (kind, most) = tk::limit_of(&request);
    let taken = q(
        &ctx.db,
        "INSERT INTO ticket_writes (session_id, kind, at) SELECT ?1, ?2, ?3 \
         WHERE (SELECT COUNT(*) FROM ticket_writes WHERE session_id = ?1 AND kind = ?2 AND at > ?4) < ?5",
        &[
            s(&session.id),
            s(kind),
            n(ctx.now),
            n(ctx.now - 60 * 60 * 1000),
            n(i64::from(most)),
        ],
    )?
    .run()
    .await?;
    if taken == 0 {
        return Err(AppError(
            ApiError::new(ErrorCode::RateLimited, tk::limited(kind, most)).with_retry_after(600),
        ));
    }
    let answer = match backend {
        Backend::Linear(l) => {
            let keys = crypto::keys(&ctx.cfg)?;
            match linear::client(&ctx.db, &ctx.cfg, &keys, &l.organization, ctx.now).await {
                Ok(client) => linear_call(ctx, session, &client, &l, request).await,
                Err(e) => Err(e),
            }
        }
        Backend::Github(repos) => github_call(ctx, session, &repos, request).await,
    };
    match answer {
        Ok(answer) => Ok(serde_json::to_value(answer)?),
        Err(e) => {
            // A write that didn't happen doesn't count; a read cost the
            // tracker's API all the same.
            if kind != "read" {
                q(
                    &ctx.db,
                    "DELETE FROM ticket_writes WHERE rowid = (SELECT rowid FROM ticket_writes \
                     WHERE session_id = ? AND kind = ? AND at = ? LIMIT 1)",
                    &[s(&session.id), s(kind), n(ctx.now)],
                )?
                .run()
                .await?;
            }
            Err(e)
        }
    }
}

/// Whether fctl workspace `workspace_id` works in Linear workspace
/// `organization`: fctl's app may stay installed there for other fctl
/// workspaces once this one disconnected it.
pub async fn works_in(db: &D1Database, workspace_id: &str, organization: &str) -> AppResult<bool> {
    let c: Option<Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM workspace_linear WHERE workspace_id = ? AND organization_id = ?",
        &[s(workspace_id), s(organization)],
    )?
    .first()
    .await?;
    Ok(c.is_some_and(|c| c.n > 0))
}

/// The tickets a session works on: the one it started from.
async fn session_tickets(db: &D1Database, session_id: &str) -> AppResult<Vec<SessionTicket>> {
    #[derive(Deserialize)]
    struct Row {
        ticket: Option<String>,
    }
    let row: Option<Row> = q(
        db,
        "SELECT ticket FROM sessions WHERE id = ?",
        &[s(session_id)],
    )?
    .first()
    .await?;
    Ok(row
        .and_then(|r| r.ticket)
        .and_then(|j| serde_json::from_str::<TicketColumn>(&j).ok())
        .map(|c| c.ticket)
        .into_iter()
        .collect())
}

/// What a session's transcript says the agent did in its tracker.
async fn told(
    ctx: &Ctx,
    session: &RunnerSession,
    code: &str,
    message: String,
    notice: TicketNotice,
) {
    delegations::notice(&ctx.env, &session.id, code, message, notice).await;
}

fn limit(asked: Option<u32>) -> u32 {
    asked.unwrap_or(tk::DEFAULT_LIST).clamp(1, tk::MAX_LIST)
}

fn check_title(title: &str) -> AppResult<String> {
    let t = fctl_domain::validate::text("title", title, tk::MAX_TITLE)?;
    if t.is_empty() {
        return Err(validation("title", "Give the ticket a title"));
    }
    Ok(t.replace(['\n', '\r'], " "))
}

fn check_body(field: &str, body: &str) -> AppResult<String> {
    Ok(fctl_domain::validate::text(field, body, tk::MAX_BODY)?)
}

// ───────────────────────────── Linear ─────────────────────────────

/// The team's (and project's) issues as a filter, in a state.
fn linear_filter(l: &LinearTickets, state: StateFilter, label: Option<&str>) -> Value {
    let mut f = json!({ "team": { "id": { "eq": l.team } } });
    if let Some(p) = &l.project {
        put(&mut f, "project", json!({ "id": { "eq": p } }));
    }
    match state {
        StateFilter::Open => put(
            &mut f,
            "state",
            json!({ "type": { "in": ["triage", "backlog", "unstarted", "started"] } }),
        ),
        StateFilter::Closed => put(
            &mut f,
            "state",
            json!({ "type": { "in": ["completed", "canceled", "duplicate"] } }),
        ),
        StateFilter::All => {}
    }
    if let Some(label) = label.map(str::trim).filter(|l| !l.is_empty()) {
        put(
            &mut f,
            "labels",
            json!({ "name": { "eqIgnoreCase": label } }),
        );
    }
    f
}

/// A Linear issue by key, when it is one the project takes: else the same
/// answer as for one that doesn't exist, so that agents learn nothing of
/// the workspace's other teams.
async fn linear_issue(
    client: &Linear<'_>,
    l: &LinearTickets,
    key: &str,
) -> AppResult<(Value, Ticket)> {
    let elsewhere = || {
        AppError(ApiError::new(
            ErrorCode::NotFound,
            format!(
                "{} has no ticket {key}: this project works on its tickets alone",
                l.team_key.as_deref().unwrap_or("The project's team")
            ),
        ))
    };
    let Some(key) = tk::linear_key(key) else {
        return Err(validation(
            "key",
            "Name a Linear ticket by its key, like ENG-123",
        ));
    };
    let d = match client
        .graphql(&lin::issue_query(), json!({ "id": key }))
        .await
    {
        Ok(d) => d,
        Err(e) if e.0.code == ErrorCode::NotFound => return Err(elsewhere()),
        Err(e) => return Err(e),
    };
    let issue = d.get("issue").cloned().unwrap_or(Value::Null);
    if !lin::takes(l, &issue) {
        return Err(elsewhere());
    }
    let issue = lin::within(l, issue);
    let ticket = lin::ticket(&issue).ok_or_else(elsewhere)?;
    Ok((issue, ticket))
}

async fn linear_states(client: &Linear<'_>, l: &LinearTickets) -> AppResult<Vec<TicketState>> {
    let d = client
        .graphql(lin::TEAM_STATES, json!({ "id": l.team }))
        .await?;
    Ok(d.get("team").map(lin::states).unwrap_or_default())
}

/// The labels a team's issues can have: (id, name).
async fn linear_labels(client: &Linear<'_>, l: &LinearTickets) -> AppResult<Vec<(String, String)>> {
    let d = client
        .graphql(lin::TEAM_LABELS, json!({ "id": l.team }))
        .await?;
    let mut out: Vec<(String, String)> = Vec::new();
    let mut take = |v: Option<&Value>| {
        for n in v
            .and_then(|c| c.get("nodes"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(id), Some(name)) = (
                n.get("id").and_then(Value::as_str),
                n.get("name").and_then(Value::as_str),
            ) && !out.iter().any(|(_, x)| x.eq_ignore_ascii_case(name))
            {
                out.push((id.to_owned(), name.to_owned()));
            }
        }
    };
    take(d.get("team").and_then(|t| t.get("labels")));
    take(d.get("issueLabels"));
    Ok(out)
}

fn label_ids(all: &[(String, String)], names: &[String]) -> AppResult<Vec<String>> {
    names
        .iter()
        .map(|name| {
            all.iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(name.trim()))
                .map(|(id, _)| id.clone())
                .ok_or_else(|| {
                    validation(
                        "labels",
                        &format!("There is no label {name}: `fctl ticket labels` lists them"),
                    )
                })
        })
        .collect()
}

async fn linear_call(
    ctx: &Ctx,
    session: &RunnerSession,
    client: &Linear<'_>,
    l: &LinearTickets,
    request: TicketRequest,
) -> AppResult<TicketAnswer> {
    Ok(match request {
        TicketRequest::About => {
            let name = linear::installation(&ctx.db, &l.organization)
                .await?
                .map(|i| i.name);
            TicketAnswer::About {
                about: TrackerAbout {
                    tracker: Tracker::Linear,
                    scope: tk::linear_scope(l, name.as_deref()),
                    files_in: Some(match (&l.team_key, &l.project_name) {
                        (Some(t), Some(p)) => format!("{t}, project {p}"),
                        (Some(t), None) => t.clone(),
                        _ => "the project's team".to_owned(),
                    }),
                    tickets: session_tickets(&ctx.db, &session.id).await?,
                },
            }
        }
        TicketRequest::List {
            state,
            query,
            label,
            limit: asked,
        } => {
            let first = limit(asked);
            let filter = linear_filter(l, state, label.as_deref());
            let (key, d) = match query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
                Some(term) => (
                    "searchIssues",
                    client
                        .graphql(
                            &lin::search_query(),
                            json!({ "term": term, "filter": filter, "first": first }),
                        )
                        .await?,
                ),
                None => (
                    "issues",
                    client
                        .graphql(
                            &lin::issues_query(),
                            json!({ "filter": filter, "first": first }),
                        )
                        .await?,
                ),
            };
            let c = d.get(key).cloned().unwrap_or(Value::Null);
            TicketAnswer::Tickets {
                // Linear's filter keeps them to the team; this again, and
                // their parents too.
                tickets: c
                    .get("nodes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|v| lin::takes(l, v))
                    .filter_map(|v| lin::ticket(&lin::within(l, v.clone())))
                    .collect(),
                more: c
                    .get("pageInfo")
                    .and_then(|p| p.get("hasNextPage"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }
        }
        TicketRequest::View { key } => TicketAnswer::Ticket {
            ticket: linear_issue(client, l, &key).await?.1,
        },
        TicketRequest::States => TicketAnswer::States {
            states: linear_states(client, l).await?,
        },
        TicketRequest::Labels => TicketAnswer::Labels {
            labels: linear_labels(client, l)
                .await?
                .into_iter()
                .map(|(_, n)| n)
                .collect(),
        },
        TicketRequest::Create {
            title,
            description,
            parent,
            labels,
            state,
            priority,
            place: _,
        } => {
            let mut input = json!({ "teamId": l.team, "title": check_title(&title)? });
            if let Some(d) = description.as_deref() {
                put(
                    &mut input,
                    "description",
                    json!(check_body("description", d)?),
                );
            }
            if let Some(p) = &l.project {
                put(&mut input, "projectId", json!(p));
            }
            if let Some(parent) = parent.as_deref() {
                put(
                    &mut input,
                    "parentId",
                    json!(linear_issue(client, l, parent).await?.1.id),
                );
            }
            if !labels.is_empty() {
                put(
                    &mut input,
                    "labelIds",
                    json!(label_ids(&linear_labels(client, l).await?, &labels)?),
                );
            }
            if let Some(state) = state.as_deref() {
                let states = linear_states(client, l).await?;
                put(
                    &mut input,
                    "stateId",
                    json!(lin::state_named(&states, state)?.id),
                );
            }
            if let Some(p) = priority.as_deref() {
                put(&mut input, "priority", json!(lin::priority(p)?));
            }
            let d = client
                .graphql(&lin::issue_create(), json!({ "input": input }))
                .await?;
            let ticket = d
                .get("issueCreate")
                .and_then(|c| c.get("issue"))
                .and_then(lin::ticket)
                .ok_or_else(|| internal("Linear created no issue"))?;
            told(
                ctx,
                session,
                "ticket_created",
                format!("Filed {} in Linear: {}", ticket.key, ticket.title),
                TicketNotice {
                    tracker: Tracker::Linear,
                    ticket: ticket.link(),
                    body: ticket.description.clone(),
                    url: None,
                    state: ticket.state.clone(),
                },
            )
            .await;
            TicketAnswer::Ticket { ticket }
        }
        TicketRequest::Comment {
            key,
            body,
            reply_to,
        } => {
            let body = check_body("body", &body)?;
            if body.is_empty() {
                return Err(validation("body", "Write the comment first"));
            }
            let (_, ticket) = linear_issue(client, l, &key).await?;
            let mut input = json!({ "issueId": ticket.id, "body": body });
            if let Some(parent) = reply_to.as_deref().filter(|p| !p.trim().is_empty()) {
                put(&mut input, "parentId", json!(parent.trim()));
            }
            let d = client
                .graphql(lin::COMMENT_CREATE, json!({ "input": input }))
                .await?;
            let c = d
                .get("commentCreate")
                .and_then(|c| c.get("comment"))
                .cloned()
                .unwrap_or(Value::Null);
            let comment = fctl_proto::tickets::TicketComment {
                id: c
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                author: None,
                body: body.clone(),
                url: c.get("url").and_then(Value::as_str).map(str::to_owned),
                created_at: Some(ctx.now),
                parent: reply_to.clone(),
            };
            told(
                ctx,
                session,
                "ticket_commented",
                format!("Commented on {} in Linear", ticket.key),
                TicketNotice {
                    tracker: Tracker::Linear,
                    ticket: ticket.link(),
                    body: Some(body),
                    url: comment.url.clone(),
                    state: None,
                },
            )
            .await;
            TicketAnswer::Comment {
                ticket: ticket.link(),
                comment,
            }
        }
        TicketRequest::Update {
            key,
            title,
            description,
            state,
            add_labels,
            remove_labels,
            parent,
            priority,
        } => {
            let (_, current) = linear_issue(client, l, &key).await?;
            let mut input = json!({});
            if let Some(t) = title.as_deref() {
                put(&mut input, "title", json!(check_title(t)?));
            }
            if let Some(d) = description.as_deref() {
                put(
                    &mut input,
                    "description",
                    json!(check_body("description", d)?),
                );
            }
            if let Some(st) = state.as_deref() {
                let states = linear_states(client, l).await?;
                put(
                    &mut input,
                    "stateId",
                    json!(lin::state_named(&states, st)?.id),
                );
            }
            if !add_labels.is_empty() || !remove_labels.is_empty() {
                let all = linear_labels(client, l).await?;
                if !add_labels.is_empty() {
                    put(
                        &mut input,
                        "addedLabelIds",
                        json!(label_ids(&all, &add_labels)?),
                    );
                }
                if !remove_labels.is_empty() {
                    put(
                        &mut input,
                        "removedLabelIds",
                        json!(label_ids(&all, &remove_labels)?),
                    );
                }
            }
            match parent.as_deref().map(str::trim) {
                Some("none" | "") => put(&mut input, "parentId", Value::Null),
                Some(p) => put(
                    &mut input,
                    "parentId",
                    json!(linear_issue(client, l, p).await?.1.id),
                ),
                None => {}
            }
            if let Some(p) = priority.as_deref() {
                put(&mut input, "priority", json!(lin::priority(p)?));
            }
            if input.as_object().is_none_or(serde_json::Map::is_empty) {
                return Err(validation("key", "Say what to change"));
            }
            let d = client
                .graphql(
                    &lin::issue_update(),
                    json!({ "id": current.id, "input": input }),
                )
                .await?;
            let ticket = d
                .get("issueUpdate")
                .and_then(|c| c.get("issue"))
                .and_then(lin::ticket)
                .unwrap_or(current);
            told(
                ctx,
                session,
                "ticket_updated",
                format!("Changed {} in Linear", ticket.key),
                TicketNotice {
                    tracker: Tracker::Linear,
                    ticket: ticket.link(),
                    body: None,
                    url: None,
                    state: ticket.state.clone(),
                },
            )
            .await;
            TicketAnswer::Ticket { ticket }
        }
    })
}

// ───────────────────────────── GitHub ─────────────────────────────

/// The repository a GitHub key names, among those the agent reaches: the
/// first, for a key without one (`#12`).
fn github_target(repos: &[RepoRef], key: &str) -> AppResult<(RepoRef, u64)> {
    let (repo, number) = tk::github_key(key).ok_or_else(|| {
        validation(
            "key",
            "Name a GitHub issue by its number, like #12 or acme/web#12",
        )
    })?;
    let repo = match repo {
        None => repos.first().cloned().ok_or_else(|| {
            forbidden("This project has no repository whose issues agents reach.")
        })?,
        Some(r) => repos.iter().find(|x| x.same(&r)).cloned().ok_or_else(|| {
            forbidden(&format!(
                "This project's agents reach the issues of {} alone.",
                repos
                    .iter()
                    .map(RepoRef::full_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?,
    };
    Ok((repo, number))
}

/// A token for `repo`'s issues, to write or to read, which never leaves the
/// control plane, within what the session's workspace may use.
async fn issue_token(
    ctx: &Ctx,
    session: &RunnerSession,
    gh: &github::Github<'_>,
    repo: &RepoRef,
    write: bool,
) -> AppResult<GithubCredential> {
    let connections = crate::workspaces::connections(&ctx.db, &session.workspace_id).await?;
    if !crate::workspaces::scope(&connections, &repo.owner).is_some_and(|s| s.allows(&repo.name)) {
        return Err(forbidden(&format!(
            "This project's workspace may not use {} any more.",
            repo.full_name()
        )));
    }
    let inst = crate::workspaces::installation(&ctx.db, gh, &connections, &repo.owner).await?;
    gh.installation_token(
        inst,
        Some(&[repo.name.as_str()]),
        Some(repos::issue_permissions(write)),
    )
    .await
}

fn github_failed(status: u16, v: &Value) -> AppError {
    let message = v
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("GitHub refused");
    let code = match status {
        404 => ErrorCode::NotFound,
        403 | 401 => ErrorCode::Forbidden,
        422 => ErrorCode::Validation,
        429 => ErrorCode::RateLimited,
        _ => ErrorCode::Unavailable,
    };
    AppError(ApiError::new(code, format!("GitHub: {message}")))
}

async fn github_call(
    ctx: &Ctx,
    session: &RunnerSession,
    repos: &[RepoRef],
    request: TicketRequest,
) -> AppResult<TicketAnswer> {
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    let first = repos
        .first()
        .cloned()
        .ok_or_else(|| forbidden("This project has no repository whose issues agents reach."))?;
    match request {
        TicketRequest::About => Ok(TicketAnswer::About {
            about: TrackerAbout {
                tracker: Tracker::Github,
                scope: match repos {
                    [only] => format!("GitHub issues of {}", only.full_name()),
                    _ => format!(
                        "GitHub issues of {}",
                        repos
                            .iter()
                            .map(RepoRef::full_name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                },
                files_in: Some(first.full_name()),
                tickets: session_tickets(&ctx.db, &session.id).await?,
            },
        }),
        TicketRequest::States => Ok(TicketAnswer::States {
            states: issues::states(),
        }),
        request => {
            let write = !request.reads();
            let target = match &request {
                TicketRequest::View { key }
                | TicketRequest::Comment { key, .. }
                | TicketRequest::Update { key, .. } => Some(github_target(repos, key)?),
                _ => None,
            };
            let repo = match (&target, &request) {
                (Some((r, _)), _) => r.clone(),
                (None, TicketRequest::Create { place: Some(p), .. }) => {
                    let r =
                        RepoRef::parse(p).ok_or_else(|| validation("place", "Use owner/name"))?;
                    repos.iter().find(|x| x.same(&r)).cloned().ok_or_else(|| {
                        forbidden(&format!(
                            "This project's agents don't file issues in {}.",
                            r.full_name()
                        ))
                    })?
                }
                _ => first.clone(),
            };
            let token = issue_token(ctx, session, &gh, &repo, write).await?;
            let answer = github_with(
                ctx,
                session,
                &gh,
                &token,
                &repo,
                target.map(|t| t.1),
                request,
            )
            .await;
            gh.revoke_installation_token(&token).await;
            answer
        }
    }
}

async fn github_issue(
    gh: &github::Github<'_>,
    token: &GithubCredential,
    repo: &RepoRef,
    number: u64,
) -> AppResult<Ticket> {
    let answer = gh
        .graphql(
            token,
            json!({ "query": issues::ISSUE, "variables": { "owner": repo.owner, "name": repo.name, "number": number } }),
        )
        .await?;
    answer
        .pointer("/data/repository/issue")
        .filter(|v| !v.is_null())
        .and_then(|v| issues::ticket_from_graphql(repo, v))
        .ok_or_else(|| not_found(&format!("Issue {}", tk::github_issue_key(repo, number))))
}

#[allow(clippy::too_many_arguments)]
async fn github_with(
    ctx: &Ctx,
    session: &RunnerSession,
    gh: &github::Github<'_>,
    token: &GithubCredential,
    repo: &RepoRef,
    number: Option<u64>,
    request: TicketRequest,
) -> AppResult<TicketAnswer> {
    let path = format!("/repos/{}/{}", repo.owner, repo.name);
    let number = || number.ok_or_else(|| internal("an issue without its number"));
    Ok(match request {
        TicketRequest::List {
            state,
            query,
            label,
            limit: asked,
        } => {
            let per = limit(asked);
            let q = issues::search_q(repo, state, query.as_deref(), label.as_deref());
            let enc: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
            let (status, v) = gh
                .with_token(
                    token,
                    Method::Get,
                    &format!("/search/issues?q={enc}&sort=updated&order=desc&per_page={per}"),
                    None,
                )
                .await?;
            if status != 200 {
                return Err(github_failed(status, &v));
            }
            let total = v.get("total_count").and_then(Value::as_u64).unwrap_or(0);
            let tickets: Vec<Ticket> = v
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|i| issues::ticket(repo, i))
                .collect();
            TicketAnswer::Tickets {
                more: total > tickets.len() as u64,
                tickets,
            }
        }
        TicketRequest::View { .. } => TicketAnswer::Ticket {
            ticket: github_issue(gh, token, repo, number()?).await?,
        },
        TicketRequest::Labels => {
            let (status, v) = gh
                .with_token(
                    token,
                    Method::Get,
                    &format!("{path}/labels?per_page=100"),
                    None,
                )
                .await?;
            if status != 200 {
                return Err(github_failed(status, &v));
            }
            TicketAnswer::Labels {
                labels: v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|l| l.get("name").and_then(Value::as_str).map(str::to_owned))
                    .collect(),
            }
        }
        TicketRequest::Create {
            title,
            description,
            parent,
            labels,
            state: _,
            priority,
            place: _,
        } => {
            if priority.is_some() {
                return Err(validation("priority", "GitHub issues have no priority"));
            }
            let mut body = json!({ "title": check_title(&title)? });
            if let Some(d) = description.as_deref() {
                put(&mut body, "body", json!(check_body("description", d)?));
            }
            if !labels.is_empty() {
                put(&mut body, "labels", json!(labels));
            }
            // The parent before the issue: a create that failed once the
            // issue existed would have the agent file it again.
            let parent = match parent.as_deref() {
                Some(p) => {
                    let (parent_repo, parent_number) =
                        github_target(std::slice::from_ref(repo), p)?;
                    github_issue(gh, token, &parent_repo, parent_number).await?;
                    Some((parent_repo, parent_number))
                }
                None => None,
            };
            let (status, v) = gh
                .with_token(token, Method::Post, &format!("{path}/issues"), Some(body))
                .await?;
            if status != 201 {
                return Err(github_failed(status, &v));
            }
            let created = v.get("number").and_then(Value::as_u64).unwrap_or_default();
            if let Some((parent_repo, parent_number)) = parent {
                let id = v.get("id").and_then(Value::as_u64).unwrap_or_default();
                let (status, pv) = gh
                    .with_token(
                        token,
                        Method::Post,
                        &format!(
                            "/repos/{}/{}/issues/{parent_number}/sub_issues",
                            parent_repo.owner, parent_repo.name
                        ),
                        Some(json!({ "sub_issue_id": id })),
                    )
                    .await?;
                if !(200..300).contains(&status) {
                    worker::console_error!(
                        "making a sub-issue: {}",
                        github_failed(status, &pv).0.message
                    );
                }
            }
            let ticket = github_issue(gh, token, repo, created).await?;
            told(
                ctx,
                session,
                "ticket_created",
                format!("Filed {} on GitHub: {}", ticket.key, ticket.title),
                TicketNotice {
                    tracker: Tracker::Github,
                    ticket: ticket.link(),
                    body: ticket.description.clone(),
                    url: None,
                    state: ticket.state.clone(),
                },
            )
            .await;
            TicketAnswer::Ticket { ticket }
        }
        TicketRequest::Comment { body, reply_to, .. } => {
            if reply_to.is_some() {
                return Err(validation(
                    "reply_to",
                    "GitHub's issue comments don't thread: comment without --reply-to, and quote what you answer",
                ));
            }
            let body = check_body("body", &body)?;
            if body.is_empty() {
                return Err(validation("body", "Write the comment first"));
            }
            let num = number()?;
            let (status, v) = gh
                .with_token(
                    token,
                    Method::Post,
                    &format!("{path}/issues/{num}/comments"),
                    Some(json!({ "body": body })),
                )
                .await?;
            if status != 201 {
                return Err(github_failed(status, &v));
            }
            let comment =
                issues::comment(&v).ok_or_else(|| internal("GitHub's comment is missing"))?;
            let link = TicketLink {
                key: tk::github_issue_key(repo, num),
                title: String::new(),
                url: format!(
                    "https://github.com/{}/{}/issues/{num}",
                    repo.owner, repo.name
                ),
                state: None,
            };
            told(
                ctx,
                session,
                "ticket_commented",
                format!("Commented on {} on GitHub", link.key),
                TicketNotice {
                    tracker: Tracker::Github,
                    ticket: link.clone(),
                    body: Some(body),
                    url: comment.url.clone(),
                    state: None,
                },
            )
            .await;
            TicketAnswer::Comment {
                ticket: link,
                comment,
            }
        }
        TicketRequest::Update {
            title,
            description,
            state,
            add_labels,
            remove_labels,
            parent,
            priority,
            ..
        } => {
            if priority.is_some() {
                return Err(validation("priority", "GitHub issues have no priority"));
            }
            if parent.is_some() {
                return Err(validation(
                    "parent",
                    "Change a GitHub issue's parent on GitHub: fctl ticket sets it only as it files one",
                ));
            }
            let num = number()?;
            let description = description
                .as_deref()
                .map(|d| check_body("description", d))
                .transpose()?;
            let title = title.as_deref().map(check_title).transpose()?;
            let patch = issues::patch(title.as_deref(), description.as_deref(), state.as_deref())?;
            let changes = patch.as_object().is_some_and(|o| !o.is_empty());
            if !changes && add_labels.is_empty() && remove_labels.is_empty() {
                return Err(validation("key", "Say what to change"));
            }
            if changes {
                let (status, v) = gh
                    .with_token(
                        token,
                        Method::Patch,
                        &format!("{path}/issues/{num}"),
                        Some(patch),
                    )
                    .await?;
                if status != 200 {
                    return Err(github_failed(status, &v));
                }
            }
            if !add_labels.is_empty() {
                let (status, v) = gh
                    .with_token(
                        token,
                        Method::Post,
                        &format!("{path}/issues/{num}/labels"),
                        Some(json!({ "labels": add_labels })),
                    )
                    .await?;
                if status != 200 {
                    return Err(github_failed(status, &v));
                }
            }
            for label in &remove_labels {
                let enc: String = url::form_urlencoded::byte_serialize(label.as_bytes()).collect();
                let (status, v) = gh
                    .with_token(
                        token,
                        Method::Delete,
                        &format!("{path}/issues/{num}/labels/{}", enc.replace('+', "%20")),
                        None,
                    )
                    .await?;
                if status != 200 && status != 404 {
                    return Err(github_failed(status, &v));
                }
            }
            let ticket = github_issue(gh, token, repo, num).await?;
            told(
                ctx,
                session,
                "ticket_updated",
                format!("Changed {} on GitHub", ticket.key),
                TicketNotice {
                    tracker: Tracker::Github,
                    ticket: ticket.link(),
                    body: None,
                    url: None,
                    state: ticket.state.clone(),
                },
            )
            .await;
            TicketAnswer::Ticket { ticket }
        }
        TicketRequest::About | TicketRequest::States => {
            return Err(internal("answered before"));
        }
    })
}

/// Whether a project's settings changed its tracker in a way that needs
/// checking with Linear as it is saved.
pub fn linear_changed(new: &Tickets, old: &Tickets) -> bool {
    new.tracker == Tracker::Linear && new.linear.is_some() && new.linear != old.linear
}

/// Sets `key` of a JSON object.
fn put(v: &mut Value, key: &str, value: Value) {
    if let Some(o) = v.as_object_mut() {
        o.insert(key.to_owned(), value);
    }
}
