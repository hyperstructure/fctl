//! The GitHub accounts workspaces work in (docs/architecture.md#github-accounts):
//! `/api/v1/workspaces/{id}/github`, the GitHub sign-in that connects one
//! (`/api/v1/workspaces/{id}/github/connect`, `/api/v1/github/connect` and
//! `/api/v1/github/authorized`, the GitHub App's callback URL), what the
//! sign-in offers to choose from (`/api/v1/workspaces/{id}/github/offers`),
//! installing the app from a workspace (`/api/v1/github/install`, and the
//! app's setup URL, `/api/v1/github/installed`) and `/api/v1/admin/github`.
//!
//! The instance has one GitHub App; installing it on an account lets the
//! instance mint tokens for that account's repositories, and they can
//! write. A workspace uses an account once one of its admins connects it,
//! signing in to GitHub, which shows what they may give the workspace
//! there (`fctl_domain::github::offer`): anyone can send the setup URL
//! another account's installation, so nobody, instance admins included,
//! connects what GitHub doesn't say is theirs to give. fctl uses the
//! GitHub sign-in once and revokes it; what it showed waits ten minutes
//! for the admin to choose from.

use crate::crypto::{self, Keys};
use crate::ctx::Ctx;
use crate::db::{b, n, os, q, s};
use crate::error::{AppError, AppResult, not_configured, not_found, validation};
use crate::github::{self, Github, InstallationInfo, Viewer};
use crate::http;
use crate::model::workspace_ref;
use crate::workspaces::{self as ws, CONNECTION_SELECT, ConnectionRow, WorkspaceRow};
use fctl_domain::cookies::Cookie;
use fctl_domain::github::{Offer, Refusal, Standing};
use fctl_domain::repos::{self, Scope};
use fctl_proto::api::*;
use fctl_proto::session::WorkspaceRole;
use fctl_proto::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use worker::{Request, Response};

/// How long the state of an installation started from a workspace works.
const STATE_TTL_MS: i64 = 60 * 60 * 1000;

/// How long a link that connects an account, and the GitHub sign-in it
/// starts, work; and how long what the sign-in showed waits for a choice.
const CONNECT_TTL_MS: i64 = 10 * 60 * 1000;

/// The longest signed state a cookie carries: browsers keep 4 KB.
const MAX_STATE: usize = 3500;

/// The most installations one GitHub sign-in offers to choose from.
const MAX_OFFERED: usize = 20;

/// What the GitHub App's setup URL gets back: who installed it for which
/// workspace (signed with the master key).
#[derive(Serialize, Deserialize)]
struct InstallState {
    purpose: String,
    workspace: String,
    user: String,
    exp: i64,
}

const PURPOSE: &str = "github-install";

/// A link that connects an account the app is installed on
/// (`POST /api/v1/workspaces/{id}/github/connect`), signed with the
/// master key.
#[derive(Serialize, Deserialize)]
struct ConnectTicket {
    purpose: String,
    workspace: String,
    user: String,
    /// None: every account GitHub shows them the app on.
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    repos: Option<Vec<String>>,
    exp: i64,
}

const CONNECT: &str = "github-connect";

/// A GitHub sign-in on its way, signed, in its cookie: what it connects to
/// which workspace, for whom, and what checks GitHub's answer.
#[derive(Serialize, Deserialize)]
struct SignIn {
    purpose: String,
    workspace: String,
    user: String,
    /// The account named, or the installation GitHub came back with from
    /// installing the app; neither: every one GitHub shows them.
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    installation: Option<u64>,
    /// Named before the sign-in, connected at once; otherwise the person
    /// chooses from what it shows.
    #[serde(default)]
    repos: Option<Vec<String>>,
    /// Sent to GitHub, which sends it back.
    state: String,
    /// PKCE.
    verifier: String,
    exp: i64,
}

const SIGN_IN: &str = "github-sign-in";

/// The keys that sign what a GitHub sign-in carries, when people can sign
/// in to GitHub here.
fn connect_keys(ctx: &Ctx) -> AppResult<Keys> {
    if !ctx
        .cfg
        .github
        .as_ref()
        .is_some_and(|g| g.client().is_some())
    {
        return Err(not_configured(
            "Connecting GitHub accounts takes signing in to GitHub, for which this instance needs its GitHub App's client ID and secret (GITHUB_CLIENT_ID, GITHUB_CLIENT_SECRET)",
        ));
    }
    ctx.keys()
}

/// Installs the app on another account, or gives it more repositories,
/// from `workspace_id`, and comes back to it (through
/// `/api/v1/github/install`, which keeps where the browser came from).
fn install_url(ctx: &Ctx, keys: &Keys, workspace_id: &str) -> AppResult<String> {
    let state = keys.sign(&serde_json::to_vec(&InstallState {
        purpose: PURPOSE.into(),
        workspace: workspace_id.to_owned(),
        user: ctx.user()?.id.clone(),
        exp: ctx.now + STATE_TTL_MS,
    })?);
    let encoded: String = url::form_urlencoded::byte_serialize(state.as_bytes()).collect();
    Ok(ctx
        .cfg
        .url(&format!("/api/v1/github/install?state={encoded}")))
}

