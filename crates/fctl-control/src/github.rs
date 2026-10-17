//! GitHub App client: app JWTs, installation tokens, repositories.
//!
//! Sessions get installation tokens scoped to their one repository, valid for
//! an hour. Hosts ask for fresh ones through the runner connection.

use crate::config::GithubConfig;
use crate::crypto;
use crate::error::{AppError, AppResult, internal, not_configured, unavailable};
use fctl_proto::api::GithubRepo;
use fctl_proto::runner::GithubCredential;
use fctl_proto::session::RepoRef;
use fctl_proto::{ApiError, ErrorCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use worker::{Fetch, Headers, Method, Request, RequestInit};

const API: &str = "https://api.github.com";

thread_local! {
    /// owner (lowercase) → installation id, per isolate.
    static INSTALLATIONS: RefCell<HashMap<String, (u64, i64)>> = RefCell::new(HashMap::new());
}

pub struct Github<'a> {
    cfg: &'a GithubConfig,
    now: i64,
}

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
    expires_at: String,
}

#[derive(Deserialize)]
struct Installation {
    id: u64,
    account: Option<Account>,
}

#[derive(Deserialize)]
struct Account {
    login: String,
}

#[derive(Deserialize)]
struct RepoJson {
    name: String,
    full_name: String,
    private: bool,
    default_branch: Option<String>,
    description: Option<String>,
    pushed_at: Option<String>,
    owner: Account,
}

#[derive(Deserialize)]
struct RepoList {
    repositories: Vec<RepoJson>,
}

/// The permissions agents get on their repository.
pub fn session_permissions() -> Value {
    json!({
        "contents": "write",
        "pull_requests": "write",
        "issues": "write",
        "workflows": "write",
        "checks": "read",
        "actions": "read",
        "statuses": "read",
        "metadata": "read"
    })
}

pub fn from_config(cfg: &crate::config::Config, now: i64) -> AppResult<Github<'_>> {
    cfg.github
        .as_ref()
        .map(|g| Github { cfg: g, now })
        .ok_or_else(|| not_configured("No GitHub App is configured on this instance"))
}

impl<'a> Github<'a> {
    pub fn default_owner(&self) -> Option<&str> {
        self.cfg.default_owner.as_deref()
    }

