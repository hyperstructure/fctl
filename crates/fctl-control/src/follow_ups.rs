//! Follow-ups (`fctl_domain::follow_ups`): what brings an agent back to a
//! pull request its session opened, where its project turns them on
//! (`ProjectSettings::follow_ups`). Comments come here from the webhook
//! ([`on_comment`]); failed checks and closed pull requests come with the
//! lookups that keep sessions' pull requests current
//! (`crate::pull_requests::hook`), which ask [`route`] who hears of them.
//! Either way the session that opened the pull request gets them
//! (SessionObject's `/follow-up`), gathers them for a few seconds and
//! passes them on to its agent in a message from fctl, which resumes it if
//! it stopped, on the Claude subscription it ran on.
//!
//! Only pull requests the instance's GitHub App opened count: one a person
//! opened is theirs to act on. Only comments from people who can write to
//! the repository reach an agent, as GitHub says when asked, and only those
//! that address it, so that nobody else on a public repository steers it.

use crate::config::Config;
use crate::ctx::{Ctx, now_ms};
use crate::db::{n, q, s};
use crate::error::AppResult;
use crate::github;
use crate::internal;
use crate::pull_requests::{in_reach, key};
use fctl_domain::follow_ups::{self as fu, Item, Place, Pull};
use fctl_domain::reviews as rv;
use fctl_proto::Millis;
use fctl_proto::event::FollowUpEvent;
use fctl_proto::session::{FollowUps, GithubAccess, ProjectSettings, RepoRef};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use worker::D1Database;

/// The GitHub App's bot, as pull requests name their author: `fctl[bot]`.
fn app_login(cfg: &Config) -> Option<String> {
    cfg.github.as_ref().map(|g| format!("{}[bot]", g.slug))
}

/// A session that lists a pull request, with what its project says of
/// follow-ups.
#[derive(Deserialize)]
struct Lister {
    session_id: String,
    branch: Option<String>,
    github_access: Option<String>,
    project_github_access: Option<String>,
    settings: String,
    archived_at: Option<f64>,
    workspace_id: Option<String>,
    project_slug: String,
}

/// The session whose agent opened a pull request, and what its project's
/// agents follow up on.
pub struct Opener {
    pub session_id: String,
    pub follow_ups: FollowUps,
    pub workspace_id: Option<String>,
    pub project_slug: String,
    /// Why its agent hears of nothing that happens there: its project is
    /// archived, or its agent only reads on GitHub now.
    pub deaf: Option<&'static str>,
}

/// The session whose agent opened pull request `number` of `repo` (from
/// branch `head_ref`, when said): of the sessions that list it, review
/// agents' left out, the one on its branch, else the one that started first
/// (`fctl_domain::reviews::opener`). Only sessions it may be looked up for,
/// and with `answered` (GitHub's ids of the account and installation that
/// said what happened), those of workspaces connected to that account
/// (`crate::pull_requests::in_reach`).
pub async fn opener(
    db: &D1Database,
    repo: &RepoRef,
    number: u64,
    head_ref: Option<&str>,
    answered: Option<(u64, u64)>,
) -> AppResult<Option<Opener>> {
    let reach = match answered {
        Some(_) => in_reach(Some(("?3", "?4"))),
        None => in_reach(None),
    };
    let mut params = vec![s(&key(repo)), n(number as i64)];
    if let Some((account, installation)) = answered {
        params.extend([n(account as i64), n(installation as i64)]);
    }
    let listers: Vec<Lister> = q(
        db,
        &format!(
            "SELECT l.session_id AS session_id, s.branch AS branch, s.github_access AS github_access, \
             json_extract(p.settings, '$.github_access') AS project_github_access, p.settings AS settings, \
             p.archived_at AS archived_at, p.workspace_id AS workspace_id, p.slug AS project_slug \
             FROM pull_request_sessions l JOIN sessions s ON s.id = l.session_id JOIN projects p ON p.id = s.project_id \
             WHERE l.repo = ?1 AND l.number = ?2 \
             AND NOT EXISTS (SELECT 1 FROM reviews rv WHERE rv.session_id = l.session_id) AND {reach} \
             ORDER BY s.created_at, s.id LIMIT 20"
        ),
        &params,
    )?
    .all()
    .await?;
    let branches: Vec<Option<&str>> = listers.iter().map(|l| l.branch.as_deref()).collect();
    let Some(l) = rv::opener(&branches, head_ref).and_then(|i| listers.get(i)) else {
        return Ok(None);
    };
    let settings: ProjectSettings = serde_json::from_str(&l.settings).unwrap_or_default();
    let known = |v: &Option<String>| {
        v.as_deref()
            .map(|v| GithubAccess::parse(v).unwrap_or(GithubAccess::Read))
    };
    let access =
        fctl_domain::limits::effective(known(&l.github_access), known(&l.project_github_access));
    let deaf = if l.archived_at.is_some() {
        Some("its project is archived")
    } else if access == GithubAccess::Read {
        Some("its agent only reads on GitHub")
    } else {
        None
    };
    Ok(Some(Opener {
        session_id: l.session_id.clone(),
        follow_ups: settings.follow_ups,
        workspace_id: l.workspace_id.clone(),
        project_slug: l.project_slug.clone(),
        deaf,
    }))
}

