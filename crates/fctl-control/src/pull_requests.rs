//! The pull requests sessions list, kept current from GitHub: which sessions
//! list which (`pull_request_sessions`), looking one up when the GitHub
//! App's webhook says it or its checks changed (`fctl_domain::webhooks`),
//! and telling the sessions that list it what GitHub says (SessionObject's
//! `/pull-requests`, which takes it in with `fctl_domain::pulls::observe`).
//!
//! Only an instance that takes the webhook (`GITHUB_WEBHOOK_SECRET`) looks
//! pull requests up: elsewhere their checks would stay as they were when a
//! session first listed them.

use crate::config::Config;
use crate::db::{int, n, os, q, s};
use crate::error::AppResult;
use crate::github;
use crate::internal;
use fctl_domain::pulls;
use fctl_domain::webhooks::{self, Hints, Hook, Looked};
use fctl_proto::session::{PullRequestRef, RepoRef};
use serde::Deserialize;
use worker::{D1Database, Env};

/// Most pull requests one webhook delivery looks up: those a commit heads.
const MAX_LOOKUPS: usize = 10;
/// Most pull requests the hourly sweep looks up.
const SWEEP_LOOKUPS: i64 = 30;
/// How long what GitHub said of a pull request that isn't merged or closed
/// stands before the sweep asks again, in case a delivery went missing.
const SWEEP_AFTER_MS: i64 = 6 * 60 * 60 * 1000;

/// Whether the instance takes GitHub's webhook, and so looks pull requests up.
pub fn enabled(cfg: &Config) -> bool {
    cfg.github
        .as_ref()
        .is_some_and(|g| g.webhook_secret.is_some())
}

/// How `pull_request_sessions` names a repository.
fn key(repo: &RepoRef) -> String {
    repo.full_name().to_ascii_lowercase()
}

/// Records that a session lists `prs`, so that webhooks about them reach
/// it. Returns the repository and number of each.
pub async fn track(
    db: &D1Database,
    session_id: &str,
    prs: &[PullRequestRef],
) -> AppResult<Vec<(RepoRef, u64)>> {
    let mut statements = Vec::new();
    let mut tracked = Vec::new();
    for pr in prs {
        let Some(repo) = pr.repo.clone().or_else(|| pulls::repo_of(&pr.url)) else {
            continue;
        };
        statements.push(q(
            db,
            "INSERT OR IGNORE INTO pull_request_sessions (repo, number, session_id, state) VALUES (?, ?, ?, ?)",
            &[
                s(&key(&repo)),
                n(pr.number as i64),
                s(session_id),
                os(pr.state.as_deref()),
            ],
        )?);
        tracked.push((repo, pr.number));
    }
    if !statements.is_empty() {
        crate::db::batch(db, statements).await?;
    }
    Ok(tracked)
}

#[derive(Deserialize)]
struct Listed {
    #[serde(deserialize_with = "int")]
    number: i64,
    session_id: String,
}

/// Does what a webhook delivery asks: looks up the pull requests it
/// concerns that sessions list, and tells those sessions what GitHub says.
/// Returns how many sessions heard.
pub async fn hook(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    hook: &Hook,
    now: i64,
) -> AppResult<usize> {
    let repo = key(&hook.repo);
    let mut which = Vec::new();
    let mut params = vec![s(&repo)];
    if !hook.numbers.is_empty() {
        which.push(format!(
            "number IN ({})",
            vec!["?"; hook.numbers.len()].join(", ")
        ));
        params.extend(hook.numbers.iter().map(|x| n(*x as i64)));
    }
    if let Some(sha) = &hook.sha {
        which.push("head_sha = ?".to_owned());
        params.push(s(sha));
    }
    if which.is_empty() {
        return Ok(0);
    }
    let listed: Vec<Listed> = q(
        db,
        &format!(
            "SELECT number, session_id FROM pull_request_sessions WHERE repo = ? AND ({})",
            which.join(" OR ")
        ),
        &params,
    )?
    .all()
    .await?;
    let mut numbers: Vec<i64> = Vec::new();
    for l in &listed {
        if !numbers.contains(&l.number) && numbers.len() < MAX_LOOKUPS {
            numbers.push(l.number);
        }
    }
    let mut told = 0;
    for number in numbers {
        let number = number as u64;
        // The pull request a `pull_request` event is about: what it says
        // helps the lookup, and stands in for it when it fails.
        let payload = hook.payload.as_ref().filter(|p| p.seen.number == number);
        let hints = payload.map(webhooks::Payload::hints).unwrap_or_default();
        let looked = match look_up(cfg, hook.installation, &hook.repo, number, hints, now).await {
            Some(l) => l,
            None => match payload {
                Some(p) => Looked {
                    pr: p.seen.clone(),
                    head: p.head.clone(),
                },
                None => continue,
            },
        };
        record(db, &repo, number, Some(&looked), now).await?;
        let sessions = listed
            .iter()
            .filter(|l| l.number as u64 == number)
            .map(|l| l.session_id.as_str());
        told += tell(env, sessions, &looked.pr).await;
    }
    Ok(told)
}

