//! fctl's Linear app (docs/architecture.md#linear): its OAuth client, which
//! installs the app in Linear workspaces and links people's Linear
//! accounts, the tokens of each Linear workspace it is installed in
//! (`linear_installations`), and its GraphQL calls
//! (`fctl_domain::linear` has the queries).
//!
//! Linear keeps one token for an app's user, so a Linear workspace has one
//! installation here, whatever fctl workspaces work in it: installing the
//! app again, as another fctl workspace's admin does to connect it,
//! replaces the token for all of them. Access tokens last a day; their
//! refresh tokens change with each refresh, and Linear answers a refresh it
//! already answered the same for half an hour, so two that race end alike.

use crate::config::{Config, LinearConfig};
use crate::crypto::Keys;
use crate::db::{n, on, os, q, s};
use crate::error::{AppError, AppResult, internal, not_configured};
use fctl_domain::linear::{self as lin, GraphqlErrorKind, Tokens};
use fctl_proto::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use worker::{D1Database, Fetch, Headers, Method, Request, RequestInit};

/// The instance's Linear app, or why it has none.
pub fn config(cfg: &Config) -> AppResult<&LinearConfig> {
    cfg.linear.as_ref().ok_or_else(|| {
        not_configured(
            "This instance has no Linear app: its admins set LINEAR_CLIENT_ID and LINEAR_CLIENT_SECRET (see the self-hosting guide)",
        )
    })
}

/// What D1 keeps sealed for an installation.
#[derive(Serialize, Deserialize)]
struct Secret {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

fn aad(organization: &str) -> String {
    format!("linear:{organization}")
}

/// A form post to Linear's OAuth endpoints: its status and answer.
async fn form(url: &str, fields: &[(&str, &str)]) -> AppResult<(u16, Value)> {
    let body: String = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish();
    let headers = Headers::new();
    headers.set("Content-Type", "application/x-www-form-urlencoded")?;
    headers.set("Accept", "application/json")?;
    headers.set("User-Agent", "fctl-control")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(body.into()));
    let mut res = Fetch::Request(Request::new_with_init(url, &init)?)
        .send()
        .await
        .map_err(|e| unavailable(&format!("Linear didn't answer: {e}")))?;
    let status = res.status_code();
    let text = res.text().await.unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

fn unavailable(message: &str) -> AppError {
    AppError(ApiError::new(ErrorCode::Unavailable, message))
}

/// Trades a sign-in's code for tokens.
pub async fn exchange(
    cfg: &LinearConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> AppResult<Tokens> {
    let (status, v) = form(
        &format!("{}/oauth/token", cfg.api_url),
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", &cfg.client_id),
            ("client_secret", &cfg.client_secret),
            ("code_verifier", verifier),
        ],
    )
    .await?;
    if status != 200 {
        worker::console_error!("Linear's token endpoint answered {status}: {v}");
        return Err(unavailable("Linear refused the sign-in: try again"));
    }
    serde_json::from_value(v)
        .map_err(|_| unavailable("Linear answered the sign-in without a token"))
}

/// Revokes a token, logging what Linear says.
pub async fn revoke(cfg: &LinearConfig, token: &str) {
    match form(
        &format!("{}/oauth/revoke", cfg.api_url),
        &[("token", token)],
    )
    .await
    {
        Ok((200, _)) => {}
        Ok((status, _)) => {
            worker::console_error!("revoking a Linear token: Linear answered {status}")
        }
        Err(e) => worker::console_error!("revoking a Linear token: {}", e.0.message),
    }
}

/// fctl's app in one Linear workspace, or a person's sign-in: GraphQL with
/// its token.
pub struct Linear<'a> {
    cfg: &'a LinearConfig,
    token: String,
    /// The Linear workspace's name, for what fctl says when it fails.
    name: String,
}

impl<'a> Linear<'a> {
    pub fn with_token(cfg: &'a LinearConfig, token: &str) -> Self {
        Self {
            cfg,
            token: token.to_owned(),
            name: "the Linear workspace".to_owned(),
        }
    }

