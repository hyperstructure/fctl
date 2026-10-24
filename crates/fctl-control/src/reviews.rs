//! Review agents (`fctl_domain::reviews`): a project's settings for them,
//! the review that GitHub's webhook starts when a pull request gets a new
//! commit, its session and its check on the commit, and what the control
//! plane posts for its agent, whose own GitHub token only reads.
//!
//! A review runs as a session of its project, on the Claude subscription of
//! the person the project's review agents run for (its operator). It takes
//! no messages; its agent reviews the pull request at one commit and hands
//! what it found to the control plane (`fctl review submit`, an agent
//! call), which posts it, a summary followed by one review, marks the
//! earlier findings it fixed, says so atop earlier reviews' summaries and
//! completes the check. A new commit stops a review of the pull request
//! that is still under way.

use crate::access;
use crate::config::Config;
use crate::ctx::{Ctx, now_ms};
use crate::db::{int, int_bool, n, opt_int, os, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, not_found};
use crate::github::{self, Github};
use crate::internal::{self, InitSession, Schedule, Stop};
use crate::model::{CredentialRow, PROJECT_SELECT, ProjectRow, UserRow, user_columns};
use fctl_domain::reviews::{self as rv, Links, PullRequest};
use fctl_proto::api::{FindingThread, PostedReview, Review, ReviewBot, ReviewState};
use fctl_proto::event::{MessageSource, UserMessage};
use fctl_proto::runner::{GithubCredential, StopReason};
use fctl_proto::session::{
    GithubAccess, PullRequestRef, RepoAccess, RepoRef, SessionStats, UserRef,
};
use fctl_proto::{ApiError, ErrorCode, SessionId, UserId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use worker::{D1Database, Env};

/// A project's review agents, as D1 keeps them (`project_reviews`).
#[derive(Clone, Debug, Deserialize)]
pub struct SettingsRow {
    #[serde(deserialize_with = "int_bool")]
    pub enabled: bool,
    pub operator_id: Option<String>,
    pub operator_name: Option<String>,
    pub operator_avatar: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub instructions: Option<String>,
    /// The apps whose pull requests start reviews by themselves, as JSON.
    #[serde(default)]
    pub bots: Option<String>,
    pub updated_by: Option<String>,
    pub updater_name: Option<String>,
    pub updater_avatar: Option<String>,
    #[serde(deserialize_with = "int")]
    pub updated_at: i64,
}

impl SettingsRow {
    pub fn operator(&self) -> Option<UserRef> {
        Some(UserRef {
            id: UserId(self.operator_id.clone()?),
            name: self.operator_name.clone().unwrap_or_default(),
            avatar_url: self.operator_avatar.clone(),
        })
    }

    pub fn updater(&self) -> Option<UserRef> {
        Some(UserRef {
            id: UserId(self.updated_by.clone()?),
            name: self.updater_name.clone().unwrap_or_default(),
            avatar_url: self.updater_avatar.clone(),
        })
    }
}

/// The apps whose pull requests start reviews by themselves in a project
/// with these settings (`None`: none yet), as their bots are named.
pub fn bots(conf: Option<&SettingsRow>) -> Vec<String> {
    match conf {
        Some(c) => rv::stored_bots(c.bots.as_deref()),
        None => rv::default_bots(),
    }
}

/// A project's review agents; `None` until someone set them.
pub async fn settings(db: &D1Database, project_id: &str) -> AppResult<Option<SettingsRow>> {
    q(
        db,
        "SELECT r.enabled, r.operator_id, ou.name AS operator_name, ou.avatar_url AS operator_avatar, \
         r.model, r.effort, r.instructions, r.bots, r.updated_by, uu.name AS updater_name, \
         uu.avatar_url AS updater_avatar, r.updated_at FROM project_reviews r \
         LEFT JOIN users ou ON ou.id = r.operator_id LEFT JOIN users uu ON uu.id = r.updated_by \
         WHERE r.project_id = ?",
        &[s(project_id)],
    )?
    .first()
    .await
}

/// What D1 says of a review while fctl posts it (`submit`), which the API
/// shows as under way.
const POSTING: &str = "posting";

/// A review, with its session (`reviews` and `sessions`).
#[derive(Clone, Debug, Deserialize)]
pub struct ReviewRow {
    pub session_id: String,
    pub project_id: String,
    pub repo: String,
    #[serde(deserialize_with = "int")]
    pub number: i64,
    pub head_sha: String,
    pub url: String,
    pub title: Option<String>,
    pub author: Option<String>,
    #[serde(deserialize_with = "int_bool", default)]
    pub draft: bool,
    #[serde(deserialize_with = "int_bool", default)]
    pub fork: bool,
    #[serde(deserialize_with = "int_bool", default)]
    pub trusted: bool,
    pub previous_sha: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub installation_id: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    pub check_run_id: Option<i64>,
    pub check_error: Option<String>,
    pub state: String,
    pub reason: Option<String>,
    #[serde(deserialize_with = "int", default)]
    pub findings: i64,
    #[serde(deserialize_with = "int", default)]
    pub resolved: i64,
    #[serde(deserialize_with = "int", default)]
    pub still_open: i64,
    pub review_url: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "opt_int", default)]
    pub finished_at: Option<i64>,
    #[serde(deserialize_with = "int", default)]
    pub session_number: i64,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub stats: Option<String>,
    pub operator_id: String,
    pub operator_name: Option<String>,
    pub operator_avatar: Option<String>,
    /// The pull request's branch, which its session is named after.
    #[serde(default)]
    pub head_ref: Option<String>,
}

/// SELECT for [`ReviewRow`]; `WHERE` and `ORDER BY` follow.
const REVIEW_SELECT: &str = "SELECT rv.session_id, rv.project_id, rv.repo, rv.number, rv.head_sha, rv.url, \
    rv.title, rv.author, rv.draft, rv.fork, rv.trusted, rv.previous_sha, rv.installation_id, rv.check_run_id, rv.check_error, rv.state, \
    rv.reason, rv.findings, rv.resolved, rv.still_open, rv.review_url, rv.created_at, rv.finished_at, \
    s.number AS session_number, s.status AS status, s.stats AS stats, s.created_by AS operator_id, \
    u.name AS operator_name, u.avatar_url AS operator_avatar, s.branch AS head_ref \
    FROM reviews rv JOIN sessions s ON s.id = rv.session_id LEFT JOIN users u ON u.id = s.created_by";

impl ReviewRow {
    pub fn repo(&self) -> Option<RepoRef> {
        RepoRef::parse(&self.repo)
    }

    pub fn state(&self) -> ReviewState {
        if self.state == POSTING {
            return ReviewState::Running;
        }
        ReviewState::parse(&self.state)
    }

    pub fn number(&self) -> u64 {
        u64::try_from(self.number).unwrap_or_default()
    }