/// Looks up pull requests sessions list and tells those sessions what
/// GitHub says: those a session just found, and those the sweep picks.
/// Returns how many sessions heard.
pub async fn refresh(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    prs: &[(RepoRef, u64)],
    now: i64,
) -> AppResult<usize> {
    let mut told = 0;
    for (repo, number) in prs {
        let looked = look_up(cfg, None, repo, *number, Hints::default(), now).await;
        record(db, &key(repo), *number, looked.as_ref(), now).await?;
        let Some(looked) = looked else { continue };
        let sessions: Vec<Listed> = q(
            db,
            "SELECT number, session_id FROM pull_request_sessions WHERE repo = ? AND number = ?",
            &[s(&key(repo)), n(*number as i64)],
        )?
        .all()
        .await?;
        told += tell(
            env,
            sessions.iter().map(|l| l.session_id.as_str()),
            &looked.pr,
        )
        .await;
    }
    Ok(told)
}

#[derive(Deserialize)]
struct Stale {
    repo: String,
    #[serde(deserialize_with = "int")]
    number: i64,
}

/// Looks up the pull requests sessions list that aren't merged or closed
/// and that nobody looked up for a while: those of sessions from before the
/// webhook, and those whose delivery went missing. From the hourly job.
pub async fn sweep(env: &Env, db: &D1Database, now: i64) -> AppResult<usize> {
    let cfg = Config::from_env(env);
    if !enabled(&cfg) {
        return Ok(0);
    }
    let stale: Vec<Stale> = q(
        db,
        "SELECT repo, number FROM pull_request_sessions \
         WHERE (state IS NULL OR state NOT IN ('merged', 'closed')) AND (checked_at IS NULL OR checked_at < ?) \
         GROUP BY repo, number ORDER BY MAX(COALESCE(checked_at, 0)) LIMIT ?",
        &[n(now - SWEEP_AFTER_MS), n(SWEEP_LOOKUPS)],
    )?
    .all()
    .await?;
    let prs: Vec<(RepoRef, u64)> = stale
        .iter()
        .filter_map(|p| Some((RepoRef::parse(&p.repo)?, u64::try_from(p.number).ok()?)))
        .collect();
    refresh(env, &cfg, db, &prs, now).await
}

/// Looks a pull request up on GitHub; `None` when that fails, or GitHub
/// has none.
async fn look_up(
    cfg: &Config,
    installation: Option<u64>,
    repo: &RepoRef,
    number: u64,
    hints: Hints<'_>,
    now: i64,
) -> Option<Looked> {
    let gh = github::from_config(cfg, now).ok()?;
    let answer = gh.pull_request(installation, repo, number).await.ok()?;
    let looked = webhooks::looked_up(&answer, now, hints);
    if looked.is_none() {
        worker::console_log!("GitHub has no pull request {}#{number}", repo.full_name());
    }
    looked
}

/// Notes what GitHub said of a pull request, and when the control plane
/// asked: its head commit, which commit statuses name, and its state, which
/// the sweep goes by.
async fn record(
    db: &D1Database,
    repo: &str,
    number: u64,
    looked: Option<&Looked>,
    now: i64,
) -> AppResult<()> {
    q(
        db,
        "UPDATE pull_request_sessions SET head_sha = COALESCE(?, head_sha), state = COALESCE(?, state), checked_at = ? \
         WHERE repo = ? AND number = ?",
        &[
            os(looked.and_then(|l| l.head.as_deref())),
            os(looked.and_then(|l| l.pr.state.as_deref())),
            n(now),
            s(repo),
            n(number as i64),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// Tells sessions what GitHub says of a pull request they list. Returns how
/// many heard.
async fn tell<'a>(
    env: &Env,
    sessions: impl Iterator<Item = &'a str>,
    pr: &PullRequestRef,
) -> usize {
    let mut told = 0;
    for id in sessions {
        let result: AppResult<serde_json::Value> = async {
            let stub = internal::session_stub(env, id)?;
            let seen = internal::PullRequestsSeen {
                prs: vec![pr.clone()],
            };
            internal::post(&stub, "/pull-requests", &seen).await
        }
        .await;
        match result {
            Ok(_) => told += 1,
            Err(e) => worker::console_error!("telling session {id} of {}: {}", pr.url, e.0.message),
        }
    }
    told
}
