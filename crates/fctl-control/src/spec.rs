//! Building the [`SessionSpec`] a host needs, with fresh credentials.

use crate::config::Config;
use crate::crypto::Keys;
use crate::db::{n, on, q, s};
use crate::error::{AppError, AppResult, internal};
use crate::github;
use crate::model::{
    CredentialRow, CredentialSecret, PROJECT_SELECT, ProjectRow, SESSION_SELECT, SessionRow,
};
use fctl_domain::size::Want;
use fctl_proto::runner::{
    ClaudeCredential, ClaudeCredentialKind, GitIdentity, GithubCredential, ProjectSpec,
    SessionCredentials, SessionSpec,
};
use fctl_proto::session::{ProjectSettings, RepoRef};
use fctl_proto::{ApiError, CredentialId, ErrorCode, ProjectId};
use std::cell::RefCell;
use std::collections::HashMap;
use worker::{D1Database, Fetch, Headers, Method, Request, RequestInit};

/// Claude Code's OAuth client and token endpoint (as `claude` itself uses them).
const CLAUDE_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// Refresh when less than this is left on an access token.
const REFRESH_MARGIN_MS: i64 = 10 * 60 * 1000;

thread_local! {
    static BOT_EMAIL: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

pub async fn session_row(db: &D1Database, id: &str) -> AppResult<Option<SessionRow>> {
    q(
        db,
        &format!("{SESSION_SELECT} WHERE s.id = ?2"),
        &[s(""), s(id)],
    )?
    .first()
    .await
}

pub async fn project_row(db: &D1Database, id: &str) -> AppResult<Option<ProjectRow>> {
    q(
        db,
        &format!("{PROJECT_SELECT} WHERE p.id = ?2"),
        &[s(""), s(id)],
    )?
    .first()
    .await
}

/// The CPUs and memory each of these sessions asks for (its project's
/// sandbox settings), in one query.
pub async fn session_wants(db: &D1Database, ids: &[&str]) -> AppResult<HashMap<String, Want>> {
    #[derive(serde::Deserialize)]
    struct Row {
        id: String,
        settings: String,
    }
    let rows: Vec<Row> = q(
        db,
        "SELECT s.id AS id, p.settings AS settings FROM sessions s JOIN projects p ON p.id = s.project_id \
         WHERE s.id IN (SELECT value FROM json_each(?))",
        &[s(&serde_json::to_string(ids)?)],
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let settings: ProjectSettings = serde_json::from_str(&r.settings).unwrap_or_default();
            (r.id, Want::of(&settings))
        })
        .collect())
}

pub async fn build(
    db: &D1Database,
    cfg: &Config,
    keys: &Keys,
    session_id: &str,
    now: i64,
) -> AppResult<SessionSpec> {
    let row = session_row(db, session_id)
        .await?
        .ok_or_else(|| internal("session vanished"))?;
    let project = project_row(db, &row.project_id)
        .await?
        .ok_or_else(|| internal("project vanished"))?;
    let cred_id = row.credential_id.clone().ok_or_else(|| {
        AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "The session has no Claude credential",
        ))
    })?;
    let claude = claude_credential(db, keys, &cred_id, now).await?;
    let repo = project.repo();
    let github = match (&repo, cfg.github.is_some()) {
        (Some(r), true) => Some(github_token(cfg, r, now).await?),
        _ => None,
    };
    let secrets = crate::api::projects::decrypted_secrets(db, keys, &project.id).await?;
    let author = creator(db, &row.created_by).await?;
    let git_author = bot_identity(cfg, now).await;
    let mut settings = project.settings();
    // Agents from before `claude_ai_connectors` ignore it, but every agent
    // passes the project's environment on to Claude. Not stored.
    settings.env = fctl_domain::settings::session_env(&settings);
    Ok(SessionSpec {
        session_id: row.session_id(),
        number: row.number.max(0) as u32,
        title: row.title.clone(),
        project: ProjectSpec {
            id: ProjectId(project.id.clone()),
            slug: project.slug.clone(),
            name: project.name.clone(),
            repo,
            default_branch: project.default_branch.clone(),
            settings,
        },
        branch: row
            .branch
            .clone()
            .unwrap_or_else(|| format!("fctl/{}", row.number)),
        base_branch: row
            .base_branch
            .clone()
            .unwrap_or_else(|| project.default_branch.clone()),
        prompt: if row.started_at.is_some() {
            None
        } else {
            Some(row.prompt.clone())
        },
        task: Some(row.prompt.clone()),
        author: row.creator_ref(),
        claude_session_id: row.claude_session_id.clone(),
        resume: row.started_at.is_some(),
        model: row.model.clone(),
        effort: row.effort.clone(),
        secrets,
        credentials: SessionCredentials { claude, github },
        git_author,
        co_authors: author.into_iter().collect(),
        web_url: cfg.url(&format!("/s/{}", row.id)),
        idle_timeout_secs: project
            .settings()
            .idle_timeout_minutes
            .map_or(2 * 60 * 60, |m| m as u64 * 60),
        run_id: None,
    })
}

async fn creator(db: &D1Database, user_id: &str) -> AppResult<Option<GitIdentity>> {
    #[derive(serde::Deserialize)]
    struct U {
        name: String,
        email: String,
    }
    let u: Option<U> = q(
        db,
        "SELECT name, email FROM users WHERE id = ?",
        &[s(user_id)],
    )?
    .first()
    .await?;
    Ok(u.map(|u| GitIdentity {
        name: u.name,
        email: u.email,
    }))
}