    pub fn to_review(&self) -> Option<Review> {
        let stats: SessionStats = self
            .stats
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default();
        Some(Review {
            session_id: SessionId(self.session_id.clone()),
            session_number: u32::try_from(self.session_number).unwrap_or_default(),
            status: self
                .status
                .as_deref()
                .and_then(fctl_proto::session::SessionStatus::parse)
                .unwrap_or(fctl_proto::session::SessionStatus::Failed),
            state: self.state(),
            reason: self.reason.clone(),
            repo: self.repo()?,
            number: self.number(),
            url: self.url.clone(),
            title: self.title.clone(),
            author: self.author.clone(),
            draft: self.draft,
            head_sha: self.head_sha.clone(),
            findings: count(self.findings),
            resolved: count(self.resolved),
            still_open: count(self.still_open),
            review_url: self.review_url.clone(),
            check_error: self.check_error.clone(),
            operator: UserRef {
                id: UserId(self.operator_id.clone()),
                name: self.operator_name.clone().unwrap_or_default(),
                avatar_url: self.operator_avatar.clone(),
            },
            created_at: self.created_at,
            finished_at: self.finished_at,
            busy_ms: stats.busy_ms,
        })
    }
}

fn count(n: i64) -> u32 {
    u32::try_from(n.max(0)).unwrap_or(u32::MAX)
}

/// The review of a review agent's session.
pub async fn of_session(db: &D1Database, session_id: &str) -> AppResult<Option<ReviewRow>> {
    q(
        db,
        &format!("{REVIEW_SELECT} WHERE rv.session_id = ?"),
        &[s(session_id)],
    )?
    .first()
    .await
}

/// A project's reviews, newest first: before `before`, at most `limit`.
pub async fn list(
    db: &D1Database,
    project_id: &str,
    before: Option<i64>,
    limit: i64,
) -> AppResult<Vec<ReviewRow>> {
    q(
        db,
        &format!(
            "{REVIEW_SELECT} WHERE rv.project_id = ?1 AND (?2 IS NULL OR rv.created_at < ?2) \
             ORDER BY rv.created_at DESC LIMIT ?3"
        ),
        &[s(project_id), crate::db::on(before), n(limit)],
    )?
    .all()
    .await
}

// ───────────────────────────── who runs them ─────────────────────────────

/// What a review runs as: the operator and their Claude connection.
pub struct Operator {
    pub user: UserRow,
    pub credential: CredentialRow,
}

/// The person a project's reviews run for and their Claude connection, or
/// why no review can run on their subscription now: there is nobody, or
/// they can no longer start sessions in the project (they left it or its
/// workspace, or were disabled), or they have no Claude connection.
pub async fn operator(
    db: &D1Database,
    project: &ProjectRow,
    conf: &SettingsRow,
) -> AppResult<Result<Operator, String>> {
    let Some(id) = conf.operator_id.as_deref() else {
        return Ok(Err(
            "Nobody runs them: turning them on runs them on your Claude subscription.".into(),
        ));
    };
    let user: Option<UserRow> = q(
        db,
        &format!("SELECT {} FROM users u WHERE u.id = ?", user_columns("u")),
        &[s(id)],
    )?
    .first()
    .await?;
    let name = conf
        .operator_name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "The person they ran for".into());
    let Some(user) = user.filter(UserRow::is_active) else {
        return Ok(Err(format!(
            "They run on {name}'s Claude subscription, and {name}'s account is gone or disabled. A project owner runs them on theirs."
        )));
    };
    let row: Option<ProjectRow> = q(
        db,
        &format!("{PROJECT_SELECT} WHERE p.id = ?2"),
        &[s(&user.id), s(&project.id)],
    )?
    .first()
    .await?;
    let settings = crate::instance::settings(db).await?;
    let role = row.and_then(|r| access::project_role(&user, &settings, None, &r));
    if !fctl_domain::perm::can_start_sessions(role) {
        return Ok(Err(format!(
            "They run on {name}'s Claude subscription, and {name} can no longer start sessions in this project. A project owner runs them on theirs."
        )));
    }
    let credential =
        crate::api::sessions::credential_for(db, &user, project.workspace_id()).await?;
    let Some(credential) = credential else {
        return Ok(Err(format!(
            "They run on {name}'s Claude subscription, and {name} has no Claude connection. They connect one under Settings → Claude, or a project owner runs reviews on theirs."
        )));
    };
    Ok(Ok(Operator { user, credential }))
}

/// The repositories whose pull requests a project's review agents review:
/// its primary repository and those it lists as read and write.
pub async fn repos(db: &D1Database, project: &ProjectRow) -> AppResult<Vec<RepoRef>> {
    let mut repos: Vec<RepoRef> = project.repo().into_iter().collect();
    for g in crate::spec::grants(db, &project.id).await? {
        if g.access == RepoAccess::Write
            && let Some(r) = g.repo()
            && !repos.iter().any(|x| x.same(&r))
        {
            repos.push(r);
        }
    }
    Ok(repos)
}

// ───────────────────────────── apps that start reviews ─────────────────────────────

/// The apps that opened the newest pull requests in the project's
/// repositories from their branches (`rv::bots_query`), through the
/// installations its workspace connected; repositories it can't reach are
/// left out.
pub async fn seen_bots(ctx: &Ctx, project: &ProjectRow) -> AppResult<Vec<ReviewBot>> {
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    let mut by_installation: Vec<(u64, Vec<RepoRef>)> = Vec::new();
    for repo in repos(&ctx.db, project)
        .await?
        .into_iter()
        .take(rv::MAX_BOTS_REPOS)
    {
        let inst = match reach(ctx, project, &repo).await {
            Ok(inst) => inst,
            Err(e) => {
                worker::console_log!(
                    "not reading the pull requests of {}: {}",
                    repo.full_name(),
                    e.0.message
                );
                continue;
            }
        };
        match by_installation.iter_mut().find(|(i, _)| *i == inst) {
            Some((_, list)) => list.push(repo),
            None => by_installation.push((inst, vec![repo])),
        }
    }
    let mut answers = Vec::new();
    for (inst, list) in by_installation {
        answers.push(gh.lookup(inst, rv::bots_query(&list)).await?);
    }
    Ok(rv::bots_seen(&answers, app_login(&ctx.cfg).as_deref()))
}

/// The app someone names (`rv::bot_login`), as GitHub shows it through an
/// installation of the project's: `None` where it shows none by that name,
/// as it shows no other account's private apps.
pub async fn find_bot(ctx: &Ctx, project: &ProjectRow, name: &str) -> AppResult<Option<ReviewBot>> {
    let Some(login) = rv::bot_login(name) else {
        return Err(AppError(ApiError::validation(
            "bots",
            "Name a GitHub App as renovate, renovate[bot] or https://github.com/apps/renovate",
        )));
    };
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    let mut why = None;
    for repo in repos(&ctx.db, project).await? {
        match reach(ctx, project, &repo).await {
            Ok(inst) => {
                let slug = login.trim_end_matches("[bot]");
                return Ok(gh.app(inst, slug).await?.as_ref().and_then(rv::app_found));
            }
            Err(e) => why = Some(e),
        }
    }
    Err(why.unwrap_or_else(|| {
        AppError(ApiError::validation(
            "bots",
            "This project has no repository its agents may write to, through which fctl would ask GitHub. Link one under Repositories.",
        ))
    }))
}

