//! The GitHub App review agents post their reviews as, where the instance
//! has one (docs/architecture.md#review-agents): where it is among a
//! workspace's GitHub accounts (`/api/v1/workspaces/{id}/github/review-app`),
//! installing it on one of them from the workspace
//! (`/api/v1/github-review/install`), and its setup URL, where GitHub sends
//! people back (`/api/v1/github-review/installed`).
//!
//! Nobody connects it: fctl uses it only on the repositories a workspace
//! connected through the GitHub App, and where it's installed on the
//! account and reaches the repository (`crate::reviews`). So installing it
//! takes no GitHub sign-in, and its setup URL changes nothing: it brings
//! the browser back to the workspace it was installed from.

use crate::api::github::{back, sign_in_first};
use crate::config::GithubConfig;
use crate::crypto::Keys;
use crate::ctx::Ctx;
use crate::db::{q, s};
use crate::error::{AppError, AppResult};
use crate::github::{self, Github};
use crate::http;
use crate::model::PROJECT_VISIBLE;
use crate::workspaces::{self as ws, ConnectionRow, WorkspaceRow};
use fctl_domain::cookies::Cookie;
use fctl_domain::review_app;
use fctl_proto::api::{GithubAppRef, ReviewAppAccount, ReviewAppState, WorkspaceReviewApp};
use fctl_proto::session::WorkspaceRole;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use worker::{Request, Response};

/// How long a link that installs the review app from a workspace works,
/// and the cookie that keeps where it came from.
const STATE_TTL_MS: i64 = 60 * 60 * 1000;

/// What installing the review app from a workspace carries to GitHub and
/// back, signed with the master key: on which account, for which
/// workspace, by whom.
#[derive(Serialize, Deserialize)]
struct InstallState {
    purpose: String,
    workspace: String,
    user: String,
    /// The account's login, as the workspace connected it.
    account: String,
    exp: i64,
}

const PURPOSE: &str = "github-review-install";

/// What [`PROJECT_VISIBLE`] reads of a project (`p`) for the caller (`?2`).
const SEEN: &str = "JOIN workspaces w ON w.id = p.workspace_id \
    LEFT JOIN workspace_members wm ON wm.workspace_id = p.workspace_id AND wm.user_id = ?2 \
    LEFT JOIN project_members m ON m.project_id = p.id AND m.user_id = ?2";

/// The state `signed` carries, when it is `user`'s, expired or not.
fn install_state(keys: &Keys, signed: &str, user: &str) -> Option<InstallState> {
    keys.verify(signed)
        .and_then(|b| serde_json::from_slice::<InstallState>(&b).ok())
        .filter(|st| st.purpose == PURPOSE && st.user == user)
}

/// An app of the instance's, with its logo.
fn app_ref(cfg: &GithubConfig) -> GithubAppRef {
    GithubAppRef {
        slug: cfg.slug.clone(),
        avatar_url: cfg
            .app_id
            .trim()
            .parse::<u64>()
            .ok()
            .map(|id| format!("https://avatars.githubusercontent.com/in/{id}")),
    }
}

