//! GitHub App client: app JWTs, installation tokens, repositories, and
//! the GitHub sign-in that shows what someone may connect of an account.
//!
//! Sessions get installation tokens for their project's repositories, valid
//! for an hour: one that can write (the primary repository and the other
//! writable ones) and one that can only read. Hosts ask for fresh ones
//! through the runner connection.

use crate::config::GithubConfig;
use crate::crypto;
use crate::error::{AppError, AppResult, internal, not_configured, unavailable};
use base64::Engine;
use fctl_domain::github::Seen;
use fctl_domain::repos::{self, ReadScope, TokenPlan};
use fctl_domain::threads;
use fctl_proto::api::GithubRepo;
use fctl_proto::runner::GithubCredential;
use fctl_proto::session::{GithubAccess, RepoRef};
use fctl_proto::{ApiError, ErrorCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use worker::{Fetch, Headers, Method, Request, RequestInit};

const API: &str = "https://api.github.com";
/// Where people sign in to GitHub.
const WEB: &str = "https://github.com";

thread_local! {
    /// owner (lowercase) → its installation, and when it was found, per isolate.
    static INSTALLATIONS: RefCell<HashMap<String, (Found, i64)>> = RefCell::new(HashMap::new());
    /// installation id → the token the control plane looks pull requests up
    /// with, per isolate, while it has a while left.
    static LOOKUP_TOKENS: RefCell<HashMap<u64, GithubCredential>> = RefCell::new(HashMap::new());
}

/// How long a cached lookup token must still be valid to be used.
const TOKEN_MARGIN_MS: i64 = 5 * 60 * 1000;

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
    #[serde(default)]
    repository_selection: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Deserialize)]
struct Account {
    login: String,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    id: Option<u64>,
}

/// The installation of the GitHub App on a login, and GitHub's id of the
/// account that has the login now.
#[derive(Clone, Copy, Debug)]
pub struct Found {
    pub id: u64,
    pub account_id: Option<u64>,
}

/// An installation of the GitHub App: on which account, and how much of it.
#[derive(Clone, Debug)]
pub struct InstallationInfo {
    pub id: u64,
    /// The account's login, as GitHub spells it.
    pub account: String,
    pub account_id: Option<u64>,
    /// `Organization` or `User`.
    pub account_type: Option<String>,
    /// `all` or `selected`.
    pub repository_selection: Option<String>,
    pub created_at: Option<i64>,
}

impl Installation {
    fn info(self) -> Option<InstallationInfo> {
        let account = self.account?;
        Some(InstallationInfo {
            id: self.id,
            account: account.login,
            account_id: account.id,
            account_type: account.kind,
            repository_selection: self.repository_selection,
            created_at: self.created_at.as_deref().and_then(parse_rfc3339_ms),
        })
    }
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

/// A repository as someone signed in to GitHub sees it.
#[derive(Deserialize)]
struct SeenJson {
    id: u64,
    name: String,
    #[serde(default)]
    permissions: Option<SeenPermissions>,
}

#[derive(Deserialize)]
struct SeenPermissions {
    #[serde(default)]
    admin: bool,
}

#[derive(Deserialize)]
struct SeenList {
    repositories: Vec<SeenJson>,
}

#[derive(Deserialize)]
struct InstallationList {
    installations: Vec<Installation>,
}

/// Who someone signed in to GitHub as.
#[derive(Clone, Debug, Deserialize)]
pub struct Viewer {
    pub login: String,
    pub id: u64,
}

#[derive(Deserialize)]
struct UserToken {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// A session's tokens (see [`TokenPlan`]).
#[derive(Default)]
pub struct SessionTokens {
    pub write: Option<GithubCredential>,
    pub read: Option<GithubCredential>,
    /// Listed repositories the installation cannot reach, left out of the
    /// tokens.
    pub unreachable: Vec<String>,
    /// Listed repositories the session's workspace may not use, left out
    /// too (`*` for an owner-wide grant).
    pub refused: Vec<String>,
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