/// The installation `signed` started, when it is `user`'s, expired or not.
fn install_state(keys: &Keys, signed: &str, user: &str) -> Option<InstallState> {
    keys.verify(signed)
        .and_then(|b| serde_json::from_slice::<InstallState>(&b).ok())
        .filter(|st| st.purpose == PURPOSE && st.user == user)
}

/// `GET /api/v1/workspaces/{id}/github`.
pub async fn workspace(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    let role = a.require_open()?;
    respond(ctx, &a.row, role).await
}

async fn respond(ctx: &Ctx, row: &WorkspaceRow, role: WorkspaceRole) -> AppResult<Response> {
    let connections = ws::connections(&ctx.db, &row.id).await?;
    let keys = connect_keys(ctx).ok();
    let offers = if role.is_admin() {
        offers(ctx, &row.id).await?
    } else {
        Vec::new()
    };
    http::ok(&WorkspaceGithub {
        configured: ctx.cfg.github.is_some(),
        app_slug: ctx.cfg.github.as_ref().map(|g| g.slug.clone()),
        install_url: match (&ctx.cfg.github, &keys, role.is_admin()) {
            (Some(_), Some(keys), true) => Some(install_url(ctx, keys, &row.id)?),
            _ => None,
        },
        connections: connections
            .iter()
            .map(ConnectionRow::to_connection)
            .collect(),
        approval_required: false,
        can_connect: keys.is_some(),
        offers: offers.iter().map(OfferRow::to_offer).collect(),
    })
}

/// The link that connects `body.account`, or what the caller chooses of
/// every account GitHub shows them, to workspace `id`, after checking that
/// the caller is one of its admins. Whether the app is installed there is
/// for the GitHub sign-in to say, to people who may know.
async fn link(ctx: &Ctx, id: &str, body: &ConnectGithubRequest) -> AppResult<ConnectGithubLink> {
    let a = ws::load(ctx, id).await?;
    a.require_admin()?;
    github::from_config(&ctx.cfg, ctx.now)?;
    let keys = connect_keys(ctx)?;
    let account = body
        .account
        .as_deref()
        .filter(|a| !a.trim().is_empty())
        .map(repos::account)
        .transpose()?;
    let repos = match (&account, body.repos.as_deref()) {
        (Some(account), Some(r)) => Some(repos::scope_names(account, r)?),
        (None, Some(_)) => {
            return Err(validation(
                "account",
                "Name the account the repositories are in",
            ));
        }
        (_, None) => None,
    };
    let user = ctx.user()?.id.clone();
    // What the sign-in's cookie will carry, at its largest.
    let carried = keys.sign(&serde_json::to_vec(&SignIn {
        purpose: SIGN_IN.into(),
        workspace: a.row.id.clone(),
        user: user.clone(),
        account: account.clone(),
        installation: Some(u64::MAX),
        repos: repos.clone(),
        state: crypto::b64url(&[0; 24]),
        verifier: crypto::b64url(&[0; 32]),
        exp: ctx.now,
    })?);
    if carried.len() > MAX_STATE {
        return Err(validation(
            "repos",
            "That's more repository names than a GitHub sign-in can carry: name fewer, or choose them after signing in",
        ));
    }
    let expires_at = ctx.now + CONNECT_TTL_MS;
    let ticket = keys.sign(&serde_json::to_vec(&ConnectTicket {
        purpose: CONNECT.into(),
        workspace: a.row.id.clone(),
        user,
        account,
        repos,
        exp: expires_at,
    })?);
    let encoded: String = url::form_urlencoded::byte_serialize(ticket.as_bytes()).collect();
    Ok(ConnectGithubLink {
        url: ctx
            .cfg
            .url(&format!("/api/v1/github/connect?ticket={encoded}")),
        expires_at,
    })
}

/// `POST /api/v1/workspaces/{id}/github/connect` (admins): the link to the
/// GitHub sign-in that connects an account the app is installed on.
pub async fn connect(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: ConnectGithubRequest = http::body(req).await?;
    http::ok(&link(ctx, id, &body).await?)
}

/// `POST /api/v1/workspaces/{id}/github`, from older CLIs, which connected
/// at once or asked an instance admin: connecting takes a GitHub sign-in
/// now, so the answer is its link, in an error they show.
pub async fn connect_from_older_cli(
    ctx: &mut Ctx,
    req: &mut Request,
    id: &str,
) -> AppResult<Response> {
    let body: ConnectGithubRequest = http::body(req).await?;
    let link = link(ctx, id, &body).await?;
    Err(AppError(ApiError::new(
        ErrorCode::BadRequest,
        format!(
            "Connecting {} takes signing in to GitHub, which shows what you may connect there. Open this link in your browser: {}",
            body.account.as_deref().unwrap_or("an account").trim(),
            link.url
        ),
    )))
}

