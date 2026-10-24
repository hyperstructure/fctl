//! Instance configuration from Worker variables and secrets.
//!
//! | Name | Kind | Purpose |
//! |---|---|---|
//! | `FCTL_PUBLIC_URL` | var | Origin of the app, e.g. `https://app.fctl.sh` |
//! | `FCTL_INSTANCE_NAME` | var | Shown in the UI and emails |
//! | `FCTL_DEV` | var | `1` in local development (relaxed origin checks, dev endpoints) |
//! | `FCTL_EARLY_ACCESS` | var | `1` where people without an account ask for early access (app.fctl.sh) |
//! | `FCTL_SEARCH_ENGINES` | var | `1` where search engines may index the public pages (app.fctl.sh) |
//! | `FCTL_MASTER_KEY` | secret | 32 random bytes, base64; encrypts credentials and signs state |
//! | `FCTL_SETUP_TOKEN` | secret | One-time token for creating the owner account |
//! | `GITHUB_APP_ID`, `GITHUB_APP_SLUG`, `GITHUB_CLIENT_ID` | var | The GitHub App |
//! | `GITHUB_APP_PRIVATE_KEY` | secret | Its private key (PKCS#8 PEM) |
//! | `GITHUB_CLIENT_SECRET` | secret | Its client secret: signing in to GitHub to connect accounts |
//! | `GITHUB_WEBHOOK_SECRET` | secret | Its webhook's secret: GitHub tells the control plane when pull requests and their checks change |
//! | `GITHUB_DEFAULT_OWNER` | var | Organization new repositories are created in |
//! | `LINEAR_CLIENT_ID` | var | fctl's Linear app (optional): its OAuth client |
//! | `LINEAR_CLIENT_SECRET` | secret | Its client secret |
//! | `LINEAR_WEBHOOK_SECRET` | secret | Its webhook's signing secret: Linear tells the control plane when people delegate issues to fctl |
//! | `LINEAR_API_URL`, `LINEAR_AUTHORIZE_URL` | var | Linear's API and sign-in page, for a stand-in in development |
//! | `GOOGLE_CLIENT_ID` | var | Google OAuth client (optional) |
//! | `GOOGLE_CLIENT_SECRET` | secret | Its secret |
//! | `RESEND_API_KEY` | secret | Transactional email (optional) |
//! | `FCTL_EMAIL_FROM` | var | Sender address for email |

use base64::Engine;
use worker::Env;

#[derive(Clone)]
pub struct GithubConfig {
    pub app_id: String,
    pub slug: String,
    /// The app's OAuth client, which signs people in to GitHub when they
    /// connect an account to a workspace (`api::github`).
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub private_key_pem: String,
    pub default_owner: Option<String>,
    /// The secret GitHub signs the app's webhook deliveries with: with it,
    /// the control plane takes them (`api::webhooks`), and keeps sessions'
    /// pull requests current from GitHub (`pull_requests`).
    pub webhook_secret: Option<String>,
}

impl GithubConfig {
    /// The client ID and secret, when people can sign in to GitHub to
    /// connect accounts.
    pub fn client(&self) -> Option<(&str, &str)> {
        Some((self.client_id.as_deref()?, self.client_secret.as_deref()?))
    }
}

/// fctl's Linear app (docs/architecture.md#linear): an OAuth client that
/// workspaces install in their Linear workspaces, as an agent people there
/// delegate issues to, and that people sign in with to link their Linear
/// accounts.
#[derive(Clone)]
pub struct LinearConfig {
    pub client_id: String,
    pub client_secret: String,
    /// The secret Linear signs the app's webhook deliveries with: without
    /// it, the control plane takes none, and nothing delegated in Linear
    /// reaches fctl.
    pub webhook_secret: Option<String>,
    /// `https://api.linear.app`, or a stand-in's.
    pub api_url: String,
    /// `https://linear.app/oauth/authorize`, or a stand-in's.
    pub authorize_url: String,
}

#[derive(Clone)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Clone)]
pub struct EmailConfig {
    pub resend_api_key: String,
    pub from: String,
}

#[derive(Clone)]
pub struct Config {
    pub instance_name: String,
    /// `https://app.example.com` (no trailing slash).
    pub public_url: String,
    pub dev: bool,
    /// The sign-in page lets people without an account ask for early
    /// access, and admins invite them from the list (`api::early_access`).
    pub early_access: bool,
    /// Search engines may index the public pages: `/robots.txt` lets them
    /// and `/sitemap.xml` lists them (`fctl_domain::crawlers`).
    pub search_engines: bool,
    pub master_key: Option<[u8; 32]>,
    pub setup_token: Option<String>,
    pub github: Option<GithubConfig>,
    pub linear: Option<LinearConfig>,
    pub google: Option<GoogleConfig>,
    pub email: Option<EmailConfig>,
}

