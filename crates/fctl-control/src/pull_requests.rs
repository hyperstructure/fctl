//! The pull requests sessions list, kept current from GitHub: which sessions
//! list which (`pull_request_sessions`), looking one up when the GitHub
//! App's webhook says it or its checks changed (`fctl_domain::webhooks`),
//! and telling the sessions that list it what GitHub says (SessionObject's
//! `/pull-requests`, which takes it in with `fctl_domain::pulls::observe`).
//!
//! Only an instance that takes the webhook (`GITHUB_WEBHOOK_SECRET`) looks
//! pull requests up: elsewhere their checks would stay as they were when a
//! session first listed them.
//!
//! One query asks GitHub about up to 50 pull requests of an account
//! (`webhooks::MAX_LOOKUP`), for one point of the rate limit of the app's
//! installation there, which the agents of its sessions share, and fewer
//! when GitHub gives up on so many with so many checks (`webhooks::fewer`).

use crate::config::Config;
use crate::ctx::now_ms;
use crate::db::{int, n, os, q, s};
use crate::error::{AppError, AppResult, unavailable};
use crate::github;
use crate::{follow_ups, internal};
use fctl_domain::follow_ups::{self as fu, Ask};
use fctl_domain::pulls;
use fctl_domain::reviews as rv;
use fctl_domain::webhooks::{self, Fields, Hints, Hook, Looked, Payload};
use fctl_proto::session::{PullRequestRef, RepoRef};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use worker::{D1Database, Env};

/// Most pull requests one webhook delivery looks up: those a commit heads.
const MAX_LOOKUPS: usize = 10;
/// Most pull request numbers of a delivery that D1 is asked about (it takes
/// 100 parameters a query).
const MAX_NUMBERS: usize = 50;
/// Most pull requests the hourly sweep looks up, in queries of 50 or fewer
/// ([`refresh`]): enough for those of all of an instance's sessions from
/// before the webhook.
const SWEEP_LOOKUPS: i64 = 500;
/// How long [`refresh`] goes on looking pull requests up, well within the
/// 15 minutes the Worker's cron trigger may run: GitHub takes up to ten
/// seconds a query, and some accounts' pull requests take one each.
const LOOKUP_TIME_MS: i64 = 5 * 60 * 1000;
/// How long what GitHub said of a pull request that isn't merged or closed
/// stands before the sweep asks again, in case a delivery went missing.
const SWEEP_AFTER_MS: i64 = 6 * 60 * 60 * 1000;
/// The points of an installation's GraphQL rate limit (5,000 an hour or
/// more) that lookups of many pull requests leave to its sessions' agents:
/// they stop for an account whose installation has fewer left, and the
/// next sweep carries on.
const RESERVE: u64 = 1_000;

/// Whether the instance takes GitHub's webhook, and so looks pull requests up.
pub fn enabled(cfg: &Config) -> bool {
    cfg.github
        .as_ref()
        .is_some_and(|g| g.webhook_secret.is_some())
}

/// GitHub's ids of the instance's GitHub App and of the app review agents
/// post as, whose review checks lookups tell apart from the others
/// (`webhooks::looked_up`).
fn app_ids(cfg: &Config) -> Vec<u64> {
    cfg.github
        .iter()
        .chain(&cfg.github_review)
        .filter_map(|g| g.app_id.trim().parse().ok())
        .collect()
}

/// How `pull_request_sessions` names a repository.
pub(crate) fn key(repo: &RepoRef) -> String {
    repo.full_name().to_ascii_lowercase()
}

