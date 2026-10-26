//! `/api/v1/auth/tokens`: the browsers, CLI sign-ins and personal access
//! tokens of the caller, and access tokens limited to what they are for
//! (`fctl_domain::limits`).

use crate::access;
use crate::auth::{self, DAY_MS};
use crate::ctx::Ctx;
use crate::db::{n, q, s};
use crate::error::{AppError, AppResult, not_found, validation};
use crate::http;
use crate::model::PROJECT_VISIBLE;
use fctl_domain::limits::{Limits, MAX_PLACES, Places};
use fctl_domain::validate;
use fctl_domain::verify::Sensitive;
use fctl_proto::TokenId;
use fctl_proto::api::*;
use serde::Deserialize;
use std::collections::HashMap;
use worker::{Request, Response};

#[derive(Deserialize)]
struct TokenRow {
    id: String,
    kind: String,
    name: Option<String>,
    user_agent: Option<String>,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    last_used_at: Option<i64>,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    expires_at: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
}

const TOKEN_COLUMNS: &str =
    "id, kind, name, user_agent, created_at, last_used_at, expires_at, scope";

/// A workspace or project as a token names it.
#[derive(Deserialize)]
struct PlaceRow {
    id: String,
    slug: String,
    name: String,
    #[serde(default)]
    workspace: Option<String>,
}

/// The workspaces and projects that tokens of `user_id` name, by id, as
/// they are now and as far as their person still sees them.
#[derive(Default)]
struct Named {
    workspaces: HashMap<String, TokenPlace>,
    projects: HashMap<String, TokenPlace>,
}

impl Named {
    async fn of(ctx: &Ctx, user_id: &str, all: &[&Limits]) -> AppResult<Self> {
        let places: Vec<&Places> = all.iter().filter_map(|l| l.places.as_ref()).collect();
        let ids = |pick: fn(&Places) -> &Vec<String>| -> String {
            let ids: Vec<&String> = places.iter().flat_map(|p| pick(p)).collect();
            serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_owned())
        };
        let mut named = Self::default();
        if places.is_empty() {
            return Ok(named);
        }
        let workspaces: Vec<PlaceRow> = q(
            &ctx.db,
            "SELECT w.id, w.slug, w.name FROM workspaces w \
             JOIN workspace_members wm ON wm.workspace_id = w.id AND wm.user_id = ?1 \
             WHERE w.id IN (SELECT value FROM json_each(?2))",
            &[s(user_id), s(&ids(|p| &p.workspaces))],
        )?
        .all()
        .await?;
        let projects: Vec<PlaceRow> = q(
            &ctx.db,
            &format!(
                "SELECT p.id, p.slug, p.name, w.name AS workspace FROM projects p \
                 JOIN workspaces w ON w.id = p.workspace_id \
                 LEFT JOIN workspace_members wm ON wm.workspace_id = p.workspace_id AND wm.user_id = ?1 \
                 LEFT JOIN project_members m ON m.project_id = p.id AND m.user_id = ?1 \
                 WHERE p.id IN (SELECT value FROM json_each(?2)) AND {PROJECT_VISIBLE}"
            ),
            &[s(user_id), s(&ids(|p| &p.projects))],
        )?
        .all()
        .await?;
        let place = |r: PlaceRow| {
            (
                r.id.clone(),
                TokenPlace {
                    id: r.id,
                    slug: Some(r.slug),
                    name: Some(r.name),
                    workspace: r.workspace,
                },
            )
        };
        named.workspaces = workspaces.into_iter().map(place).collect();
        named.projects = projects.into_iter().map(place).collect();
        Ok(named)
    }

    /// What `limits` say, with the names of their places.
    fn scope(&self, limits: &Limits) -> TokenScope {
        let list = |ids: &[String], names: &HashMap<String, TokenPlace>| {
            ids.iter()
                .map(|id| {
                    names.get(id).cloned().unwrap_or(TokenPlace {
                        id: id.clone(),
                        ..Default::default()
                    })
                })
                .collect()
        };
        TokenScope {
            access: limits.access,
            places: limits.places.as_ref().map(|p| TokenPlaces {
                workspaces: list(&p.workspaces, &self.workspaces),
                projects: list(&p.projects, &self.projects),
            }),
            github_access: limits.github_access,
        }
    }
}

impl TokenRow {
    fn limits(&self) -> Option<Limits> {
        Limits::parse(self.scope.as_deref())
    }

    fn info(self, current: &str, named: &Named) -> TokenInfo {
        let scope = self.limits().map(|l| named.scope(&l));
        TokenInfo {
            current: self.id == current,
            id: TokenId(self.id),
            kind: TokenKind::parse(&self.kind).unwrap_or(TokenKind::Pat),
            name: self.name,
            created_at: self.created_at,
            last_used_at: self.last_used_at,
            expires_at: self.expires_at,
            user_agent: self.user_agent,
            scope,
        }
    }
}

/// `GET /api/v1/auth/tokens`: where the caller is signed in, and their
/// access tokens with what each is limited to.
pub async fn list(ctx: &Ctx) -> AppResult<Response> {
    let user = ctx.user()?;
    let current = ctx
        .auth
        .as_ref()
        .map(|a| a.token_id.clone())
        .unwrap_or_default();
    let rows: Vec<TokenRow> = q(
        &ctx.db,
        &format!(
            "SELECT {TOKEN_COLUMNS} FROM tokens \
             WHERE user_id = ? AND (expires_at IS NULL OR expires_at > ?) ORDER BY COALESCE(last_used_at, created_at) DESC"
        ),
        &[s(&user.id), n(ctx.now)],
    )?
    .all()
    .await?;
    let limits: Vec<Limits> = rows.iter().filter_map(TokenRow::limits).collect();
    let named = Named::of(ctx, &user.id, &limits.iter().collect::<Vec<_>>()).await?;
    http::ok(&TokenList {
        tokens: rows.into_iter().map(|r| r.info(&current, &named)).collect(),
    })
}

