//! The fctl control plane: a Cloudflare Worker (Rust → wasm) with Durable
//! Objects. See docs/architecture.md.

mod access;
mod api;
mod auth;
mod config;
mod crypto;
mod ctx;
mod db;
mod do_files;
mod do_fleet;
mod do_notify;
mod do_session;
mod email;
mod error;
mod github;
mod housekeeping;
mod http;
mod instance;
mod internal;
mod model;
mod notify;
mod push;
mod spec;
mod usage;

use ctx::Ctx;
pub use do_files::FilesObject;
pub use do_fleet::FleetObject;
pub use do_notify::NotifyObject;
pub use do_session::SessionObject;
use error::{AppError, AppResult};
use fctl_proto::ApiError;
use worker::{Context, Env, Method, Request, Response, ScheduleContext, ScheduledEvent, event};

#[event(fetch)]
pub async fn fetch(mut req: Request, env: Env, wctx: Context) -> worker::Result<Response> {
    let path = req.path();
    if !path.starts_with("/api/") {
        // Static assets are served before the Worker runs; anything that
        // reaches us here is the SPA fallback.
        return match env.assets("ASSETS") {
            Ok(assets) => assets.fetch_request(req).await,
            Err(_) => Response::error("Not found", 404),
        };
    }
    let mut ctx = match Ctx::new(env, wctx, &req) {
        Ok(c) => c,
        Err(e) => return Ok(http::error_response(&e.0)),
    };
    let result = route(&mut ctx, &mut req, &path).await;
    let mut response = match result {
        Ok(r) => r,
        Err(e) => http::error_response(&e.0),
    };
    let headers = response.headers_mut();
    for c in &ctx.cookies {
        let _ = headers.append("Set-Cookie", c);
    }
    Ok(response)
}

/// The hourly cron trigger (see `housekeeping`).
#[event(scheduled)]
pub async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    if let Err(e) = housekeeping::run(&env, ctx::now_ms()).await {
        worker::console_error!("housekeeping failed: {}", e.0.message);
    }
}