/// Whether the session of a `pull_request_sessions` row `l` may have its
/// pull request looked up now: its project uses the repository (its primary
/// one, one it lists, or every one of the account) and its workspace has an
/// active connection to the account, for every repository or this one; the
/// SQL of `fctl_domain::repos::may_look_up`. A host or an agent can claim
/// any pull request, and a workspace can lose an account, so rows kept from
/// before are held to it too. `l.repo` is `owner/name` in lowercase.
///
/// With `answered` (parameters, `?N`): GitHub's id of the account that
/// answered and of the installation it answered through, only where the
/// connection is to that account, or, from before ids were kept, recorded
/// that installation, since a login can change hands.
pub(crate) fn in_reach(answered: Option<(&str, &str)>) -> String {
    let account = answered
        .map(|(account, installation)| {
            format!(
                " AND (rg.account_id = {account} OR (rg.account_id IS NULL AND rg.installation_id = {installation}))"
            )
        })
        .unwrap_or_default();
    format!(
        "EXISTS (SELECT 1 FROM sessions rs JOIN projects rp ON rp.id = rs.project_id \
         JOIN workspace_github rg ON rg.workspace_id = rp.workspace_id AND rg.status = 'active' \
         AND lower(rg.account) = substr(l.repo, 1, instr(l.repo, '/') - 1){account} \
         WHERE rs.id = l.session_id \
         AND (rg.repos IS NULL OR EXISTS (SELECT 1 FROM json_each(rg.repos) rr \
           WHERE lower(rr.value) = substr(l.repo, instr(l.repo, '/') + 1))) \
         AND (lower(rp.repo_owner || '/' || rp.repo_name) = l.repo \
           OR EXISTS (SELECT 1 FROM project_repos pr WHERE pr.project_id = rp.id \
             AND lower(pr.owner) = substr(l.repo, 1, instr(l.repo, '/') - 1) \
             AND (pr.name = '*' OR lower(pr.name) = substr(l.repo, instr(l.repo, '/') + 1)))))"
    )
}

