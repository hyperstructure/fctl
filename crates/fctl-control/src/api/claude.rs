//! `/api/v1/me/claude/*`: connecting Claude subscriptions.

use crate::auth;
use crate::ctx::Ctx;
use crate::db::{n, os, q, s};
use crate::error::{AppResult, not_found, unavailable, validation};
use crate::http;
use crate::internal::{self, LoginCode, LoginStart};
use crate::model::{CredentialRow, CredentialSecret, UserRow};
use fctl_domain::verify::Sensitive;
use fctl_proto::LoginJobId;
use fctl_proto::api::*;
use fctl_proto::runner::ClaudeCredentialKind;
use worker::{Request, Response};

pub async fn list(ctx: &Ctx) -> AppResult<Response> {
    let user = ctx.user()?;
    http::ok(&auth::me(ctx, user).await?.claude)
}

/// Stores a `claude setup-token` token.
pub async fn add_token(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: AddClaudeTokenRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    let token = body.token.trim().to_owned();
    if let Some(problem) = fctl_domain::claude::setup_token_problem(&token) {
        return Err(validation("token", problem));
    }
    let label = body
        .label
        .as_deref()
        .map(|l| fctl_domain::validate::name("label", l))
        .transpose()?;
    ctx.verified(Sensitive::ConnectClaude)?;
    let id = fctl_domain::ids::credential_id(ctx.now);
    let secret = CredentialSecret {
        access_token: token,
        refresh_token: None,
        expires_at: None,
    };
    let sealed = ctx
        .keys()?
        .seal_json(&CredentialRow::aad(id.as_str(), &user.id), &secret)?;
    q(
        &ctx.db,
        "INSERT INTO claude_credentials (id, user_id, kind, label, secret_enc, scopes, status, expires_at, created_at, updated_at) \
         VALUES (?, ?, 'setup_token', ?, ?, 'user:inference', 'active', ?, ?, ?)",
        &[
            s(id.as_str()),
            s(&user.id),
            os(label.as_deref()),
            s(&sealed),
            n(ctx.now + 365 * auth::DAY_MS),
            n(ctx.now),
            n(ctx.now),
        ],
    )?
    .run()
    .await?;
    q(
        &ctx.db,
        "UPDATE users SET default_credential_id = ? WHERE id = ?",
        &[s(id.as_str()), s(&user.id)],
    )?
    .run()
    .await?;
    ctx.audit("claude.token_added", Some(id.as_str()), None);
    // Best effort: the token is connected, whatever the email does.
    if ctx.cfg.email.is_some() {
        let instance = crate::instance::name(&ctx.db, &ctx.cfg)
            .await
            .unwrap_or_else(|_| ctx.cfg.instance_name.clone());
        let (subject, text) = connected_notice(&instance, &user.email, &ctx.cfg);
        crate::email::notice(ctx, &user, &subject, text);
    }
    let user = auth::user_by_id(ctx, &user.id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::created(&auth::me(ctx, &user).await?)
}

/// The email that tells someone a Claude subscription was connected to
/// their account, with a token or a sign-in: the sessions they start run
/// on it from now on, and its Claude account sees them, so one they didn't
/// connect is someone else watching their sessions.
pub fn connected_notice(
    instance: &str,
    email: &str,
    cfg: &crate::config::Config,
) -> (String, String) {
    (
        "A Claude subscription was connected to your account".to_owned(),
        format!(
            "A new Claude connection now runs the sessions you start on {instance} ({email}), and its Claude account sees them.\n\nIf you didn't connect it, remove it in Settings now, sign out your other devices there and change your password: {}",
            cfg.url("/settings#claude")
        ),
    )
}

/// Connects the Claude token the caller keeps on their runners (`fctl
/// host claude set-token`), which they never give to fctl: a connection
/// without a secret whose sessions run only on those of their runners that
/// keep one (`spec::claude_credential`, FleetDO's scheduling). One per
/// person: connecting it again makes it their default again, and names it
/// anew with a label.
pub async fn add_on_runners(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: AddClaudeOnRunnersRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    let label = body
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| fctl_domain::validate::name("label", l))
        .transpose()?;
    #[derive(serde::Deserialize)]
    struct Existing {
        id: String,
    }
    let existing: Option<Existing> = q(
        &ctx.db,
        "SELECT id FROM claude_credentials WHERE user_id = ? AND on_runners = 1",
        &[s(&user.id)],
    )?
    .first()
    .await?;
    let (id, created) = match existing {
        Some(e) => {
            if let Some(l) = &label {
                q(
                    &ctx.db,
                    "UPDATE claude_credentials SET label = ?, updated_at = ? WHERE id = ?",
                    &[s(l), n(ctx.now), s(&e.id)],
                )?
                .run()
                .await?;
            }
            (e.id, false)
        }
        None => {
            let id = fctl_domain::ids::credential_id(ctx.now).as_str().to_owned();
            // `INSERT OR IGNORE`: a request alongside this one may have
            // connected it first (the index allows one per person).
            q(
                &ctx.db,
                "INSERT OR IGNORE INTO claude_credentials (id, user_id, kind, label, secret_enc, scopes, status, on_runners, created_at, updated_at) \
                 VALUES (?, ?, 'setup_token', ?, '', 'user:inference', 'active', 1, ?, ?)",
                &[
                    s(&id),
                    s(&user.id),
                    os(label.as_deref()),
                    n(ctx.now),
                    n(ctx.now),
                ],
            )?
            .run()
            .await?;
            let stored: Option<Existing> = q(
                &ctx.db,
                "SELECT id FROM claude_credentials WHERE user_id = ? AND on_runners = 1",
                &[s(&user.id)],
            )?
            .first()
            .await?;
            let stored = stored.ok_or_else(|| not_found("Credential"))?.id;
            let created = stored == id;
            (stored, created)
        }
    };
    q(
        &ctx.db,
        "UPDATE users SET default_credential_id = ? WHERE id = ?",
        &[s(&id), s(&user.id)],
    )?
    .run()
    .await?;
    if created {
        ctx.audit("claude.on_runners_added", Some(&id), None);
    }
    let user = auth::user_by_id(ctx, &user.id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    let me = auth::me(ctx, &user).await?;
    if created {
        http::created(&me)
    } else {
        http::ok(&me)
    }
}

pub async fn remove(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let changed = q(
        &ctx.db,
        "DELETE FROM claude_credentials WHERE id = ? AND user_id = ?",
        &[s(id), s(&user.id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(not_found("Credential"));
    }
    q(
        &ctx.db,
        "UPDATE users SET default_credential_id = NULL WHERE id = ? AND default_credential_id = ?",
        &[s(&user.id), s(id)],
    )?
    .run()
    .await?;
    ctx.audit("claude.removed", Some(id), None);
    let user = auth::user_by_id(ctx, &user.id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::ok(&auth::me(ctx, &user).await?)
}

pub async fn set_default(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let owned: Option<crate::db::Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM claude_credentials WHERE id = ? AND user_id = ?",
        &[s(id), s(&user.id)],
    )?
    .first()
    .await?;
    if owned.map_or(0, |c| c.n) == 0 {
        return Err(not_found("Credential"));
    }
    q(
        &ctx.db,
        "UPDATE users SET default_credential_id = ? WHERE id = ?",
        &[s(id), s(&user.id)],
    )?
    .run()
    .await?;
    let user = auth::user_by_id(ctx, &user.id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::ok(&auth::me(ctx, &user).await?)
}

#[derive(serde::Deserialize)]
pub struct JobRow {
    pub id: String,
    pub state: String,
    #[serde(deserialize_with = "crate::db::int")]
    pub created_at: i64,
    #[serde(deserialize_with = "crate::db::int")]
    pub updated_at: i64,
}

impl JobRow {
    pub fn to_job(&self) -> ClaudeLoginJob {
        ClaudeLoginJob {
            id: LoginJobId(self.id.clone()),
            state: serde_json::from_str(&self.state).unwrap_or(ClaudeLoginState::Failed {
                error: "unreadable state".into(),
            }),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Starts `claude auth login` on a runner, for a full-scope login (Remote Control).
pub async fn start_login(ctx: &mut Ctx) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    // Before the rate limit, so that asking to confirm costs no sign-in.
    ctx.verified(Sensitive::ConnectClaude)?;
    ctx.rate_limit(&format!("claude-login:{}", user.id)).await?;
    let id = fctl_domain::ids::login_job_id(ctx.now);
    let state = serde_json::to_string(&ClaudeLoginState::Pending)?;
    q(
        &ctx.db,
        "INSERT INTO claude_login_jobs (id, user_id, state, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        &[s(id.as_str()), s(&user.id), s(&state), n(ctx.now), n(ctx.now)],
    )?
    .run()
    .await?;
    let stub = internal::fleet_stub(&ctx.env)?;
    let started: serde_json::Value = internal::post(
        &stub,
        "/login/start",
        &LoginStart {
            job_id: id.clone(),
            user: user.to_ref(),
            runners: Some(login_runners(ctx, &user).await?),
        },
    )
    .await?;
    if started.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(unavailable(
            "None of the runners that may run your Claude sign-in is online: yours, and those that run your sessions in your workspaces. Start one, or connect a long-lived token instead.",
        ));
    }
    job(ctx, id.as_str()).await
}

pub async fn job(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let user = ctx.user()?;
    let row: Option<JobRow> = q(
        &ctx.db,
        "SELECT * FROM claude_login_jobs WHERE id = ? AND user_id = ?",
        &[s(id), s(&user.id)],
    )?
    .first()
    .await?;
    http::ok(&row.ok_or_else(|| not_found("Sign-in"))?.to_job())
}

pub async fn submit_code(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: ClaudeLoginCodeRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    let code = body.code.trim().to_owned();
    if code.is_empty() || code.len() > 2000 || !code.contains('#') {
        return Err(validation(
            "code",
            "Paste the whole code claude.ai showed (it contains a #)",
        ));
    }
    let row: Option<JobRow> = q(
        &ctx.db,
        "SELECT * FROM claude_login_jobs WHERE id = ? AND user_id = ?",
        &[s(id), s(&user.id)],
    )?
    .first()
    .await?;
    row.ok_or_else(|| not_found("Sign-in"))?;
    let stub = internal::fleet_stub(&ctx.env)?;
    let _: serde_json::Value = internal::post(
        &stub,
        "/login/code",
        &LoginCode {
            job_id: LoginJobId(id.to_owned()),
            code,
        },
    )
    .await?;
    job(ctx, id).await
}

pub fn _kind_name(k: ClaudeCredentialKind) -> &'static str {
    k.as_str()
}

/// The runners that may run someone's Claude sign-in, which passes their
/// tokens through the host, in the order to try them: the ones they
/// registered, then those that run their sessions in the workspaces open to
/// them (not ones whose two-factor requirement shuts them out). A runner of
/// someone else's runs them only where it runs everyone's sessions and
/// they don't keep theirs on their own runners
/// (`fctl_domain::schedule::runs_for`).
async fn login_runners(ctx: &Ctx, user: &UserRow) -> AppResult<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct R {
        id: String,
        owner: Option<String>,
        workspace_id: Option<String>,
        #[serde(deserialize_with = "crate::db::int_bool", default)]
        personal: bool,
        #[serde(deserialize_with = "crate::db::int_bool", default)]
        own_runners: bool,
    }
    let rows: Vec<R> = q(
        &ctx.db,
        concat!(
            "SELECT r.id, r.created_by AS owner, rw.workspace_id, rw.personal, m.own_runners FROM runners r \
             LEFT JOIN runner_workspaces rw ON rw.runner_id = r.id \
             LEFT JOIN workspace_members m ON m.workspace_id = rw.workspace_id AND m.user_id = ?1 \
             WHERE r.enabled = 1 AND ((m.user_id IS NOT NULL AND ",
            crate::serves!(),
            ") OR r.created_by = ?1)"
        ),
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let open: Vec<String> = crate::workspaces::mine(&ctx.db, &user.id)
        .await?
        .into_iter()
        .filter(|w| !crate::workspaces::blocked_for(ctx, user, w.require_two_factor))
        .map(|w| w.id)
        .collect();
    let mine = |r: &R| r.owner.as_deref() == Some(user.id.as_str());
    let mut ids: Vec<String> = rows
        .iter()
        .filter(|r| mine(r))
        .map(|r| r.id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    let mut others: Vec<String> = rows
        .iter()
        .filter(|r| {
            !mine(r)
                && r.workspace_id.as_ref().is_some_and(|w| open.contains(w))
                && fctl_domain::schedule::runs_for(
                    r.owner.as_deref(),
                    r.personal,
                    Some(&user.id),
                    r.own_runners,
                )
        })
        .map(|r| r.id.clone())
        .collect();
    others.sort();
    others.dedup();
    ids.extend(others);
    Ok(ids)
}