/// The access token of the request, as `GET /auth/me` names it: `None`
/// for a browser or the CLI's sign-in.
pub async fn current(ctx: &Ctx) -> AppResult<Option<TokenInfo>> {
    let Some(a) = ctx.auth.as_ref().filter(|a| a.kind == TokenKind::Pat) else {
        return Ok(None);
    };
    let row: Option<TokenRow> = q(
        &ctx.db,
        &format!("SELECT {TOKEN_COLUMNS} FROM tokens WHERE id = ?"),
        &[s(&a.token_id)],
    )?
    .first()
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let limits = row.limits();
    let named = Named::of(ctx, &a.user.id, &limits.iter().collect::<Vec<_>>()).await?;
    Ok(Some(row.info(&a.token_id, &named)))
}

/// The places a new token names: workspaces the caller is a member of and
/// projects they see, by id or slug. `None` when it names none.
async fn places(
    ctx: &Ctx,
    workspaces: &[String],
    projects: &[String],
) -> AppResult<Option<Places>> {
    if workspaces.is_empty() && projects.is_empty() {
        return Ok(None);
    }
    if workspaces.len() + projects.len() > MAX_PLACES {
        return Err(validation(
            "projects",
            &format!("A token reaches at most {MAX_PLACES} workspaces and projects"),
        ));
    }
    // A blank one would leave the token reaching every project.
    if workspaces
        .iter()
        .chain(projects)
        .any(|p| p.trim().is_empty())
    {
        return Err(validation(
            "projects",
            "Name each workspace and project the token reaches, or none for all of them",
        ));
    }
    let user = ctx.user()?;
    let mut out = Places::default();
    for w in workspaces.iter().map(|w| w.trim()) {
        #[derive(Deserialize)]
        struct Id {
            id: String,
        }
        let found: Option<Id> = q(
            &ctx.db,
            "SELECT w.id FROM workspaces w JOIN workspace_members wm ON wm.workspace_id = w.id AND wm.user_id = ?1 \
             WHERE w.id = ?2 OR w.slug = ?2",
            &[s(&user.id), s(w)],
        )?
        .first()
        .await?;
        let Some(found) = found else {
            return Err(validation(
                "workspaces",
                &format!("You aren't a member of a workspace called {w}"),
            ));
        };
        if !out.workspaces.contains(&found.id) {
            out.workspaces.push(found.id);
        }
    }
    for p in projects.iter().map(|p| p.trim()) {
        let found = access::project(ctx, p)
            .await
            .map_err(|_| validation("projects", &format!("You don't see a project called {p}")))?;
        if !out.projects.contains(&found.row.id) {
            out.projects.push(found.row.id);
        }
    }
    Ok(Some(out))
}

/// `POST /api/v1/auth/tokens`: a personal access token, limited to what
/// the request asks.
pub async fn create(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: CreateTokenRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    let name = validate::name("name", &body.name)?;
    let expires_at = match body.expires_in_days {
        Some(0) | None => None,
        Some(d) if d <= 3650 => Some(ctx.now + d as i64 * DAY_MS),
        Some(_) => return Err(validation("expires_in_days", "Use at most 3650 days")),
    };
    let access = body.access.unwrap_or_default();
    fctl_domain::limits::check(access, body.github_access).map_err(AppError)?;
    let places = places(ctx, &body.workspaces, &body.projects).await?;
    let limits = Limits::new(access, places, body.github_access);
    ctx.verified(Sensitive::NewToken)?;
    let (id, secret) = auth::issue_token(
        ctx,
        &user.id,
        TokenKind::Pat,
        Some(&name),
        expires_at,
        limits.as_ref(),
        None,
    )
    .await?;
    let scope = match &limits {
        Some(l) => Some(Named::of(ctx, &user.id, &[l]).await?.scope(l)),
        None => None,
    };
    ctx.audit(
        "token.created",
        Some(&id),
        Some(serde_json::json!({ "name": name, "scope": limits })),
    );
    http::created(&CreatedToken {
        token: TokenInfo {
            id: TokenId(id),
            kind: TokenKind::Pat,
            name: Some(name),
            created_at: ctx.now,
            last_used_at: None,
            expires_at,
            user_agent: None,
            current: false,
            scope,
        },
        secret,
    })
}

/// `DELETE /api/v1/auth/tokens/{id}`: signs a browser or the CLI out, or
/// revokes an access token.
pub async fn delete(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let changed = q(
        &ctx.db,
        "DELETE FROM tokens WHERE id = ? AND user_id = ?",
        &[s(id), s(&user.id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(not_found("Token"));
    }
    if ctx
        .auth
        .as_ref()
        .is_some_and(|a| a.token_id == id && a.via_cookie)
    {
        auth::clear_session_cookie(ctx);
    }
    http::ok(&Ok { ok: true })
}

/// What a limited token sees of its person (`GET /auth/me`): who they are,
/// and the token. Their other addresses, sign-in methods, second factors
/// and Claude connections stay with tokens without limits.
pub fn limited_me(user: &crate::model::UserRow, token: Option<TokenInfo>) -> Me {
    Me {
        user: User {
            emails: Vec::new(),
            has_password: false,
            two_factor: false,
            passkeys: 0,
            last_login_at: None,
            last_active_at: None,
            workspaces: 0,
            ..user.to_user()
        },
        identities: Vec::new(),
        claude: Vec::new(),
        commit_email: None,
        two_factor: TwoFactor::default(),
        passkeys: Vec::new(),
        emails: Vec::new(),
        token,
    }
}