    /// The answer's `data`, or what went wrong as an error for people.
    pub async fn graphql(&self, query: &str, variables: Value) -> AppResult<Value> {
        let headers = Headers::new();
        headers.set("Authorization", &format!("Bearer {}", self.token))?;
        headers.set("Content-Type", "application/json")?;
        headers.set("User-Agent", "fctl-control")?;
        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_headers(headers)
            .with_body(Some(
                json!({ "query": query, "variables": variables })
                    .to_string()
                    .into(),
            ));
        let req = Request::new_with_init(&format!("{}/graphql", self.cfg.api_url), &init)?;
        let mut res = Fetch::Request(req)
            .send()
            .await
            .map_err(|e| unavailable(&format!("Linear didn't answer: {e}")))?;
        let status = res.status_code();
        let text = res.text().await.unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(e) = lin::errors(&v) {
            return Err(self.failed(e.kind, &e.message));
        }
        match status {
            200 => Ok(v.get("data").cloned().unwrap_or(Value::Null)),
            401 => Err(self.failed(GraphqlErrorKind::Unauthorized, "")),
            429 => Err(self.failed(GraphqlErrorKind::RateLimited, "")),
            _ => {
                worker::console_error!("Linear's GraphQL API answered {status}");
                Err(unavailable("Linear didn't answer: try again in a minute"))
            }
        }
    }

    fn failed(&self, kind: GraphqlErrorKind, message: &str) -> AppError {
        let (code, message) = match kind {
            GraphqlErrorKind::RateLimited => (
                ErrorCode::RateLimited,
                "Linear asks fctl to slow down: try again in a minute".to_owned(),
            ),
            GraphqlErrorKind::Unauthorized => (
                ErrorCode::NotConfigured,
                format!(
                    "Linear no longer takes fctl's access to {}: an admin of the fctl workspace connects it again",
                    self.name
                ),
            ),
            GraphqlErrorKind::Forbidden => (
                ErrorCode::Forbidden,
                format!("Linear didn't let fctl do that: {message}"),
            ),
            GraphqlErrorKind::Invalid => (ErrorCode::Validation, format!("Linear: {message}")),
            GraphqlErrorKind::NotFound => (ErrorCode::NotFound, format!("Linear: {message}")),
            GraphqlErrorKind::Other => (ErrorCode::Unavailable, format!("Linear: {message}")),
        };
        AppError(ApiError::new(code, message))
    }
}

// ───────────────────────────── installations ─────────────────────────────

/// fctl's app in a Linear workspace, as D1 keeps it.
#[derive(Clone, Debug, Deserialize)]
pub struct InstallationRow {
    pub organization_id: String,
    pub name: String,
    secret_enc: String,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    expires_at: Option<i64>,
    pub status: String,
}

impl InstallationRow {
    pub fn active(&self) -> bool {
        self.status == "active"
    }
}

pub const INSTALLATION_SELECT: &str =
    "SELECT organization_id, name, secret_enc, expires_at, status FROM linear_installations";

pub async fn installation(
    db: &D1Database,
    organization: &str,
) -> AppResult<Option<InstallationRow>> {
    q(
        db,
        &format!("{INSTALLATION_SELECT} WHERE organization_id = ?"),
        &[s(organization)],
    )?
    .first()
    .await
}

/// The Linear workspace a sign-in as fctl's app came back from, and who
/// signed in.
pub struct Installed {
    pub organization: String,
    pub name: String,
    pub url_key: String,
    pub logo_url: Option<String>,
    /// fctl's app user there, with its name and handle.
    pub app_user: Option<String>,
    pub app_name: Option<String>,
    pub app_handle: Option<String>,
}

