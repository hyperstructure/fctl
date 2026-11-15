//! The checks on a base branch after the pull requests of autopilots'
//! sessions merged (`fctl_domain::merge_checks`), which wake those
//! autopilots once they end, so that none needs a script to wait for main
//! to go green.
//!
//! GitHub's webhook says a pull request merged, with the commit that holds
//! it on its base branch ([`merged`] keeps it, for each session an
//! autopilot started that lists it), and when a check suite there
//! completes, or a commit status there no longer waits. Then the control
//! plane looks that commit up (`fctl_domain::webhooks::commit_query`): its
//! checks, the commits before it and where its branch is now. Once all of
//! its checks ended, it settles the merges it holds ([`settle`]), and the
//! autopilots wake a minute later, gathered like other news of their
//! sessions, to hear how they went ([`untold`]).

use crate::config::Config;
use crate::db::{int, n, opt_int, os, q, s};
use crate::error::AppResult;
use crate::github;
use crate::pull_requests::{in_reach, key};
use fctl_domain::autopilot as ap;
use fctl_domain::merge_checks::{self as mc, Outcome};
use fctl_domain::webhooks::{self, Commit, Hook};
use fctl_proto::autopilot::AutopilotWakeReason;
use fctl_proto::session::RepoRef;
use serde::Deserialize;
use worker::{D1Database, Env};

/// How long after a merge its row stays, told or not: housekeeping deletes
/// it then. Checks that haven't ended by then never will, and an
/// autopilot paused for longer has older news to read.
pub const KEEP_MS: i64 = 3 * 86_400_000;

/// Most merges one delivery settles, and branches it looks up commits on.
const MAX_WAITING: i64 = 50;
const MAX_BRANCHES: usize = 3;

/// How long after a merge, and after it last looked, the hourly job looks
/// up the checks of a merge still waiting itself ([`sweep`]), how many
/// times at most (a merge into a branch whose commits run no checks never
/// settles), and the most commits it looks up in a run.
const SWEEP_AFTER_MS: i64 = 15 * 60_000;
const SWEEP_AGAIN_MS: i64 = 50 * 60_000;
const SWEEP_LOOKS: i64 = 6;
const SWEEP_LOOKUPS: usize = 30;

/// A pull request that merged, as a `pull_request` delivery says.
pub struct Merged<'a> {
    pub repo: &'a RepoRef,
    pub number: u64,
    pub url: &'a str,
    pub title: Option<&'a str>,
    /// The branch it merged into, and the commit that holds it there.
    pub base: &'a str,
    pub sha: &'a str,
    pub at: i64,
}

/// Keeps that a pull request merged, for each session an autopilot
/// started that lists it, whose workspace connects the account that
/// answered (`answered`: GitHub's ids of the account and the installation,
/// [`in_reach`]), so that the checks on its base branch wake that
/// autopilot as they end. Returns how many it kept.
pub async fn merged(
    db: &D1Database,
    m: &Merged<'_>,
    answered: (u64, u64),
    now: i64,
) -> AppResult<usize> {
    q(
        db,
        &format!(
            "INSERT OR IGNORE INTO merge_checks (repo, number, session_id, url, title, autopilot_id, base_ref, merge_sha, merged_at) \
             SELECT l.repo, l.number, l.session_id, ?3, ?4, s.autopilot_id, ?5, ?6, ?7 FROM pull_request_sessions l \
             JOIN sessions s ON s.id = l.session_id \
             WHERE l.repo = ?1 AND l.number = ?2 AND s.autopilot_role = 'crew' AND s.autopilot_id IS NOT NULL AND {}",
            in_reach(Some(("?8", "?9")))
        ),
        &[
            s(&key(m.repo)),
            n(m.number as i64),
            s(m.url),
            os(m.title),
            s(m.base),
            s(m.sha),
            n(if m.at > 0 { m.at } else { now }),
            n(answered.0 as i64),
            n(answered.1 as i64),
        ],
    )?
    .run()
    .await
}

/// A merge whose checks on its base branch haven't ended yet.
#[derive(Clone, Debug, Deserialize)]
pub struct Waiting {
    #[serde(deserialize_with = "int")]
    pub number: i64,
    pub session_id: String,
    pub autopilot_id: String,
    pub base_ref: String,
    pub merge_sha: String,
}