/// Whether a project that follows up on `follow_ups` takes `event`.
fn takes(follow_ups: &FollowUps, event: &FollowUpEvent) -> bool {
    match event {
        FollowUpEvent::Comment(_) => follow_ups.comments,
        FollowUpEvent::ChecksFailed { .. } => follow_ups.checks,
        FollowUpEvent::Merged { .. } | FollowUpEvent::Closed { .. } => follow_ups.closed,
        FollowUpEvent::Other => false,
    }
}

// ───────────────────────────── comments ─────────────────────────────

/// Passes on a comment that a webhook delivery of `event` brings
/// (`issue_comment`, `pull_request_review`, `pull_request_review_comment`):
/// a person who can write to the repository addressed the agent, on a pull
/// request fctl's app opened, whose project follows up on comments. A
/// review brings its comments on lines. Once the session has it, an 👀
/// reaction on the comment says so. Returns how many sessions got one.
pub async fn on_comment(ctx: &Ctx, event: &str, payload: &Value) -> AppResult<usize> {
    let Some(c) = fu::commented(event, payload) else {
        return Ok(0);
    };
    let pr = c.pull.reference();
    let who = c.comment.login.clone();
    if let Some(why) = fu::passed_over(&c, app_login(&ctx.cfg).as_deref()) {
        // Most comments are none of fctl's business: only those that
        // address the agent are worth a line.
        if fu::addressed(&c.comment.body) {
            worker::console_log!("not passing on @{who}'s comment on {pr}: {why}");
        }
        return Ok(0);
    }
    let Some(inst) = c.installation else {
        return Ok(0);
    };
    let answered = c.owner_id.map(|account| (account, inst));
    let Some(opener) = opener(
        &ctx.db,
        &c.pull.repo,
        c.pull.number,
        c.head_ref.as_deref(),
        answered,
    )
    .await?
    else {
        worker::console_log!("not passing on @{who}'s comment on {pr}: no session lists it");
        return Ok(0);
    };
    let id = opener.session_id.as_str();
    if !opener.follow_ups.comments {
        worker::console_log!(
            "not passing on @{who}'s comment on {pr} to session {id}: its project doesn't follow up on comments"
        );
        return Ok(0);
    }
    if let Some(why) = opener.deaf {
        worker::console_log!("not passing on @{who}'s comment on {pr} to session {id}: {why}");
        return Ok(0);
    }
    // GitHub says who may write: a member of the repository's organization
    // may only read it.
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    match gh.permission(inst, &c.pull.repo, &who).await? {
        Ok(answer) if fu::can_write(&answer) => {}
        Ok(_) => {
            worker::console_log!(
                "not passing on @{who}'s comment on {pr}: they can't write to {}",
                c.pull.repo.full_name()
            );
            return Ok(0);
        }
        Err(why) => {
            worker::console_error!(
                "not passing on @{who}'s comment on {pr}: GitHub didn't say whether they may write ({why})"
            );
            return Ok(0);
        }
    }
    let mut items = vec![Item::new(
        c.pull.clone(),
        FollowUpEvent::Comment(c.comment.clone()),
        None,
    )];
    if c.place() == Place::Review {
        match gh
            .review_comments_of(inst, &c.pull.repo, c.pull.number, c.comment.id)
            .await?
        {
            Ok(listed) => items.extend(
                fu::review_comments(&listed)
                    .into_iter()
                    .map(|lc| Item::new(c.pull.clone(), FollowUpEvent::Comment(lc), None)),
            ),
            Err(why) => worker::console_error!("listing review {}'s comments: {why}", c.comment.id),
        }
    }
    let stub = internal::session_stub(&ctx.env, id)?;
    let taken: internal::FollowUpsTaken =
        internal::post(&stub, "/follow-up", &internal::FollowUps { items }).await?;
    if let Some(why) = taken.not_sent {
        worker::console_log!("not passing on @{who}'s comment on {pr} to session {id}: {why}");
        return Ok(0);
    }
    if taken.taken == 0 {
        return Ok(0);
    }
    ctx.audit_in(
        opener.workspace_id.as_deref(),
        "follow_up.comment",
        Some(id),
        Some(json!({
            "project": opener.project_slug,
            "pull_request": pr,
            "github_user": who,
            "comment": c.comment.url,
        })),
    );
    // 👀 on what they wrote: the agent will hear of it.
    if let Some(node) = c.node_id.clone() {
        let (cfg, repo) = (ctx.cfg.clone(), c.pull.repo.clone());
        ctx.defer(async move {
            let gh = github::from_config(&cfg, now_ms())?;
            if let Err(why) = gh.react_to(inst, &repo, &node, "EYES").await? {
                worker::console_error!("reacting to a comment fctl passed on: {why}");
            }
            Ok(())
        });
    }
    Ok(1)
}