/// Why no review of the project starts now, or would start once its
/// review agents are on.
pub async fn problem(
    cfg: &Config,
    db: &D1Database,
    project: &ProjectRow,
    conf: Option<&SettingsRow>,
) -> AppResult<Option<String>> {
    let Some(gh) = cfg.github.as_ref() else {
        return Ok(Some(
            "This instance has no GitHub App, so nothing reviews pull requests. Its admins set one up (docs/self-hosting.md).".into(),
        ));
    };
    if repos(db, project).await?.is_empty() {
        return Ok(Some(
            "This project has no repository its agents may write to, whose pull requests they would review. Link one under Repositories.".into(),
        ));
    }
    if let Some(conf) = conf.filter(|c| c.enabled)
        && let Err(why) = operator(db, project, conf).await?
    {
        return Ok(Some(why));
    }
    if gh.webhook_secret.is_none() {
        return Ok(Some(
            "This instance takes no GitHub webhooks, so no review starts on its own when a pull request changes; reviews people ask for still run. Its admins set GITHUB_WEBHOOK_SECRET.".into(),
        ));
    }
    Ok(None)
}

// ───────────────────────────── starting reviews ─────────────────────────────

/// What came of starting a review.
pub enum Started {
    /// A new review, by its session.
    Review(String),
    /// The commit has its review already.
    Already,
    /// No review runs, and why.
    Skipped(String),
}

/// Starts the reviews a `pull_request` delivery asks for: one in each
/// project whose review agents are on and whose agents may write to its
/// repository, through the installation the project's workspace connected.
/// Returns how many started.
pub async fn on_delivery(ctx: &Ctx, payload: &Value) -> AppResult<usize> {
    let Some((action, pr)) = rv::delivered(payload) else {
        return Ok(0);
    };
    if !rv::starts_review(&action, &pr) {
        return Ok(0);
    }
    // A review runs on its operator's Claude subscription, whose token is in
    // its sandbox, where Claude acts on what the pull request says: text in
    // anyone's pull request could make it hand that token over, with nobody
    // in the project having looked. Only pull requests of people who may
    // write to the repository, fctl's own agents' and those of the apps the
    // project's owners chose (whose code its agent doesn't run) start one by
    // themselves; anyone else's waits for someone in the project to ask for
    // it (`POST /api/v1/projects/{id}/reviews`).
    let app = app_login(&ctx.cfg);
    if !rv::could_start_by_itself(&pr, app.as_deref()) {
        worker::console_log!(
            "not reviewing {}#{} by itself: its author can't write to the repository",
            pr.repo.full_name(),
            pr.number
        );
        return Ok(0);
    }
    let trusted = rv::trusted(&pr, app.as_deref());
    let sent_by = payload.pointer("/installation/id").and_then(Value::as_u64);
    #[derive(Deserialize)]
    struct Id {
        id: String,
    }
    let key = pr.repo.full_name().to_ascii_lowercase();
    let ids: Vec<Id> = q(
        &ctx.db,
        "SELECT p.id AS id FROM project_reviews r JOIN projects p ON p.id = r.project_id \
         WHERE r.enabled = 1 AND p.archived_at IS NULL AND (lower(p.repo_owner || '/' || p.repo_name) = ?1 \
         OR EXISTS (SELECT 1 FROM project_repos pr WHERE pr.project_id = p.id AND pr.access = 'write' \
           AND lower(pr.owner || '/' || pr.name) = ?1)) ORDER BY p.created_at LIMIT 20",
        &[s(&key)],
    )?
    .all()
    .await?;
    let mut started = 0;
    for Id { id } in ids {
        let Some(project) = crate::spec::project_row(&ctx.db, &id).await? else {
            continue;
        };
        if !trusted && !rv::by_bot(&pr, &bots(settings(&ctx.db, &id).await?.as_ref())) {
            worker::console_log!(
                "not reviewing {}#{} by itself in {}: its owners don't let {} start reviews",
                pr.repo.full_name(),
                pr.number,
                project.slug,
                pr.author.as_deref().unwrap_or("its app")
            );
            continue;
        }
        let inst = match reach(ctx, &project, &pr.repo).await {
            Ok(inst) => inst,
            Err(e) => {
                worker::console_log!(
                    "no review of {}#{} in {}: {}",
                    pr.repo.full_name(),
                    pr.number,
                    project.slug,
                    e.0.message
                );
                continue;
            }
        };
        // Only the installation the workspace connected speaks for it.
        if sent_by.is_some_and(|s| s != inst) {
            continue;
        }
        match start(ctx, &project, &pr, inst, None).await {
            Ok(Started::Review(_)) => started += 1,
            Ok(Started::Already) => {}
            Ok(Started::Skipped(why)) => worker::console_log!(
                "no review of {}#{} in {}: {why}",
                pr.repo.full_name(),
                pr.number,
                project.slug
            ),
            Err(e) => worker::console_error!(
                "starting a review of {}#{} in {}: {}",
                pr.repo.full_name(),
                pr.number,
                project.slug,
                e.0.message
            ),
        }
    }
    Ok(started)
}

/// The installation of the GitHub App through which `project` reaches
/// `repo`: one its agents may write to, of an account its workspace
/// connected, within the repositories the workspace may use.
pub async fn reach(ctx: &Ctx, project: &ProjectRow, repo: &RepoRef) -> AppResult<u64> {
    if !repos(&ctx.db, project).await?.iter().any(|r| r.same(repo)) {
        return Err(AppError(ApiError::validation(
            "pull_request",
            "Review agents review pull requests in the repositories this project's agents may write to",
        )));
    }
    let connections = crate::workspaces::connections(&ctx.db, project.workspace_id()).await?;
    let allowed = crate::workspaces::scope(&connections, &repo.owner)
        .is_some_and(|scope| scope.allows(&repo.name));
    if !allowed {
        return Err(forbidden(&format!(
            "This project's workspace may not use {}. Its admins connect {} under the workspace's GitHub tab.",
            repo.full_name(),
            repo.owner
        )));
    }
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    crate::workspaces::installation(&ctx.db, &gh, &connections, &repo.owner).await
}