/// The merges into `branches` of `repo` still waiting for their checks,
/// of sessions whose workspaces connect the account that answered.
pub async fn waiting(
    db: &D1Database,
    repo: &RepoRef,
    branches: &[String],
    answered: (u64, u64),
) -> AppResult<Vec<Waiting>> {
    q(
        db,
        &format!(
            "SELECT l.number AS number, l.session_id AS session_id, l.autopilot_id AS autopilot_id, \
             l.base_ref AS base_ref, l.merge_sha AS merge_sha FROM merge_checks l \
             WHERE l.repo = ?1 AND l.ended_at IS NULL AND l.base_ref IN (SELECT value FROM json_each(?2)) \
             AND {} ORDER BY l.merged_at LIMIT {MAX_WAITING}",
            in_reach(Some(("?3", "?4")))
        ),
        &[
            s(&key(repo)),
            s(&serde_json::to_string(branches)?),
            n(answered.0 as i64),
            n(answered.1 as i64),
        ],
    )?
    .all()
    .await
}

/// Settles the merges among `waiting` (into one branch of `repo`) that
/// commit `c` holds, once its checks tell how they went
/// (`fctl_domain::merge_checks::settles`), and wakes their autopilots a
/// minute later in the same batch, so that a merge settled never misses
/// its wake. Returns the autopilots of those it settled, each once, which
/// a delivery settling them too at the same time leaves out; ring
/// ScheduleObject's alarm for them (`crate::autopilots::ring`).
pub async fn settle(
    db: &D1Database,
    repo: &RepoRef,
    waiting: &[Waiting],
    c: &Commit,
    now: i64,
) -> AppResult<Vec<String>> {
    let merges: Vec<&str> = waiting.iter().map(|w| w.merge_sha.as_str()).collect();
    let Some((outcome, held)) = mc::settles(c, &merges) else {
        return Ok(Vec::new());
    };
    let failing = serde_json::to_string(&c.checks.failing)?;
    let mut statements = Vec::new();
    let mut settled = Vec::new();
    for w in held.iter().filter_map(|&i| waiting.get(i)) {
        statements.push(q(
            db,
            "UPDATE merge_checks SET sha = ?, ended_at = ?, outcome = ?, passed = ?, failed = ?, cancelled = ?, failing = ? \
             WHERE repo = ? AND number = ? AND session_id = ? AND ended_at IS NULL",
            &[
                s(&c.sha),
                n(now),
                s(outcome.as_str()),
                n(i64::from(c.checks.passed)),
                n(i64::from(c.checks.failed)),
                n(i64::from(c.checks.cancelled)),
                s(&failing),
                s(&key(repo)),
                n(w.number),
                s(&w.session_id),
            ],
        )?);
        settled.push(w.autopilot_id.clone());
    }
    // Each autopilot wakes once the batch settled one of its merges, which
    // the merges it settled say: a delivery that settled them first wakes
    // it instead.
    let mut autopilots = settled.clone();
    autopilots.sort();
    autopilots.dedup();
    for a in &autopilots {
        statements.push(crate::autopilots::nudging(
            db,
            a,
            AutopilotWakeReason::MergeChecks,
            now + ap::CREW_GATHER_MS,
            Some((
                "EXISTS (SELECT 1 FROM merge_checks WHERE autopilot_id = ?1 AND sha = ?4 AND ended_at = ?5 AND told_at IS NULL)",
                vec![s(&c.sha), n(now)],
            )),
        )?);
    }
    let changes = crate::db::batch_changes(db, statements).await?;
    let mut autopilots: Vec<String> = settled
        .into_iter()
        .zip(changes)
        .filter(|(_, changed)| *changed > 0)
        .map(|(a, _)| a)
        .collect();
    autopilots.sort();
    autopilots.dedup();
    Ok(autopilots)
}

/// Looks commit `sha` up as one on `branch` and settles the merges into it
/// among `waiting` that it holds ([`settle`]).
#[allow(clippy::too_many_arguments)]
async fn settle_on(
    gh: &github::Github<'_>,
    db: &D1Database,
    repo: &RepoRef,
    installation: u64,
    sha: &str,
    branch: &str,
    waiting: &[Waiting],
    now: i64,
) -> AppResult<Vec<String>> {
    let answer = gh
        .lookup(installation, webhooks::commit_query(repo, sha, branch))
        .await?;
    let Some(commit) = webhooks::looked_up_commit(&answer) else {
        return Ok(Vec::new());
    };
    let on: Vec<Waiting> = waiting
        .iter()
        .filter(|w| w.base_ref == branch)
        .cloned()
        .collect();
    settle(db, repo, &on, &commit, now).await
}