// ───────────────────────────── checks and merges ─────────────────────────────

/// What a delivery says happened to a pull request a session lists, as
/// the lookup after it found it: failed checks of its newest commit, or
/// that it merged or closed (`at`: when, which tells a close from the
/// next).
pub struct News {
    pub pull: Pull,
    /// Its author's GitHub login, and the branch it comes from.
    pub author: Option<String>,
    pub head_ref: Option<String>,
    pub events: Vec<(FollowUpEvent, Option<Millis>)>,
}

/// Who hears of `news`: for each pull request fctl's app opened, the
/// session that opened it, where its project's follow-ups take what
/// happened. `answered`: GitHub's ids of the account and installation that
/// said it. By session.
pub async fn route(
    cfg: &Config,
    db: &D1Database,
    news: &[News],
    answered: Option<(u64, u64)>,
) -> AppResult<BTreeMap<String, Vec<Item>>> {
    let app = app_login(cfg);
    let mut out: BTreeMap<String, Vec<Item>> = BTreeMap::new();
    for n in news {
        if n.events.is_empty() || !rv::by_app(n.author.as_deref(), app.as_deref()) {
            continue;
        }
        let pr = n.pull.reference();
        let Some(opener) = opener(
            db,
            &n.pull.repo,
            n.pull.number,
            n.head_ref.as_deref(),
            answered,
        )
        .await?
        else {
            continue;
        };
        let id = opener.session_id.clone();
        if let Some(why) = opener.deaf {
            worker::console_log!("not following up on {pr} in session {id}: {why}");
            continue;
        }
        let items: Vec<Item> = n
            .events
            .iter()
            .filter(|(e, _)| takes(&opener.follow_ups, e))
            .map(|(e, at)| Item::new(n.pull.clone(), e.clone(), *at))
            .collect();
        if !items.is_empty() {
            worker::console_log!(
                "following up on {pr} in session {id}: {}",
                items
                    .iter()
                    .map(|i| i.key.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            out.entry(id).or_default().extend(items);
        }
    }
    Ok(out)
}