/// Keeps the tokens of fctl's app in a Linear workspace, in place of any it
/// had there.
pub async fn store(
    db: &D1Database,
    keys: &Keys,
    i: &Installed,
    tokens: &Tokens,
    by: &str,
    now: i64,
) -> AppResult<()> {
    let sealed = keys.seal_json(
        &aad(&i.organization),
        &Secret {
            access_token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone(),
        },
    )?;
    q(
        db,
        "INSERT INTO linear_installations (organization_id, name, url_key, logo_url, app_user_id, app_user_name, app_user_handle, secret_enc, expires_at, scopes, status, installed_by, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?) ON CONFLICT (organization_id) DO UPDATE SET name = excluded.name, \
         url_key = excluded.url_key, logo_url = excluded.logo_url, app_user_id = excluded.app_user_id, app_user_name = excluded.app_user_name, \
         app_user_handle = excluded.app_user_handle, secret_enc = excluded.secret_enc, \
         expires_at = excluded.expires_at, scopes = excluded.scopes, status = 'active', installed_by = excluded.installed_by, updated_at = excluded.updated_at",
        &[
            s(&i.organization),
            s(&i.name),
            s(&i.url_key),
            os(i.logo_url.as_deref()),
            os(i.app_user.as_deref()),
            os(i.app_name.as_deref()),
            os(i.app_handle.as_deref()),
            s(&sealed),
            on(tokens.expires_at(now)),
            s(&tokens.scopes().join(" ")),
            s(by),
            n(now),
            n(now),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// Marks fctl's app in a Linear workspace revoked, and forgets its tokens,
/// which no longer work: uninstalled there, or its token no longer renews.
/// Its fctl workspaces' admins connect it again.
pub async fn mark_revoked(db: &D1Database, organization: &str, now: i64) -> AppResult<()> {
    q(
        db,
        "UPDATE linear_installations SET status = 'revoked', secret_enc = '', expires_at = NULL, updated_at = ? \
         WHERE organization_id = ?",
        &[n(now), s(organization)],
    )?
    .run()
    .await?;
    Ok(())
}

/// Removes fctl's app from a Linear workspace no fctl workspace works in
/// any more: Linear revokes its token, and D1 forgets it.
pub async fn uninstall_if_unused(
    db: &D1Database,
    cfg: &Config,
    keys: &Keys,
    organization: &str,
) -> AppResult<()> {
    #[derive(Deserialize)]
    struct Used {
        #[serde(deserialize_with = "crate::db::int")]
        n: i64,
    }
    let used: Option<Used> = q(
        db,
        "SELECT COUNT(*) AS n FROM workspace_linear WHERE organization_id = ?",
        &[s(organization)],
    )?
    .first()
    .await?;
    if used.is_some_and(|u| u.n > 0) {
        return Ok(());
    }
    if let (Some(row), Some(linear)) = (installation(db, organization).await?, cfg.linear.as_ref())
        && let Ok(secret) = keys.open_json::<Secret>(&aad(organization), &row.secret_enc)
    {
        revoke(linear, &secret.access_token).await;
    }
    q(
        db,
        "DELETE FROM linear_installations WHERE organization_id = ?",
        &[s(organization)],
    )?
    .run()
    .await?;
    Ok(())
}

/// fctl's app in Linear workspace `organization`, with a token that works
/// for a while yet: renewed first when it runs out within ten minutes.
pub async fn client<'a>(
    db: &D1Database,
    cfg: &'a Config,
    keys: &Keys,
    organization: &str,
    now: i64,
) -> AppResult<Linear<'a>> {
    let linear = config(cfg)?;
    let row = installation(db, organization).await?.ok_or_else(|| {
        not_configured("fctl's app isn't installed in this Linear workspace: an admin of the fctl workspace connects it")
    })?;
    if !row.active() {
        return Err(not_configured(&format!(
            "Linear no longer takes fctl's access to {}: an admin of the fctl workspace connects it again",
            row.name
        )));
    }
    let mut secret: Secret = keys.open_json(&aad(organization), &row.secret_enc)?;
    if row
        .expires_at
        .is_some_and(|e| e - now < lin::REFRESH_MARGIN_MS)
    {
        secret = renew(db, linear, keys, &row, secret, now).await?;
    }
    Ok(Linear {
        cfg: linear,
        token: secret.access_token,
        name: row.name,
    })
}

/// A new access token for an installation, kept in place of the old.
async fn renew(
    db: &D1Database,
    linear: &LinearConfig,
    keys: &Keys,
    row: &InstallationRow,
    secret: Secret,
    now: i64,
) -> AppResult<Secret> {
    let Some(refresh) = secret.refresh_token.as_deref() else {
        // A token from before refresh tokens lasts until it doesn't.
        return Ok(secret);
    };
    let (status, v) = form(
        &format!("{}/oauth/token", linear.api_url),
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", &linear.client_id),
            ("client_secret", &linear.client_secret),
        ],
    )
    .await?;
    // Only a refresh token Linear no longer takes ends the installation: a
    // client it refuses (a secret being changed) or a malformed request
    // says nothing about the token, which stays for the next try.
    if (status == 400 || status == 401)
        && v.get("error").and_then(Value::as_str) == Some("invalid_grant")
    {
        // Another refresh may have used it first, more than half an hour
        // ago: what it kept works.
        if let Some(now_row) = installation(db, &row.organization_id).await?
            && now_row.secret_enc != row.secret_enc
            && now_row.active()
        {
            return keys.open_json(&aad(&row.organization_id), &now_row.secret_enc);
        }
        worker::console_error!(
            "Linear refused to renew the token of {}: {v}",
            row.organization_id
        );
        mark_revoked(db, &row.organization_id, now).await?;
        return Err(not_configured(&format!(
            "Linear no longer takes fctl's access to {}: an admin of the fctl workspace connects it again",
            row.name
        )));
    }
    if status == 400 || status == 401 {
        worker::console_error!(
            "Linear refused to renew the token of {} ({status}): {v}",
            row.organization_id
        );
        return Err(not_configured(
            "Linear refused fctl's app: the instance's admins check LINEAR_CLIENT_ID and LINEAR_CLIENT_SECRET",
        ));
    }
    if status != 200 {
        return Err(unavailable(
            "Linear didn't renew fctl's access: try again in a minute",
        ));
    }
    let tokens: Tokens = serde_json::from_value(v).map_err(internal)?;
    let new = Secret {
        access_token: tokens.access_token.clone(),
        refresh_token: tokens.refresh_token.clone().or(secret.refresh_token),
    };
    q(
        db,
        // Active again, in case another renewal of the same token gave up
        // meanwhile: this one won.
        "UPDATE linear_installations SET secret_enc = ?, expires_at = ?, status = 'active', updated_at = ? \
         WHERE organization_id = ?",
        &[
            s(&keys.seal_json(&aad(&row.organization_id), &new)?),
            on(tokens.expires_at(now)),
            n(now),
            s(&row.organization_id),
        ],
    )?
    .run()
    .await?;
    Ok(new)
}