fn var(env: &Env, name: &str) -> Option<String> {
    env.var(name)
        .ok()
        .map(|v| v.to_string())
        .or_else(|| env.secret(name).ok().map(|v| v.to_string()))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

impl Config {
    pub fn from_env(env: &Env) -> Self {
        let public_url = var(env, "FCTL_PUBLIC_URL")
            .unwrap_or_else(|| "http://localhost:8787".to_owned())
            .trim_end_matches('/')
            .to_owned();
        let master_key = var(env, "FCTL_MASTER_KEY").and_then(|k| {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(k.as_bytes())
                .ok()?;
            <[u8; 32]>::try_from(bytes.as_slice()).ok()
        });
        let github = match (
            var(env, "GITHUB_APP_ID"),
            var(env, "GITHUB_APP_PRIVATE_KEY"),
        ) {
            (Some(app_id), Some(private_key_pem)) => Some(GithubConfig {
                app_id,
                slug: var(env, "GITHUB_APP_SLUG").unwrap_or_else(|| "fctl".to_owned()),
                client_id: var(env, "GITHUB_CLIENT_ID"),
                client_secret: var(env, "GITHUB_CLIENT_SECRET"),
                private_key_pem,
                default_owner: var(env, "GITHUB_DEFAULT_OWNER"),
                webhook_secret: var(env, "GITHUB_WEBHOOK_SECRET"),
            }),
            _ => None,
        };
        let linear = match (
            var(env, "LINEAR_CLIENT_ID"),
            var(env, "LINEAR_CLIENT_SECRET"),
        ) {
            (Some(client_id), Some(client_secret)) => Some(LinearConfig {
                client_id,
                client_secret,
                webhook_secret: var(env, "LINEAR_WEBHOOK_SECRET"),
                api_url: var(env, "LINEAR_API_URL")
                    .unwrap_or_else(|| fctl_domain::linear::API_URL.to_owned())
                    .trim_end_matches('/')
                    .to_owned(),
                authorize_url: var(env, "LINEAR_AUTHORIZE_URL")
                    .unwrap_or_else(|| fctl_domain::linear::AUTHORIZE_URL.to_owned()),
            }),
            _ => None,
        };
        let google = match (
            var(env, "GOOGLE_CLIENT_ID"),
            var(env, "GOOGLE_CLIENT_SECRET"),
        ) {
            (Some(client_id), Some(client_secret)) => Some(GoogleConfig {
                client_id,
                client_secret,
            }),
            _ => None,
        };
        let email = var(env, "RESEND_API_KEY").map(|resend_api_key| EmailConfig {
            resend_api_key,
            from: var(env, "FCTL_EMAIL_FROM")
                .unwrap_or_else(|| "fctl <noreply@fctl.sh>".to_owned()),
        });
        Self {
            instance_name: var(env, "FCTL_INSTANCE_NAME").unwrap_or_else(|| "fctl".to_owned()),
            public_url,
            dev: var(env, "FCTL_DEV").is_some_and(|v| v == "1" || v == "true"),
            early_access: var(env, "FCTL_EARLY_ACCESS").is_some_and(|v| v == "1" || v == "true"),
            search_engines: var(env, "FCTL_SEARCH_ENGINES")
                .is_some_and(|v| v == "1" || v == "true"),
            master_key,
            setup_token: var(env, "FCTL_SETUP_TOKEN"),
            github,
            linear,
            google,
            email,
        }
    }

    /// The origin the browser sends in `Origin` (scheme://host[:port]).
    pub fn origin(&self) -> &str {
        &self.public_url
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.public_url, path)
    }

    /// The app's host name, without its scheme and port (`app.fctl.sh`).
    pub fn host(&self) -> &str {
        let rest = self
            .public_url
            .split_once("://")
            .map_or(self.public_url.as_str(), |(_, rest)| rest);
        rest.split([':', '/']).next().unwrap_or_default()
    }

    /// Whether cookies can carry the `Secure` attribute (always, except plain
    /// http outside localhost).
    pub fn secure_cookies(&self) -> bool {
        fctl_domain::cookies::secure(&self.public_url)
    }
}