/// Starts a review of `pr` at its head in `project`, through installation
/// `inst`, as its review agents' operator: a session, its row in
/// `reviews`, and on GitHub its check and a reaction. A review of the pull
/// request still under way stops: the new one takes its place. `asked_by`:
/// the person who asked for it; a delivery's review of a commit that has
/// one already isn't repeated.
pub async fn start(
    ctx: &Ctx,
    project: &ProjectRow,
    pr: &PullRequest,
    inst: u64,
    asked_by: Option<&UserRow>,
) -> AppResult<Started> {
    let Some(conf) = settings(&ctx.db, &project.id).await?.filter(|c| c.enabled) else {
        return Ok(Started::Skipped("its review agents are off".into()));
    };
    let op = match operator(&ctx.db, project, &conf).await? {
        Ok(op) => op,
        Err(why) => return Ok(Started::Skipped(why)),
    };
    let key = pr.repo.full_name().to_ascii_lowercase();
    #[derive(Deserialize)]
    struct Attempt {
        #[serde(deserialize_with = "opt_int", default)]
        attempt: Option<i64>,
    }
    let last: Option<Attempt> = q(
        &ctx.db,
        "SELECT MAX(attempt) AS attempt FROM reviews WHERE project_id = ? AND repo_key = ? AND number = ? AND head_sha = ?",
        &[s(&project.id), s(&key), n(pr.number as i64), s(&pr.head_sha)],
    )?
    .first()
    .await?;
    let attempt = match (last.and_then(|a| a.attempt), asked_by) {
        (Some(_), None) => return Ok(Started::Already),
        (Some(a), Some(_)) => a + 1,
        (None, _) => 0,
    };
    // Deliveries come out of order: one about an older head, late, starts
    // nothing in place of the review of a newer one.
    if asked_by.is_none()
        && let Some(updated) = pr.updated_at
    {
        #[derive(Deserialize)]
        struct Newest {
            #[serde(deserialize_with = "opt_int", default)]
            at: Option<i64>,
        }
        let newest: Option<Newest> = q(
            &ctx.db,
            "SELECT MAX(pr_updated_at) AS at FROM reviews WHERE project_id = ? AND repo_key = ? AND number = ?",
            &[s(&project.id), s(&key), n(pr.number as i64)],
        )?
        .first()
        .await?;
        if newest.and_then(|n| n.at).is_some_and(|at| at > updated) {
            return Ok(Started::Skipped(
                "a newer commit of it has its review already".into(),
            ));
        }
    }
    #[derive(Deserialize)]
    struct Head {
        head_sha: String,
    }
    let previous: Option<Head> = q(
        &ctx.db,
        "SELECT head_sha FROM reviews WHERE project_id = ? AND repo_key = ? AND number = ? AND head_sha != ? \
         AND state IN ('passed', 'commented') ORDER BY created_at DESC LIMIT 1",
        &[s(&project.id), s(&key), n(pr.number as i64), s(&pr.head_sha)],
    )?
    .first()
    .await?;
    let previous = previous.map(|h| h.head_sha);
    let because = if asked_by.is_some() && attempt > 0 {
        "Someone asked for another review of this commit.".to_owned()
    } else {
        format!(
            "A newer commit came, {}: fctl reviews that one instead.",
            rv::short(&pr.head_sha)
        )
    };

    let id = fctl_domain::ids::session_id(ctx.now);
    let title = rv::title(pr);
    let task = rv::task(pr, previous.as_deref());
    let listed = PullRequestRef {
        number: pr.number,
        url: pr.url.clone(),
        title: Some(pr.title.clone()),
        state: Some(if pr.draft { "draft" } else { "open" }.to_owned()),
        repo: Some(pr.repo.clone()),
        ..Default::default()
    };
    let pull_requests = serde_json::to_string(&[&listed])?;
    let statements = vec![
        q(
            &ctx.db,
            "UPDATE projects SET session_counter = session_counter + 1 WHERE id = ?",
            &[s(&project.id)],
        )?,
        q(
            &ctx.db,
            "INSERT INTO sessions (id, project_id, number, title, prompt, status, created_by, credential_id, branch, base_branch, \
             model, effort, claude_session_id, stats, created_at, updated_at, last_activity_at, github_access, pull_requests) \
             VALUES (?1, ?2, (SELECT session_counter FROM projects WHERE id = ?2), ?3, ?4, 'queued', ?5, ?6, ?7, ?8, ?9, ?10, ?11, '{}', ?12, ?12, ?12, 'read', ?13)",
            &[
                s(id.as_str()),
                s(&project.id),
                s(&title),
                s(&task),
                s(&op.user.id),
                s(&op.credential.id),
                s(&pr.head_ref),
                s(&pr.base_ref),
                os(conf.model.as_deref()),
                os(conf.effort.as_deref()),
                s(&fctl_domain::ids::uuid_v4()),
                n(ctx.now),
                s(&pull_requests),
            ],
        )?,
        q(
            &ctx.db,
            "INSERT INTO reviews (session_id, project_id, repo, repo_key, number, head_sha, attempt, url, title, author, draft, \
             fork, trusted, previous_sha, pr_updated_at, installation_id, state, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'running', ?)",
            &[
                s(id.as_str()),
                s(&project.id),
                s(&pr.repo.full_name()),
                s(&key),
                n(pr.number as i64),
                s(&pr.head_sha),
                n(attempt),
                s(&pr.url),
                s(&pr.title),
                os(pr.author.as_deref()),
                crate::db::b(pr.draft),
                crate::db::b(pr.fork),
                crate::db::b(rv::trusted(pr, app_login(&ctx.cfg).as_deref())),
                os(previous.as_deref()),
                crate::db::on(pr.updated_at),
                n(inst as i64),
                n(ctx.now),
            ],
        )?,
    ];
    if let Err(e) = crate::db::batch(&ctx.db, statements).await {
        // Another delivery about the same commit started it meanwhile.
        if e.0.message.contains("UNIQUE") {
            return Ok(Started::Already);
        }
        return Err(e);
    }
    // The reviews of the pull request still under way stop: this one takes
    // their place. Only once it is in, so that a delivery that turns out
    // to repeat another stops nothing.
    supersede(ctx, project, pr, inst, id.as_str(), &because).await?;
    let session = crate::spec::session_row(&ctx.db, id.as_str())
        .await?
        .ok_or_else(|| crate::error::internal("the review's session vanished"))?
        .to_session(None);
    let stub = internal::session_stub(&ctx.env, id.as_str())?;
    let _: Value = internal::post(
        &stub,
        "/init",
        &InitSession {
            session,
            prompt: UserMessage {
                ticket: None,
                id: fctl_domain::ids::delivery_id(ctx.now),
                author: None,
                text: task,
                source: MessageSource::System,
                attachments: Vec::new(),
                follow_up: None,
                review: None,
            },
        },
    )
    .await?;
    let fleet = internal::fleet_stub(&ctx.env)?;
    let _: Value = internal::post(
        &fleet,
        "/schedule",
        &Schedule {
            session_id: id.clone(),
        },
    )
    .await?;
    // So that what GitHub says of it reaches the session.
    if let Err(e) = crate::pull_requests::track(&ctx.db, id.as_str(), &[listed]).await {
        worker::console_error!("following a reviewed pull request: {}", e.0.message);
    }
    ctx.audit_in(
        Some(project.workspace_id()),
        "review.started",
        Some(id.as_str()),
        Some(json!({
            "project": project.slug,
            "pull_request": format!("{}#{}", pr.repo.full_name(), pr.number),
            "commit": pr.head_sha,
            "operator": op.user.id,
            "asked_by": asked_by.map(|u| &u.id),
        })),
    );
    // Its check and the reaction that says fctl picked the pull request up,
    // after the answer: GitHub's webhook waits for it ten seconds at most.
    let (env, cfg, pr, session_id) = (ctx.env.clone(), ctx.cfg.clone(), pr.clone(), id.0.clone());
    ctx.defer(async move { announce(&env, &cfg, &pr, inst, &session_id).await });
    Ok(Started::Review(id.0))
}

/// The GitHub App's bot, as pull requests name their author:
/// `fctl[bot]`.
fn app_login(cfg: &Config) -> Option<String> {
    cfg.github.as_ref().map(|g| format!("{}[bot]", g.slug))
}

/// What a review's agent is told and checks out (`SessionSpec::review`).
pub fn spec_of(
    row: &ReviewRow,
    head_ref: &str,
    base_ref: &str,
    project: Option<&str>,
) -> Option<fctl_proto::runner::ReviewSpec> {
    let repo = row.repo()?;
    let pr = PullRequest {
        repo: repo.clone(),
        number: row.number(),
        url: row.url.clone(),
        title: row.title.clone().unwrap_or_default(),
        head_sha: row.head_sha.clone(),
        head_ref: head_ref.to_owned(),
        base_ref: base_ref.to_owned(),
        author: row.author.clone(),
        association: None,
        draft: row.draft,
        open: true,
        fork: row.fork,
        updated_at: None,
    };
    Some(fctl_proto::runner::ReviewSpec {
        instructions: rv::instructions(&pr, row.trusted, project),
        repo,
        number: pr.number,
        url: pr.url,
        title: pr.title,
        head_sha: pr.head_sha,
        head_ref: pr.head_ref,
        base_ref: pr.base_ref,
        previous_sha: row.previous_sha.clone(),
    })
}