/// `GET /api/v1/workspaces/{id}/github/review-app`: where the review app
/// is among the accounts the workspace works in, with links that install
/// it for the workspace's admins.
pub async fn workspace(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    let role = a.require_open()?;
    let github_app = ctx.cfg.github.as_ref().map(app_ref);
    let (Some(review), Some(review_cfg)) = (
        github::review_app(&ctx.cfg, ctx.now),
        ctx.cfg.github_review.as_ref(),
    ) else {
        return http::ok(&WorkspaceReviewApp {
            configured: false,
            app: None,
            github_app,
            accounts: Vec::new(),
            problem: None,
        });
    };
    let connections: Vec<ConnectionRow> = ws::connections(&ctx.db, a.id())
        .await?
        .into_iter()
        .filter(ConnectionRow::active)
        .collect();
    let writable = writable(ctx, a.id(), &ctx.user()?.id).await?;
    let keys = match role {
        r if r.is_admin() => ctx.keys().ok(),
        _ => None,
    };
    let none = Vec::new();
    let states = futures::future::join_all(connections.iter().map(|c| {
        let repos = writable.get(&c.account.to_lowercase()).unwrap_or(&none);
        standing(&review, c, repos)
    }))
    .await;
    let mut accounts = Vec::with_capacity(connections.len());
    for (c, (state, unreached)) in connections.iter().zip(states) {
        let install_url = match &keys {
            Some(keys) => Some(install_url(ctx, keys, &a.row, &c.account)?),
            None => None,
        };
        accounts.push(ReviewAppAccount {
            account: c.account.clone(),
            state,
            unreached,
            install_url,
        });
    }
    // Where GitHub didn't say, the review app's own key may be what fails:
    // then nothing works as it, and every review posts as the GitHub App.
    let mut problem = None;
    if accounts.iter().any(|a| a.state == ReviewAppState::Unknown) {
        problem = review.key_problem().await.map(|why| {
            let ours = github_app
                .as_ref()
                .map_or("the GitHub App", |g| g.slug.as_str());
            format!(
                "fctl can't sign in to GitHub as {}, so every review posts as {ours}: {why}.",
                review.slug()
            )
        });
    }
    http::ok(&WorkspaceReviewApp {
        configured: true,
        app: Some(app_ref(review_cfg)),
        github_app,
        accounts,
        problem,
    })
}

/// The repositories the workspace's projects may write to, whose pull
/// requests their review agents review, by account (in lowercase): their
/// primary repositories and those they list as read and write, but the
/// archived projects'. Only of the projects `user_id` sees there: those
/// it hides from a member name repositories that member isn't told of.
async fn writable(
    ctx: &Ctx,
    workspace_id: &str,
    user_id: &str,
) -> AppResult<BTreeMap<String, Vec<String>>> {
    #[derive(Deserialize)]
    struct Repo {
        owner: String,
        name: String,
    }
    let rows: Vec<Repo> = q(
        &ctx.db,
        &format!(
            "SELECT p.repo_owner AS owner, p.repo_name AS name FROM projects p {SEEN} \
             WHERE p.workspace_id = ?1 AND p.archived_at IS NULL AND p.repo_owner IS NOT NULL \
             AND p.repo_name IS NOT NULL AND {PROJECT_VISIBLE} \
             UNION SELECT r.owner AS owner, r.name AS name FROM project_repos r JOIN projects p ON p.id = r.project_id {SEEN} \
             WHERE p.workspace_id = ?1 AND p.archived_at IS NULL AND r.access = 'write' AND r.name IS NOT NULL \
             AND {PROJECT_VISIBLE}"
        ),
        &[s(workspace_id), s(user_id)],
    )?
    .all()
    .await?;
    let mut by_account: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in rows {
        by_account
            .entry(r.owner.to_lowercase())
            .or_default()
            .push(r.name);
    }
    Ok(by_account)
}

/// How the review app stands on connection `c`'s account, by what GitHub
/// says of its installation there, and those of `repos` it doesn't reach.
async fn standing(
    review: &Github<'_>,
    c: &ConnectionRow,
    repos: &[String],
) -> (ReviewAppState, Vec<String>) {
    let found = match review.account_installation(&c.account).await {
        Ok(found) => found,
        Err(e) => {
            worker::console_error!(
                "finding {} on {}: {}",
                review.slug(),
                c.account,
                e.0.message
            );
            return (ReviewAppState::Unknown, Vec::new());
        }
    };
    let connected = c.account_id.and_then(|id| u64::try_from(id).ok());
    let installed = found.as_ref().map(|i| review_app::Installed {
        account_id: i.account_id,
        all: i.repository_selection.as_deref() == Some("all"),
    });
    let mut reached = None;
    if let (Some(i), Some(inst)) = (&found, &installed)
        && !inst.all
        && (connected.is_none() || inst.account_id == connected)
    {
        reached = match review.reached_names(i.id).await {
            Ok(names) => names,
            Err(e) => {
                worker::console_error!(
                    "listing what {} reaches on {}: {}",
                    review.slug(),
                    c.account,
                    e.0.message
                );
                return (ReviewAppState::Unknown, Vec::new());
            }
        };
    }
    review_app::state(installed.as_ref(), connected, reached.as_ref(), repos)
}