/// `DELETE /api/v1/workspaces/{id}/github/{account}`: the workspace's
/// admins and instance admins take an account away from a workspace. Its
/// projects' sessions get no more tokens for it.
pub async fn disconnect(ctx: &mut Ctx, id: &str, account: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let a = ws::load(ctx, id).await?;
    let role = a.require_open()?;
    if !role.is_admin() && !user.is_admin() {
        return Err(forbidden_here());
    }
    let removed = q(
        &ctx.db,
        "DELETE FROM workspace_github WHERE workspace_id = ? AND account = ?",
        &[s(a.id()), s(account)],
    )?
    .run()
    .await?;
    if removed == 0 {
        return Err(not_found("GitHub account"));
    }
    ctx.audit_in(
        Some(a.id()),
        "workspace.github_disconnected",
        Some(account),
        None,
    );
    ws::changed(ctx, ws::member_ids(&ctx.db, a.id()).await?);
    respond(ctx, &a.row, role).await
}

fn forbidden_here() -> AppError {
    crate::error::forbidden("Only the workspace's admins can do this")
}

// ───────────────────────────── choosing ─────────────────────────────

/// What a GitHub sign-in showed someone they may connect of an account to
/// a workspace (`github_offers`), while they choose.
#[derive(Deserialize)]
struct OfferRow {
    account: String,
    #[serde(deserialize_with = "crate::db::int")]
    installation_id: i64,
    account_type: Option<String>,
    github_user: String,
    #[serde(deserialize_with = "crate::db::int_bool")]
    all_repos: bool,
    repos: String,
    #[serde(deserialize_with = "crate::db::int")]
    others: i64,
    #[serde(deserialize_with = "crate::db::int")]
    expires_at: i64,
}

const OFFER_SELECT: &str = "SELECT account, installation_id, account_type, github_user, all_repos, repos, \
    others, expires_at FROM github_offers";

impl OfferRow {
    fn offer(&self) -> Offer {
        Offer {
            all: self.all_repos,
            repos: ws::strings(Some(&self.repos)),
            others: usize::try_from(self.others).unwrap_or_default(),
        }
    }

    fn to_offer(&self) -> GithubOffer {
        let offer = self.offer();
        GithubOffer {
            account: self.account.clone(),
            account_type: self.account_type.clone(),
            github_user: self.github_user.clone(),
            all: offer.all,
            repos: offer.repos,
            others: u32::try_from(self.others).unwrap_or_default(),
            expires_at: self.expires_at,
        }
    }
}

/// The caller's offers in workspace `workspace_id`, while they last.
async fn offers(ctx: &Ctx, workspace_id: &str) -> AppResult<Vec<OfferRow>> {
    let user = ctx.user()?;
    q(
        &ctx.db,
        &format!(
            "{OFFER_SELECT} WHERE workspace_id = ? AND user_id = ? AND expires_at > ? ORDER BY created_at, account"
        ),
        &[s(workspace_id), s(&user.id), n(ctx.now)],
    )?
    .all()
    .await
}

