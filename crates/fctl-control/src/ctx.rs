//! Per-request context: configuration, database, caller, and response cookies.

use crate::config::Config;
use crate::crypto::{self, Keys};
use crate::db::{n, os, q, s};
use crate::error::{AppError, AppResult};
use crate::http;
use crate::model::UserRow;
use fctl_domain::limits::Limits;
use fctl_domain::verify::{self, Sensitive};
use fctl_proto::api::{InstanceSettings, TokenKind};
use fctl_proto::{ApiError, ErrorCode};
use std::future::Future;
use worker::{D1Database, Env, Request};

/// The authenticated caller.
#[derive(Clone, Debug)]
pub struct Authed {
    pub user: UserRow,
    pub token_id: String,
    pub kind: TokenKind,
    /// Authenticated by the session cookie (browser), not a bearer token.
    pub via_cookie: bool,
    /// When the token was issued: the sign-in, for a browser session.
    pub signed_in_at: i64,
    /// When it last confirmed it's its person, before sensitive actions
    /// ([`Ctx::verified`]).
    pub verified_at: Option<i64>,
    /// The instance requires two-factor of this account and it has none
    /// yet: only setting it up works (`crate::api::two_factor::gate`).
    pub two_factor_setup_required: bool,
    /// The instance's settings, read with the token.
    pub settings: InstanceSettings,
    /// What the access token is limited to, when it is
    /// (`fctl_domain::limits`).
    pub limits: Option<Limits>,
}

impl Authed {
    /// The browser session `user` just signed in with.
    pub fn signed_in(
        user: &UserRow,
        token_id: String,
        now: i64,
        settings: InstanceSettings,
        verified_at: Option<i64>,
    ) -> Self {
        Self {
            user: user.clone(),
            token_id,
            kind: TokenKind::Web,
            via_cookie: true,
            signed_in_at: now,
            verified_at,
            two_factor_setup_required: false,
            settings,
            limits: None,
        }
    }
}

pub struct Ctx {
    pub env: Env,
    pub cfg: Config,
    pub db: D1Database,
    pub now: i64,
    pub ip: Option<String>,
    pub location: Option<String>,
    pub user_agent: Option<String>,
    pub auth: Option<Authed>,
    /// `Set-Cookie` values added to the response.
    pub cookies: Vec<String>,
    wctx: worker::Context,
}

pub fn now_ms() -> i64 {
    js_sys::Date::now() as i64
}

impl Ctx {
    pub fn new(env: Env, wctx: worker::Context, req: &Request) -> AppResult<Self> {
        let cfg = Config::from_env(&env);
        let db = env.d1("DB")?;
        Ok(Self {
            cfg,
            db,
            env,
            now: now_ms(),
            ip: http::client_ip(req),
            location: http::client_location(req),
            user_agent: http::header(req, "User-Agent").map(|ua| ua.chars().take(300).collect()),
            auth: None,
            cookies: Vec::new(),
            wctx,
        })
    }

    /// The signed-in user, or `unauthorized`.
    pub fn user(&self) -> AppResult<&UserRow> {
        self.auth
            .as_ref()
            .map(|a| &a.user)
            .ok_or_else(|| AppError(ApiError::unauthorized()))
    }

    /// The signed-in user if they are an instance owner or admin.
    pub fn admin(&self) -> AppResult<&UserRow> {
        let u = self.user()?;
        if u.is_admin() {
            Ok(u)
        } else {
            Err(AppError(ApiError::forbidden("Only admins can do this")))
        }
    }

    pub fn keys(&self) -> AppResult<Keys> {
        crypto::keys(&self.cfg)
    }

    /// Runs `fut` after the response is sent (logging failures).
    pub fn defer<F>(&self, fut: F)
    where
        F: Future<Output = AppResult<()>> + 'static,
    {
        self.wctx.wait_until(async move {
            if let Err(e) = fut.await {
                worker::console_error!("deferred task failed: {}", e.0.message);
            }
        });
    }

    /// Refuses a sensitive action until the caller confirms it's them,
    /// with a passkey or a code (`POST /api/v1/auth/verify`), where the
    /// instance asks for that and they have one to confirm it with
    /// (`fctl_domain::verify`). Call it once the action is allowed and
    /// checked, right before it changes anything, so that nobody confirms
    /// for something that then fails.
    pub fn verified(&self, action: Sensitive) -> AppResult<()> {
        let a = self
            .auth
            .as_ref()
            .ok_or_else(|| AppError(ApiError::unauthorized()))?;
        let factors = a.user.factors();
        if !verify::needed(&a.settings, factors, a.verified_at, self.now) {
            return Ok(());
        }
        let mut message = verify::message(action, factors);
        if a.limits.is_some() {
            message.push_str(
                " Access tokens with limits can't: do it in the browser, or with the CLI signed in as you.",
            );
        }
        Err(AppError(ApiError::new(
            ErrorCode::VerificationRequired,
            message,
        )))
    }

    /// What the caller's access token is limited to, when it is.
    pub fn limits(&self) -> Option<&Limits> {
        self.auth.as_ref().and_then(|a| a.limits.as_ref())
    }

    /// The instance's settings, as read with the caller's token (defaults
    /// when signed out).
    pub fn settings(&self) -> InstanceSettings {
        self.auth
            .as_ref()
            .map(|a| a.settings.clone())
            .unwrap_or_default()
    }

    /// Records an entry in the instance's audit log (after the response).
    pub fn audit(&self, action: &str, target: Option<&str>, data: Option<serde_json::Value>) {
        self.audit_in(None, action, target, data);
    }

    /// Records an audit log entry (after the response): in a workspace's
    /// log, or the instance's (`None`).
    pub fn audit_in(
        &self,
        workspace_id: Option<&str>,
        action: &str,
        target: Option<&str>,
        data: Option<serde_json::Value>,
    ) {
        let db = crate::db::handle(&self.db);
        let workspace_id = workspace_id.filter(|w| !w.is_empty()).map(str::to_owned);
        let id = fctl_domain::ids::new_id("aud", self.now);
        let now = self.now;
        let actor = self.auth.as_ref().map(|a| a.user.id.clone());
        let ip = self.ip.clone();
        let action = action.to_owned();
        let target = target.map(str::to_owned);
        let data = data.map(|d| d.to_string());
        self.defer(async move {
            q(
                &db,
                "INSERT INTO audit_log (id, ts, actor_id, action, target, data, ip, workspace_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                &[s(&id), n(now), os(actor.as_deref()), s(&action), os(target.as_deref()), os(data.as_deref()), os(ip.as_deref()), os(workspace_id.as_deref())],
            )?
            .run()
            .await?;
            Ok(())
        });
    }

    /// Applies the `AUTH_LIMITER` rate limit to `key`.
    pub async fn rate_limit(&self, key: &str) -> AppResult<()> {
        let Ok(limiter) = self.env.rate_limiter("AUTH_LIMITER") else {
            return Ok(());
        };
        match limiter.limit(key.to_owned()).await {
            Ok(outcome) if !outcome.success => Err(AppError(
                ApiError::new(
                    ErrorCode::RateLimited,
                    "Too many attempts. Wait a minute and try again.",
                )
                .with_retry_after(60),
            )),
            _ => Ok(()),
        }
    }

    pub fn ip_key(&self) -> String {
        self.ip.clone().unwrap_or_else(|| "unknown".to_owned())
    }
}