    /// The installation on an account (organization or user), by its
    /// login, and the id of the account that has the login now.
    pub async fn installation(&self, owner: &str) -> AppResult<Found> {
        let key = owner.to_lowercase();
        if let Some(found) = INSTALLATIONS.with(|m| {
            m.borrow()
                .get(&key)
                .filter(|(_, at)| self.now - at < 10 * 60 * 1000)
                .map(|(found, _)| *found)
        }) {
            return Ok(found);
        }
        for path in [
            format!("/orgs/{owner}/installation"),
            format!("/users/{owner}/installation"),
        ] {
            let (status, v) = self.app_call(Method::Get, &path, None).await?;
            if status == 200 {
                let inst: Installation = serde_json::from_value(v)?;
                let found = Found {
                    id: inst.id,
                    account_id: inst.account.and_then(|a| a.id),
                };
                INSTALLATIONS.with(|m| m.borrow_mut().insert(key.clone(), (found, self.now)));
                return Ok(found);
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

    /// The installation id for an account (organization or user).
    pub async fn installation_id(&self, owner: &str) -> AppResult<u64> {
        Ok(self.installation(owner).await?.id)
    }

    /// An installation by its id, while it exists.
    pub async fn installation_by_id(&self, id: u64) -> AppResult<Option<InstallationInfo>> {
        let (status, v) = self
            .app_call(Method::Get, &format!("/app/installations/{id}"), None)
            .await?;
        match status {
            200 => Ok(serde_json::from_value::<Installation>(v)?.info()),
            404 => Ok(None),
            _ => Err(unavailable("GitHub did not answer")),
        }
    }

    /// Every installation of the app.
    pub async fn installation_list(&self) -> AppResult<Vec<InstallationInfo>> {
        let mut out = Vec::new();
        for page in 1..=10 {
            let (status, v) = self
                .app_call(
                    Method::Get,
                    &format!("/app/installations?per_page=100&page={page}"),
                    None,
                )
                .await?;
            if status != 200 {
                return Err(unavailable("GitHub did not list the app's installations"));
            }
            let list: Vec<Installation> = serde_json::from_value(v)?;
            let n = list.len();
            out.extend(list.into_iter().filter_map(Installation::info));
            if n < 100 {
                break;
            }
        }
        Ok(out)
    }

    /// The installation on `owner`, when the app is installed there.
    pub async fn installation_on(&self, owner: &str) -> AppResult<Option<InstallationInfo>> {
        for path in [
            format!("/orgs/{owner}/installation"),
            format!("/users/{owner}/installation"),
        ] {
            let (status, v) = self.app_call(Method::Get, &path, None).await?;
            if status == 200 {
                let inst: Installation = serde_json::from_value(v)?;
                return Ok(inst.info());
            }
        }
        Ok(None)
    }

    /// The repositories an installation reaches.
    pub async fn repos_of(&self, inst: u64) -> AppResult<Vec<GithubRepo>> {
        let token = self.installation_token(inst, None, None).await?;
        let mut out = Vec::new();
        for page in 1..=10 {
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
        Ok(out)
    }

    /// The repositories an installation reaches, by id: none when they are
    /// more than its first 3,000, which offer nobody every repository
    /// (`fctl_domain::github::offer`).
    pub async fn reached(&self, inst: u64) -> AppResult<Vec<u64>> {
        #[derive(Deserialize)]
        struct Reached {
            id: u64,
        }
        #[derive(Deserialize)]
        struct ReachedList {
            repositories: Vec<Reached>,
        }
        let token = self.installation_token(inst, None, None).await?;
        let mut out = Vec::new();
        for page in 1..=30 {
            let (status, v) = self
                .call(
                    Method::Get,
                    &format!("/installation/repositories?per_page=100&page={page}"),
                    &format!("token {}", token.token),
                    None,
                )
                .await?;
            if status != 200 {
                return Err(unavailable(
                    "GitHub did not list the installation's repositories",
                ));
            }
            let list: ReachedList = serde_json::from_value(v)?;
            let n = list.repositories.len();
            out.extend(list.repositories.into_iter().map(|r| r.id));
            if n < 100 {
                return Ok(out);
            }
        }
        Ok(Vec::new())
    }

    // ───────────────────────── signing in to GitHub ─────────────────────────

    /// The app's OAuth client.
    fn client(&self) -> AppResult<(&str, &str)> {
        self.cfg.client().ok_or_else(|| {
            not_configured("Connecting GitHub accounts takes the GitHub App's client ID and secret")
        })
    }

    /// Where people sign in to GitHub with the app, which sends them back
    /// to `redirect_uri` with a code and `state` (PKCE: `challenge`).
    pub fn authorize_url(
        &self,
        redirect_uri: &str,
        state: &str,
        challenge: &str,
    ) -> AppResult<String> {
        let (client_id, _) = self.client()?;
        let mut url = url::Url::parse(&format!("{WEB}/login/oauth/authorize"))
            .map_err(crate::error::internal)?;
        url.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("allow_signup", "false");
        Ok(url.into())
    }

    /// Trades the code GitHub sent back for a user access token, which
    /// acts as the person within the app's permissions; `None` when GitHub
    /// refuses the code. fctl uses it once and revokes it ([`Self::revoke`]).
    pub async fn exchange(
        &self,
        code: &str,
        redirect_uri: &str,
        verifier: &str,
    ) -> AppResult<Option<String>> {
        let (client_id, client_secret) = self.client()?;
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", client_id)
            .append_pair("client_secret", client_secret)
            .append_pair("code", code)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("code_verifier", verifier)
            .finish();
        let headers = Headers::new();
        headers.set("Accept", "application/json")?;
        headers.set("Content-Type", "application/x-www-form-urlencoded")?;
        headers.set("User-Agent", "fctl-control")?;
        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_headers(headers)
            .with_body(Some(form.into()));
        let req = Request::new_with_init(&format!("{WEB}/login/oauth/access_token"), &init)?;
        let mut res = Fetch::Request(req).send().await?;
        let status = res.status_code();
        let text = res.text().await.unwrap_or_default();
        // GitHub answers a refused code with 200 and an error.
        let answer: Option<UserToken> = serde_json::from_str(&text).ok();
        match answer {
            Some(UserToken {
                access_token: Some(token),
                ..
            }) if status == 200 => Ok(Some(token)),
            other => {
                worker::console_error!(
                    "GitHub refused a sign-in code: {status} {}",
                    other.and_then(|t| t.error).unwrap_or_default()
                );
                Ok(None)
            }
        }
    }

    /// Who a user access token is for.
    pub async fn viewer(&self, token: &str) -> AppResult<Viewer> {
        let (status, v) = self
            .call(Method::Get, "/user", &format!("Bearer {token}"), None)
            .await?;
        if status != 200 {
            return Err(unavailable("GitHub did not say who signed in"));
        }
        Ok(serde_json::from_value(v)?)
    }

    /// The installations of the app that the person a user access token is
    /// for can see: those on their own account, and those that reach a
    /// repository they can see. Nobody learns from it where else the app
    /// is installed.
    pub async fn user_installations(&self, token: &str) -> AppResult<Vec<InstallationInfo>> {
        let mut out = Vec::new();
        for page in 1..=10 {
            let (status, v) = self
                .call(
                    Method::Get,
                    &format!("/user/installations?per_page=100&page={page}"),
                    &format!("Bearer {token}"),
                    None,
                )
                .await?;
            if status != 200 {
                return Err(unavailable(
                    "GitHub did not list the installations you can see",
                ));
            }
            let list: InstallationList = serde_json::from_value(v)?;
            let n = list.installations.len();
            out.extend(
                list.installations
                    .into_iter()
                    .filter_map(Installation::info),
            );
            if n < 100 {
                break;
            }
        }
        Ok(out)
    }

    /// The repositories of installation `inst` that the person a user
    /// access token is for can see, and whether they are an admin of each:
    /// none when they can't see the installation.
    pub async fn seen(&self, token: &str, inst: u64) -> AppResult<Vec<Seen>> {
        let mut out = Vec::new();
        for page in 1..=30 {
            let (status, v) = self
                .call(
                    Method::Get,
                    &format!("/user/installations/{inst}/repositories?per_page=100&page={page}"),
                    &format!("Bearer {token}"),
                    None,
                )
                .await?;
            if status == 404 {
                break;
            }
            if status != 200 {
                return Err(unavailable(
                    "GitHub did not list the repositories you can see there",
                ));
            }
            let list: SeenList = serde_json::from_value(v)?;
            let n = list.repositories.len();
            out.extend(list.repositories.into_iter().map(|r| Seen {
                id: r.id,
                name: r.name,
                admin: r.permissions.is_some_and(|p| p.admin),
            }));
            if n < 100 {
                break;
            }
        }
        Ok(out)
    }

    /// Revokes a user access token: fctl keeps none.
    pub async fn revoke(&self, token: &str) -> AppResult<()> {
        let (client_id, client_secret) = self.client()?;
        let basic = base64::engine::general_purpose::STANDARD
            .encode(format!("{client_id}:{client_secret}"));
        let (status, _) = self
            .call(
                Method::Delete,
                &format!("/applications/{client_id}/token"),
                &format!("Basic {basic}"),
                Some(json!({ "access_token": token })),
            )
            .await?;
        if status != 204 {
            worker::console_error!("revoking a GitHub sign-in: {status}");
        }
        Ok(())
    }

    /// Asks for a token; GitHub's refusal comes back as `Ok(Err((status, message)))`.
    async fn mint(
        &self,
        installation_id: u64,
        repos: Option<&[&str]>,
        permissions: Option<Value>,
    ) -> AppResult<Result<GithubCredential, (u16, String)>> {
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
                .unwrap_or("unknown error")
                .to_owned();
            worker::console_error!("installation token failed: {status} {msg}");
            return Ok(Err((status, msg)));
        }
        let t: TokenResponse = serde_json::from_value(v)?;
        let expires_at = parse_rfc3339_ms(&t.expires_at).unwrap_or(self.now + 55 * 60 * 1000);
        Ok(Ok(GithubCredential {
            token: t.token,
            expires_at,
        }))
    }

    /// A token for one installation, optionally narrowed to repositories and permissions.
    pub async fn installation_token(
        &self,
        installation_id: u64,
        repos: Option<&[&str]>,
        permissions: Option<Value>,
    ) -> AppResult<GithubCredential> {
        self.mint(installation_id, repos, permissions)
            .await?
            .map_err(|(_, msg)| refused(&msg))
    }

    /// Tokens for a session's repositories, whose agent may do `access` on
    /// GitHub in those it may write to. A listed repository the
    /// installation cannot reach (it left the installation, was renamed or
    /// deleted) is left out and reported, so one stale entry does not stop
    /// every session; the caller decides about the primary repository.
    pub async fn session_tokens(
        &self,
        inst: u64,
        plan: &TokenPlan,
        access: GithubAccess,
    ) -> AppResult<SessionTokens> {
        let mut listed = plan.write.clone();
        if let ReadScope::Repos(names) = &plan.read {
            listed.extend(names.iter().cloned());
        }
        let mut reach = Reach {
            listed,
            reachable: None,
            unreachable: Vec::new(),
        };
        let write = self
            .listed_token(inst, &plan.write, repos::permissions(access), &mut reach)
            .await?;
        let read = match &plan.read {
            ReadScope::None => None,
            ReadScope::All => Some(
                self.installation_token(inst, None, Some(repos::read_permissions()))
                    .await?,
            ),
            ReadScope::Repos(names) => {
                self.listed_token(inst, names, repos::read_permissions(), &mut reach)
                    .await?
            }
        };
        Ok(SessionTokens {
            write,
            read,
            unreachable: reach.unreachable,
            refused: Vec::new(),
        })
    }

    /// A token for `names`, without the ones the installation cannot reach.
    async fn listed_token(
        &self,
        inst: u64,
        names: &[String],
        permissions: Value,
        reach: &mut Reach,
    ) -> AppResult<Option<GithubCredential>> {
        if names.is_empty() {
            return Ok(None);
        }
        let all: Vec<&str> = names.iter().map(String::as_str).collect();
        let msg = match self
            .mint(inst, Some(&all), Some(permissions.clone()))
            .await?
        {
            Ok(token) => return Ok(Some(token)),
            // "There is at least one repository that does not exist or is
            // not accessible to the parent installation."
            Err((422, msg)) => msg,
            Err((_, msg)) => return Err(refused(&msg)),
        };
        let reachable = match &reach.reachable {
            Some(r) => r,
            None => reach
                .reachable
                .insert(self.reachable(inst, &reach.listed).await?),
        };
        let (keep, gone): (Vec<&str>, Vec<&str>) = all
            .iter()
            .partition(|n| reachable.contains(&n.to_lowercase()));
        if gone.is_empty() {
            return Err(refused(&msg));
        }
        reach
            .unreachable
            .extend(gone.iter().map(|n| (*n).to_owned()));
        if keep.is_empty() {
            return Ok(None);
        }
        Ok(Some(
            self.installation_token(inst, Some(&keep), Some(permissions))
                .await?,
        ))
    }

    /// Which of `names` (on the installation's account) the installation can
    /// reach, lowercase.
    async fn reachable(&self, inst: u64, names: &[String]) -> AppResult<HashSet<String>> {
        let wanted: HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();
        let token = self.installation_token(inst, None, None).await?;
        let mut found = HashSet::new();
        for page in 1..=30 {
            let (status, v) = self
                .call(
                    Method::Get,
                    &format!("/installation/repositories?per_page=100&page={page}"),
                    &format!("token {}", token.token),
                    None,
                )
                .await?;
            if status != 200 {
                return Err(unavailable(
                    "GitHub did not list the installation's repositories",
                ));
            }
            let list: RepoList = serde_json::from_value(v)?;
            let n = list.repositories.len();
            found.extend(
                list.repositories
                    .into_iter()
                    .map(|r| r.name.to_lowercase())
                    .filter(|name| wanted.contains(name)),
            );
            if n < 100 || found.len() == wanted.len() {
                break;
            }
        }
        Ok(found)
    }

    /// A token of installation `inst` for the control plane's own lookups,
    /// minted once while it stays valid.
    async fn lookup_token(&self, inst: u64) -> AppResult<String> {
        let cached = LOOKUP_TOKENS.with(|m| {
            m.borrow()
                .get(&inst)
                .filter(|t| t.expires_at - self.now > TOKEN_MARGIN_MS)
                .map(|t| t.token.clone())
        });
        if let Some(token) = cached {
            return Ok(token);
        }
        let fresh = self.installation_token(inst, None, None).await?;
        let token = fresh.token.clone();
        LOOKUP_TOKENS.with(|m| m.borrow_mut().insert(inst, fresh));
        Ok(token)
    }

    /// Looks pull requests up, one to `fctl_domain::webhooks::MAX_LOOKUP`
    /// of one account, with one query (`fctl_domain::webhooks::query`)
    /// through installation `inst`, or else the one on that account, and
    /// returns GitHub's answer. Fails when GitHub gives none: it refused or
    /// failed, or the installation's rate limit ran out.
    pub async fn pull_requests(
        &self,
        inst: Option<u64>,
        prs: &[(RepoRef, u64)],
    ) -> AppResult<Value> {
        let Some((repo, number)) = prs.first() else {
            return Err(internal("no pull requests to look up"));
        };
        let which = match prs.len() {
            1 => format!("{}#{number}", repo.full_name()),
            n => format!("{}#{number} and {} more", repo.full_name(), n - 1),
        };
        let inst = match inst {
            Some(i) => i,
            None => self.installation_id(&repo.owner).await?,
        };
        let token = self.lookup_token(inst).await?;
        let (status, answer) = self
            .call(
                Method::Post,
                "/graphql",
                &format!("token {token}"),
                Some(fctl_domain::webhooks::query(prs)),
            )
            .await?;
        if status == 401 {
            LOOKUP_TOKENS.with(|m| m.borrow_mut().remove(&inst));
        }
        if status != 200 {
            worker::console_error!("looking up {which}: GitHub answered {status}");
            return Err(unavailable("GitHub did not answer"));
        }
        // NOT_FOUND names a pull request or repository GitHub doesn't have.
        if let Some(errors) = answer.get("errors") {
            worker::console_error!("looking up {which}: {errors}");
        }
        if answer.get("data").is_none_or(Value::is_null) {
            return Err(unavailable("GitHub did not answer"));
        }
        Ok(answer)
    }

    /// Review thread `id`, as installation `inst` sees it: the pull
    /// request it is on, and whether it is resolved. `None` when GitHub has
    /// no thread by that id there.
    pub async fn review_thread(&self, inst: u64, id: &str) -> AppResult<Option<threads::Found>> {
        let token = self.lookup_token(inst).await?;
        let (status, answer) = self
            .call(
                Method::Post,
                "/graphql",
                &format!("token {token}"),
                Some(threads::lookup(id)),
            )
            .await?;
        if status == 401 {
            LOOKUP_TOKENS.with(|m| m.borrow_mut().remove(&inst));
        }
        if status != 200 {
            worker::console_error!("looking up a review thread: GitHub answered {status}");
            return Err(unavailable("GitHub did not answer"));
        }
        Ok(threads::found(&answer))
    }

    /// Resolves review thread `id` of `repo`, or opens it again, and says
    /// whether it is resolved now. GitHub asks for a token that may write
    /// `repo`'s contents, which could push: this one is for `repo` alone,
    /// does this alone and is revoked after it, and never leaves the
    /// control plane.
    pub async fn resolve_thread(
        &self,
        inst: u64,
        repo: &RepoRef,
        id: &str,
        unresolve: bool,
    ) -> AppResult<bool> {
        let token = self
            .installation_token(
                inst,
                Some(&[repo.name.as_str()]),
                Some(repos::resolve_permissions()),
            )
            .await?;
        let auth = format!("token {}", token.token);
        let answer = self
            .call(
                Method::Post,
                "/graphql",
                &auth,
                Some(threads::resolve(id, unresolve)),
            )
            .await;
        // An hour left on it, which nothing needs.
        if let Err(e) = self
            .call(Method::Delete, "/installation/token", &auth, None)
            .await
        {
            worker::console_error!("revoking a token: {}", e.0.message);
        }
        let (status, answer) = answer?;
        if status != 200 {
            worker::console_error!("resolving a review thread: GitHub answered {status}");
            return Err(unavailable("GitHub did not answer"));
        }
        threads::resolved(&answer).map_err(|message| {
            AppError(ApiError::new(
                ErrorCode::Forbidden,
                format!("GitHub didn't change the review thread: {message}"),
            ))
        })
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

/// The listed repositories of a session, and which the installation reaches.
struct Reach {
    listed: Vec<String>,
    reachable: Option<HashSet<String>>,
    unreachable: Vec<String>,
}

fn refused(msg: &str) -> AppError {
    unavailable(&format!("GitHub refused a token: {msg}"))
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
