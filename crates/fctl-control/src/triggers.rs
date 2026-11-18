//! Schedules that run on events (`fctl_domain::triggers`): GitHub's webhook
//! says what happened in a repository, and each schedule that waits for it,
//! in a project of a workspace connected to that repository's account,
//! starts a run of its own on its owner's Claude subscription
//! (docs/architecture.md#schedules).
//!
//! A delivery GitHub sends again, or another about the same thing, starts
//! nothing twice: the run is claimed in D1 by its key before anything
//! starts (`schedule_runs_event_key`). A run starts none, and says why,
//! where a time's would (its owner can't run it, which pauses the
//! schedule, or their plan is at its limit), once its events started as
//! many sessions as it lets them in an hour or a day ([`tg::caps`]), and
//! while the schedule's run on the same issue, pull request, release or
//! branch still works on it.

use crate::ctx::Ctx;
use crate::db::{int, n, os, q, s};
use crate::error::AppResult;
use crate::model::ProjectRow;
use crate::schedules::{self, Outcome, SCHEDULE_SELECT, ScheduleRow, Start};
use fctl_domain::hooks::Kind;
use fctl_domain::schedules::{self as sch, RunNote, StartedBy};
use fctl_domain::triggers::{self as tg, Happened, ProjectRepo, Takes};
use fctl_domain::webhooks;
use fctl_proto::ScheduleRunId;
use fctl_proto::api::{ScheduleEventKind, ScheduleRunOutcome, ScheduleTrigger, UnheardEvent};
use fctl_proto::event::MessageSource;
use fctl_proto::session::{RepoAccess, RepoRef, SessionStatus};
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use worker::D1Database;

/// Schedules whose runs one delivery decides at most, started or not, a
/// dozen queries or so each, so that its answer comes in the ten seconds
/// GitHub waits, within a Worker's queries.
const PER_DELIVERY: usize = 10;
/// Schedules past those that take it too, which record why they started
/// none (`pass_over`), a query each; any more only log it.
const PASSED_OVER: usize = 20;
/// How long the control plane goes by what it last heard of the webhook
/// events the GitHub App is subscribed to.
const SUBSCRIBED_MS: i64 = 10 * 60_000;
/// How long a run claimed and still undecided counts toward its
/// schedule's caps (`started_lately`): a delivery's answer comes in seconds.
const IN_FLIGHT_MS: i64 = 10 * fctl_domain::cron::MINUTE_MS;
/// What a claimed run says until it is decided: what it says for good if
/// the Worker stops before then.
const UNDECIDED: &str = "fctl stopped before it started the session.";

// ───────────────────────────── the project's side ─────────────────────────────

/// The repositories a project's schedules can wait on: its primary one,
/// which its agents write to, and those it lists by name. An owner-wide
/// grant, which reads every repository of an account, names none.
pub async fn project_repos(db: &D1Database, project: &ProjectRow) -> AppResult<Vec<ProjectRepo>> {
    let mut out: Vec<ProjectRepo> = project
        .repo()
        .map(|repo| ProjectRepo { repo, writes: true })
        .into_iter()
        .collect();
    for g in crate::spec::grants(db, &project.id).await? {
        let Some(name) = g.name else {
            continue;
        };
        let repo = RepoRef {
            owner: g.owner,
            name,
        };
        if out.iter().any(|r| r.repo.same(&repo)) {
            continue;
        }
        out.push(ProjectRepo {
            repo,
            writes: g.access == RepoAccess::Write,
        });
    }
    Ok(out)
}

thread_local! {
    /// What the GitHub App is subscribed to, and when the control plane
    /// asked: once in [`SUBSCRIBED_MS`] for each of the Worker's isolates.
    static SUBSCRIBED: RefCell<Option<(i64, Option<Vec<String>>)>> = const { RefCell::new(None) };
}

/// The webhook events the instance's GitHub App is subscribed to: `None`
/// when GitHub doesn't say.
async fn subscribed(ctx: &Ctx) -> Option<Vec<String>> {
    let kept = SUBSCRIBED.with(|c| c.borrow().clone());
    if let Some((at, events)) = kept
        && ctx.now - at < SUBSCRIBED_MS
    {
        return events;
    }
    let gh = crate::github::from_config(&ctx.cfg, ctx.now).ok()?;
    let events = match gh.subscribed().await {
        Ok(events) => events,
        Err(e) => {
            worker::console_error!(
                "asking GitHub what the app is subscribed to: {}",
                e.0.message
            );
            None
        }
    };
    SUBSCRIBED.with(|c| *c.borrow_mut() = Some((ctx.now, events.clone())));
    events
}