/// The link that installs the review app on `account` from workspace
/// `row`, or chooses its repositories there, and comes back to it.
fn install_url(ctx: &Ctx, keys: &Keys, row: &WorkspaceRow, account: &str) -> AppResult<String> {
    let state = keys.sign(&serde_json::to_vec(&InstallState {
        purpose: PURPOSE.into(),
        workspace: row.id.clone(),
        user: ctx.user()?.id.clone(),
        account: account.to_owned(),
        exp: ctx.now + STATE_TTL_MS,
    })?);
    let encoded: String = url::form_urlencoded::byte_serialize(state.as_bytes()).collect();
    Ok(ctx
        .cfg
        .url(&format!("/api/v1/github-review/install?state={encoded}")))
}

/// `GET /api/v1/github-review/install?state=…`: on to GitHub, to install
/// the review app on the account the link names, on the repositories the
/// workspace connected there (every one, where it connected every one), or
/// where it's installed already, to its settings there, where the
/// account's admins choose its repositories. GitHub sends the state back
/// to the setup URL after installing the app, but not after a change to an
/// installation, so the browser keeps it too, for the hour it works.
pub async fn install(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let (Some(_), Ok(keys)) = (ctx.cfg.github_review.as_ref(), ctx.keys()) else {
        return http::redirect("/");
    };
    let Some((signed, state)) = http::query(req, "state")
        .and_then(|signed| install_state(&keys, &signed, &user.id).map(|st| (signed, st)))
    else {
        return http::redirect("/");
    };
    let Ok(a) = ws::load(ctx, &state.workspace).await else {
        return http::redirect("/");
    };
    let account = Some(state.account.as_str());
    if state.exp <= ctx.now {
        return http::redirect(&back(&a.row, "review_expired", account, None));
    }
    let connections = ws::connections(&ctx.db, a.id()).await?;
    let connection = connections
        .iter()
        .find(|c| c.active() && c.account.eq_ignore_ascii_case(&state.account));
    let (Some(connection), Ok(_)) = (connection, a.require_admin()) else {
        return http::redirect(&back(&a.row, "review_failed", account, None));
    };
    ctx.cookies.push(http::set_cookie(
        Cookie::GithubReviewInstall,
        &signed,
        ((state.exp - ctx.now) / 1000) as u64,
        ctx.cfg.secure_cookies(),
    ));
    let Some(review) = github::review_app(&ctx.cfg, ctx.now) else {
        return http::redirect("/");
    };
    let connected = connection.account_id.and_then(|id| u64::try_from(id).ok());
    match review.account_installation(&connection.account).await {
        Ok(Some(found)) if connected.is_none() || found.account_id == connected => {
            return http::redirect(&review_app::settings_url(
                &found.account,
                found.account_type.as_deref(),
                found.id,
            ));
        }
        Ok(_) => {}
        Err(e) => worker::console_error!(
            "finding {} on {}: {}",
            review.slug(),
            connection.account,
            e.0.message
        ),
    }
    let repos = match connection.repos() {
        Some(names) => repository_ids(ctx, &connections, connection, &names).await,
        None => Vec::new(),
    };
    http::redirect(&review_app::install_url(
        review.slug(),
        connected,
        &repos,
        &signed,
    ))
}