/// What a webhook delivery means for merges' checks: a pull request of an
/// autopilot's session merged, which [`merged`] keeps; or the checks of a
/// commit on a branch that merges wait on may have ended, which looks the
/// commit up and settles those it holds, waking their autopilots. Returns
/// how many autopilots it woke.
pub async fn on_delivery(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    hook: &Hook,
    now: i64,
) -> AppResult<usize> {
    // Without GitHub's word of who answered, nobody hears of anything.
    let Some(answered) = hook.owner_id.zip(hook.installation) else {
        return Ok(0);
    };
    if let Some(p) = &hook.payload
        && p.action == "closed"
        && let (Some(sha), Some(base)) = (&p.merge_sha, &p.base_ref)
    {
        merged(
            db,
            &Merged {
                repo: &hook.repo,
                number: p.seen.number,
                url: &p.seen.url,
                title: p.seen.title.as_deref(),
                base,
                sha,
                at: p.seen.state_changed_at.unwrap_or(now),
            },
            answered,
            now,
        )
        .await?;
        return Ok(0);
    }
    let Some(sha) = hook.sha.as_deref().filter(|_| hook.ended) else {
        return Ok(0);
    };
    if hook.branches.is_empty() {
        return Ok(0);
    }
    let waiting = waiting(db, &hook.repo, &hook.branches, answered).await?;
    if waiting.is_empty() {
        return Ok(0);
    }
    let gh = github::from_config(cfg, now)?;
    let mut branches: Vec<&str> = waiting.iter().map(|w| w.base_ref.as_str()).collect();
    branches.sort_unstable();
    branches.dedup();
    // Each branch apart: one that fails (GitHub's answer, D1) leaves the
    // others, and what they settled, to wake their autopilots.
    let mut autopilots: Vec<String> = Vec::new();
    let mut failed = None;
    for branch in branches.into_iter().take(MAX_BRANCHES) {
        match settle_on(&gh, db, &hook.repo, answered.1, sha, branch, &waiting, now).await {
            Ok(woke) => autopilots.extend(woke),
            Err(e) => {
                failed.get_or_insert(e);
            }
        }
    }
    autopilots.sort();
    autopilots.dedup();
    if !autopilots.is_empty() {
        crate::autopilots::ring(env).await;
    }
    match failed {
        Some(e) if autopilots.is_empty() => Err(e),
        Some(e) => {
            worker::console_error!(
                "the checks after merges into {}: {}",
                hook.repo.full_name(),
                e.0.message
            );
            Ok(autopilots.len())
        }
        None => Ok(autopilots.len()),
    }
}

/// A merge still waiting, with its repository, for [`sweep`].
#[derive(Clone, Debug, Deserialize)]
pub struct Stale {
    pub repo: String,
    #[serde(deserialize_with = "int")]
    pub number: i64,
    pub session_id: String,
    pub autopilot_id: String,
    pub base_ref: String,
    pub merge_sha: String,
}

impl Stale {
    fn waiting(&self) -> Waiting {
        Waiting {
            number: self.number,
            session_id: self.session_id.clone(),
            autopilot_id: self.autopilot_id.clone(),
            base_ref: self.base_ref.clone(),
            merge_sha: self.merge_sha.clone(),
        }
    }
}

/// The merges [`sweep`] looks the checks of up: still waiting
/// [`SWEEP_AFTER_MS`] after they merged, and as long since it last looked,
/// fewer than [`SWEEP_LOOKS`] times, those it looked at least first, of
/// sessions whose workspaces still connect their repository's account.
pub async fn stale(db: &D1Database, now: i64) -> AppResult<Vec<Stale>> {
    q(
        db,
        &format!(
            "SELECT l.repo AS repo, l.number AS number, l.session_id AS session_id, l.autopilot_id AS autopilot_id, \
             l.base_ref AS base_ref, l.merge_sha AS merge_sha FROM merge_checks l \
             WHERE l.ended_at IS NULL AND l.merged_at < ?1 AND (l.looked_at IS NULL OR l.looked_at < ?2) \
             AND l.looks < {SWEEP_LOOKS} AND {} \
             ORDER BY l.looks, l.merged_at LIMIT {MAX_WAITING}",
            in_reach(None)
        ),
        &[n(now - SWEEP_AFTER_MS), n(now - SWEEP_AGAIN_MS)],
    )?
    .all()
    .await
}