/// The kinds of events that wouldn't start runs of a project with `repos`,
/// and why: the instance has no GitHub App or takes no webhook from
/// GitHub, the project has no repository there, or the app isn't
/// subscribed to the event.
pub async fn unheard(ctx: &Ctx, repos: &[ProjectRepo]) -> Vec<UnheardEvent> {
    let all = |reason: &str| -> Vec<UnheardEvent> {
        ScheduleEventKind::ALL
            .iter()
            .map(|kind| UnheardEvent {
                kind: *kind,
                reason: reason.to_owned(),
            })
            .collect()
    };
    let Some(github) = ctx.cfg.github.as_ref() else {
        return all(
            "This instance has no GitHub App, so GitHub tells it of nothing that happens there.",
        );
    };
    if github.webhook_secret.is_none() {
        return all(
            "This instance takes no webhook from GitHub (it has no GITHUB_WEBHOOK_SECRET), so GitHub tells it of nothing that happens there.",
        );
    }
    if repos.is_empty() {
        return all("The project has no repository on GitHub for it to happen in.");
    }
    let Some(events) = subscribed(ctx).await else {
        return Vec::new();
    };
    ScheduleEventKind::ALL
        .iter()
        .filter_map(|kind| {
            let (event, _) = tg::github_event(*kind)?;
            if events.iter().any(|e| e == event) {
                return None;
            }
            Some(UnheardEvent {
                kind: *kind,
                reason: tg::unsubscribed(*kind)?,
            })
        })
        .collect()
}

// ───────────────────────────── GitHub's side ─────────────────────────────

/// What a webhook delivery of `event` (GitHub's id for it: `delivery`)
/// starts: a run of each schedule that waits for what it says happened.
/// Returns how many sessions started.
pub async fn on_delivery(
    ctx: &Ctx,
    event: &str,
    delivery: &str,
    payload: &Value,
) -> AppResult<usize> {
    // Only GitHub's word of the account and the installation that sent it
    // reaches a workspace's schedules: those of workspaces connected to it.
    let installation = payload.pointer("/installation/id").and_then(Value::as_u64);
    let account = payload
        .pointer("/repository/owner/id")
        .and_then(Value::as_u64);
    let (Some(installation), Some(account)) = (installation, account) else {
        return Ok(0);
    };
    let sent = Sent {
        installation,
        account,
        delivery,
    };
    if let Some(h) = tg::happened(event, payload) {
        let rows = waiting(&ctx.db, &h.repo, h.kind(), &sent).await?;
        return start_all(ctx, &h, &sent, rows).await;
    }
    let mut started = 0;
    for ended in tg::checks_ended(event, payload) {
        let rows: Vec<ScheduleRow> =
            waiting(&ctx.db, &ended.repo, ScheduleEventKind::ChecksFailed, &sent)
                .await?
                .into_iter()
                .filter(|r| r.event().is_some_and(|e| tg::watches(&e.branches, &ended)))
                .collect();
        if rows.is_empty() {
            continue;
        }
        // Whether all of the commit's checks ended, and how: a lookup, for a
        // point of the installation's rate limit.
        let gh = crate::github::from_config(&ctx.cfg, ctx.now)?;
        let answer = gh
            .lookup(
                installation,
                webhooks::commit_query(&ended.repo, &ended.sha, &ended.branch),
            )
            .await?;
        let Some(commit) = webhooks::looked_up_commit(&answer) else {
            continue;
        };
        if let Some(h) = tg::checks_failed(&ended, &commit) {
            started += start_all(ctx, &h, &sent, rows).await?;
        }
    }
    Ok(started)
}

/// Who sent a delivery: the GitHub App's installation, and the account the
/// repository belongs to, by GitHub's ids.
struct Sent<'a> {
    installation: u64,
    account: u64,
    delivery: &'a str,
}

/// The schedules that are on and wait for `kind` in `repo`: of projects
/// that aren't archived, that have the repository, and whose workspace
/// connected its account through the installation that sent the delivery,
/// with the repository among those it may use (as
/// `crate::pull_requests::in_reach` holds sessions to). All of them, oldest
/// first: what narrows each down decides, in memory, before any cap, and
/// there are at most 50 for each project that has the repository.
async fn waiting(
    db: &D1Database,
    repo: &RepoRef,
    kind: ScheduleEventKind,
    sent: &Sent<'_>,
) -> AppResult<Vec<ScheduleRow>> {
    let key = repo.full_name().to_ascii_lowercase();
    q(
        db,
        &format!(
            "{SCHEDULE_SELECT} JOIN projects p ON p.id = sc.project_id \
             WHERE sc.enabled = 1 AND sc.event_kind = ?2 AND p.archived_at IS NULL \
             AND EXISTS (SELECT 1 FROM workspace_github g WHERE g.workspace_id = p.workspace_id AND g.status = 'active' \
               AND lower(g.account) = substr(?1, 1, instr(?1, '/') - 1) \
               AND (g.installation_id = ?3 OR (g.installation_id IS NULL AND g.account_id = ?4)) \
               AND (g.repos IS NULL OR EXISTS (SELECT 1 FROM json_each(g.repos) gr \
                 WHERE lower(gr.value) = substr(?1, instr(?1, '/') + 1)))) \
             AND (lower(p.repo_owner || '/' || p.repo_name) = ?1 OR EXISTS (SELECT 1 FROM project_repos pr \
               WHERE pr.project_id = p.id AND lower(pr.owner || '/' || pr.name) = ?1)) \
             ORDER BY sc.created_at, sc.id"
        ),
        &[
            s(&key),
            s(kind.as_str()),
            n(i64::try_from(sent.installation).unwrap_or(i64::MAX)),
            n(i64::try_from(sent.account).unwrap_or(i64::MAX)),
        ],
    )?
    .all()
    .await
}