/// Records that a session lists `prs`, so that webhooks about them reach
/// it: those its tokens reach (`fctl_domain::repos::may_look_up`), the only
/// ones the control plane asks GitHub about for it. Returns the repository
/// and number of each.
pub async fn track(
    db: &D1Database,
    session_id: &str,
    prs: &[PullRequestRef],
) -> AppResult<Vec<(RepoRef, u64)>> {
    let Some(session) = crate::spec::session_row(db, session_id).await? else {
        return Ok(Vec::new());
    };
    let Some(project) = crate::spec::project_row(db, &session.project_id).await? else {
        return Ok(Vec::new());
    };
    let primary = project.repo();
    let grants = crate::spec::grants(db, &project.id).await?;
    let connections = crate::workspaces::connections(db, project.workspace_id()).await?;
    let mut statements = Vec::new();
    let mut tracked = Vec::new();
    for pr in prs {
        let Some(repo) = pr.repo.clone().or_else(|| pulls::repo_of(&pr.url)) else {
            continue;
        };
        let scope = crate::workspaces::scope(&connections, &repo.owner);
        if !fctl_domain::repos::may_look_up(primary.as_ref(), &grants, scope.as_ref(), &repo) {
            continue;
        }
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

/// A session that lists a pull request, with its branch and what GitHub
/// last said of the pull request's state.
#[derive(Deserialize)]
pub struct Lister {
    pub session_id: String,
    pub branch: Option<String>,
    pub state: Option<String>,
}

/// The sessions that list a pull request, but review agents' ones, in the
/// order they started: those it may be looked up for ([`in_reach`]), with
/// `answered` (GitHub's ids of the account it is in and the installation
/// that answered for it) only in workspaces connected to that account.
pub async fn listed_by(
    db: &D1Database,
    repo: &RepoRef,
    number: u64,
    answered: Option<(u64, u64)>,
) -> AppResult<Vec<Lister>> {
    let reach = match answered {
        Some(_) => in_reach(Some(("?3", "?4"))),
        None => in_reach(None),
    };
    let mut params = vec![s(&key(repo)), n(number as i64)];
    if let Some((account, installation)) = answered {
        params.extend([n(account as i64), n(installation as i64)]);
    }
    q(
        db,
        &format!(
            "SELECT l.session_id AS session_id, s.branch AS branch, l.state AS state FROM pull_request_sessions l \
             JOIN sessions s ON s.id = l.session_id WHERE l.repo = ?1 AND l.number = ?2 \
             AND NOT EXISTS (SELECT 1 FROM reviews rv WHERE rv.session_id = l.session_id) AND {reach} \
             ORDER BY s.created_at, s.id LIMIT 20"
        ),
        &params,
    )?
    .all()
    .await
}

#[derive(Deserialize)]
struct Listed {
    #[serde(deserialize_with = "int")]
    number: i64,
}

/// Does what a webhook delivery asks: looks up the pull requests it
/// concerns that sessions list, and tells those sessions what GitHub says.
/// A merge moved its base branch too: the other pull requests that may
/// conflict with it now are looked up as well ([`moved_onto`]), since
/// GitHub works that out only when asked. Returns how many sessions heard.
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
    let numbers: Vec<u64> = hook.numbers.iter().copied().take(MAX_NUMBERS).collect();
    if !numbers.is_empty() {
        which.push(format!(
            "l.number IN ({})",
            vec!["?"; numbers.len()].join(", ")
        ));
        params.extend(numbers.iter().map(|x| n(*x as i64)));
    }
    if let Some(sha) = &hook.sha {
        which.push("l.head_sha = ?".to_owned());
        params.push(s(sha));
    }
    let listed: Vec<Listed> = if which.is_empty() {
        Vec::new()
    } else {
        q(
            db,
            &format!(
                "SELECT DISTINCT l.number AS number FROM pull_request_sessions l WHERE l.repo = ? AND ({}) \
                 AND {} LIMIT {MAX_LOOKUPS}",
                which.join(" OR "),
                in_reach(None)
            ),
            &params,
        )?
        .all()
        .await?
    };
    let prs: Vec<(RepoRef, u64)> = listed
        .iter()
        .filter_map(|l| Some((hook.repo.clone(), u64::try_from(l.number).ok()?)))
        .collect();
    let answered = hook.owner_id.zip(hook.installation);
    let merged = hook
        .payload
        .as_ref()
        .filter(|p| p.action == "closed" && p.seen.state.as_deref() == Some("merged"));
    if prs.is_empty() && merged.is_none() {
        return Ok(0);
    }
    let gh = github::from_config(cfg, now).ok();
    let mut seen = Vec::new();
    if !prs.is_empty() {
        let answer = match &gh {
            Some(gh) => gh
                .pull_requests(hook.installation, &prs, Fields::All)
                .await
                .ok()
                .flatten(),
            None => None,
        };
        for (i, (repo, number)) in prs.into_iter().enumerate() {
            // The pull request a `pull_request` event is about: what it says
            // helps the lookup, and stands in for it when it fails.
            let payload = hook.payload.as_ref().filter(|p| p.seen.number == number);
            let hints = payload.map(webhooks::Payload::hints).unwrap_or_default();
            let looked = answer
                .as_ref()
                .and_then(|a| webhooks::looked_up(a, i, now, hints, &app_ids(cfg)))
                .or_else(|| {
                    payload.map(|p| Looked {
                        pr: p.seen.clone(),
                        head: p.head.clone(),
                        author: p.author.clone(),
                        head_ref: p.head_ref.clone(),
                        base_ref: p.base_ref.clone(),
                        failing: Vec::new(),
                        mergeable: None,
                    })
                });
            if looked.is_some() {
                seen.push(Seen {
                    repo,
                    number,
                    looked,
                    checks: true,
                });
            }
        }
    }
    // Its base branch moved: what goes into it may conflict with it now.
    // Without GitHub's word of who answered, nobody would hear of it.
    if let (Some(p), Some(gh), Some(_)) = (merged, &gh, answered) {
        let others = match moved_onto(db, &hook.repo, p.seen.number).await {
            Ok(others) => others,
            Err(e) => {
                worker::console_error!(
                    "finding what went into {}'s base: {}",
                    p.seen.url,
                    e.0.message
                );
                Vec::new()
            }
        };
        if !others.is_empty() {
            let answer = gh
                .pull_requests(hook.installation, &others, Fields::WithoutChecks)
                .await
                .ok()
                .flatten();
            for (i, (repo, number)) in others.into_iter().enumerate() {
                let looked = answer
                    .as_ref()
                    .and_then(|a| webhooks::looked_up(a, i, now, Hints::default(), &app_ids(cfg)));
                if looked.is_some() {
                    seen.push(Seen {
                        repo,
                        number,
                        looked,
                        checks: false,
                    });
                }
            }
        }
    }
    if seen.is_empty() {
        return Ok(0);
    }
    // What the agents that opened them hear of: failed checks after a
    // delivery about checks, a merge or a close after the one that says it,
    // and conflicts with their base branches after any. Without GitHub's
    // word of who answered, nobody hears of anything.
    let news: Vec<follow_ups::News> = match answered {
        Some(_) => seen
            .iter()
            .filter_map(|x| {
                let payload = hook.payload.as_ref().filter(|p| p.seen.number == x.number);
                news(x, payload, hook.checks, 1, now)
            })
            .collect(),
        None => Vec::new(),
    };
    let routed = route(cfg, db, &news, answered).await;
    settle(env, db, &seen, now, answered, routed).await
}

/// The pull requests of `repo` that sessions list, but `merged`, that may
/// conflict with their base branch now that `merged` went into it: those
/// neither merged nor closed whose sessions' projects follow up on
/// conflicts, newest first, up to [`webhooks::MAX_LOOKUP`]. fctl doesn't
/// keep which branch each goes into: GitHub says, and conflicts with
/// another base are news all the same.
async fn moved_onto(
    db: &D1Database,
    repo: &RepoRef,
    merged: u64,
) -> AppResult<Vec<(RepoRef, u64)>> {
    let listed: Vec<Listed> = q(
        db,
        &format!(
            "SELECT DISTINCT l.number AS number FROM pull_request_sessions l \
             JOIN sessions ms ON ms.id = l.session_id JOIN projects mp ON mp.id = ms.project_id \
             WHERE l.repo = ? AND l.number != ? AND (l.state IS NULL OR l.state NOT IN ('merged', 'closed')) \
             AND ms.archived = 0 AND mp.archived_at IS NULL \
             AND json_extract(mp.settings, '$.follow_ups.conflicts') = 1 \
             AND NOT EXISTS (SELECT 1 FROM reviews rv WHERE rv.session_id = l.session_id) \
             AND {} ORDER BY l.number DESC LIMIT {}",
            in_reach(None),
            webhooks::MAX_LOOKUP
        ),
        &[s(&key(repo)), n(merged as i64)],
    )?
    .all()
    .await?;
    Ok(listed
        .iter()
        .filter_map(|l| Some((repo.clone(), u64::try_from(l.number).ok()?)))
        .collect())
}

/// Who hears of `news` ([`follow_ups::route`]): nobody when finding out
/// fails.
async fn route(
    cfg: &Config,
    db: &D1Database,
    news: &[follow_ups::News],
    answered: Option<(u64, u64)>,
) -> follow_ups::Routed {
    if news.is_empty() {
        return follow_ups::Routed::default();
    }
    match follow_ups::route(cfg, db, news, answered).await {
        Ok(routed) => routed,
        Err(e) => {
            worker::console_error!("following up on pull requests: {}", e.0.message);
            follow_ups::Routed::default()
        }
    }
}

/// What a lookup found of a pull request that the agent that opened it may
/// hear of ([`follow_ups::route`]): failed checks of its newest commit,
/// with `checks` (the delivery is about checks); that it merged or closed,
/// when its delivery's `payload` says so; that it conflicts with its base
/// branch; and that GitHub didn't know yet whether it does, asked `asked`
/// times now. `now`: when the lookup asked.
fn news(
    x: &Seen,
    payload: Option<&Payload>,
    checks: bool,
    asked: u32,
    now: i64,
) -> Option<follow_ups::News> {
    let looked = x.looked.as_ref()?;
    let mut events = Vec::new();
    if checks && let Some(e) = fu::checks_failed(looked) {
        events.push((e, None));
    }
    if let Some(p) = payload
        && let Some(e) = fu::closed(p)
    {
        events.push((e, p.seen.state_changed_at));
    }
    if let Some(e) = fu::conflicts(looked) {
        events.push((e, None));
    }
    let unsure = fu::unsure(looked).then_some(asked);
    if events.is_empty() && unsure.is_none() {
        return None;
    }
    let base = looked
        .base_ref
        .clone()
        .or_else(|| payload.and_then(|p| p.base_ref.clone()));
    Some(follow_ups::News {
        pull: fu::Pull {
            repo: x.repo.clone(),
            number: x.number,
            url: looked.pr.url.clone(),
            title: looked.pr.title.clone(),
            state: payload
                .and_then(|p| p.seen.state.clone())
                .or_else(|| looked.pr.state.clone()),
            base,
        },
        author: looked
            .author
            .clone()
            .or_else(|| payload.and_then(|p| p.author.clone())),
        head_ref: looked
            .head_ref
            .clone()
            .or_else(|| payload.and_then(|p| p.head_ref.clone())),
        events,
        unsure,
        seen: now,
    })
}

/// Looks up pull requests sessions list and tells those sessions what
/// GitHub says: those a session just `found`, and those the sweep picks.
/// Asks about those of each account 50 at a time; when GitHub gives up on
/// so many at once, asks again about fewer (`webhooks::fewer`), and as few
/// at a time about the rest of the account's. Leaves the rest of an
/// account's to the next sweep when GitHub doesn't answer, gives up on one
/// pull request alone, or its installation has less than [`RESERVE`] of
/// its rate limit left, and all that are left once it has looked pull
/// requests up for [`LOOKUP_TIME_MS`]. Returns how many times sessions
/// heard.
///
/// The sweep brings no agent back. Those a session just found, the agents
/// that opened them hear of their conflicts with their base branches: a
/// pull request opened with conflicts gets no check deliveries that would
/// say so, as GitHub runs none of its `pull_request` workflows.
pub async fn refresh(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    prs: &[(RepoRef, u64)],
    now: i64,
    found: bool,
) -> AppResult<usize> {
    let gh = github::from_config(cfg, now)?;
    let started = now_ms();
    let mut accounts: BTreeMap<String, Vec<(RepoRef, u64)>> = BTreeMap::new();
    for (repo, number) in prs {
        accounts
            .entry(repo.owner.to_ascii_lowercase())
            .or_default()
            .push((repo.clone(), *number));
    }
    let mut told = 0;
    for (account, prs) in accounts {
        // Its installation, and who has the login now: what it says goes to
        // the sessions of workspaces connected to that account.
        let installed = gh.installation(&account).await;
        let answered = installed
            .as_ref()
            .ok()
            .and_then(|f| Some((f.account_id?, f.id)));
        let mut at_once = webhooks::MAX_LOOKUP;
        let mut rest = prs.as_slice();
        while !rest.is_empty() {
            if now_ms() - started > LOOKUP_TIME_MS {
                worker::console_log!(
                    "leaving {account}'s and later accounts' pull requests for later: looking up took {} s",
                    (now_ms() - started) / 1000
                );
                return Ok(told);
            }
            let (batch, after) = rest.split_at(at_once.min(rest.len()));
            let answer = match &installed {
                Ok(f) => gh.pull_requests(Some(f.id), batch, Fields::All).await,
                Err(e) => Err(AppError(e.0.clone())),
            };
            // GitHub gave up on so many at once: ask again about fewer, and
            // about as few at a time from now on, since the account's other
            // pull requests are likely to have as many checks.
            if matches!(answer, Ok(None))
                && let Some(fewer) = webhooks::fewer(batch.len())
            {
                worker::console_log!(
                    "looking up {account}'s pull requests {fewer} at a time: GitHub gave up on {} at once",
                    batch.len()
                );
                at_once = fewer;
                continue;
            }
            let answer = answer.and_then(|a| {
                a.ok_or_else(|| unavailable("GitHub gave up on looking one up alone"))
            });
            let seen: Vec<Seen> = batch
                .iter()
                .enumerate()
                .map(|(i, (repo, number))| Seen {
                    repo: repo.clone(),
                    number: *number,
                    looked: answer.as_ref().ok().and_then(|a| {
                        webhooks::looked_up(a, i, now, Hints::default(), &app_ids(cfg))
                    }),
                    checks: true,
                })
                .collect();
            let routed = match answered.filter(|_| found) {
                Some(_) => {
                    let news: Vec<follow_ups::News> = seen
                        .iter()
                        .filter_map(|x| news(x, None, false, 1, now))
                        .collect();
                    route(cfg, db, &news, answered).await
                }
                None => follow_ups::Routed::default(),
            };
            // Those GitHub didn't answer for wait their six hours too, so
            // that one that always fails can't hold up the others.
            told += settle(env, db, &seen, now, answered, routed).await?;
            let left = match &answer {
                Ok(a) => webhooks::remaining(a),
                Err(e) => {
                    worker::console_error!("looking up {account}'s pull requests: {}", e.0.message);
                    break;
                }
            };
            rest = after;
            if !rest.is_empty()
                && let Some(left) = left.filter(|left| *left < RESERVE)
            {
                worker::console_log!(
                    "leaving {account}'s other pull requests for later: {left} points of its rate limit left"
                );
                break;
            }
        }
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
        &format!(
            "SELECT l.repo AS repo, l.number AS number FROM pull_request_sessions l \
             WHERE (l.state IS NULL OR l.state NOT IN ('merged', 'closed')) AND (l.checked_at IS NULL OR l.checked_at < ?) \
             AND {} GROUP BY l.repo, l.number ORDER BY MAX(COALESCE(l.checked_at, 0)) LIMIT ?",
            in_reach(None)
        ),
        &[n(now - SWEEP_AFTER_MS), n(SWEEP_LOOKUPS)],
    )?
    .all()
    .await?;
    let prs: Vec<(RepoRef, u64)> = stale
        .iter()
        .filter_map(|p| Some((RepoRef::parse(&p.repo)?, u64::try_from(p.number).ok()?)))
        .collect();
    refresh(env, &cfg, db, &prs, now, false).await
}

/// Asks GitHub again whether pull requests conflict with their base
/// branches, where it didn't know when it was last asked: `asks`, from a
/// session's alarm. The sessions that list them hear what it says now, and
/// the agents that opened them of their conflicts; those GitHub still
/// doesn't know about, their sessions ask about again later
/// (`fctl_domain::follow_ups::ask_again_in`). Returns how many sessions
/// heard.
pub async fn ask_again(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    asks: &[Ask],
    now: i64,
) -> AppResult<usize> {
    if !enabled(cfg) {
        return Ok(0);
    }
    let gh = github::from_config(cfg, now)?;
    let mut accounts: BTreeMap<String, Vec<&Ask>> = BTreeMap::new();
    for a in asks {
        accounts
            .entry(a.repo.owner.to_ascii_lowercase())
            .or_default()
            .push(a);
    }
    let mut told = 0;
    for (account, asks) in accounts {
        let installed = match gh.installation(&account).await {
            Ok(f) => f,
            Err(e) => {
                worker::console_error!(
                    "asking whether {account}'s pull requests conflict: {}",
                    e.0.message
                );
                continue;
            }
        };
        let answered = installed.account_id.map(|a| (a, installed.id));
        let asks: Vec<&Ask> = asks.into_iter().take(webhooks::MAX_LOOKUP).collect();
        let prs: Vec<(RepoRef, u64)> = asks.iter().map(|a| (a.repo.clone(), a.number)).collect();
        let answer = match gh
            .pull_requests(Some(installed.id), &prs, Fields::WithoutChecks)
            .await
        {
            Ok(Some(answer)) => answer,
            Ok(None) => {
                worker::console_error!(
                    "asking whether {account}'s pull requests conflict: GitHub gave up"
                );
                continue;
            }
            Err(e) => {
                worker::console_error!(
                    "asking whether {account}'s pull requests conflict: {}",
                    e.0.message
                );
                continue;
            }
        };
        let seen: Vec<Seen> = prs
            .into_iter()
            .enumerate()
            .map(|(i, (repo, number))| Seen {
                repo,
                number,
                looked: webhooks::looked_up(&answer, i, now, Hints::default(), &app_ids(cfg)),
                checks: false,
            })
            .collect();
        let news: Vec<follow_ups::News> = match answered {
            Some(_) => seen
                .iter()
                .zip(&asks)
                .filter_map(|(x, a)| news(x, None, false, a.asked.saturating_add(1), now))
                .collect(),
            None => Vec::new(),
        };
        let routed = route(cfg, db, &news, answered).await;
        told += settle(env, db, &seen, now, answered, routed).await?;
    }
    Ok(told)
}

/// What GitHub said of a pull request sessions list: nothing when it didn't
/// answer, or has none.
struct Seen {
    repo: RepoRef,
    number: u64,
    looked: Option<Looked>,
    /// The lookup asked about its checks too ([`Fields::All`]): only such a
    /// lookup spares it the sweep's.
    checks: bool,
}

#[derive(Deserialize)]
struct Listing {
    repo: String,
    #[serde(deserialize_with = "int")]
    number: i64,
    session_id: String,
}

/// Tells the sessions that list pull requests what GitHub said of them,
/// each session all of its at once, and then notes what it said and when
/// the control plane asked: their head commits, which commit statuses
/// name, and their states, which the sweep goes by. A pull request keeps
/// its old state there until every session that lists it heard, so that
/// the sweep asks again for those that didn't. Returns how many heard.
///
/// `answered`: GitHub's ids of the account that answered and of the
/// installation it answered through. What it said reaches the sessions
/// whose workspaces connected that account ([`in_reach`]), and nobody when
/// GitHub didn't say who answered. `routed`: what the sessions whose
/// agents opened them hear of, and ask GitHub again, by session
/// ([`follow_ups::route`]).
async fn settle(
    env: &Env,
    db: &D1Database,
    seen: &[Seen],
    now: i64,
    answered: Option<(u64, u64)>,
    mut routed: follow_ups::Routed,
) -> AppResult<usize> {
    if seen.is_empty() {
        return Ok(0);
    }
    let found: Vec<_> = seen
        .iter()
        .filter(|x| x.looked.is_some())
        .map(|x| json!([key(&x.repo), x.number]))
        .collect();
    let listing: Vec<Listing> = match answered {
        Some((account, installation)) if !found.is_empty() => q(
            db,
            &format!(
                "SELECT l.repo AS repo, l.number AS number, l.session_id AS session_id FROM pull_request_sessions l \
                 WHERE (l.repo, l.number) IN \
                 (SELECT json_extract(value, '$[0]'), json_extract(value, '$[1]') FROM json_each(?1)) AND {}",
                in_reach(Some(("?2", "?3")))
            ),
            &[
                s(&json!(found).to_string()),
                n(account as i64),
                n(installation as i64),
            ],
        )?
        .all()
        .await?,
        _ => Vec::new(),
    };
    let of = |l: &Listing| {
        seen.iter()
            .find(|x| x.number as i64 == l.number && key(&x.repo) == l.repo)
    };
    let reviewed = if listing.is_empty() {
        BTreeMap::new()
    } else {
        reviewed(db, seen).await
    };
    let mut sessions: BTreeMap<&str, Vec<PullRequestRef>> = BTreeMap::new();
    for l in &listing {
        if let Some(x) = of(l)
            && let Some(looked) = &x.looked
        {
            let mut pr = looked.pr.clone();
            if let Some(c) = pr.checks.as_mut()
                && let Some(check) = c.review.take()
            {
                let kept = check
                    .session_id
                    .as_ref()
                    .and_then(|id| reviewed.get(id.as_str()));
                c.review = Some(rv::checked(
                    check,
                    &x.repo,
                    x.number,
                    c.sha.as_deref(),
                    kept,
                ));
            }
            sessions.entry(&l.session_id).or_default().push(pr);
        }
    }
    let mut told = 0;
    let mut unheard = Vec::new();
    for (id, prs) in sessions {
        let items = routed.items.remove(id).unwrap_or_default();
        let asks = routed.asks.remove(id).unwrap_or_default();
        if tell(env, id, prs, items.clone(), asks).await {
            told += 1;
        } else {
            unheard.push(id);
            // The sweep asks GitHub again for its pull requests, but brings
            // no follow-ups: they go once more on their own. What GitHub
            // didn't know yet waits for the next delivery.
            if !items.is_empty()
                && let Err(e) = follow_ups::hand(env, id, items).await
            {
                worker::console_error!("following up in session {id}: {}", e.0.message);
            }
        }
    }
    let mut statements = Vec::new();
    for x in seen {
        let repo = key(&x.repo);
        let heard = !listing.iter().any(|l| {
            l.number == x.number as i64
                && l.repo == repo
                && unheard.contains(&l.session_id.as_str())
        });
        let state = x
            .looked
            .as_ref()
            .filter(|_| heard)
            .and_then(|l| l.pr.state.as_deref());
        // Asked, for the sweep; what GitHub said, only for the sessions it
        // could tell.
        if x.checks {
            statements.push(q(
                db,
                "UPDATE pull_request_sessions SET checked_at = ? WHERE repo = ? AND number = ?",
                &[n(now), s(&repo), n(x.number as i64)],
            )?);
        }
        if let Some((account, installation)) = answered {
            statements.push(q(
                db,
                &format!(
                    "UPDATE pull_request_sessions SET head_sha = COALESCE(?1, head_sha), state = COALESCE(?2, state) \
                     WHERE repo = ?3 AND number = ?4 AND session_id IN (SELECT l.session_id FROM pull_request_sessions l \
                     WHERE l.repo = ?3 AND l.number = ?4 AND {})",
                    in_reach(Some(("?5", "?6")))
                ),
                &[
                    os(x.looked.as_ref().and_then(|l| l.head.as_deref())),
                    os(state),
                    s(&repo),
                    n(x.number as i64),
                    n(account as i64),
                    n(installation as i64),
                ],
            )?);
        }
    }
    crate::db::batch(db, statements).await?;
    Ok(told)
}

/// What the instance kept of the reviews whose checks `seen` show, by their
/// sessions: how each went and what it found (`crate::reviews::kept`).
/// Without it, the checks say how they went alone.
async fn reviewed(db: &D1Database, seen: &[Seen]) -> BTreeMap<String, rv::Kept> {
    let ids: Vec<&str> = seen
        .iter()
        .filter_map(|x| {
            x.looked
                .as_ref()?
                .pr
                .checks
                .as_ref()?
                .review
                .as_ref()?
                .session_id
                .as_ref()
        })
        .map(|id| id.as_str())
        .collect();
    match crate::reviews::kept(db, &ids).await {
        Ok(kept) => kept,
        Err(e) => {
            worker::console_error!(
                "reading the reviews of pull requests' checks: {}",
                e.0.message
            );
            BTreeMap::new()
        }
    }
}

/// Tells a session what GitHub says of pull requests it lists, with what
/// its agent hears of those it opened (`follow_ups`) and those to ask
/// GitHub about again (`asks`). Returns whether it heard.
async fn tell(
    env: &Env,
    id: &str,
    prs: Vec<PullRequestRef>,
    follow_ups: Vec<fu::Item>,
    asks: Vec<Ask>,
) -> bool {
    let result: AppResult<serde_json::Value> = async {
        let stub = internal::session_stub(env, id)?;
        internal::post(
            &stub,
            "/pull-requests",
            &internal::PullRequestsSeen {
                prs,
                follow_ups,
                asks,
            },
        )
        .await
    }
    .await;
    if let Err(e) = &result {
        worker::console_error!("telling session {id} of its pull requests: {}", e.0.message);
    }
    result.is_ok()
}