/// Marks merges into `branch` of `repo` that still wait looked at once
/// more: [`sweep`] looked up a commit that holds them.
pub async fn looked(
    db: &D1Database,
    repo: &RepoRef,
    branch: &str,
    merges: &[i64],
    now: i64,
) -> AppResult<()> {
    q(
        db,
        "UPDATE merge_checks SET looked_at = ?, looks = looks + 1 WHERE repo = ? AND base_ref = ? AND ended_at IS NULL \
         AND number IN (SELECT value FROM json_each(?))",
        &[
            n(now),
            s(&key(repo)),
            s(branch),
            s(&serde_json::to_string(merges)?),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// Looks up, from the hourly job, the checks of merges whose webhook
/// delivery never came or whose lookup failed then (GitHub sends a
/// delivery once, however the control plane answered), and settles those
/// they tell of ([`sweep_branch`]), which wakes their autopilots. Returns
/// how many autopilots it woke.
pub async fn sweep(env: &Env, db: &D1Database, now: i64) -> AppResult<usize> {
    let cfg = Config::from_env(env);
    if !crate::pull_requests::enabled(&cfg) {
        return Ok(0);
    }
    let stale = stale(db, now).await?;
    if stale.is_empty() {
        return Ok(0);
    }
    let gh = github::from_config(&cfg, now)?;
    let mut branches: Vec<((String, String), Vec<Waiting>)> = Vec::new();
    for m in &stale {
        let at = (m.repo.clone(), m.base_ref.clone());
        match branches.iter_mut().find(|(b, _)| *b == at) {
            Some((_, w)) => w.push(m.waiting()),
            None => branches.push((at, vec![m.waiting()])),
        }
    }
    let mut lookups = 0;
    let mut autopilots: Vec<String> = Vec::new();
    for ((repo, branch), waiting) in branches {
        if lookups >= SWEEP_LOOKUPS {
            break;
        }
        let Some(repo) = RepoRef::parse(&repo) else {
            continue;
        };
        // Only the merges a lookup reached count as looked at: a failure,
        // or the run's last lookup, leaves the others to the next run.
        let mut reached: Vec<i64> = Vec::new();
        let swept = sweep_branch(
            &gh,
            db,
            &repo,
            &branch,
            waiting,
            &mut lookups,
            &mut reached,
            &mut autopilots,
            now,
        )
        .await;
        if let Err(e) = swept {
            worker::console_error!(
                "looking up the checks after merges into {} {branch}: {}",
                repo.full_name(),
                e.0.message
            );
        }
        if reached.is_empty() {
            continue;
        }
        if let Err(e) = looked(db, &repo, &branch, &reached, now).await {
            worker::console_error!(
                "marking the merges into {} {branch} looked at: {}",
                repo.full_name(),
                e.0.message
            );
        }
    }
    autopilots.sort();
    autopilots.dedup();
    if !autopilots.is_empty() {
        crate::autopilots::ring(env).await;
    }
    Ok(autopilots.len())
}

/// Looks up commits on `branch` of `repo` for the merges into it among
/// `waiting` and settles those they tell of ([`settle`]), adding their
/// autopilots to `autopilots`: the commit of the first merge (looked at
/// least, then oldest), then the commit the branch is at now, which holds
/// them all, then each other merge's in that order, until none waits, with
/// at most
/// [`SWEEP_LOOKUPS`] lookups counted in `lookups`. Adds to `reached` the
/// merges GitHub answered for: those whose commit it looked up, found or
/// not, and those that a commit on the branch it looked up holds.
#[allow(clippy::too_many_arguments)]
async fn sweep_branch(
    gh: &github::Github<'_>,
    db: &D1Database,
    repo: &RepoRef,
    branch: &str,
    mut waiting: Vec<Waiting>,
    lookups: &mut usize,
    reached: &mut Vec<i64>,
    autopilots: &mut Vec<String>,
    now: i64,
) -> AppResult<()> {
    let installation = gh.installation(&repo.owner).await?.id;
    let mut tried: Vec<String> = Vec::new();
    let mut head: Option<String> = None;
    let new = |sha: &String, tried: &[String]| !tried.iter().any(|t| t.eq_ignore_ascii_case(sha));
    while !waiting.is_empty() && *lookups < SWEEP_LOOKUPS {
        let next = head
            .as_ref()
            .filter(|h| new(h, &tried))
            .or_else(|| {
                waiting
                    .iter()
                    .map(|w| &w.merge_sha)
                    .find(|s| new(s, &tried))
            })
            .cloned();
        let Some(sha) = next else {
            break;
        };
        tried.push(sha.clone());
        *lookups += 1;
        let answer = gh
            .lookup(installation, webhooks::commit_query(repo, &sha, branch))
            .await?;
        let commit = webhooks::looked_up_commit(&answer);
        for number in answered_for(&waiting, &sha, commit.as_ref()) {
            if !reached.contains(&number) {
                reached.push(number);
            }
        }
        let Some(commit) = commit else {
            continue;
        };
        if head.is_none() {
            head = commit.head.clone();
        }
        let merges: Vec<&str> = waiting.iter().map(|w| w.merge_sha.as_str()).collect();
        let Some((_, held)) = mc::settles(&commit, &merges) else {
            continue;
        };
        autopilots.extend(settle(db, repo, &waiting, &commit, now).await?);
        waiting = waiting
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !held.contains(i))
            .map(|(_, w)| w)
            .collect();
    }
    Ok(())
}

/// The merges among `waiting` that GitHub's answer to a lookup of commit
/// `sha` (`commit`, `None` when it has none) answered for: those of that
/// commit, found or not, and those that a commit on the branch holds.
fn answered_for(waiting: &[Waiting], sha: &str, commit: Option<&Commit>) -> Vec<i64> {
    waiting
        .iter()
        .filter(|w| {
            w.merge_sha.eq_ignore_ascii_case(sha)
                || commit.is_some_and(|c| c.on_branch && c.holds(&w.merge_sha))
        })
        .map(|w| w.number)
        .collect()
}

/// A merge of an autopilot's session whose checks on its base branch
/// ended, which the autopilot hasn't heard of yet.
#[derive(Clone, Debug, Deserialize)]
pub struct Ended {
    #[serde(deserialize_with = "int")]
    pub id: i64,
    pub repo: String,
    #[serde(deserialize_with = "int")]
    pub number: i64,
    pub url: String,
    pub title: Option<String>,
    pub base_ref: String,
    pub merge_sha: String,
    pub sha: String,
    pub outcome: String,
    #[serde(deserialize_with = "opt_int", default)]
    pub passed: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    pub failed: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    pub cancelled: Option<i64>,
    pub failing: Option<String>,
    pub project_slug: String,
    #[serde(deserialize_with = "int")]
    pub session_number: i64,
}

impl Ended {
    /// The names of the checks that failed.
    pub fn failing(&self) -> Vec<String> {
        self.failing
            .as_deref()
            .and_then(|f| serde_json::from_str(f).ok())
            .unwrap_or_default()
    }

    /// What fctl tells the autopilot of it (`fctl_domain::merge_checks::line`),
    /// with `failing` from [`Ended::failing`]. `None` for an outcome a newer
    /// control plane wrote.
    pub fn view<'a>(&'a self, failing: &'a [String]) -> Option<mc::View<'a>> {
        let count = |c: Option<i64>| u32::try_from(c.unwrap_or(0)).unwrap_or(0);
        let pull = match fctl_domain::pulls::repo_of(&self.url) {
            Some(repo) => format!("{}#{}", repo.full_name(), self.number),
            None => format!("{}#{}", self.repo, self.number),
        };
        Some(mc::View {
            pull,
            title: self.title.as_deref(),
            session: format!("{}#{}", self.project_slug, self.session_number),
            base: &self.base_ref,
            merge_sha: &self.merge_sha,
            sha: &self.sha,
            outcome: Outcome::parse(&self.outcome)?,
            passed: count(self.passed),
            failed: count(self.failed),
            cancelled: count(self.cancelled),
            failing,
        })
    }
}

/// The merges of autopilot `autopilot_id`'s sessions whose checks on their
/// base branch ended and that it hasn't heard of, in the order they
/// ended. A pull request that several of its sessions list comes once, but
/// [`told`] takes all of their ids.
pub async fn untold(db: &D1Database, autopilot_id: &str) -> AppResult<(Vec<Ended>, Vec<i64>)> {
    let rows: Vec<Ended> = q(
        db,
        "SELECT m.rowid AS id, m.repo, m.number, m.url, m.title, m.base_ref, m.merge_sha, m.sha, m.outcome, \
         m.passed, m.failed, m.cancelled, m.failing, p.slug AS project_slug, s.number AS session_number \
         FROM merge_checks m JOIN sessions s ON s.id = m.session_id JOIN projects p ON p.id = s.project_id \
         WHERE m.autopilot_id = ? AND m.ended_at IS NOT NULL AND m.told_at IS NULL \
         ORDER BY m.ended_at, m.repo, m.number, s.created_at LIMIT 50",
        &[s(autopilot_id)],
    )?
    .all()
    .await?;
    let ids = rows.iter().map(|r| r.id).collect();
    let mut once: Vec<Ended> = Vec::new();
    for r in rows {
        if !once
            .iter()
            .any(|o| o.repo == r.repo && o.number == r.number)
        {
            once.push(r);
        }
    }
    Ok((once, ids))
}

/// Marks merges' checks told: a wake told the autopilot of them.
pub async fn told(db: &D1Database, ids: &[i64], now: i64) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    q(
        db,
        "UPDATE merge_checks SET told_at = ? WHERE told_at IS NULL AND rowid IN (SELECT value FROM json_each(?))",
        &[n(now), s(&serde_json::to_string(ids)?)],
    )?
    .run()
    .await?;
    Ok(())
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::error::internal;
    use crate::testing;
    use fctl_domain::webhooks::CommitChecks;
    use wasm_bindgen_test::wasm_bindgen_test;

    const NOW: i64 = 1_800_000_000_000;
    /// GitHub's ids of the acme account and its installation of the app.
    const ANSWERED: (u64, u64) = (314, 42);

    fn repo() -> RepoRef {
        RepoRef {
            owner: "acme".into(),
            name: "web".into(),
        }
    }

    /// Acme's workspace with its web project, connected to the acme
    /// account, and its autopilot's sessions #1 and #2, which list
    /// acme/web#12 and #13, and a person's #3, which lists #14.
    async fn world(db: &D1Database) -> AppResult<()> {
        for sql in [
            "INSERT INTO users (id, email, name, created_at, updated_at) VALUES ('usr_ada', 'ada@example.com', 'Ada', 0, 0)",
            "INSERT INTO workspaces (id, slug, name, created_at, updated_at) VALUES ('wsp_acme', 'acme', 'Acme', 0, 0)",
            "INSERT INTO workspace_github (workspace_id, account, installation_id, account_id, status, created_at, updated_at) \
             VALUES ('wsp_acme', 'acme', 42, 314, 'active', 0, 0)",
            "INSERT INTO projects (id, slug, name, repo_owner, repo_name, created_by, created_at, updated_at, workspace_id) \
             VALUES ('prj_web', 'web', 'Web', 'acme', 'web', 'usr_ada', 0, 0, 'wsp_acme')",
            "INSERT INTO sessions (id, project_id, number, title, prompt, status, created_by, claude_session_id, created_at, updated_at, autopilot_id, autopilot_role) \
             VALUES ('ses_1', 'prj_web', 1, 'Add Apple Pay', 'x', 'idle', 'usr_ada', 'c1', 1, 0, 'prj_web', 'crew'), \
                    ('ses_2', 'prj_web', 2, 'Wishlists', 'x', 'stopped', 'usr_ada', 'c2', 2, 0, 'prj_web', 'crew'), \
                    ('ses_3', 'prj_web', 3, 'By hand', 'x', 'idle', 'usr_ada', 'c3', 3, 0, NULL, NULL)",
            "INSERT INTO pull_request_sessions (repo, number, session_id, state) VALUES \
             ('acme/web', 12, 'ses_1', 'merged'), ('acme/web', 13, 'ses_2', 'merged'), ('acme/web', 12, 'ses_2', 'merged'), \
             ('acme/web', 14, 'ses_3', 'merged')",
            "INSERT INTO autopilots (project_id, workspace_id, owner_id, direction, direction_at, wake_at, wake_reasons, created_at, updated_at) \
             VALUES ('prj_web', 'wsp_acme', 'usr_ada', 'Keep the shop running.', 0, 1900000000000, 'crew', 0, 0)",
        ] {
            q(db, sql, &[])?.run().await?;
        }
        Ok(())
    }

    async fn merge(db: &D1Database, number: u64, sha: &str) -> AppResult<usize> {
        let r = repo();
        let url = format!("https://github.com/acme/web/pull/{number}");
        merged(
            db,
            &Merged {
                repo: &r,
                number,
                url: &url,
                title: Some("A change"),
                base: "main",
                sha,
                at: NOW,
            },
            ANSWERED,
            NOW,
        )
        .await
    }

    fn commit(sha: &str, history: &[&str], head: &str, checks: CommitChecks) -> Commit {
        Commit {
            sha: sha.into(),
            history: history.iter().map(|h| (*h).to_owned()).collect(),
            head: Some(head.into()),
            on_branch: true,
            checks,
        }
    }

    fn checks(passed: u32, failed: u32, cancelled: u32, pending: u32) -> CommitChecks {
        CommitChecks {
            passed,
            failed,
            cancelled,
            pending,
            failing: if failed > 0 {
                vec!["Rust tests".into()]
            } else {
                Vec::new()
            },
        }
    }

    async fn waits(db: &D1Database) -> AppResult<Vec<Waiting>> {
        waiting(db, &repo(), &["main".to_owned()], ANSWERED).await
    }

    /// When the autopilot wakes next, and why.
    async fn wake(db: &D1Database) -> AppResult<(Option<i64>, String)> {
        #[derive(Deserialize)]
        struct Row {
            #[serde(deserialize_with = "opt_int", default)]
            wake_at: Option<i64>,
            wake_reasons: String,
        }
        let r: Option<Row> = q(
            db,
            "SELECT wake_at, wake_reasons FROM autopilots WHERE project_id = 'prj_web'",
            &[],
        )?
        .first()
        .await?;
        let r = r.ok_or_else(|| internal("no autopilot"))?;
        Ok((r.wake_at, r.wake_reasons))
    }

    /// Two merges in a row: the first's run is cancelled as the second's
    /// takes over, and the second's passes for both.
    #[wasm_bindgen_test]
    async fn a_newer_merges_checks_tell_for_one_whose_run_it_cancelled() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        // Of the autopilot's sessions alone: #12 is listed by both of its.
        assert_eq!(merge(&db, 12, "aaa").await?, 2);
        assert_eq!(merge(&db, 13, "bbb").await?, 1);
        assert_eq!(merge(&db, 14, "ccc").await?, 0);
        // GitHub says it merged once more: nothing new.
        assert_eq!(merge(&db, 13, "bbb").await?, 0);
        // Another branch's merges wait apart.
        let w = waiting(&db, &repo(), &["release".to_owned()], ANSWERED).await?;
        assert!(w.is_empty());
        // Nor does another installation's word reach them.
        let w = waiting(&db, &repo(), &["main".to_owned()], (999, 7)).await?;
        assert!(w.is_empty());
        let w = waits(&db).await?;
        assert_eq!(w.len(), 3);
        // The first's run was cancelled as main moved on: nothing yet.
        let first = commit("aaa", &["aaa"], "bbb", checks(10, 0, 4, 0));
        assert!(settle(&db, &repo(), &w, &first, NOW).await?.is_empty());
        // The second's still runs.
        let second = commit("bbb", &["bbb", "aaa"], "bbb", checks(10, 0, 0, 4));
        assert!(settle(&db, &repo(), &w, &second, NOW).await?.is_empty());
        let (ended, _) = untold(&db, "prj_web").await?;
        assert!(ended.is_empty());
        // Nothing woke the autopilot sooner than it would.
        assert_eq!(
            wake(&db).await?,
            (Some(1_900_000_000_000), "crew".to_owned())
        );
        // Then it passes, for both merges, and wakes the autopilot a
        // minute later with what woke it already.
        let second = commit("bbb", &["bbb", "aaa"], "bbb", checks(14, 0, 0, 0));
        assert_eq!(
            settle(&db, &repo(), &w, &second, NOW + 1).await?,
            vec!["prj_web".to_owned()]
        );
        assert!(waits(&db).await?.is_empty());
        let woke = (
            Some(NOW + 1 + ap::CREW_GATHER_MS),
            "crew,merge_checks".to_owned(),
        );
        assert_eq!(wake(&db).await?, woke);
        // A later delivery about the same commit settles nothing again,
        // nor wakes it.
        assert!(settle(&db, &repo(), &w, &second, NOW + 2).await?.is_empty());
        assert_eq!(wake(&db).await?, woke);
        // The autopilot hears of each pull request once.
        let (ended, ids) = untold(&db, "prj_web").await?;
        assert_eq!(ids.len(), 3);
        let lines: Vec<String> = ended
            .iter()
            .map(|e| {
                let failing = e.failing();
                e.view(&failing)
                    .map(|v| mc::line(&v))
                    .ok_or_else(|| internal("no view"))
            })
            .collect::<AppResult<_>>()?;
        assert_eq!(
            lines,
            vec![
                "acme/web#12 “A change” (web#1), into main: its 14 checks passed at bbb, a later commit that holds it.".to_owned(),
                "acme/web#13 “A change” (web#2), into main: its 14 checks passed at bbb.".to_owned(),
            ]
        );
        told(&db, &ids, NOW + 3).await?;
        let (ended, _) = untold(&db, "prj_web").await?;
        assert!(ended.is_empty());
        Ok(())
    }

    /// The hourly job looks up merges still waiting a while after they
    /// merged, each again an hour later at the soonest.
    #[wasm_bindgen_test]
    async fn the_hourly_job_looks_up_merges_still_waiting() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        merge(&db, 12, "aaa").await?;
        merge(&db, 13, "bbb").await?;
        let numbers = |s: Vec<Stale>| {
            let mut n: Vec<(i64, String)> =
                s.into_iter().map(|m| (m.number, m.session_id)).collect();
            n.sort();
            n
        };
        // Just merged: the webhook still has time to say.
        assert!(stale(&db, NOW + 60_000).await?.is_empty());
        let later = NOW + SWEEP_AFTER_MS + 1;
        assert_eq!(
            numbers(stale(&db, later).await?),
            vec![
                (12, "ses_1".to_owned()),
                (12, "ses_2".to_owned()),
                (13, "ses_2".to_owned())
            ]
        );
        // Looked at: not again within the hour, while #13 settles.
        looked(&db, &repo(), "main", &[12, 13], later).await?;
        assert!(stale(&db, later + 60_000).await?.is_empty());
        let w = waits(&db).await?;
        let c = commit("bbb", &["bbb"], "bbb", checks(14, 0, 0, 0));
        assert_eq!(settle(&db, &repo(), &w, &c, later).await?.len(), 1);
        let again = later + SWEEP_AGAIN_MS + 1;
        assert_eq!(
            numbers(stale(&db, again).await?),
            vec![(12, "ses_1".to_owned()), (12, "ses_2".to_owned())]
        );
        // After six looks it stops (the first was above): a branch whose
        // commits run no checks never settles them.
        let mut at = again;
        for _ in 2..SWEEP_LOOKS {
            looked(&db, &repo(), "main", &[12], at).await?;
            at += SWEEP_AGAIN_MS + 1;
            assert_eq!(stale(&db, at).await?.len(), 2);
        }
        looked(&db, &repo(), "main", &[12], at).await?;
        assert!(stale(&db, at + SWEEP_AGAIN_MS + 1).await?.is_empty());
        // Nor while the workspace no longer connects the account.
        q(&db, "UPDATE merge_checks SET looks = 0", &[])?
            .run()
            .await?;
        assert_eq!(stale(&db, at + SWEEP_AGAIN_MS + 1).await?.len(), 2);
        q(&db, "UPDATE workspace_github SET status = 'requested'", &[])?
            .run()
            .await?;
        assert!(stale(&db, at + SWEEP_AGAIN_MS + 1).await?.is_empty());
        Ok(())
    }

    /// A merge counts as looked at once GitHub answered for it: its own
    /// commit looked up, found or not, or one on the branch that holds it.
    #[wasm_bindgen_test]
    async fn a_merge_counts_as_looked_at_once_github_answered_for_it() -> AppResult<()> {
        let merge = |number: i64, sha: &str| Waiting {
            number,
            session_id: format!("ses_{number}"),
            autopilot_id: "prj_web".into(),
            base_ref: "main".into(),
            merge_sha: sha.into(),
        };
        let waiting = [merge(12, "aaa"), merge(13, "bbb"), merge(14, "ccc")];
        // The head, which holds the two before it.
        let head = commit("ccc", &["ccc", "bbb"], "ccc", checks(1, 0, 0, 3));
        assert_eq!(answered_for(&waiting, "ccc", Some(&head)), vec![13, 14]);
        // #12's own commit, which GitHub no longer has.
        assert_eq!(answered_for(&waiting, "AAA", None), vec![12]);
        // A fork's commit of main's name holds nothing on main.
        let fork = Commit {
            on_branch: false,
            ..commit(
                "fff",
                &["fff", "ccc", "bbb", "aaa"],
                "ccc",
                checks(4, 0, 0, 0),
            )
        };
        assert!(answered_for(&waiting, "fff", Some(&fork)).is_empty());
        Ok(())
    }

    /// Checks that failed settle the merge at once, and say which.
    #[wasm_bindgen_test]
    async fn failed_checks_tell_which() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        merge(&db, 13, "bbb").await?;
        let w = waits(&db).await?;
        // A commit from before the merge doesn't hold it.
        let before = commit("aaa", &["aaa"], "bbb", checks(10, 1, 0, 0));
        assert!(settle(&db, &repo(), &w, &before, NOW).await?.is_empty());
        // Nor does one that isn't on main: a fork's, whose branch has its name.
        let fork = Commit {
            on_branch: false,
            ..commit("fff", &["fff", "bbb", "aaa"], "bbb", checks(14, 0, 0, 0))
        };
        assert!(settle(&db, &repo(), &w, &fork, NOW).await?.is_empty());
        let failed = commit("bbb", &["bbb", "aaa"], "bbb", checks(13, 1, 0, 0));
        assert_eq!(settle(&db, &repo(), &w, &failed, NOW).await?.len(), 1);
        let (ended, _) = untold(&db, "prj_web").await?;
        let failing = ended.first().map(Ended::failing).unwrap_or_default();
        let line = ended
            .first()
            .and_then(|e| e.view(&failing))
            .map(|v| mc::line(&v));
        assert_eq!(
            line.as_deref(),
            Some(
                "acme/web#13 “A change” (web#2), into main: 1 of 14 checks failed at bbb: Rust tests."
            )
        );
        Ok(())
    }
}