/// Shows on GitHub that a review started: its check on the commit, queued
/// until a runner takes the session, and an eyes reaction on the pull
/// request. Keeps the check's id, or why GitHub has none.
async fn announce(
    env: &Env,
    cfg: &Config,
    pr: &PullRequest,
    inst: u64,
    session_id: &str,
) -> AppResult<()> {
    let db = env.d1("DB")?;
    let gh = github::from_config(cfg, now_ms())?;
    let session_url = cfg.url(&format!("/s/{session_id}"));
    let (token, mut check_error) = gh.review_token(inst, &pr.repo).await?;
    let mut check = None;
    if check_error.is_none() {
        match gh
            .create_check(
                &token,
                &pr.repo,
                rv::check_create(&pr.head_sha, session_id, &session_url),
            )
            .await
        {
            Ok(Ok(id)) => check = Some(id),
            Ok(Err(why)) => check_error = Some(format!("GitHub refused the check: {why}")),
            Err(e) => check_error = Some(format!("GitHub refused the check: {}", e.0.message)),
        }
    }
    match gh.react(&token, &pr.repo, pr.number, "eyes").await {
        Ok(Ok(())) => {}
        Ok(Err(why)) => worker::console_error!("reacting to a pull request: {why}"),
        Err(e) => worker::console_error!("reacting to a pull request: {}", e.0.message),
    }
    gh.revoke_installation_token(&token).await;
    q(
        &db,
        "UPDATE reviews SET check_run_id = ?, check_error = ? WHERE session_id = ?",
        &[
            crate::db::on(check.and_then(|c| i64::try_from(c).ok())),
            os(check_error.as_deref()),
            s(session_id),
        ],
    )?
    .run()
    .await?;
    // A session that started meanwhile, before its check existed.
    if let Some(check) = check
        && let Some(row) = of_session(&db, session_id).await?
        && row.state() == ReviewState::Running
        && row.status.as_deref().is_some_and(|st| st != "queued")
    {
        let (token, _) = gh.review_token(inst, &pr.repo).await?;
        let _ = gh
            .update_check(
                &token,
                &pr.repo,
                check,
                rv::check_running(true, &pr.head_sha, &session_url),
            )
            .await;
        gh.revoke_installation_token(&token).await;
    }
    Ok(())
}

/// Stops the reviews of `pr` in `project` that are still under way, but
/// `new`, for `because`: the review of a newer commit, or of the same one
/// again, takes their place. Their checks say so.
async fn supersede(
    ctx: &Ctx,
    project: &ProjectRow,
    pr: &PullRequest,
    inst: u64,
    new: &str,
    because: &str,
) -> AppResult<()> {
    #[derive(Deserialize)]
    struct Gone {
        session_id: String,
        #[serde(deserialize_with = "opt_int", default)]
        check_run_id: Option<i64>,
    }
    let gone: Vec<Gone> = q(
        &ctx.db,
        "UPDATE reviews SET state = 'superseded', reason = ?, finished_at = ? WHERE project_id = ? AND repo_key = ? \
         AND number = ? AND state = 'running' AND session_id != ? RETURNING session_id, check_run_id",
        &[
            s(because),
            n(ctx.now),
            s(&project.id),
            s(&pr.repo.full_name().to_ascii_lowercase()),
            n(pr.number as i64),
            s(new),
        ],
    )?
    .all()
    .await?;
    if gone.is_empty() {
        return Ok(());
    }
    let fleet = internal::fleet_stub(&ctx.env)?;
    for g in &gone {
        let stopped: AppResult<Value> = internal::post(
            &fleet,
            "/stop",
            &Stop {
                session_id: SessionId(g.session_id.clone()),
                reason: StopReason::User,
                destroy: false,
            },
        )
        .await;
        if let Err(e) = stopped {
            worker::console_error!("stopping review {}: {}", g.session_id, e.0.message);
        }
    }
    let checks: Vec<(String, u64)> = gone
        .iter()
        .filter_map(|g| Some((g.session_id.clone(), u64::try_from(g.check_run_id?).ok()?)))
        .collect();
    if checks.is_empty() {
        return Ok(());
    }
    let (env, cfg, repo, because) = (
        ctx.env.clone(),
        ctx.cfg.clone(),
        pr.repo.clone(),
        because.to_owned(),
    );
    ctx.defer(async move {
        let gh = github::from_config(&cfg, now_ms())?;
        let (token, _) = gh.review_token(inst, &repo).await?;
        for (session_id, check) in checks {
            let url = cfg.url(&format!("/s/{session_id}"));
            let body = rv::check_ended(ReviewState::Superseded, &because, &url);
            if let Ok(Err(why)) = gh.update_check(&token, &repo, check, body).await {
                worker::console_error!("ending a superseded review's check: {why}");
            }
        }
        gh.revoke_installation_token(&token).await;
        let _ = env;
        Ok(())
    });
    Ok(())
}

// ───────────────────────────── the session's course ─────────────────────────────

/// The token a review's check and comments are written with, through the
/// installation it started with.
async fn token_for(
    gh: &Github<'_>,
    row: &ReviewRow,
) -> AppResult<(u64, RepoRef, GithubCredential, Option<String>)> {
    let repo = row
        .repo()
        .ok_or_else(|| crate::error::internal("a review without a repository"))?;
    let inst = row
        .installation_id
        .and_then(|i| u64::try_from(i).ok())
        .ok_or_else(|| crate::error::internal("a review without an installation"))?;
    let (token, no_checks) = gh.review_token(inst, &repo).await?;
    Ok((inst, repo, token, no_checks))
}

/// A review session's runner took it: its check says it is under way.
/// From SessionObject, as the session starts.
pub async fn started(env: &Env, session_id: &str) -> AppResult<()> {
    let db = env.d1("DB")?;
    let cfg = Config::from_env(env);
    let Some(row) = of_session(&db, session_id).await? else {
        return Ok(());
    };
    let Some(check) = row.check_run_id.and_then(|c| u64::try_from(c).ok()) else {
        return Ok(());
    };
    if row.state() != ReviewState::Running {
        return Ok(());
    }
    let gh = github::from_config(&cfg, now_ms())?;
    let (_, repo, token, _) = token_for(&gh, &row).await?;
    let url = cfg.url(&format!("/s/{session_id}"));
    let r = gh
        .update_check(
            &token,
            &repo,
            check,
            rv::check_running(true, &row.head_sha, &url),
        )
        .await;
    gh.revoke_installation_token(&token).await;
    if let Ok(Err(why)) = r {
        worker::console_error!("starting a review's check: {why}");
    }
    Ok(())
}