/// Keeps what `offer` lets the caller connect of `info`'s account to
/// `row`, for them to choose from.
async fn keep_offer(
    ctx: &Ctx,
    row: &WorkspaceRow,
    info: &InstallationInfo,
    github_user: &str,
    offer: &Offer,
) -> AppResult<()> {
    let user = ctx.user()?;
    q(
        &ctx.db,
        "INSERT INTO github_offers (workspace_id, user_id, account, installation_id, account_type, github_user, \
         all_repos, repos, others, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (workspace_id, user_id, account) DO UPDATE SET account = excluded.account, \
         installation_id = excluded.installation_id, account_type = excluded.account_type, \
         github_user = excluded.github_user, all_repos = excluded.all_repos, repos = excluded.repos, \
         others = excluded.others, created_at = excluded.created_at, expires_at = excluded.expires_at",
        &[
            s(&row.id),
            s(&user.id),
            s(&info.account),
            n(info.id as i64),
            os(info.account_type.as_deref()),
            s(github_user),
            b(offer.all),
            s(&serde_json::to_string(&offer.repos)?),
            n(offer.others as i64),
            n(ctx.now),
            n(ctx.now + CONNECT_TTL_MS),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// `POST /api/v1/workspaces/{id}/github/offers/{account}` (admins): connects
/// what the caller chose of what their GitHub sign-in offered them there.
pub async fn choose(
    ctx: &mut Ctx,
    req: &mut Request,
    id: &str,
    account: &str,
) -> AppResult<Response> {
    let body: ChooseGithubRepos = http::body(req).await?;
    let a = ws::load(ctx, id).await?;
    let role = a.require_admin()?;
    let user = ctx.user()?.id.clone();
    let offer: Option<OfferRow> = q(
        &ctx.db,
        &format!(
            "{OFFER_SELECT} WHERE workspace_id = ? AND user_id = ? AND account = ? AND expires_at > ?"
        ),
        &[s(a.id()), s(&user), s(account), n(ctx.now)],
    )?
    .first()
    .await?;
    let Some(offer) = offer else {
        return Err(AppError(ApiError::new(
            ErrorCode::NotFound,
            format!(
                "What GitHub showed you of {account} waits 10 minutes for your choice, and that ran out. Connect it again."
            ),
        )));
    };
    let scope = fctl_domain::github::choose(&offer.offer(), body.repos.as_deref())?;
    connect_account(
        ctx,
        &a.row,
        &offer.account,
        offer.installation_id,
        &scope,
        &offer.github_user,
    )
    .await?;
    respond(ctx, &a.row, role).await
}

/// `DELETE /api/v1/workspaces/{id}/github/offers/{account}`: the caller
/// connects nothing of what their GitHub sign-in offered them there.
pub async fn dismiss(ctx: &mut Ctx, id: &str, account: &str) -> AppResult<Response> {
    let a = ws::load(ctx, id).await?;
    let role = a.require_open()?;
    let user = ctx.user()?.id.clone();
    q(
        &ctx.db,
        "DELETE FROM github_offers WHERE workspace_id = ? AND user_id = ? AND account = ?",
        &[s(a.id()), s(&user), s(account)],
    )?
    .run()
    .await?;
    respond(ctx, &a.row, role).await
}

// ───────────────────────────── signing in to GitHub ─────────────────────────────

/// Sends people who aren't signed in to fctl to sign in first, and back here.
fn sign_in_first(req: &Request) -> AppResult<Response> {
    let here = req.url()?;
    let next: String = url::form_urlencoded::byte_serialize(
        format!("{}?{}", here.path(), here.query().unwrap_or_default()).as_bytes(),
    )
    .collect();
    http::redirect(&format!("/login?next={next}"))
}

/// The workspace's GitHub tab, saying how connecting `account` went.
fn back(
    row: &WorkspaceRow,
    outcome: &str,
    account: Option<&str>,
    reason: Option<Refusal>,
) -> String {
    let mut url = format!("/w/{}?tab=github&github={outcome}", row.slug);
    if let Some(a) = account {
        let a: String = url::form_urlencoded::byte_serialize(a.as_bytes()).collect();
        url.push_str(&format!("&account={a}"));
    }
    if let Some(r) = reason {
        url.push_str(&format!("&reason={}", r.as_str()));
    }
    url
}

fn redirect_uri(ctx: &Ctx) -> String {
    ctx.cfg.url("/api/v1/github/authorized")
}

/// Goes on to GitHub to sign in, with what the sign-in connects in its
/// cookie.
fn begin(ctx: &mut Ctx, keys: &Keys, mut sign_in: SignIn) -> AppResult<Response> {
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    let (verifier, challenge) = crypto::pkce_pair();
    sign_in.purpose = SIGN_IN.into();
    sign_in.state = crypto::b64url(&fctl_domain::random_bytes::<24>());
    sign_in.verifier = verifier;
    sign_in.exp = ctx.now + CONNECT_TTL_MS;
    let signed = keys.sign(&serde_json::to_vec(&sign_in)?);
    let url = gh.authorize_url(&redirect_uri(ctx), &sign_in.state, &challenge)?;
    ctx.cookies.push(http::set_cookie(
        Cookie::GithubConnect,
        &signed,
        (CONNECT_TTL_MS / 1000) as u64,
        ctx.cfg.secure_cookies(),
    ));
    http::redirect(&url)
}

/// `GET /api/v1/github/connect?ticket=…`: a link that connects an account
/// goes on to GitHub, in the browser of the admin it is for.
pub async fn connect_link(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let Ok(keys) = connect_keys(ctx) else {
        return http::redirect("/");
    };
    let ticket = http::query(req, "ticket")
        .and_then(|t| keys.verify(&t))
        .and_then(|b| serde_json::from_slice::<ConnectTicket>(&b).ok())
        .filter(|t| t.purpose == CONNECT);
    let Some(ticket) = ticket else {
        return http::redirect("/");
    };
    let Ok(a) = ws::load(ctx, &ticket.workspace).await else {
        return http::redirect("/");
    };
    let account = ticket.account.as_deref();
    if ticket.exp <= ctx.now {
        return http::redirect(&back(&a.row, "expired", account, None));
    }
    if ticket.user != user.id {
        return http::redirect(&back(&a.row, "other_user", account, None));
    }
    if a.require_admin().is_err() {
        return http::redirect(&back(&a.row, "failed", account, None));
    }
    begin(
        ctx,
        &keys,
        SignIn {
            purpose: SIGN_IN.into(),
            workspace: a.row.id.clone(),
            user: user.id,
            account: ticket.account,
            installation: None,
            repos: ticket.repos,
            state: String::new(),
            verifier: String::new(),
            exp: 0,
        },
    )
}

/// `GET /api/v1/github/install?state=…`: on to GitHub to install the app
/// from a workspace, or to give an installation more repositories. GitHub
/// sends the state back to the setup URL after installing the app, but not
/// after a change to an installation, so the browser keeps it too, for the
/// hour it works.
pub async fn install(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let (Some(slug), Ok(keys)) = (
        ctx.cfg.github.as_ref().map(|g| g.slug.clone()),
        connect_keys(ctx),
    ) else {
        return http::redirect("/");
    };
    let Some((signed, state)) = http::query(req, "state")
        .and_then(|signed| install_state(&keys, &signed, &user.id).map(|st| (signed, st)))
    else {
        return http::redirect("/");
    };
    if state.exp <= ctx.now {
        return match ws::load(ctx, &state.workspace).await {
            Ok(a) => http::redirect(&back(&a.row, "expired", None, None)),
            Err(_) => http::redirect("/"),
        };
    }
    ctx.cookies.push(http::set_cookie(
        Cookie::GithubInstall,
        &signed,
        ((state.exp - ctx.now) / 1000) as u64,
        ctx.cfg.secure_cookies(),
    ));
    let encoded: String = url::form_urlencoded::byte_serialize(signed.as_bytes()).collect();
    http::redirect(&format!(
        "https://github.com/apps/{slug}/installations/new?state={encoded}"
    ))
}

/// `GET /api/v1/github/installed`: the GitHub App's setup URL, where GitHub
/// sends people after they install it or change an installation. Started
/// from a workspace, it goes on to the GitHub sign-in that shows what they
/// may connect of the installation, to choose from back in the workspace.
pub async fn installed(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let secure = ctx.cfg.secure_cookies();
    // GitHub sends the state back after installing the app; after a change
    // to an installation, only the browser has it.
    let kept = http::cookie(req, Cookie::GithubInstall, secure);
    if kept.is_some() {
        ctx.cookies
            .push(http::set_cookie(Cookie::GithubInstall, "", 0, secure));
    }
    let keys = connect_keys(ctx).ok();
    let state = keys.as_ref().and_then(|keys| {
        [http::query(req, "state"), kept]
            .into_iter()
            .flatten()
            .filter_map(|signed| install_state(keys, &signed, &user.id))
            .find(|st| st.exp > ctx.now)
    });
    let installation = http::query(req, "installation_id").and_then(|i| i.parse::<u64>().ok());
    let (Some(state), Some(keys)) = (state, keys) else {
        return installed_elsewhere(ctx, &user.id, installation).await;
    };
    let Ok(a) = ws::load(ctx, &state.workspace).await else {
        return http::redirect("/");
    };
    if http::query(req, "setup_action").as_deref() == Some("request") {
        // An organization's member asked its owners to install the app.
        return http::redirect(&back(&a.row, "asked_owner", None, None));
    }
    let (Some(installation), Ok(_)) = (installation, a.require_admin()) else {
        return http::redirect(&back(&a.row, "failed", None, None));
    };
    begin(
        ctx,
        &keys,
        SignIn {
            purpose: SIGN_IN.into(),
            workspace: a.row.id.clone(),
            user: user.id,
            account: None,
            installation: Some(installation),
            repos: None,
            state: String::new(),
            verifier: String::new(),
            exp: 0,
        },
    )
}

/// Installed or changed from GitHub rather than from a workspace (or its
/// state ran out): to the GitHub tab of the workspace the person is an
/// admin of that works in the installation's account, or of the only one
/// they are an admin of, else home. They sign in to GitHub from there to
/// choose what it works in. Which account stays unsaid: anyone can send
/// this any installation's id, and it finds only their own workspaces.
async fn installed_elsewhere(
    ctx: &Ctx,
    user_id: &str,
    installation: Option<u64>,
) -> AppResult<Response> {
    #[derive(Deserialize)]
    struct Using {
        workspace_id: String,
    }
    let mine = ws::mine(&ctx.db, user_id).await?;
    let admin_of: Vec<&WorkspaceRow> = mine
        .iter()
        .filter(|w| w.role().is_some_and(WorkspaceRole::is_admin))
        .collect();
    let using: Vec<Using> = match installation {
        Some(id) => {
            q(
                &ctx.db,
                "SELECT DISTINCT workspace_id FROM workspace_github WHERE installation_id = ?",
                &[n(id as i64)],
            )?
            .all()
            .await?
        }
        None => Vec::new(),
    };
    let theirs: Vec<&WorkspaceRow> = admin_of
        .iter()
        .copied()
        .filter(|w| using.iter().any(|u| u.workspace_id == w.id))
        .collect();
    let ((&[only], _) | (&[], &[only])) = (theirs.as_slice(), admin_of.as_slice()) else {
        return http::redirect("/");
    };
    http::redirect(&back(only, "installed", None, None))
}

/// `GET /api/v1/github/authorized`: the GitHub App's callback URL, where
/// GitHub sends people back from signing in. What GitHub shows they may
/// connect waits for them to choose in the workspace, or the repositories
/// they named are connected, and the browser goes back to it.
pub async fn authorized(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    let Some(user) = ctx.auth.as_ref().map(|a| a.user.clone()) else {
        return sign_in_first(req);
    };
    let secure = ctx.cfg.secure_cookies();
    let cookie = http::cookie(req, Cookie::GithubConnect, secure);
    ctx.cookies
        .push(http::set_cookie(Cookie::GithubConnect, "", 0, secure));
    let sign_in = cookie
        .zip(ctx.keys().ok())
        .and_then(|(c, keys)| keys.verify(&c))
        .and_then(|b| serde_json::from_slice::<SignIn>(&b).ok())
        .filter(|st| st.purpose == SIGN_IN && st.user == user.id);
    let Some(sign_in) = sign_in else {
        return http::redirect("/");
    };
    let Ok(a) = ws::load(ctx, &sign_in.workspace).await else {
        return http::redirect("/");
    };
    let account = sign_in.account.as_deref();
    if sign_in.exp <= ctx.now {
        return http::redirect(&back(&a.row, "expired", account, None));
    }
    if http::query(req, "state").as_deref() != Some(sign_in.state.as_str()) {
        return http::redirect(&back(&a.row, "failed", account, None));
    }
    if let Some(error) = http::query(req, "error") {
        let outcome = if error == "access_denied" {
            "cancelled"
        } else {
            "failed"
        };
        return http::redirect(&back(&a.row, outcome, account, None));
    }
    let (Some(code), Ok(_)) = (http::query(req, "code"), a.require_admin()) else {
        return http::redirect(&back(&a.row, "failed", account, None));
    };
    let gh = github::from_config(&ctx.cfg, ctx.now)?;
    let Some(token) = gh
        .exchange(&code, &redirect_uri(ctx), &sign_in.verifier)
        .await?
    else {
        return http::redirect(&back(&a.row, "failed", account, None));
    };
    let outcome = match sign_in.repos.as_deref() {
        Some(names) => check_and_connect(ctx, &gh, &token, &a.row, &sign_in, names).await,
        None => check_and_offer(ctx, &gh, &token, &a.row, &sign_in).await,
    };
    let (cfg, now) = (ctx.cfg.clone(), ctx.now);
    ctx.defer(async move { github::from_config(&cfg, now)?.revoke(&token).await });
    let url = match outcome {
        Ok(Outcome::Connected { account }) => back(&a.row, "connected", Some(&account), None),
        Ok(Outcome::Offered { account }) => back(&a.row, "choose", account.as_deref(), None),
        Ok(Outcome::Refused { account, reason }) => {
            back(&a.row, "refused", account.as_deref(), Some(reason))
        }
        Err(e) => {
            worker::console_error!("connecting a GitHub account: {}", e.0.message);
            back(&a.row, "failed", account, None)
        }
    };
    http::redirect(&url)
}

/// How a GitHub sign-in went.
enum Outcome {
    Connected {
        account: String,
    },
    /// What they may connect waits for their choice: `account`, when there
    /// is one.
    Offered {
        account: Option<String>,
    },
    /// `account`: when they named it, or GitHub showed it to them.
    Refused {
        account: Option<String>,
        reason: Refusal,
    },
}

/// The installations `sign_in` is for, of those GitHub shows the person:
/// the one it names, or every one when it names none. Never one by its id
/// alone, so that the answer says nothing about where else the app is
/// installed.
fn installations_for(theirs: Vec<InstallationInfo>, sign_in: &SignIn) -> Vec<InstallationInfo> {
    theirs
        .into_iter()
        .filter(|i| match (sign_in.installation, &sign_in.account) {
            (Some(id), _) => i.id == id,
            (None, Some(account)) => i.account.eq_ignore_ascii_case(account),
            (None, None) => true,
        })
        .take(MAX_OFFERED)
        .collect()
}

/// What GitHub says, with the person's sign-in, about them and the
/// installation `info`. `named`: they named repositories, so how many it
/// reaches doesn't matter.
async fn standing(
    gh: &Github<'_>,
    token: &str,
    info: &InstallationInfo,
    viewer: &Viewer,
    named: bool,
) -> AppResult<Standing> {
    let own = info.account_type.as_deref() == Some("User")
        && match info.account_id {
            Some(id) => id == viewer.id,
            None => info.account.eq_ignore_ascii_case(&viewer.login),
        };
    let reaches_all = info.repository_selection.as_deref() == Some("all");
    Ok(Standing {
        own,
        reaches_all,
        // Only every repository of someone else's account asks how many.
        reached: if !named && !own && reaches_all {
            gh.reached(info.id).await?
        } else {
            0
        },
        seen: gh.seen(token, info.id).await?,
    })
}

/// Connects the repositories the person named before signing in, as far
/// as GitHub says they may.
async fn check_and_connect(
    ctx: &Ctx,
    gh: &Github<'_>,
    token: &str,
    row: &WorkspaceRow,
    sign_in: &SignIn,
    names: &[String],
) -> AppResult<Outcome> {
    let found = installations_for(gh.user_installations(token).await?, sign_in);
    let Some(info) = found.into_iter().next() else {
        return Ok(Outcome::Refused {
            account: sign_in.account.clone(),
            reason: Refusal::NoInstallation,
        });
    };
    let viewer = gh.viewer(token).await?;
    let standing = standing(gh, token, &info, &viewer, true).await?;
    match fctl_domain::github::grant(&standing, names) {
        Ok(scope) => {
            connect_account(
                ctx,
                row,
                &info.account,
                info.id as i64,
                &scope,
                &viewer.login,
            )
            .await?;
            Ok(Outcome::Connected {
                account: info.account,
            })
        }
        Err((reason, names)) => {
            refused(ctx, row, Some(&info.account), reason, &names, &viewer.login);
            Ok(Outcome::Refused {
                account: Some(info.account),
                reason,
            })
        }
    }
}

/// Keeps what GitHub says the person may connect of the installation
/// `sign_in` names, or of every one GitHub shows them when it names none,
/// for them to choose from.
async fn check_and_offer(
    ctx: &Ctx,
    gh: &Github<'_>,
    token: &str,
    row: &WorkspaceRow,
    sign_in: &SignIn,
) -> AppResult<Outcome> {
    let named = sign_in.installation.is_some() || sign_in.account.is_some();
    let found = installations_for(gh.user_installations(token).await?, sign_in);
    if found.is_empty() {
        return Ok(Outcome::Refused {
            account: sign_in.account.clone(),
            reason: Refusal::NoInstallation,
        });
    }
    let viewer = gh.viewer(token).await?;
    let standings = futures::future::try_join_all(
        found
            .iter()
            .map(|info| standing(gh, token, info, &viewer, false)),
    )
    .await?;
    let mut offered: Vec<&str> = Vec::new();
    for (info, standing) in found.iter().zip(&standings) {
        if let Ok(offer) = fctl_domain::github::offer(standing) {
            keep_offer(ctx, row, info, &viewer.login, &offer).await?;
            offered.push(&info.account);
        }
    }
    match offered.as_slice() {
        [] => {
            let account = found.first().filter(|_| named).map(|i| i.account.clone());
            refused(
                ctx,
                row,
                account.as_deref(),
                Refusal::NoAdmin,
                &[],
                &viewer.login,
            );
            Ok(Outcome::Refused {
                account,
                reason: Refusal::NoAdmin,
            })
        }
        [one] => Ok(Outcome::Offered {
            account: Some((*one).to_owned()),
        }),
        _ => Ok(Outcome::Offered { account: None }),
    }
}

/// Records in the workspace's audit log that GitHub let the person connect
/// nothing of `account`, and why.
fn refused(
    ctx: &Ctx,
    row: &WorkspaceRow,
    account: Option<&str>,
    reason: Refusal,
    names: &[String],
    github_user: &str,
) {
    ctx.audit_in(
        Some(&row.id),
        "workspace.github_refused",
        account,
        Some(serde_json::json!({
            "reason": reason.as_str(),
            "repos": names,
            "github_user": github_user,
        })),
    );
}

/// Connects `account` (installation `installation_id`) to a workspace
/// with `scope`, as the caller, who signed in to GitHub as `github_user`.
/// What their sign-in offered of it is done with.
async fn connect_account(
    ctx: &Ctx,
    row: &WorkspaceRow,
    account: &str,
    installation_id: i64,
    scope: &Scope,
    github_user: &str,
) -> AppResult<()> {
    let user = ctx.user()?.clone();
    let repos = match scope {
        Scope::All => None,
        Scope::Repos(names) => Some(names.clone()),
    };
    let repos_json = repos.as_ref().map(serde_json::to_string).transpose()?;
    crate::db::batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "INSERT INTO workspace_github (workspace_id, account, installation_id, repos, status, requested_by, approved_by, github_user, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, 'active', ?, NULL, ?, ?, ?) ON CONFLICT (workspace_id, account) DO UPDATE SET account = excluded.account, \
                 installation_id = excluded.installation_id, repos = excluded.repos, status = 'active', \
                 requested_by = excluded.requested_by, approved_by = NULL, github_user = excluded.github_user, \
                 updated_at = excluded.updated_at",
                &[
                    s(&row.id),
                    s(account),
                    n(installation_id),
                    os(repos_json.as_deref()),
                    s(&user.id),
                    s(github_user),
                    n(ctx.now),
                    n(ctx.now),
                ],
            )?,
            q(
                &ctx.db,
                "DELETE FROM github_offers WHERE workspace_id = ? AND user_id = ? AND account = ?",
                &[s(&row.id), s(&user.id), s(account)],
            )?,
        ],
    )
    .await?;
    ctx.audit_in(
        Some(&row.id),
        "workspace.github_connected",
        Some(account),
        Some(serde_json::json!({
            "account": account,
            "repos": repos,
            "github_user": github_user,
        })),
    );
    ws::changed(ctx, ws::member_ids(&ctx.db, &row.id).await?);
    Ok(())
}