/// Commits are authored by the GitHub App's bot account.
async fn bot_identity(cfg: &Config, _now: i64) -> GitIdentity {
    let Some(gh) = cfg.github.as_ref() else {
        return GitIdentity {
            name: "fctl agent".into(),
            email: "agent@fctl.invalid".into(),
        };
    };
    let login = format!("{}[bot]", gh.slug);
    if let Some((name, email)) = BOT_EMAIL.with(|b| b.borrow().clone()) {
        return GitIdentity { name, email };
    }
    let mut email = format!("{login}@users.noreply.github.com");
    if let Ok(req) = Request::new(
        &format!("https://api.github.com/users/{}%5Bbot%5D", gh.slug),
        Method::Get,
    ) {
        let mut req = req;
        if let Ok(h) = req.headers_mut() {
            let _ = h.set("User-Agent", "fctl-control");
            let _ = h.set("Accept", "application/vnd.github+json");
        }
        if let Ok(mut res) = Fetch::Request(req).send().await
            && let Ok(v) = res.json::<serde_json::Value>().await
            && let Some(id) = v.get("id").and_then(serde_json::Value::as_u64)
        {
            email = format!("{id}+{login}@users.noreply.github.com");
        }
    }
    BOT_EMAIL.with(|b| *b.borrow_mut() = Some((login.clone(), email.clone())));
    GitIdentity { name: login, email }
}

pub async fn github_token(cfg: &Config, repo: &RepoRef, now: i64) -> AppResult<GithubCredential> {
    github::from_config(cfg, now)?.session_token(repo).await
}

/// The credential with a valid access token, refreshing an OAuth login when
/// it is about to expire. Callers serialize refreshes per credential.
pub async fn claude_credential(
    db: &D1Database,
    keys: &Keys,
    id: &str,
    now: i64,
) -> AppResult<ClaudeCredential> {
    let row: Option<CredentialRow> = q(
        db,
        "SELECT * FROM claude_credentials WHERE id = ?",
        &[s(id)],
    )?
    .first()
    .await?;
    let row = row.ok_or_else(|| {
        AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "The Claude credential was removed",
        ))
    })?;
    if row.status != "active" {
        return Err(AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "The Claude credential is no longer valid. Connect Claude again in Settings.",
        )));
    }
    let aad = CredentialRow::aad(&row.id, &row.user_id);
    let mut secret: CredentialSecret = keys.open_json(&aad, &row.secret_enc)?;
    let kind = row.kind();
    if kind == ClaudeCredentialKind::Oauth
        && secret
            .expires_at
            .is_some_and(|e| e - now < REFRESH_MARGIN_MS)
    {
        secret = refresh(db, keys, &row, &secret, now).await?;
    }
    q(
        db,
        "UPDATE claude_credentials SET last_used_at = ? WHERE id = ?",
        &[n(now), s(&row.id)],
    )?
    .run()
    .await?;
    Ok(ClaudeCredential {
        id: CredentialId(row.id.clone()),
        kind,
        access_token: secret.access_token,
        scopes: row.scopes(),
        expires_at: secret.expires_at,
        subscription_type: row.subscription_type.clone(),
        account: row.account(),
    })
}

async fn refresh(
    db: &D1Database,
    keys: &Keys,
    row: &CredentialRow,
    secret: &CredentialSecret,
    now: i64,
) -> AppResult<CredentialSecret> {
    let refresh_token = secret.refresh_token.clone().ok_or_else(|| {
        AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "This Claude login cannot be refreshed. Connect Claude again.",
        ))
    })?;
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLAUDE_CLIENT_ID,
        "scope": row.scopes().join(" "),
    });
    let headers = Headers::new();
    headers.set("Content-Type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(body.to_string().into()));
    let mut res = Fetch::Request(Request::new_with_init(CLAUDE_TOKEN_URL, &init)?)
        .send()
        .await?;
    let status = res.status_code();
    let v: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
    if status == 400 || status == 401 {
        // invalid_grant: the login was revoked or used elsewhere.
        q(
            db,
            "UPDATE claude_credentials SET status = 'invalid', updated_at = ? WHERE id = ?",
            &[n(now), s(&row.id)],
        )?
        .run()
        .await?;
        return Err(AppError(ApiError::new(
            ErrorCode::NotConfigured,
            "Claude rejected the stored login. Connect Claude again in Settings.",
        )));
    }
    if status != 200 {
        return Err(AppError(ApiError::new(
            ErrorCode::Unavailable,
            format!("Claude's token service answered {status}"),
        )));
    }
    let access_token = v
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| internal("token response without access_token"))?
        .to_owned();
    let new = CredentialSecret {
        access_token,
        refresh_token: v
            .get("refresh_token")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .or(secret.refresh_token.clone()),
        expires_at: v
            .get("expires_in")
            .and_then(serde_json::Value::as_i64)
            .map(|s| now + s * 1000),
    };
    let sealed = keys.seal_json(&CredentialRow::aad(&row.id, &row.user_id), &new)?;
    q(
        db,
        "UPDATE claude_credentials SET secret_enc = ?, expires_at = ?, updated_at = ? WHERE id = ?",
        &[s(&sealed), on(new.expires_at), n(now), s(&row.id)],
    )?
    .run()
    .await?;
    Ok(new)
}
