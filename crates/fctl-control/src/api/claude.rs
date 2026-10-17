//! `/api/v1/me/claude/*`: connecting Claude subscriptions.

use crate::auth;
use crate::ctx::Ctx;
use crate::db::{n, os, q, s};
use crate::error::{not_found, unavailable, validation, AppResult};
use crate::http;
use crate::internal::{self, LoginCode, LoginStart};
use crate::model::{CredentialRow, CredentialSecret};
use fctl_proto::api::*;
use fctl_proto::runner::ClaudeCredentialKind;
use fctl_proto::LoginJobId;
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
    // Long-lived tokens look like sk-ant-oat01-… (an API key is sk-ant-api…).
    if token.starts_with("sk-ant-api") {
        return Err(validation(
            "token",
            "That is an API key, which bills per token. fctl only runs on subscriptions: run `claude setup-token` and paste its output.",
        ));
    }
    if !token.starts_with("sk-ant-oat") || token.len() < 40 || token.len() > 500 || token.contains(char::is_whitespace) {
        return Err(validation("token", "Paste the token `claude setup-token` printed (it starts with sk-ant-oat)"));
    }
    let label = body.label.as_deref().map(|l| fctl_domain::validate::name("label", l)).transpose()?;
    let id = fctl_domain::ids::credential_id(ctx.now);
    let secret = CredentialSecret { access_token: token, refresh_token: None, expires_at: None };
    let sealed = ctx.keys()?.seal_json(&CredentialRow::aad(id.as_str(), &user.id), &secret)?;
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
    q(&ctx.db, "UPDATE users SET default_credential_id = ? WHERE id = ?", &[s(id.as_str()), s(&user.id)])?
        .run()
        .await?;
    ctx.audit("claude.token_added", Some(id.as_str()), None);
    let user = auth::user_by_id(ctx, &user.id).await?.ok_or_else(|| not_found("User"))?;
    http::created(&auth::me(ctx, &user).await?)
}

pub async fn remove(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let changed = q(&ctx.db, "DELETE FROM claude_credentials WHERE id = ? AND user_id = ?", &[s(id), s(&user.id)])?
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
    let user = auth::user_by_id(ctx, &user.id).await?.ok_or_else(|| not_found("User"))?;
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
    q(&ctx.db, "UPDATE users SET default_credential_id = ? WHERE id = ?", &[s(id), s(&user.id)])?
        .run()
        .await?;
    let user = auth::user_by_id(ctx, &user.id).await?.ok_or_else(|| not_found("User"))?;
    http::ok(&auth::me(ctx, &user).await?)
}

#[derive(serde::Deserialize)]
pub struct JobRow {
    pub id: String,
    pub user_id: String,
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
            state: serde_json::from_str(&self.state).unwrap_or(ClaudeLoginState::Failed { error: "unreadable state".into() }),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Starts `claude auth login` on a runner, for a full-scope login (Remote Control).
pub async fn start_login(ctx: &mut Ctx) -> AppResult<Response> {
    let user = ctx.user()?.clone();
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
    let started: serde_json::Value = internal::post(&stub, "/login/start", &LoginStart { job_id: id.clone(), user: user.to_ref() }).await?;
    if started.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(unavailable("No runner is online to run the Claude sign-in. Start a host first."));
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
        return Err(validation("code", "Paste the whole code claude.ai showed (it contains a #)"));
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
    let _: serde_json::Value = internal::post(&stub, "/login/code", &LoginCode { job_id: LoginJobId(id.to_owned()), code }).await?;
    job(ctx, id).await
}

pub fn _kind_name(k: ClaudeCredentialKind) -> &'static str {
    k.as_str()
}