/// Who a token signs in as, and their Linear workspace (`viewer`).
pub struct Viewer {
    pub id: String,
    pub name: String,
    /// The handle people mention them by (`displayName`).
    pub handle: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub app: bool,
    pub organization: String,
    pub organization_name: String,
    pub url_key: String,
    pub logo_url: Option<String>,
}

impl Linear<'_> {
    pub async fn viewer(&self) -> AppResult<Viewer> {
        let d = self.graphql(lin::VIEWER, json!({})).await?;
        let v = d
            .get("viewer")
            .ok_or_else(|| internal("Linear's viewer is missing"))?;
        let o = v
            .get("organization")
            .ok_or_else(|| internal("Linear's viewer has no organization"))?;
        let st = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        Ok(Viewer {
            id: st(v, "id").ok_or_else(|| internal("Linear's viewer has no id"))?,
            name: st(v, "name")
                .or_else(|| st(v, "displayName"))
                .unwrap_or_else(|| "Someone".into()),
            handle: st(v, "displayName"),
            email: st(v, "email"),
            avatar_url: st(v, "avatarUrl"),
            app: v.get("app").and_then(Value::as_bool).unwrap_or(false),
            organization: st(o, "id").ok_or_else(|| internal("Linear's organization has no id"))?,
            organization_name: st(o, "name").unwrap_or_else(|| "Linear".into()),
            url_key: st(o, "urlKey").unwrap_or_default(),
            logo_url: st(o, "logoUrl"),
        })
    }

    /// Posts an agent activity (`fctl_domain::linear::activity`).
    pub async fn activity(&self, input: Value) -> AppResult<()> {
        self.graphql(lin::ACTIVITY_CREATE, json!({ "input": input }))
            .await?;
        Ok(())
    }

    /// Changes an agent session: its links, its plan.
    pub async fn update_session(&self, id: &str, input: Value) -> AppResult<()> {
        self.graphql(lin::SESSION_UPDATE, json!({ "id": id, "input": input }))
            .await?;
        Ok(())
    }
}