// ───────────────────────────── instance admins ─────────────────────────────

#[derive(Deserialize)]
struct AdminConnectionRow {
    #[serde(flatten)]
    connection: ConnectionRow,
    slug: String,
    name: String,
    color: Option<String>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    avatar_updated_at: Option<i64>,
}

/// `GET /api/v1/admin/github`: every account the app is installed on, and
/// the workspaces that use it.
pub async fn admin_list(ctx: &Ctx) -> AppResult<Response> {
    ctx.admin()?;
    let rows: Vec<AdminConnectionRow> = q(
        &ctx.db,
        &format!(
            "SELECT * FROM ({CONNECTION_SELECT}) c JOIN (SELECT id AS wid, slug, name, color, avatar_updated_at FROM workspaces) w \
             ON w.wid = c.workspace_id ORDER BY c.created_at"
        ),
        &[],
    )?
    .all()
    .await?;
    let (installations, slug) = match ctx.cfg.github.as_ref() {
        Some(g) => (
            github::from_config(&ctx.cfg, ctx.now)?
                .installation_list()
                .await
                .unwrap_or_default(),
            Some(g.slug.clone()),
        ),
        None => (Vec::new(), None),
    };
    let connection = |r: &AdminConnectionRow| GithubWorkspaceConnection {
        workspace: workspace_ref(
            &r.connection.workspace_id,
            &r.slug,
            &r.name,
            r.color.as_deref(),
            r.avatar_updated_at,
        ),
        repos: r.connection.repos(),
        status: r.connection.status(),
        requested_by: r.connection.requester(),
        github_user: r.connection.github_user.clone(),
        created_at: r.connection.created_at,
    };
    let mut out: Vec<GithubInstallation> = installations
        .iter()
        .map(|i| GithubInstallation {
            account: i.account.clone(),
            account_type: i.account_type.clone(),
            installation_id: Some(i.id),
            repository_selection: i.repository_selection.clone(),
            installed_at: i.created_at,
            workspaces: rows
                .iter()
                .filter(|r| r.connection.account.eq_ignore_ascii_case(&i.account))
                .map(connection)
                .collect(),
        })
        .collect();
    // Accounts workspaces use where the app isn't installed (any more).
    for r in &rows {
        if let Some(entry) = out
            .iter_mut()
            .find(|i| i.account.eq_ignore_ascii_case(&r.connection.account))
        {
            if entry.installation_id.is_none() {
                entry.workspaces.push(connection(r));
            }
            continue;
        }
        out.push(GithubInstallation {
            account: r.connection.account.clone(),
            account_type: None,
            installation_id: None,
            repository_selection: None,
            installed_at: None,
            workspaces: vec![connection(r)],
        });
    }
    out.sort_by_key(|i| i.account.to_lowercase());
    http::ok(&GithubInstallations {
        configured: slug.is_some(),
        install_url: None,
        app_slug: slug,
        installations: out,
    })
}

/// `DELETE /api/v1/admin/github/{account}/workspaces/{workspace}`: the
/// workspace no longer uses the account. Only its admins connect it again.
pub async fn admin_disconnect(
    ctx: &mut Ctx,
    account: &str,
    workspace: &str,
) -> AppResult<Response> {
    let admin = ctx.admin()?.clone();
    let row = ws::find(&ctx.db, &admin.id, workspace)
        .await?
        .ok_or_else(|| not_found("Workspace"))?;
    let removed = q(
        &ctx.db,
        "DELETE FROM workspace_github WHERE workspace_id = ? AND account = ?",
        &[s(&row.id), s(account)],
    )?
    .run()
    .await?;
    if removed == 0 {
        return Err(not_found("GitHub account"));
    }
    ctx.audit(
        "github.disconnected",
        Some(account),
        Some(serde_json::json!({ "workspace": row.id })),
    );
    ctx.audit_in(
        Some(&row.id),
        "workspace.github_disconnected",
        Some(account),
        None,
    );
    ws::changed(ctx, ws::member_ids(&ctx.db, &row.id).await?);
    admin_list(ctx).await
}