/// Starts a run of each of `rows` that takes what happened, or records why
/// it starts none, for [`PER_DELIVERY`] of them at most; the next
/// [`PASSED_OVER`] that take it record at once that they came too late,
/// and the rest only a log line, so that a delivery's queries stay few
/// whatever waits for it. A run claimed already, by a delivery GitHub sent
/// before, costs a query and counts for nothing. Each project is read once,
/// and one that can't be read leaves the others to go on. Returns how many
/// sessions started.
async fn start_all(
    ctx: &Ctx,
    h: &Happened,
    sent: &Sent<'_>,
    rows: Vec<ScheduleRow>,
) -> AppResult<usize> {
    let app = ctx.cfg.github.as_ref().map(|g| format!("{}[bot]", g.slug));
    // Whether the author may write to the repository, and whether whoever
    // added the label may triage it, each asked once.
    let mut writes: Option<bool> = None;
    let mut triages: Option<bool> = None;
    // Each project with the repositories its agents write to, read once.
    let mut projects: HashMap<String, Option<(ProjectRow, Vec<RepoRef>)>> = HashMap::new();
    let (mut started, mut decided, mut passed_over, mut left) = (0, 0, 0, 0);
    for row in rows {
        let Some(event) = row.event() else {
            continue;
        };
        if !projects.contains_key(&row.project_id) {
            let read = match read_project(&ctx.db, &row.project_id).await {
                Ok(read) => read,
                Err(e) => {
                    worker::console_error!("project {}: {}", row.project_id, e.0.message);
                    None
                }
            };
            projects.insert(row.project_id.clone(), read);
        }
        let Some(Some((project, writable))) = projects.get(&row.project_id) else {
            continue;
        };
        match tg::takes(&event, h, writable, app.as_deref()) {
            Takes::No => continue,
            Takes::Yes => {}
            Takes::IfAuthorWrites => {
                let w = match writes {
                    Some(w) => w,
                    None => {
                        let author = h.author.as_ref().map(|a| a.login.as_str());
                        let w = may(ctx, h, sent.installation, author, Access::Write).await;
                        writes = Some(w);
                        w
                    }
                };
                if !w {
                    continue;
                }
            }
            Takes::IfLabelerTriages => {
                let t = match triages {
                    Some(t) => t,
                    None => {
                        let by = h.labeled_by.as_ref().map(|b| b.login.as_str());
                        let t = may(ctx, h, sent.installation, by, Access::Triage).await;
                        triages = Some(t);
                        t
                    }
                };
                if !t {
                    continue;
                }
            }
        }
        if decided >= PER_DELIVERY {
            if passed_over >= PASSED_OVER {
                left += 1;
                continue;
            }
            passed_over += 1;
            if let Err(e) = pass_over(&ctx.db, &row, h, sent.delivery, ctx.now).await {
                worker::console_error!(
                    "schedule {} on {}: {}",
                    row.id,
                    tg::subject(h),
                    e.0.message
                );
            }
            continue;
        }
        match run(ctx, &row, project, h, sent.delivery).await {
            Ok(None) => {}
            Ok(Some(session)) => {
                decided += 1;
                started += usize::from(session);
            }
            Err(e) => {
                decided += 1;
                worker::console_error!("schedule {} on {}: {}", row.id, tg::subject(h), e.0.message)
            }
        }
    }
    if left > 0 {
        worker::console_error!(
            "{left} more schedules wait for {} and start nothing: one delivery decides the runs of at most {PER_DELIVERY}",
            tg::subject(h)
        );
    }
    Ok(started)
}

/// A project, and the repositories its agents write to.
async fn read_project(
    db: &D1Database,
    project_id: &str,
) -> AppResult<Option<(ProjectRow, Vec<RepoRef>)>> {
    let Some(project) = crate::spec::project_row(db, project_id).await? else {
        return Ok(None);
    };
    let writable = project_repos(db, &project)
        .await?
        .into_iter()
        .filter(|r| r.writes)
        .map(|r| r.repo)
        .collect();
    Ok(Some((project, writable)))
}