/// A review session stopped or failed before its agent posted a review:
/// the review ended that way, and its check says why. From SessionObject,
/// and the hourly job for those it missed.
pub async fn ended(
    env: &Env,
    session_id: &str,
    failed: bool,
    reason: Option<&str>,
) -> AppResult<()> {
    let db = env.d1("DB")?;
    let cfg = Config::from_env(env);
    let now = now_ms();
    let (state, why) = if failed {
        (
            ReviewState::Failed,
            match reason.map(str::trim).filter(|r| !r.is_empty()) {
                Some(r) => format!("The review session failed: {r}"),
                None => "The review session failed.".to_owned(),
            },
        )
    } else {
        (
            ReviewState::Stopped,
            "The review session ended before its agent posted a review.".to_owned(),
        )
    };
    let changed = q(
        &db,
        "UPDATE reviews SET state = ?, reason = ?, finished_at = ? WHERE session_id = ? AND state = 'running'",
        &[s(state.as_str()), s(&why), n(now), s(session_id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Ok(());
    }
    let Some(row) = of_session(&db, session_id).await? else {
        return Ok(());
    };
    let Some(check) = row.check_run_id.and_then(|c| u64::try_from(c).ok()) else {
        return Ok(());
    };
    let gh = github::from_config(&cfg, now)?;
    let (_, repo, token, _) = token_for(&gh, &row).await?;
    let url = cfg.url(&format!("/s/{session_id}"));
    let r = gh
        .update_check(&token, &repo, check, rv::check_ended(state, &why, &url))
        .await;
    gh.revoke_installation_token(&token).await;
    if let Ok(Err(why)) = r {
        worker::console_error!("ending a review's check: {why}");
    }
    Ok(())
}

/// Reviews still under way whose sessions ended without SessionObject
/// saying so (an error, a deploy in between): from the hourly job.
pub async fn sweep(env: &Env, db: &D1Database) -> AppResult<usize> {
    #[derive(Deserialize)]
    struct Ended {
        session_id: String,
        status: String,
        status_reason: Option<String>,
    }
    let ended_rows: Vec<Ended> = q(
        db,
        "SELECT rv.session_id AS session_id, s.status AS status, s.status_reason AS status_reason FROM reviews rv \
         JOIN sessions s ON s.id = rv.session_id WHERE rv.state IN ('running', 'posting') \
         AND s.status IN ('stopped', 'failed') AND COALESCE(s.ended_at, 0) < ? LIMIT 50",
        &[n(now_ms() - 10 * 60 * 1000)],
    )?
    .all()
    .await?;
    // A posting that never finished (the Worker went away meanwhile) went
    // nowhere: the review ends as its session did.
    let unstuck = q(
        db,
        "UPDATE reviews SET state = 'running' WHERE state = 'posting' AND session_id IN \
         (SELECT id FROM sessions WHERE status IN ('stopped', 'failed') AND COALESCE(ended_at, 0) < ?)",
        &[n(now_ms() - 10 * 60 * 1000)],
    )?
    .run()
    .await?;
    if unstuck > 0 {
        worker::console_log!("housekeeping: {unstuck} reviews never finished posting");
    }
    let mut done = 0;
    for e in ended_rows {
        match ended(
            env,
            &e.session_id,
            e.status == "failed",
            e.status_reason.as_deref(),
        )
        .await
        {
            Ok(()) => done += 1,
            Err(err) => worker::console_error!("ending review {}: {}", e.session_id, err.0.message),
        }
    }
    Ok(done)
}

/// Ends the checks of a project's reviews under way, as the project is
/// deleted with them.
pub async fn cancel_project(ctx: &Ctx, project_id: &str) {
    let rows: AppResult<Vec<ReviewRow>> = async {
        q(
            &ctx.db,
            &format!("{REVIEW_SELECT} WHERE rv.project_id = ? AND rv.state = 'running'"),
            &[s(project_id)],
        )?
        .all()
        .await
    }
    .await;
    let rows = match rows {
        Ok(rows) if !rows.is_empty() => rows,
        Ok(_) => return,
        Err(e) => {
            worker::console_error!("finding a project's reviews: {}", e.0.message);
            return;
        }
    };
    let cfg = ctx.cfg.clone();
    ctx.defer(async move {
        let gh = github::from_config(&cfg, now_ms())?;
        for row in rows {
            let Some(check) = row.check_run_id.and_then(|c| u64::try_from(c).ok()) else {
                continue;
            };
            let (_, repo, token, _) = token_for(&gh, &row).await?;
            let url = cfg.url(&format!("/s/{}", row.session_id));
            let body = rv::check_ended(
                ReviewState::Stopped,
                "The project this review ran in was deleted.",
                &url,
            );
            let _ = gh.update_check(&token, &repo, check, body).await;
            gh.revoke_installation_token(&token).await;
        }
        Ok(())
    });
}

// ───────────────────────────── what the agent asks ─────────────────────────────

/// The review of a review session, for its agent's calls.
async fn for_agent(db: &D1Database, session_id: &str) -> AppResult<ReviewRow> {
    of_session(db, session_id).await?.ok_or_else(|| {
        forbidden(
            "This session doesn't review a pull request: `fctl review` is for review agents' sessions.",
        )
    })
}

/// fctl's earlier findings on the pull request a review session reviews
/// (`fctl review threads`).
pub async fn threads(ctx: &Ctx, session_id: &str) -> AppResult<Vec<FindingThread>> {
    let row = for_agent(&ctx.db, session_id).await?;
    let repo = row
        .repo()
        .ok_or_else(|| crate::error::internal("a review without a repository"))?;
    let inst = row
        .installation_id
        .and_then(|i| u64::try_from(i).ok())
        .ok_or_else(|| crate::error::internal("a review without an installation"))?;
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    let answer = gh
        .lookup(inst, rv::threads_query(&repo, row.number()))
        .await?;
    Ok(rv::finding_threads(&rv::threads(&answer)))
}

/// Posts what a review session's agent found (`fctl review submit`), once:
/// a summary on the pull request, then its findings in one review of the
/// commit, on their lines where the diff shows them (those it doesn't
/// show go in the summary); the earlier findings it says the new commits
/// fixed, marked fixed and resolved; atop earlier reviews' summaries, how
/// the issues they found stand, hiding those whose issues are all
/// addressed as outdated; and its check, passed with nothing to look at,
/// neutral otherwise. A second submission gets what the first posted.
pub async fn submit(
    ctx: &Ctx,
    session_id: &str,
    summary: Option<&str>,
    findings: &[fctl_proto::api::ReviewFinding],
    resolved: &[fctl_proto::api::ResolvedFinding],
) -> AppResult<PostedReview> {
    let row = for_agent(&ctx.db, session_id).await?;
    if row.state == POSTING {
        return Err(conflict(
            "fctl is posting this review already: wait for its answer.",
        ));
    }
    match row.state() {
        ReviewState::Running => {}
        state @ (ReviewState::Passed | ReviewState::Commented) => {
            return Ok(PostedReview {
                state,
                url: row.review_url.clone(),
                comments: count(row.findings),
                in_text: 0,
                resolved: Vec::new(),
                still_open: count(row.still_open),
                again: true,
                sent_to: None,
            });
        }
        ReviewState::Superseded => {
            return Err(conflict(&format!(
                "{} Nothing was posted for this one.",
                row.reason
                    .as_deref()
                    .unwrap_or("A newer review took this one's place.")
            )));
        }
        ReviewState::Stopped | ReviewState::Failed => {
            return Err(conflict(
                "This review ended already; nothing was posted for it.",
            ));
        }
    }
    let sub = rv::check(summary, findings, resolved)?;
    // One submission at a time: a second one waits for the first's answer.
    let claimed = q(
        &ctx.db,
        "UPDATE reviews SET state = ? WHERE session_id = ? AND state = 'running'",
        &[s(POSTING), s(session_id)],
    )?
    .run()
    .await?;
    if claimed == 0 {
        return Err(conflict(
            "fctl is posting this review already: wait for its answer.",
        ));
    }
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    let posted = async {
        let (inst, repo, token, _) = token_for(&gh, &row).await?;
        let posted = post(ctx, &gh, &row, inst, &repo, &token, &sub).await;
        gh.revoke_installation_token(&token).await;
        posted.map(|p| (p, repo))
    }
    .await;
    let (posting, repo) = match posted {
        Ok(p) => p,
        Err(e) => {
            // Nothing was kept: the agent may submit again.
            q(
                &ctx.db,
                "UPDATE reviews SET state = 'running' WHERE session_id = ? AND state = ?",
                &[s(session_id), s(POSTING)],
            )?
            .run()
            .await?;
            return Err(e);
        }
    };
    ctx.audit_in(
        None,
        "review.posted",
        Some(session_id),
        Some(json!({
            "pull_request": format!("{}#{}", repo.full_name(), row.number),
            "commit": row.head_sha,
            "findings": sub.findings.len(),
            "resolved": posting.review.resolved.len(),
        })),
    );
    let sent_to = match pass_on(ctx, &row, &repo, &sub, &posting).await {
        Ok(to) => to,
        Err(e) => {
            worker::console_error!("passing review {session_id}'s findings on: {}", e.0.message);
            None
        }
    };
    Ok(PostedReview {
        sent_to,
        ..posting.review
    })
}

/// What [`post`] posted: what the review agent hears, and where each
/// finding went on GitHub, for the agent that opened the pull request.
struct Posting {
    review: PostedReview,
    /// The review on GitHub, by its id.
    review_id: Option<u64>,
    /// Each finding's place ([`rv::spots`]), when it goes to that agent.
    spots: Vec<rv::Spot>,
}

/// Sends what a review found to the session whose agent opened the pull
/// request, where one of fctl's own agents did (the GitHub App is its
/// author): a message from fctl with the findings and their threads,
/// which resumes the session if it stopped, as a person's would, on the
/// subscription it ran on. Its agent decides what to do about each
/// finding, and answers in its thread. Not for pull requests that merged
/// or closed meanwhile, nor sessions whose agent only reads on GitHub now.
/// Returns the session, once it has the message.
async fn pass_on(
    ctx: &Ctx,
    row: &ReviewRow,
    repo: &RepoRef,
    sub: &rv::Submission,
    posting: &Posting,
) -> AppResult<Option<SessionId>> {
    if sub.findings.is_empty() || !rv::by_app(row.author.as_deref(), app_login(&ctx.cfg).as_deref())
    {
        return Ok(None);
    }
    let pr = format!("{}#{}", repo.full_name(), row.number);
    // Only to sessions of workspaces connected to the account the review's
    // workspace connected, by GitHub's id: not to one whose connection is
    // to a login that has changed hands since.
    let answered = answered_for(&ctx.db, row, repo).await?;
    let listing = crate::pull_requests::listed_by(&ctx.db, repo, row.number(), answered).await?;
    let branches: Vec<Option<&str>> = listing.iter().map(|l| l.branch.as_deref()).collect();
    let Some(opener) = rv::opener(&branches, row.head_ref.as_deref()).and_then(|i| listing.get(i))
    else {
        worker::console_log!(
            "no session lists {pr}, whose review found {} issues",
            sub.findings.len()
        );
        return Ok(None);
    };
    let id = opener.session_id.as_str();
    if let Some(state @ ("merged" | "closed")) = opener.state.as_deref() {
        worker::console_log!("not passing {pr}'s review on to session {id}: it is {state}");
        return Ok(None);
    }
    let Some(session) = crate::spec::session_row(&ctx.db, id).await? else {
        return Ok(None);
    };
    if session.github_access() == GithubAccess::Read {
        worker::console_log!(
            "not passing {pr}'s review on to session {id}: its agent only reads on GitHub"
        );
        return Ok(None);
    }
    let found = rv::Found {
        repo,
        number: row.number(),
        url: &row.url,
        title: row.title.as_deref().unwrap_or_default(),
        sha: &row.head_sha,
        session: &row.session_id,
        summary_url: posting.review.url.as_deref(),
        review_id: posting.review_id,
        findings: &sub.findings,
        spots: &posting.spots,
    };
    let stub = internal::session_stub(&ctx.env, id)?;
    let told: internal::FindingsTold = internal::post(
        &stub,
        "/review-findings",
        &internal::ReviewFindings {
            review: SessionId(row.session_id.clone()),
            text: rv::passed_on(&found),
            held: rv::held_back(&found),
            shown: Some(rv::passed_on_review(&found)),
        },
    )
    .await?;
    if let Some(why) = told.not_sent {
        worker::console_log!("not passing {pr}'s review on to session {id}: {why}");
        return Ok(None);
    }
    ctx.audit_in(
        session.workspace.workspace_id.as_deref(),
        "review.passed_on",
        Some(id),
        Some(json!({
            "review": row.session_id,
            "pull_request": pr,
            "commit": row.head_sha,
            "findings": sub.findings.len(),
        })),
    );
    Ok(Some(SessionId(id.to_owned())))
}

/// GitHub's ids of the account a review's pull request is in, as the
/// review's workspace connected it, and of the installation the review
/// went through; `None` for a connection from before ids were kept.
async fn answered_for(
    db: &D1Database,
    row: &ReviewRow,
    repo: &RepoRef,
) -> AppResult<Option<(u64, u64)>> {
    let Some(installation) = row.installation_id.and_then(|i| u64::try_from(i).ok()) else {
        return Ok(None);
    };
    let Some(project) = crate::spec::project_row(db, &row.project_id).await? else {
        return Ok(None);
    };
    let connections = crate::workspaces::connections(db, project.workspace_id()).await?;
    Ok(connections
        .iter()
        .find(|c| c.account.eq_ignore_ascii_case(&repo.owner))
        .and_then(|c| c.account_id)
        .and_then(|a| u64::try_from(a).ok())
        .map(|account| (account, installation)))
}

/// [`submit`]'s work on GitHub, with a token of the review's repository.
async fn post(
    ctx: &Ctx,
    gh: &Github<'_>,
    row: &ReviewRow,
    inst: u64,
    repo: &RepoRef,
    token: &GithubCredential,
    sub: &rv::Submission,
) -> AppResult<Posting> {
    let number = row.number();
    let session_url = ctx.cfg.url(&format!("/s/{}", row.session_id));
    let commit_url = format!("{}/commits/{}", row.url, row.head_sha);
    let project_slug = project_slug(&ctx.db, &row.project_id).await?;
    let settings_url = ctx.cfg.url(&format!("/p/{project_slug}?tab=reviews"));
    let links = Links {
        sha: &row.head_sha,
        commit_url: &commit_url,
        session: &row.session_id,
        session_url: &session_url,
        settings_url: &settings_url,
    };
    // fctl's threads on the pull request: those it may resolve, and those
    // still open after; and its summaries of earlier reviews.
    let answer = gh.graphql(token, rv::posting_query(repo, number)).await?;
    let threads = rv::threads(&answer);
    let summaries = rv::summaries(&answer);
    let ours: Vec<&rv::Thread> = threads.iter().filter(|t| t.ours).collect();
    let mut fixing = Vec::new();
    for r in &sub.resolved {
        match ours.iter().find(|t| t.id == r.thread) {
            Some(t) if !t.resolved => fixing.push((*t, r.note.as_str())),
            Some(_) => {}
            None => {
                return Err(AppError(ApiError::validation(
                    "resolved",
                    format!(
                        "{} isn't one of fctl's findings on this pull request: `fctl review threads` lists them",
                        r.thread
                    ),
                )));
            }
        }
    }
    // The findings: first their summary, so that GitHub shows it above
    // them, then one review with those that go on their lines, where the
    // diff shows them. The others go in the summary, as do all of them
    // when GitHub refuses lines it was given.
    let mut url = None;
    let mut review_id = None;
    let mut spots = Vec::new();
    let mut in_text = 0;
    if !sub.findings.is_empty() {
        let files = gh.pull_files(token, repo, number).await?;
        let mut placed = rv::place(&sub.findings, &files);
        let text = |placed: &[rv::Placed]| {
            rv::summary_text(
                sub.summary.as_deref(),
                &sub.findings,
                placed,
                fixing.len(),
                &links,
            )
        };
        let (summary, summary_url) = gh
            .comment(token, repo, number, &text(&placed))
            .await?
            .map_err(|(_, why)| {
                AppError(ApiError::new(
                    ErrorCode::Unavailable,
                    format!("GitHub refused the review's summary: {why}"),
                ))
            })?;
        if let Some(review) = rv::review_request(&sub.findings, &placed, &links) {
            let refused = |why: String| {
                AppError(ApiError::new(
                    ErrorCode::Unavailable,
                    format!("GitHub refused the review: {why}"),
                ))
            };
            let posted = match gh.post_review(token, repo, number, review).await {
                Ok(Ok((_, id))) => {
                    review_id = id;
                    Ok(())
                }
                Ok(Err((422, why))) => {
                    worker::console_error!(
                        "GitHub refused a review's lines ({why}): its findings go in its summary"
                    );
                    placed = vec![rv::Placed::Text; sub.findings.len()];
                    gh.edit_comment(token, repo, summary, &text(&placed))
                        .await
                        .and_then(|r| r.map_err(refused))
                }
                Ok(Err((_, why))) => Err(refused(why)),
                Err(e) => Err(e),
            };
            if let Err(e) = posted {
                // Nor does its summary stay: the agent may submit again.
                match gh.delete_comment(token, repo, summary).await {
                    Ok(Ok(())) => {}
                    Ok(Err(why)) => worker::console_error!("deleting a review's summary: {why}"),
                    Err(e) => {
                        worker::console_error!("deleting a review's summary: {}", e.0.message)
                    }
                }
                return Err(e);
            }
        }
        in_text = placed.iter().filter(|p| **p == rv::Placed::Text).count();
        url = Some(summary_url);
        // Where each finding went, for the agent that opened the pull
        // request, where one of fctl's did (`pass_on`).
        if rv::by_app(row.author.as_deref(), app_login(&ctx.cfg).as_deref()) {
            let mut listed = Value::Null;
            if let Some(id) = review_id {
                match gh.review_comments(token, repo, number, id).await {
                    Ok(Ok(comments)) => listed = comments,
                    Ok(Err(why)) => worker::console_error!("listing a review's comments: {why}"),
                    Err(e) => {
                        worker::console_error!("listing a review's comments: {}", e.0.message)
                    }
                }
            }
            spots = rv::spots(&listed, &sub.findings, &placed);
        }
    }
    // The earlier findings the new commits fixed: marked, then resolved.
    let mut resolved_ids = Vec::new();
    for (t, note) in &fixing {
        let edit = gh
            .graphql(
                token,
                rv::edit_comment(&t.comment_id, &rv::fixed(&t.body, note, &links)),
            )
            .await;
        if let Err(e) = edit {
            worker::console_error!("marking a finding fixed: {}", e.0.message);
        }
    }
    let ids: Vec<String> = fixing.iter().map(|(t, _)| t.id.clone()).collect();
    for (id, result) in ids.iter().zip(gh.resolve_threads(inst, repo, &ids).await?) {
        match result {
            Ok(_) => resolved_ids.push(id.clone()),
            Err(why) => worker::console_error!("resolving {id}: {why}"),
        }
    }
    let fixed: BTreeSet<&str> = resolved_ids.iter().map(String::as_str).collect();
    // The summaries of earlier reviews: how the issues each found stand
    // now, atop it, and hidden as outdated once all are addressed.
    for s in summaries
        .iter()
        .filter(|s| !s.minimized && s.note.session != row.session_id)
    {
        let standing = rv::standing(s, &threads, &fixed);
        if let Some(body) = rv::restated(s, standing, &links)
            && let Err(e) = gh.graphql(token, rv::edit_summary(&s.id, &body)).await
        {
            worker::console_error!("restating an earlier review's summary: {}", e.0.message);
        }
        if standing.addressed()
            && let Err(e) = gh.graphql(token, rv::hide_outdated(&s.id)).await
        {
            worker::console_error!("hiding an earlier review's summary: {}", e.0.message);
        }
    }
    let still_open = ours
        .iter()
        .filter(|t| !t.resolved && !fixed.contains(t.id.as_str()))
        .count();
    let state = rv::outcome(sub.findings.len(), still_open);
    if let Some(check) = row.check_run_id.and_then(|c| u64::try_from(c).ok()) {
        let body = rv::check_posted(
            &sub.findings,
            resolved_ids.len(),
            still_open,
            url.as_deref(),
            &links,
        );
        if let Err(why) = gh.update_check(token, repo, check, body).await? {
            worker::console_error!("completing a review's check: {why}");
        }
    }
    q(
        &ctx.db,
        "UPDATE reviews SET state = ?, findings = ?, resolved = ?, still_open = ?, review_url = ?, finished_at = ?, \
         reason = NULL WHERE session_id = ?",
        &[
            s(state.as_str()),
            n(sub.findings.len() as i64),
            n(resolved_ids.len() as i64),
            n(still_open as i64),
            os(url.as_deref()),
            n(ctx.now),
            s(&row.session_id),
        ],
    )?
    .run()
    .await?;
    Ok(Posting {
        review: PostedReview {
            state,
            url,
            comments: count((sub.findings.len() - in_text) as i64),
            in_text: count(in_text as i64),
            resolved: resolved_ids,
            still_open: count(still_open as i64),
            again: false,
            sent_to: None,
        },
        review_id,
        spots,
    })
}

async fn project_slug(db: &D1Database, project_id: &str) -> AppResult<String> {
    #[derive(Deserialize)]
    struct Slug {
        slug: String,
    }
    let slug: Option<Slug> = q(
        db,
        "SELECT slug FROM projects WHERE id = ?",
        &[s(project_id)],
    )?
    .first()
    .await?;
    slug.map(|s| s.slug).ok_or_else(|| not_found("Project"))
}