    /// An app JWT (RS256, valid 9 minutes, backdated 60 s for clock skew).
    async fn app_jwt(&self) -> AppResult<String> {
        let iat = self.now / 1000 - 60;
        let header = crypto::b64url(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = crypto::b64url(
            json!({ "iat": iat, "exp": iat + 540, "iss": self.cfg.app_id })
                .to_string()
                .as_bytes(),
        );
        let signing_input = format!("{header}.{claims}");
        let sig = crypto::rs256_sign(&self.cfg.private_key_pem, signing_input.as_bytes()).await?;
        Ok(format!("{signing_input}.{}", crypto::b64url(&sig)))
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        auth: &str,
        body: Option<Value>,
    ) -> AppResult<(u16, Value)> {
        let headers = Headers::new();
        headers.set("Authorization", auth)?;
        headers.set("Accept", "application/vnd.github+json")?;
        headers.set("X-GitHub-Api-Version", "2022-11-28")?;
        headers.set("User-Agent", "fctl-control")?;
        let mut init = RequestInit::new();
        init.with_method(method).with_headers(headers);
        if let Some(b) = body {
            init.with_body(Some(b.to_string().into()));
        }
        let req = Request::new_with_init(&format!("{API}{path}"), &init)?;
        let mut res = Fetch::Request(req).send().await?;
        let status = res.status_code();
        let text = res.text().await.unwrap_or_default();
        let value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok((status, value))
    }

    async fn app_call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> AppResult<(u16, Value)> {
        let jwt = self.app_jwt().await?;
        self.call(method, path, &format!("Bearer {jwt}"), body)
            .await
    }

    /// The installation id for an account (organization or user).
    pub async fn installation_id(&self, owner: &str) -> AppResult<u64> {
        let key = owner.to_lowercase();
        if let Some(id) = INSTALLATIONS.with(|m| {
            m.borrow()
                .get(&key)
                .filter(|(_, at)| self.now - at < 10 * 60 * 1000)
                .map(|(id, _)| *id)
        }) {
            return Ok(id);
        }
        for path in [
            format!("/orgs/{owner}/installation"),
            format!("/users/{owner}/installation"),
        ] {
            let (status, v) = self.app_call(Method::Get, &path, None).await?;
            if status == 200 {
                let inst: Installation = serde_json::from_value(v)?;
                INSTALLATIONS.with(|m| m.borrow_mut().insert(key.clone(), (inst.id, self.now)));
                return Ok(inst.id);
            }
        }
        Err(AppError(ApiError::new(
            ErrorCode::Forbidden,
            format!(
                "The {} GitHub App is not installed on {owner}",
                self.cfg.slug
            ),
        )))
    }

    /// Accounts the app is installed on.
    pub async fn installations(&self) -> AppResult<Vec<(u64, String)>> {
        let (status, v) = self
            .app_call(Method::Get, "/app/installations?per_page=100", None)
            .await?;
        if status != 200 {
            return Err(unavailable("GitHub did not list the app's installations"));
        }
        let list: Vec<Installation> = serde_json::from_value(v)?;
        Ok(list
            .into_iter()
            .filter_map(|i| i.account.map(|a| (i.id, a.login)))
            .collect())
    }

    /// A token for one installation, optionally narrowed to repositories and permissions.
    pub async fn installation_token(
        &self,
        installation_id: u64,
        repos: Option<&[&str]>,
        permissions: Option<Value>,
    ) -> AppResult<GithubCredential> {
        let mut body = serde_json::Map::new();
        if let Some(r) = repos {
            body.insert("repositories".into(), json!(r));
        }
        if let Some(p) = permissions {
            body.insert("permissions".into(), p);
        }
        let (status, v) = self
            .app_call(
                Method::Post,
                &format!("/app/installations/{installation_id}/access_tokens"),
                Some(Value::Object(body)),
            )
            .await?;
        if status != 201 {
            let msg = v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            worker::console_error!("installation token failed: {status} {msg}");
            return Err(unavailable(&format!("GitHub refused a token: {msg}")));
        }
        let t: TokenResponse = serde_json::from_value(v)?;
        let expires_at = parse_rfc3339_ms(&t.expires_at).unwrap_or(self.now + 55 * 60 * 1000);
        Ok(GithubCredential {
            token: t.token,
            expires_at,
        })
    }

    /// A token for agents working on `repo`.
    pub async fn session_token(&self, repo: &RepoRef) -> AppResult<GithubCredential> {
        let inst = self.installation_id(&repo.owner).await?;
        self.installation_token(
            inst,
            Some(&[repo.name.as_str()]),
            Some(session_permissions()),
        )
        .await
    }

    async fn installation_call(
        &self,
        owner: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> AppResult<(u16, Value)> {
        let inst = self.installation_id(owner).await?;
        let token = self.installation_token(inst, None, None).await?;
        self.call(method, path, &format!("token {}", token.token), body)
            .await
    }

    pub async fn get_repo(&self, repo: &RepoRef) -> AppResult<GithubRepo> {
        let (status, v) = self
            .installation_call(
                &repo.owner,
                Method::Get,
                &format!("/repos/{}/{}", repo.owner, repo.name),
                None,
            )
            .await?;
        if status == 404 {
            return Err(AppError(ApiError::validation(
                "repo",
                format!(
                    "{} was not found, or the {} app cannot access it",
                    repo.full_name(),
                    self.cfg.slug
                ),
            )));
        }
        if status != 200 {
            return Err(unavailable("GitHub did not answer"));
        }
        Ok(to_repo(serde_json::from_value(v)?))
    }

    /// Repositories of every installation.
    pub async fn list_repos(&self) -> AppResult<Vec<GithubRepo>> {
        let mut out = Vec::new();
        for (inst, _) in self.installations().await? {
            let token = self.installation_token(inst, None, None).await?;
            for page in 1..=5 {
                let (status, v) = self
                    .call(
                        Method::Get,
                        &format!("/installation/repositories?per_page=100&page={page}"),
                        &format!("token {}", token.token),
                        None,
                    )
                    .await?;
                if status != 200 {
                    break;
                }
                let list: RepoList = serde_json::from_value(v)?;
                let n = list.repositories.len();
                out.extend(list.repositories.into_iter().map(to_repo));
                if n < 100 {
                    break;
                }
            }
        }
        out.sort_by(|a, b| b.pushed_at.cmp(&a.pushed_at));
        Ok(out)
    }

    /// Creates a repository in `owner` (an organization the app is installed on).
    pub async fn create_repo(
        &self,
        owner: &str,
        name: &str,
        private: bool,
        description: Option<&str>,
    ) -> AppResult<GithubRepo> {
        let body = json!({
            "name": name,
            "private": private,
            "description": description,
            "auto_init": true,
            "has_wiki": false,
        });
        let (status, v) = self
            .installation_call(
                owner,
                Method::Post,
                &format!("/orgs/{owner}/repos"),
                Some(body),
            )
            .await?;
        if status == 422 {
            return Err(AppError(ApiError::validation(
                "create_repo.name",
                v.pointer("/errors/0/message")
                    .and_then(Value::as_str)
                    .unwrap_or("GitHub rejected this repository name"),
            )));
        }
        if status != 201 {
            let msg = v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(internal(format!("create repo: {status} {msg}")));
        }
        Ok(to_repo(serde_json::from_value(v)?))
    }
}

fn to_repo(r: RepoJson) -> GithubRepo {
    GithubRepo {
        owner: r.owner.login,
        name: r.name,
        full_name: r.full_name,
        private: r.private,
        default_branch: r.default_branch.unwrap_or_else(|| "main".to_owned()),
        description: r.description,
        pushed_at: r.pushed_at,
        project_id: None,
    }
}

/// `2026-10-17T14:00:00Z` → milliseconds.
pub fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let d = js_sys::Date::parse(s);
    (!d.is_nan()).then_some(d as i64)
}