/// Records, in one statement, that `row`'s run for what happened starts
/// no session: the delivery started as many as one may. A delivery GitHub
/// sends again finds it taken, as it finds a run that started. Returns how
/// many it recorded: none when it was taken.
async fn pass_over(
    db: &D1Database,
    row: &ScheduleRow,
    h: &Happened,
    delivery: &str,
    now: i64,
) -> AppResult<usize> {
    q(
        db,
        "INSERT OR IGNORE INTO schedule_runs (id, schedule_id, due_at, at, outcome, reason, trigger, user_id, \
         event, event_key, subject) VALUES (?, ?, ?, ?, 'skipped', ?, 'event', ?, ?, ?, ?)",
        &[
            s(&fctl_domain::ids::new_id(ScheduleRunId::PREFIX, now)),
            s(&row.id),
            n(now),
            n(now),
            s(&format!(
                "{PER_DELIVERY} other schedules took it first: one event goes to at most {PER_DELIVERY}."
            )),
            os(row.owner_id.as_deref()),
            s(&serde_json::to_string(&tg::run_event(h)).unwrap_or_default()),
            s(&tg::key(h, delivery)),
            s(&tg::subject(h)),
        ],
    )?
    .run()
    .await
}

/// What someone may do to a repository, as a schedule asks GitHub.
#[derive(Clone, Copy)]
enum Access {
    /// Write to it: the author of an issue or pull request, for those of
    /// people who may write.
    Write,
    /// Triage it, as adding a label takes: whoever added the label.
    Triage,
}

/// Whether GitHub says the person `login` (the author of the issue or pull
/// request, or whoever added the label) may do `access` to its repository.
/// No when GitHub doesn't say.
async fn may(
    ctx: &Ctx,
    h: &Happened,
    installation: u64,
    login: Option<&str>,
    access: Access,
) -> bool {
    let Some(login) = login else {
        return false;
    };
    let (allows, verb): (fn(&Value) -> bool, &str) = match access {
        Access::Write => (fctl_domain::follow_ups::can_write, "write to"),
        Access::Triage => (fctl_domain::follow_ups::can_triage, "triage"),
    };
    let gh = match crate::github::from_config(&ctx.cfg, ctx.now) {
        Ok(gh) => gh,
        Err(_) => return false,
    };
    match gh.permission(installation, &h.repo, login).await {
        Ok(Ok(answer)) => allows(&answer),
        Ok(Err(why)) => {
            worker::console_log!(
                "GitHub didn't say whether @{login} may {verb} {}: {why}",
                h.repo.full_name()
            );
            false
        }
        Err(e) => {
            worker::console_error!(
                "asking whether @{login} may {verb} {}: {}",
                h.repo.full_name(),
                e.0.message
            );
            false
        }
    }
}

// ───────────────────────────── a run ─────────────────────────────

/// Claims `row`'s run for what happened, then starts its session, or
/// records why it starts none: whether a session started, or `None` when
/// the run was claimed already, by an earlier delivery of the same.
async fn run(
    ctx: &Ctx,
    row: &ScheduleRow,
    project: &ProjectRow,
    h: &Happened,
    delivery: &str,
) -> AppResult<Option<bool>> {
    let db = &ctx.db;
    let now = ctx.now;
    let id = fctl_domain::ids::new_id(ScheduleRunId::PREFIX, now);
    let event = serde_json::to_string(&tg::run_event(h)).unwrap_or_default();
    let claimed = q(
        db,
        "INSERT OR IGNORE INTO schedule_runs (id, schedule_id, due_at, at, outcome, reason, trigger, user_id, \
         event, event_key, subject) VALUES (?, ?, ?, ?, 'failed', ?, 'event', ?, ?, ?, ?)",
        &[
            s(&id),
            s(&row.id),
            n(now),
            n(now),
            s(UNDECIDED),
            os(row.owner_id.as_deref()),
            s(&event),
            s(&tg::key(h, delivery)),
            s(&tg::subject(h)),
        ],
    )?
    .run()
    .await?;
    if claimed == 0 {
        return Ok(None);
    }
    let outcome = match decide(ctx, row, project, h, &id).await {
        Ok(o) => o,
        Err(e) => Outcome::Failed(e.0.message),
    };
    let (status, reason, session) = match &outcome {
        Outcome::Started(session) => (ScheduleRunOutcome::Started, None, Some(session.as_str())),
        Outcome::Skipped(why) => (ScheduleRunOutcome::Skipped, Some(why.as_str()), None),
        Outcome::Failed(why) => (ScheduleRunOutcome::Failed, Some(why.as_str()), None),
    };
    q(
        db,
        "UPDATE schedule_runs SET outcome = ?, reason = ?, session_id = ?, at = ? WHERE id = ?",
        &[
            s(status.as_str()),
            os(reason),
            os(session),
            n(ctx.now),
            s(&id),
        ],
    )?
    .run()
    .await?;
    schedules::trim_runs(db, &row.id, ctx.now).await?;
    if let Some(run) = outcome.skipped(
        id.clone(),
        ScheduleTrigger::Event,
        ctx.now,
        Some(tg::run_event(h).url),
    ) {
        let (env, row, now) = (ctx.env.clone(), row.clone(), ctx.now);
        ctx.defer(async move {
            let db = env.d1("DB")?;
            schedules::to_webhooks(
                &env,
                &db,
                &row,
                Kind::ScheduleRunSkipped,
                Some(run),
                None,
                now,
            )
            .await;
            Ok(())
        });
    }
    Ok(Some(matches!(outcome, Outcome::Started(_))))
}