async fn route(ctx: &mut Ctx, req: &mut Request, path: &str) -> AppResult<Response> {
    let method = req.method();
    let segs: Vec<&str> = path.trim_end_matches('/').split('/').skip(3).collect();
    if !path.starts_with("/api/v1/") {
        return Err(AppError(ApiError::not_found("Endpoint")));
    }
    // Authenticate everything; anonymous endpoints simply ignore the result.
    auth::authenticate(ctx, req).await?;
    if !http::is_safe_method(&method) && ctx.auth.as_ref().is_some_and(|a| a.via_cookie) {
        http::check_same_origin(req, ctx.cfg.origin(), ctx.cfg.dev)?;
    }
    use Method::*;
    match (method, segs.as_slice()) {
        (Get, ["instance"]) => api::auth::instance(ctx).await,
        (Post, ["setup"]) => api::auth::setup(ctx, req).await,
        (Get, ["invites", token]) => api::auth::invite_preview(ctx, token).await,
        (Post, ["auth", "signup"]) => api::auth::signup(ctx, req).await,
        (Post, ["auth", "login"]) => api::auth::login(ctx, req).await,
        (Post, ["auth", "logout"]) => api::auth::logout(ctx).await,
        (Get, ["auth", "me"]) => api::auth::get_me(ctx).await,
        (Patch, ["auth", "me"]) => api::auth::update_me(ctx, req).await,
        (Post, ["auth", "password"]) => api::auth::change_password(ctx, req).await,
        (Post, ["auth", "password", "forgot"]) => api::auth::forgot_password(ctx, req).await,
        (Post, ["auth", "password", "reset"]) => api::auth::reset_password(ctx, req).await,
        (Get, ["auth", "google", "start"]) => api::google::start(ctx, req).await,
        (Get, ["auth", "google", "callback"]) => api::google::callback(ctx, req).await,
        (Delete, ["auth", "identities", id]) => api::google::unlink(ctx, id).await,
        (Get, ["auth", "tokens"]) => api::auth::list_tokens(ctx).await,
        (Post, ["auth", "tokens"]) => api::auth::create_token(ctx, req).await,
        (Delete, ["auth", "tokens", id]) => api::auth::delete_token(ctx, id).await,
        (Post, ["auth", "device"]) => api::auth::device_start(ctx, req).await,
        (Post, ["auth", "device", "token"]) => api::auth::device_poll(ctx, req).await,
        (Get, ["auth", "device", code]) => api::auth::device_info(ctx, code).await,
        (Post, ["auth", "device", code]) => api::auth::device_decide(ctx, req, code).await,
        (Get, ["health"]) => {
            http::ok(&serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
        }

        // Admin
        (Get, ["admin", "users"]) => api::admin::users(ctx).await,
        (Patch, ["admin", "users", id]) => api::admin::update_user(ctx, req, id).await,
        (Delete, ["admin", "users", id]) => api::admin::delete_user(ctx, id).await,
        (Post, ["admin", "users", id, "reset-link"]) => api::admin::reset_link(ctx, id).await,
        (Get, ["admin", "allowlist"]) => api::admin::allowlist(ctx).await,
        (Post, ["admin", "allowlist"]) => api::admin::add_allow(ctx, req).await,
        (Delete, ["admin", "allowlist", id]) => api::admin::remove_allow(ctx, id).await,
        (Post, ["admin", "invites"]) => api::admin::create_invite(ctx, req).await,
        (Delete, ["admin", "invites", id]) => api::admin::revoke_invite(ctx, id).await,
        (Get, ["admin", "audit"]) => api::admin::audit(ctx, req).await,
        (Get, ["admin", "settings"]) => api::admin::settings(ctx).await,
        (Patch, ["admin", "settings"]) => api::admin::update_settings(ctx, req).await,

        // Runners (admin) and hosts
        (Get, ["runners"]) => api::runners::list(ctx).await,
        (Post, ["runners"]) => api::runners::create(ctx, req).await,
        (Patch, ["runners", id]) => api::runners::update(ctx, req, id).await,
        (Delete, ["runners", id]) => api::runners::delete(ctx, id).await,
        (Post, ["runners", id, "token"]) => api::runners::rotate_token(ctx, id).await,
        (Get, ["runner", "connect"]) => api::runners::connect(ctx, req).await,
        (Get, ["runner", "sessions", id, "uplink"]) => api::runners::uplink(ctx, req, id).await,
        (Get, ["runner", "sessions", id, "attachments", file]) => {
            api::attachments::runner_download(ctx, req, id, file).await
        }
        (Post, ["runner", "sessions", id, "published"]) => {
            api::published::runner_publish(ctx, req, id).await
        }
        (Get, ["feed", "ws"]) => api::runners::feed(ctx, req).await,

        // Projects
        (Get, ["projects"]) => api::projects::list(ctx, req).await,
        (Post, ["projects"]) => api::projects::create(ctx, req).await,
        (Get, ["projects", id]) => api::projects::get(ctx, id).await,
        (Patch, ["projects", id]) => api::projects::update(ctx, req, id).await,
        (Delete, ["projects", id]) => api::projects::delete(ctx, id).await,
        (Get, ["projects", id, "members"]) => api::projects::members(ctx, id).await,
        (Post, ["projects", id, "members"]) => api::projects::add_member(ctx, req, id).await,
        (Patch, ["projects", id, "members", user]) => {
            api::projects::set_member_role(ctx, req, id, user).await
        }
        (Delete, ["projects", id, "members", user]) => {
            api::projects::remove_member(ctx, id, user).await
        }
        (Get, ["projects", id, "secrets"]) => api::projects::secrets(ctx, id).await,
        (Put, ["projects", id, "secrets", name]) => {
            api::projects::set_secret(ctx, req, id, name).await
        }
        (Delete, ["projects", id, "secrets", name]) => {
            api::projects::delete_secret(ctx, id, name).await
        }
        (Get, ["projects", id, "repos"]) => api::repos::list(ctx, id).await,
        (Post, ["projects", id, "repos"]) => api::repos::add(ctx, req, id).await,
        (Delete, ["projects", id, "repos", owner, name]) => {
            api::repos::remove(ctx, id, owner, name).await
        }
        (Post, ["projects", id, "sessions"]) => api::sessions::create(ctx, req, id).await,
        (Post, ["projects", id, "attachments"]) => api::attachments::upload(ctx, req, id).await,
        (Get, ["github"]) => api::projects::github_info(ctx).await,
        (Get, ["github", "repos"]) => api::projects::github_repos(ctx).await,

        // Sessions
        (Get, ["sessions"]) => api::sessions::list(ctx, req).await,
        (Get, ["sessions", id]) => api::sessions::get(ctx, id).await,
        (Patch, ["sessions", id]) => api::sessions::update(ctx, req, id).await,
        (Post, ["sessions", id, "messages"]) => api::sessions::send_message(ctx, req, id).await,
        (Post, ["sessions", id, "interrupt"]) => api::sessions::interrupt(ctx, id).await,
        (Post, ["sessions", id, "stop"]) => api::sessions::stop(ctx, id).await,
        (Post, ["sessions", id, "resume"]) => api::sessions::resume(ctx, req, id).await,
        (Get, ["sessions", id, "events"]) => api::sessions::events(ctx, req, id).await,
        (Get, ["sessions", id, "ws"]) => api::sessions::websocket(ctx, req, id).await,
        (Post, ["sessions", id, "attachments"]) => {
            api::attachments::upload_to_session(ctx, req, id).await
        }
        (Get, ["sessions", id, "attachments", file]) => {
            api::attachments::download(ctx, id, file).await
        }
        // Published screenshots: anyone with a link reads it.
        (Get | Head, ["published", token, ..]) => api::published::serve(ctx, req, token).await,
        (Delete, ["published", token, ..]) => api::published::delete(ctx, token).await,
        (Put, ["sessions", id, "follow"]) => api::sessions::follow(ctx, id).await,
        (Delete, ["sessions", id, "follow"]) => api::sessions::unfollow(ctx, id).await,
        (Get, ["sessions", id, "shares"]) => api::sessions::shares(ctx, id).await,
        (Post, ["sessions", id, "shares"]) => api::sessions::add_share(ctx, req, id).await,
        (Delete, ["sessions", id, "shares", user]) => {
            api::sessions::remove_share(ctx, id, user).await
        }

        // Claude credentials
        (Get, ["me", "claude"]) => api::claude::list(ctx).await,
        (Post, ["me", "claude", "token"]) => api::claude::add_token(ctx, req).await,
        (Post, ["me", "claude", "login"]) => api::claude::start_login(ctx).await,
        (Get, ["me", "claude", "login", id]) => api::claude::job(ctx, id).await,
        (Post, ["me", "claude", "login", id, "code"]) => {
            api::claude::submit_code(ctx, req, id).await
        }
        (Post, ["me", "claude", id, "default"]) => api::claude::set_default(ctx, id).await,
        (Delete, ["me", "claude", id]) => api::claude::remove(ctx, id).await,
        // Notifications
        (Get, ["me", "notifications"]) => api::notifications::get(ctx).await,
        (Patch, ["me", "notifications"]) => api::notifications::update(ctx, req).await,
        (Post, ["me", "notifications", "devices"]) => {
            api::notifications::add_device(ctx, req).await
        }
        (Delete, ["me", "notifications", "devices", id]) => {
            api::notifications::remove_device(ctx, id).await
        }
        (Post, ["me", "notifications", "test"]) => api::notifications::test(ctx).await,
        (Get, ["me", "export"]) => api::account::export(ctx).await,
        (Get, ["me", "deletion"]) => api::account::deletion(ctx).await,
        (Post, ["me", "delete"]) => api::account::delete(ctx, req).await,
        _ => Err(AppError(ApiError::not_found("Endpoint"))),
    }
}