/// GitHub's ids of repositories `names` of connection `c`'s account, as
/// the GitHub App sees them, for GitHub's installation page to choose
/// them: none when GitHub doesn't say, and the page chooses every one.
async fn repository_ids(
    ctx: &Ctx,
    connections: &[ConnectionRow],
    c: &ConnectionRow,
    names: &[String],
) -> Vec<u64> {
    let ids = async {
        let gh = github::from_config(&ctx.cfg, ctx.now)?;
        let inst = ws::installation(&ctx.db, &gh, connections, &c.account).await?;
        let answer = gh
            .lookup(inst, review_app::repository_ids_query(&c.account, names))
            .await?;
        Ok::<_, AppError>(review_app::repository_ids(&answer, names.len()))
    }
    .await;
    ids.unwrap_or_else(|e| {
        worker::console_error!("looking up {}'s repositories: {}", c.account, e.0.message);
        Vec::new()
    })
}

/// `GET /api/v1/github-review/installed`: the review app's setup URL, where
/// GitHub sends people after they install it or change an installation,
/// back to the workspace's GitHub tab they installed it from, which shows
/// how it stands there now.
pub async fn installed(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let secure = ctx.cfg.secure_cookies();
    // GitHub sends the state back after installing the app; after a change
    // to an installation, only the browser has it.
    let kept = http::cookie(req, Cookie::GithubReviewInstall, secure);
    if kept.is_some() {
        ctx.cookies
            .push(http::set_cookie(Cookie::GithubReviewInstall, "", 0, secure));
    }
    let keys = ctx.keys().ok();
    let state = keys.as_ref().and_then(|keys| {
        [http::query(req, "state"), kept]
            .into_iter()
            .flatten()
            .filter_map(|signed| install_state(keys, &signed, &user.id))
            .find(|st| st.exp > ctx.now)
    });
    let Some(state) = state else {
        let installation = http::query(req, "installation_id").and_then(|i| i.parse::<u64>().ok());
        return installed_elsewhere(ctx, &user.id, installation).await;
    };
    let Ok(a) = ws::load(ctx, &state.workspace).await else {
        return http::redirect("/");
    };
    // An organization's member asked its owners to install it.
    let outcome = match http::query(req, "setup_action").as_deref() {
        Some("request") => "review_asked",
        _ => "review_installed",
    };
    http::redirect(&back(&a.row, outcome, Some(&state.account), None))
}

/// Installed or changed from GitHub rather than from a workspace (or its
/// state ran out): to the GitHub tab of the workspace the person is an
/// admin of that works in the installation's account, or of the only one
/// they are an admin of, else home. Which account stays unsaid: anyone can
/// send this any installation's id, and it finds only their own
/// workspaces.
async fn installed_elsewhere(
    ctx: &Ctx,
    user_id: &str,
    installation: Option<u64>,
) -> AppResult<Response> {
    let mine = ws::mine(&ctx.db, user_id).await?;
    let admin_of: Vec<&WorkspaceRow> = mine
        .iter()
        .filter(|w| w.role().is_some_and(WorkspaceRole::is_admin))
        .collect();
    let found = match (github::review_app(&ctx.cfg, ctx.now), installation) {
        (Some(review), Some(id)) => review.installation_by_id(id).await.ok().flatten(),
        _ => None,
    };
    let mut theirs: Vec<&WorkspaceRow> = Vec::new();
    if let Some(found) = &found {
        for w in &admin_of {
            let uses = ws::connections(&ctx.db, &w.id).await?.iter().any(|c| {
                c.active()
                    && match (c.account_id, found.account_id) {
                        (Some(c), Some(f)) => u64::try_from(c).is_ok_and(|c| c == f),
                        _ => c.account.eq_ignore_ascii_case(&found.account),
                    }
            });
            if uses {
                theirs.push(w);
            }
        }
    }
    let ((&[only], _) | (&[], &[only])) = (theirs.as_slice(), admin_of.as_slice()) else {
        return http::redirect("/");
    };
    http::redirect(&back(only, "review_changed", None, None))
}