/// Whether the claimed run `run_id` starts a session, and the session it
/// started, or why it starts none.
async fn decide(
    ctx: &Ctx,
    row: &ScheduleRow,
    project: &ProjectRow,
    h: &Happened,
    run_id: &str,
) -> AppResult<Outcome> {
    let db = &ctx.db;
    let now = ctx.now;
    let skip = |why: String| Ok(Outcome::Skipped(why));
    if project.archived_at.is_some() {
        let why = "Its project is archived.";
        schedules::pause(&ctx.env, db, &row.id, why, now).await?;
        return skip(why.into());
    }
    let owner = match schedules::owner(db, project, row).await? {
        Ok(o) => o,
        Err(why) => {
            schedules::pause(&ctx.env, db, &row.id, &why, now).await?;
            return skip(why);
        }
    };
    let zone = schedules::zone(&row.timezone);
    let limits = schedules::usage_of(db, &owner.credential.id).await?;
    if let Some(limit) = sch::at_limit(&limits, now) {
        return skip(format!(
            "{} The events until it resets start no run.",
            sch::limit_reason(&owner.user.name, limit, &*zone, false)
        ));
    }
    let caps = row.event().map(|e| tg::caps(&e)).unwrap_or(tg::Caps {
        hour: tg::DEFAULT_PER_HOUR,
        day: tg::DEFAULT_PER_DAY,
    });
    let started = started_lately(db, &row.id, caps, now, Some(run_id)).await?;
    if let Some(c) = tg::capped(caps, &started, now) {
        return skip(tg::capped_reason(&c, &*zone));
    }
    if let Some(number) = still_at_it(db, &row.id, &tg::subject(h), run_id).await? {
        return skip(format!(
            "Its run on {}, session #{number}, was still at work on it.",
            tg::short(h)
        ));
    }
    let github = fctl_domain::limits::effective(row.run_github(), project.settings().github_access);
    let previous = schedules::previous(db, &row.id).await?;
    let previous_at = previous.as_ref().map(|p| sch::stamp(&*zone, p.at));
    let when = row
        .when()
        .map(|w| w.rhythm())
        .unwrap_or_else(|| "when something happens on GitHub".into());
    let context = sch::context(Some(&tg::context(h))).ok().flatten();
    let text = sch::prompt(
        &row.prompt,
        &RunNote {
            name: &row.name,
            when: &when,
            timezone: &row.timezone,
            owner: &owner.user.name,
            by: StartedBy::Event(&tg::what_happened(h)),
            now: &sch::stamp(&*zone, now),
            previous: previous
                .as_ref()
                .zip(previous_at.as_deref())
                .map(|(p, at)| (p.number, at)),
            context: context.as_deref(),
        },
    );
    let (session, number) = schedules::start_session(
        &ctx.env,
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
            autopilot: None,
            about: Some(tg::short(h)),
            service_account: None,
            now,
        },
    )
    .await?;
    schedules::audit(
        db,
        project.workspace_id(),
        "session.created",
        Some(session.as_str()),
        json!({
            "project": project.slug,
            "number": number,
            "github_access": github,
            "schedule": row.id,
            "event": h.kind().as_str(),
            "on": tg::subject(h),
            "for": owner.user.id,
        }),
        now,
    )
    .await;
    Ok(Outcome::Started(session))
}

/// When `schedule`'s events started its sessions of the last day, newest
/// first, as many as its caps count at most. For the run `claimed`, those
/// that other deliveries claimed before it and still decide count too, so
/// that a burst of events, whose deliveries GitHub sends at once, starts no
/// more than the caps allow. "Before" goes by the order D1 inserted the
/// claims (their rowid), not by their ids: an id has the time its request
/// came, and a random part within the millisecond, so a delivery that came
/// later can claim first. Of two claims at once, the one inserted second
/// always sees the first, which counts it.
pub(crate) async fn started_lately(
    db: &D1Database,
    schedule: &str,
    caps: tg::Caps,
    now: i64,
    claimed: Option<&str>,
) -> AppResult<Vec<i64>> {
    #[derive(Deserialize)]
    struct Row {
        #[serde(deserialize_with = "int")]
        at: i64,
    }
    let rows: Vec<Row> = q(
        db,
        "SELECT r.at FROM schedule_runs r WHERE r.schedule_id = ?1 AND r.trigger = 'event' AND r.at > ?2 \
         AND (r.outcome = 'started' OR (r.outcome = 'failed' AND r.reason = ?3 AND r.at > ?6 \
           AND r.rowid < (SELECT c.rowid FROM schedule_runs c WHERE c.id = ?4))) \
         ORDER BY r.at DESC LIMIT ?5",
        &[
            s(schedule),
            n(now - fctl_domain::cron::DAY_MS),
            s(UNDECIDED),
            // No run has the empty id, so none in flight counts.
            s(claimed.unwrap_or_default()),
            n(i64::from(caps.day.max(caps.hour))),
            // One that never got decided, as its Worker stopped, counts no
            // longer than a delivery takes.
            n(now - IN_FLIGHT_MS),
        ],
    )?
    .all()
    .await?;
    Ok(rows.into_iter().map(|r| r.at).collect())
}

/// The session of `schedule`'s latest run on `subject` (but `run_id`), by
/// its number, while it still works on it.
async fn still_at_it(
    db: &D1Database,
    schedule: &str,
    subject: &str,
    run_id: &str,
) -> AppResult<Option<u32>> {
    #[derive(Deserialize)]
    struct Row {
        #[serde(deserialize_with = "int")]
        number: i64,
        status: String,
    }
    let row: Option<Row> = q(
        db,
        "SELECT s.number, s.status FROM schedule_runs r JOIN sessions s ON s.id = r.session_id \
         WHERE r.schedule_id = ? AND r.subject = ? AND r.id != ? ORDER BY r.at DESC, r.id DESC LIMIT 1",
        &[s(schedule), s(subject), s(run_id)],
    )?
    .first()
    .await?;
    Ok(row
        .filter(|r| SessionStatus::parse(&r.status).is_some_and(sch::still_working))
        .map(|r| u32::try_from(r.number).unwrap_or_default()))
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::testing;
    use wasm_bindgen_test::wasm_bindgen_test;

    async fn run_sql(db: &D1Database, sql: &str) -> AppResult<()> {
        q(db, sql, &[])?.run().await?;
        Ok(())
    }

    /// A workspace connected to acme through installation 7, with web
    /// (acme/web) waiting for new issues, and others that don't take them.
    async fn world(db: &D1Database) -> AppResult<()> {
        for sql in [
            "INSERT INTO users (id, email, name, created_at, updated_at) VALUES ('usr_ada', 'ada@example.com', 'Ada', 0, 0)",
            "INSERT INTO workspaces (id, slug, name, created_by, created_at, updated_at) \
             VALUES ('wsp_1', 'acme', 'Acme', 'usr_ada', 0, 0)",
            "INSERT INTO workspace_github (workspace_id, account, account_id, installation_id, status, created_at, updated_at) \
             VALUES ('wsp_1', 'acme', 70, 7, 'active', 0, 0)",
            "INSERT INTO projects (id, slug, name, created_by, created_at, updated_at, workspace_id, repo_owner, repo_name) \
             VALUES ('prj_web', 'web', 'Web', 'usr_ada', 0, 0, 'wsp_1', 'acme', 'web')",
            "INSERT INTO projects (id, slug, name, created_by, created_at, updated_at, workspace_id, repo_owner, repo_name) \
             VALUES ('prj_api', 'api', 'Api', 'usr_ada', 0, 0, 'wsp_1', 'acme', 'api')",
            "INSERT INTO projects (id, slug, name, created_by, created_at, updated_at, workspace_id, repo_owner, repo_name, archived_at) \
             VALUES ('prj_old', 'old', 'Old', 'usr_ada', 0, 0, 'wsp_1', 'acme', 'web', 5)",
            "INSERT INTO schedules (id, project_id, name, prompt, owner_id, created_at, updated_at, event_kind, event_filter) \
             VALUES ('sch_triage', 'prj_web', 'Triage', 'Triage it.', 'usr_ada', 1, 0, 'issue_opened', '{}')",
            "INSERT INTO schedules (id, project_id, name, prompt, owner_id, created_at, updated_at, event_kind, event_filter, enabled) \
             VALUES ('sch_paused', 'prj_web', 'Paused', 'Triage it.', 'usr_ada', 2, 0, 'issue_opened', '{}', 0)",
            "INSERT INTO schedules (id, project_id, name, prompt, owner_id, created_at, updated_at, event_kind, event_filter) \
             VALUES ('sch_merged', 'prj_web', 'Merged', 'Learn.', 'usr_ada', 3, 0, 'pull_request_merged', '{}')",
            "INSERT INTO schedules (id, project_id, name, prompt, owner_id, created_at, updated_at, event_kind, event_filter) \
             VALUES ('sch_api', 'prj_api', 'Api', 'Triage it.', 'usr_ada', 4, 0, 'issue_opened', '{}')",
            "INSERT INTO schedules (id, project_id, name, prompt, owner_id, created_at, updated_at, event_kind, event_filter) \
             VALUES ('sch_old', 'prj_old', 'Old', 'Triage it.', 'usr_ada', 5, 0, 'issue_opened', '{}')",
            "INSERT INTO schedules (id, project_id, name, prompt, owner_id, created_at, updated_at, cron) \
             VALUES ('sch_nightly', 'prj_web', 'Nightly', 'Update.', 'usr_ada', 6, 0, '0 3 * * *')",
        ] {
            run_sql(db, sql).await?;
        }
        Ok(())
    }

    fn sent(installation: u64, account: u64) -> Sent<'static> {
        Sent {
            installation,
            account,
            delivery: "d1",
        }
    }

    fn ids(rows: &[ScheduleRow]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[wasm_bindgen_test]
    async fn finds_the_schedules_that_wait_for_an_event() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        let web = RepoRef {
            owner: "Acme".into(),
            name: "Web".into(),
        };
        let found = waiting(&db, &web, ScheduleEventKind::IssueOpened, &sent(7, 70)).await?;
        // Not the paused one, another kind's, another repository's, an
        // archived project's, nor one that runs at its time.
        assert_eq!(ids(&found), ["sch_triage"]);
        assert_eq!(
            found.first().and_then(ScheduleRow::event).map(|e| e.kind),
            Some(ScheduleEventKind::IssueOpened)
        );
        // More than a delivery starts sessions for: each is looked at, so
        // that what narrows them down decides, not how old they are.
        for i in 0..12 {
            run_sql(
                &db,
                &format!(
                    "INSERT INTO schedules (id, project_id, name, prompt, owner_id, created_at, updated_at, event_kind, event_filter) \
                     VALUES ('sch_bug{i}', 'prj_web', 'Bugs', 'Fix it.', 'usr_ada', {}, 0, 'issue_opened', '{{\"labels\":[\"bug\"]}}')",
                    10 + i
                ),
            )
            .await?;
        }
        let all = waiting(&db, &web, ScheduleEventKind::IssueOpened, &sent(7, 70)).await?;
        assert_eq!(all.len(), 13);
        assert_eq!(
            all.last().and_then(ScheduleRow::event).map(|e| e.labels),
            Some(vec!["bug".to_owned()])
        );
        // Another installation on the account speaks for no workspace here.
        assert!(
            waiting(&db, &web, ScheduleEventKind::IssueOpened, &sent(8, 70))
                .await?
                .is_empty()
        );
        // A workspace that may use only some of the account's repositories.
        run_sql(
            &db,
            "UPDATE workspace_github SET repos = '[\"api\"]' WHERE workspace_id = 'wsp_1'",
        )
        .await?;
        assert!(
            waiting(&db, &web, ScheduleEventKind::IssueOpened, &sent(7, 70))
                .await?
                .is_empty()
        );
        Ok(())
    }

    async fn claim(db: &D1Database, id: &str, key: &str) -> AppResult<usize> {
        q(
            db,
            "INSERT OR IGNORE INTO schedule_runs (id, schedule_id, due_at, at, outcome, trigger, event_key, subject) \
             VALUES (?, 'sch_triage', 1, 1, 'failed', 'event', ?, 'acme/web#41')",
            &[s(id), s(key)],
        )?
        .run()
        .await
    }

    #[wasm_bindgen_test]
    async fn a_run_is_claimed_once() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        assert_eq!(claim(&db, "shr_1", "issue_opened:acme/web#41").await?, 1);
        // GitHub sent it again.
        assert_eq!(claim(&db, "shr_2", "issue_opened:acme/web#41").await?, 0);
        assert_eq!(claim(&db, "shr_3", "issue_opened:acme/web#42").await?, 1);
        // Runs at a time keep no key, and many have none.
        for id in ["shr_4", "shr_5"] {
            q(
                &db,
                "INSERT INTO schedule_runs (id, schedule_id, due_at, at, outcome, trigger) VALUES (?, 'sch_triage', 1, 1, 'started', 'time')",
                &[s(id)],
            )?
            .run()
            .await?;
        }
        Ok(())
    }

    /// A schedule past those one delivery starts sessions for records why,
    /// once, in one statement.
    #[wasm_bindgen_test]
    async fn one_past_the_cap_says_so_once() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        let web = RepoRef {
            owner: "acme".into(),
            name: "web".into(),
        };
        let rows = waiting(&db, &web, ScheduleEventKind::IssueOpened, &sent(7, 70)).await?;
        let Some(row) = rows.first() else {
            return Err(crate::error::internal("no schedule waits"));
        };
        let h = Happened {
            kind: Some(ScheduleEventKind::IssueOpened),
            repo: web.clone(),
            number: Some(41),
            url: "https://github.com/acme/web/issues/41".into(),
            ..Default::default()
        };
        assert_eq!(pass_over(&db, row, &h, "d1", 5).await?, 1);
        assert_eq!(pass_over(&db, row, &h, "d2", 6).await?, 0);
        let runs = schedules::runs(&db, &row.id, None, 10).await?;
        let said: Vec<(String, Option<String>)> = runs
            .iter()
            .map(|r| {
                (
                    r.outcome.clone(),
                    r.event.clone().map(|_| "event".to_owned()),
                )
            })
            .collect();
        assert_eq!(said, [("skipped".to_owned(), Some("event".to_owned()))]);
        Ok(())
    }

    /// The caps count the sessions events started in the last day, and the
    /// runs other deliveries claimed just before and still decide, in the
    /// order D1 inserted them, whatever their ids.
    #[wasm_bindgen_test]
    async fn the_caps_count_the_days_sessions_and_those_starting() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        let now = 10 * fctl_domain::cron::DAY_MS;
        let hour = fctl_domain::cron::HOUR_MS;
        let minute = fctl_domain::cron::MINUTE_MS;
        // In the order they are inserted.
        let runs = [
            // Started by events: an hour ago, 23 hours ago, and two days ago.
            ("shr_a", "sch_triage", now - hour, "started", "event", None),
            (
                "shr_b",
                "sch_triage",
                now - 23 * hour,
                "started",
                "event",
                None,
            ),
            (
                "shr_c",
                "sch_triage",
                now - 2 * fctl_domain::cron::DAY_MS,
                "started",
                "event",
                None,
            ),
            // Skipped, failed, run by a person, or another schedule's.
            (
                "shr_d",
                "sch_triage",
                now - minute,
                "skipped",
                "event",
                Some("Busy."),
            ),
            (
                "shr_e",
                "sch_triage",
                now - minute,
                "failed",
                "event",
                Some("No runner."),
            ),
            (
                "shr_f",
                "sch_triage",
                now - minute,
                "started",
                "person",
                None,
            ),
            (
                "shr_g",
                "sch_merged",
                now - minute,
                "started",
                "event",
                None,
            ),
            // Claims still undecided: one whose Worker stopped an hour ago,
            // then three of a burst, the first with the largest id, as a
            // delivery that came later can claim first.
            (
                "shr_h0",
                "sch_triage",
                now - hour,
                "failed",
                "event",
                Some(UNDECIDED),
            ),
            (
                "shr_z9",
                "sch_triage",
                now - 1_000,
                "failed",
                "event",
                Some(UNDECIDED),
            ),
            (
                "shr_x1",
                "sch_triage",
                now - 1_000,
                "failed",
                "event",
                Some(UNDECIDED),
            ),
            (
                "shr_x2",
                "sch_triage",
                now - 1_000,
                "failed",
                "event",
                Some(UNDECIDED),
            ),
        ];
        for (id, schedule, at, outcome, trigger, reason) in runs {
            q(
                &db,
                "INSERT INTO schedule_runs (id, schedule_id, due_at, at, outcome, trigger, reason) VALUES (?, ?, ?, ?, ?, ?, ?)",
                &[s(id), s(schedule), n(at), n(at), s(outcome), s(trigger), os(reason)],
            )?
            .run()
            .await?;
        }
        let caps = tg::Caps { hour: 3, day: 10 };
        let started = [now - hour, now - 23 * hour];
        // Listed, the schedule counts what started.
        assert_eq!(
            started_lately(&db, "sch_triage", caps, now, None).await?,
            started
        );
        // The burst's first claim counts none of the others; each later one
        // counts those claimed before it.
        assert_eq!(
            started_lately(&db, "sch_triage", caps, now, Some("shr_z9")).await?,
            started
        );
        assert_eq!(
            started_lately(&db, "sch_triage", caps, now, Some("shr_x1")).await?,
            [now - 1_000, now - hour, now - 23 * hour]
        );
        assert_eq!(
            started_lately(&db, "sch_triage", caps, now, Some("shr_x2")).await?,
            [now - 1_000, now - 1_000, now - hour, now - 23 * hour]
        );
        // As many as the caps count at most, newest first.
        assert_eq!(
            started_lately(
                &db,
                "sch_triage",
                tg::Caps { hour: 1, day: 1 },
                now,
                Some("shr_x2")
            )
            .await?,
            [now - 1_000]
        );
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn a_run_waits_while_the_last_on_the_same_thing_works() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        for sql in [
            "INSERT INTO claude_credentials (id, user_id, kind, secret_enc, created_at, updated_at) \
             VALUES ('crd_ada', 'usr_ada', 'oauth', 'sealed', 0, 0)",
            "INSERT INTO sessions (id, project_id, number, title, prompt, status, created_by, credential_id, \
             claude_session_id, created_at, updated_at, schedule_id) \
             VALUES ('ses_1', 'prj_web', 1, 'Triage: web#41', 'Triage it.', 'running', 'usr_ada', 'crd_ada', 'ses_1', 1, 1, 'sch_triage')",
            "INSERT INTO schedule_runs (id, schedule_id, due_at, at, outcome, trigger, session_id, event_key, subject) \
             VALUES ('shr_1', 'sch_triage', 1, 1, 'started', 'event', 'ses_1', 'k1', 'acme/web#41')",
        ] {
            run_sql(&db, sql).await?;
        }
        assert_eq!(
            still_at_it(&db, "sch_triage", "acme/web#41", "shr_2").await?,
            Some(1)
        );
        assert_eq!(
            still_at_it(&db, "sch_triage", "acme/web#42", "shr_2").await?,
            None
        );
        run_sql(
            &db,
            "UPDATE sessions SET status = 'idle' WHERE id = 'ses_1'",
        )
        .await?;
        assert_eq!(
            still_at_it(&db, "sch_triage", "acme/web#41", "shr_2").await?,
            None
        );
        Ok(())
    }
}
