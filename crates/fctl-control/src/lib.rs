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
mod do_schedule;
mod do_session;
mod email;
mod error;
mod follow_ups;
mod github;
mod housekeeping;
mod http;
mod instance;
mod internal;
mod model;
mod notify;
mod pull_requests;
mod push;
mod reviews;
mod schedules;
mod spec;
mod usage;
mod workspaces;

use ctx::Ctx;
pub use do_files::FilesObject;
pub use do_fleet::FleetObject;
pub use do_notify::NotifyObject;
pub use do_schedule::ScheduleObject;
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
    // Accounts the instance requires two-factor of, without it, only set it up.
    if ctx
        .auth
        .as_ref()
        .is_some_and(|a| a.two_factor_setup_required)
        && !api::two_factor::open_while_required(&method, segs.as_slice())
    {
        return Err(api::two_factor::setup_required_error(ctx));
    }
    // Limited access tokens reach projects and sessions only
    // (`fctl_domain::limits`); their roles there are capped as well.
    if let Some(limits) = ctx.limits() {
        fctl_domain::limits::allows(limits.access, method.as_ref(), segs.as_slice())
            .map_err(AppError)?;
    }
    use Method::*;
    match (method, segs.as_slice()) {
        (Get, ["instance"]) => api::auth::instance(ctx).await,
        (Post, ["setup"]) => api::auth::setup(ctx, req).await,
        (Get, ["invites", token]) => api::auth::invite_preview(ctx, token).await,
        (Post, ["access-requests"]) => api::early_access::request(ctx, req).await,
        (Post, ["access-requests", "withdraw"]) => api::early_access::withdraw(ctx, req).await,
        (Post, ["invites", token, "connect"]) => api::emails::connect_invite(ctx, token).await,
        (Post, ["auth", "signup"]) => api::auth::signup(ctx, req).await,
        (Post, ["auth", "login"]) => api::auth::login(ctx, req).await,
        (Post, ["auth", "logout"]) => api::auth::logout(ctx).await,
        (Get, ["auth", "me"]) => api::auth::get_me(ctx).await,
        (Patch, ["auth", "me"]) => api::auth::update_me(ctx, req).await,
        (Post, ["auth", "password"]) => api::auth::change_password(ctx, req).await,
        (Post, ["auth", "password", "forgot"]) => api::auth::forgot_password(ctx, req).await,
        (Post, ["auth", "password", "reset"]) => api::auth::reset_password(ctx, req).await,
        (Get, ["auth", "two-factor", "challenge"]) => api::two_factor::current(ctx, req).await,
        (Post, ["auth", "two-factor", "verify"]) => api::two_factor::verify(ctx, req).await,
        (Post, ["auth", "two-factor", "cancel"]) => api::two_factor::cancel(ctx, req).await,
        (Post, ["auth", "two-factor", "passkey"]) => {
            api::passkeys::second_factor_options(ctx, req).await
        }
        (Get, ["auth", "verify"]) => api::verify::status(ctx).await,
        (Post, ["auth", "verify"]) => api::verify::verify(ctx, req).await,
        (Post, ["auth", "verify", "passkey"]) => api::passkeys::verify_options(ctx).await,
        (Post, ["auth", "verify", "device"]) => api::verify::device(ctx, req).await,
        (Post, ["auth", "passkey", "options"]) => api::passkeys::sign_in_options(ctx).await,
        (Post, ["auth", "passkey"]) => api::passkeys::sign_in(ctx, req).await,
        (Get, ["auth", "google", "start"]) => api::google::start(ctx, req).await,
        (Get, ["auth", "google", "callback"]) => api::google::callback(ctx, req).await,
        (Delete, ["auth", "identities", id]) => api::google::unlink(ctx, id).await,
        (Get, ["auth", "tokens"]) => api::tokens::list(ctx).await,
        (Post, ["auth", "tokens"]) => api::tokens::create(ctx, req).await,
        (Delete, ["auth", "tokens", id]) => api::tokens::delete(ctx, id).await,
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
        (Delete, ["admin", "users", id, "two-factor"]) => {
            api::two_factor::admin_reset(ctx, id).await
        }
        (Get, ["admin", "allowlist"]) => api::admin::allowlist(ctx).await,
        (Post, ["admin", "allowlist"]) => api::admin::add_allow(ctx, req).await,
        (Delete, ["admin", "allowlist", id]) => api::admin::remove_allow(ctx, id).await,
        (Post, ["admin", "invites"]) => api::admin::create_invite(ctx, req).await,
        (Delete, ["admin", "invites", id]) => api::admin::revoke_invite(ctx, id).await,
        (Get, ["admin", "access-requests"]) => api::early_access::list(ctx).await,
        (Delete, ["admin", "access-requests", id]) => api::early_access::remove(ctx, id).await,
        (Get, ["admin", "audit"]) => api::admin::audit(ctx, req).await,
        (Get, ["admin", "settings"]) => api::admin::settings(ctx).await,
        (Patch, ["admin", "settings"]) => api::admin::update_settings(ctx, req).await,
        (Post, ["admin", "transfer"]) => api::admin::transfer(ctx, req).await,
        (Get, ["admin", "workspaces"]) => api::admin::workspaces(ctx).await,
        (Post, ["admin", "workspaces", id, "delete"]) => {
            api::admin::delete_workspace(ctx, req, id).await
        }
        (Post, ["admin", "workspaces", id, "primary"]) => {
            api::admin::workspace_primary(ctx, req, id).await
        }
        (Get, ["admin", "github"]) => api::github::admin_list(ctx).await,
        (Delete, ["admin", "github", account, "workspaces", workspace]) => {
            api::github::admin_disconnect(ctx, &http::decode(account), workspace).await
        }

        // Workspaces
        (Get, ["workspaces"]) => api::workspaces::list(ctx).await,
        (Post, ["workspaces"]) => api::workspaces::create(ctx, req).await,
        (Get, ["workspaces", id]) => api::workspaces::get(ctx, id).await,
        (Patch, ["workspaces", id]) => api::workspaces::update(ctx, req, id).await,
        (Post, ["workspaces", id, "delete"]) => api::workspaces::delete(ctx, req, id).await,
        (Get, ["workspaces", id, "avatar"]) => api::workspaces::avatar(ctx, id).await,
        (Put, ["workspaces", id, "avatar"]) => api::workspaces::set_avatar(ctx, req, id).await,
        (Delete, ["workspaces", id, "avatar"]) => api::workspaces::remove_avatar(ctx, id).await,
        (Get, ["workspaces", id, "members"]) => api::workspaces::members(ctx, id).await,
        (Patch, ["workspaces", id, "members", user]) => {
            api::workspaces::set_member_role(ctx, req, id, user).await
        }
        (Delete, ["workspaces", id, "members", user]) => {
            api::workspaces::remove_member(ctx, id, user).await
        }
        (Post, ["workspaces", id, "invitations"]) => {
            api::workspaces::create_invitation(ctx, req, id).await
        }
        (Delete, ["workspaces", id, "invitations", invitation]) => {
            api::workspaces::revoke_invitation(ctx, id, invitation).await
        }
        (Post, ["workspaces", id, "transfer"]) => api::workspaces::transfer(ctx, req, id).await,
        (Patch, ["workspaces", id, "membership"]) => {
            api::workspaces::update_membership(ctx, req, id).await
        }
        (Get, ["workspaces", id, "audit"]) => api::workspaces::audit(ctx, req, id).await,
        (Get, ["workspaces", id, "runners"]) => api::workspaces::runners(ctx, id).await,
        (Post, ["workspaces", id, "runners"]) => api::workspaces::create_runner(ctx, req, id).await,
        (Put, ["workspaces", id, "runners", runner]) => {
            api::workspaces::add_runner(ctx, req, id, runner).await
        }
        (Delete, ["workspaces", id, "runners", runner]) => {
            api::workspaces::remove_runner(ctx, id, runner).await
        }
        (Get, ["workspaces", id, "github"]) => api::github::workspace(ctx, id).await,
        (Post, ["workspaces", id, "github"]) => {
            api::github::connect_from_older_cli(ctx, req, id).await
        }
        (Post, ["workspaces", id, "github", "connect"]) => api::github::connect(ctx, req, id).await,
        (Delete, ["workspaces", id, "github", account]) => {
            api::github::disconnect(ctx, id, &http::decode(account)).await
        }
        (Post, ["workspaces", id, "github", "offers", account]) => {
            api::github::choose(ctx, req, id, &http::decode(account)).await
        }
        (Delete, ["workspaces", id, "github", "offers", account]) => {
            api::github::dismiss(ctx, id, &http::decode(account)).await
        }
        (Post, ["invitations", id, "accept"]) => api::workspaces::accept(ctx, id).await,
        (Post, ["invitations", id, "decline"]) => api::workspaces::decline(ctx, id).await,
        (Get, ["github", "install"]) => api::github::install(ctx, req).await,
        (Get, ["github", "installed"]) => api::github::installed(ctx, req).await,
        (Get, ["github", "connect"]) => api::github::connect_link(ctx, req).await,
        (Get, ["github", "authorized"]) => api::github::authorized(ctx, req).await,
        // GitHub's webhook: signed with its secret, not by a person.
        (Post, ["github", "webhook"]) => api::webhooks::receive(ctx, req).await,

        // Runners (admin) and hosts
        (Get, ["runners"]) => api::runners::list(ctx).await,
        (Post, ["runners"]) => api::runners::create(ctx, req).await,
        (Patch, ["runners", id]) => api::runners::update(ctx, req, id).await,
        (Delete, ["runners", id]) => api::runners::delete(ctx, id).await,
        (Post, ["runners", id, "token"]) => api::runners::rotate_token(ctx, id).await,
        (Get, ["runner"]) => api::runners::this_runner(ctx, req).await,
        (Post, ["runner", "drain"]) => api::runners::drain_this_runner(ctx, req).await,
        (Get, ["runner", "connect"]) => api::runners::connect(ctx, req).await,
        (Get, ["runner", "sessions", id, "uplink"]) => api::runners::uplink(ctx, req, id).await,
        (Get, ["runner", "sessions", id, "attachments", file]) => {
            api::attachments::runner_download(ctx, req, id, file).await
        }
        (Post, ["runner", "sessions", id, "published"]) => {
            api::published::runner_publish(ctx, req, id).await
        }
        (Post, ["runner", "sessions", id, "calls"]) => api::calls::runner_call(ctx, req, id).await,
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
        (Get, ["projects", id, "memories"]) => api::memories::project_list(ctx, req, id).await,
        (Post, ["projects", id, "memories"]) => api::memories::add(ctx, req, id).await,
        (Get, ["memories", id]) => api::memories::get(ctx, id).await,
        (Patch, ["memories", id]) => api::memories::update(ctx, req, id).await,
        (Delete, ["memories", id]) => api::memories::delete(ctx, id).await,
        (Get, ["projects", id, "review-agents"]) => api::reviews::get_settings(ctx, id).await,
        (Patch, ["projects", id, "review-agents"]) => {
            api::reviews::update_settings(ctx, req, id).await
        }
        (Get, ["projects", id, "reviews"]) => api::reviews::list(ctx, req, id).await,
        (Post, ["projects", id, "reviews"]) => api::reviews::start(ctx, req, id).await,
        (Get, ["projects", id, "retrospectives"]) => {
            api::retrospectives::project_list(ctx, req, id).await
        }
        (Delete, ["retrospectives", id]) => api::retrospectives::delete(ctx, id).await,
        (Get, ["projects", id, "schedules"]) => api::schedules::list(ctx, id).await,
        (Post, ["projects", id, "schedules"]) => api::schedules::create(ctx, req, id).await,
        (Get, ["schedules", id]) => api::schedules::get(ctx, id).await,
        (Patch, ["schedules", id]) => api::schedules::update(ctx, req, id).await,
        (Delete, ["schedules", id]) => api::schedules::delete(ctx, id).await,
        (Get, ["schedules", id, "runs"]) => api::schedules::runs(ctx, req, id).await,
        (Post, ["schedules", id, "runs"]) => api::schedules::run(ctx, req, id).await,
        (Post, ["projects", id, "attachments"]) => api::attachments::upload(ctx, req, id).await,
        (Get, ["github"]) => api::projects::github_info(ctx).await,
        (Get, ["github", "repos"]) => api::projects::github_repos(ctx, req).await,

        // Sessions
        (Get, ["sessions"]) => api::sessions::list(ctx, req).await,
        (Get, ["sessions", id]) => api::sessions::get(ctx, id).await,
        (Patch, ["sessions", id]) => api::sessions::update(ctx, req, id).await,
        (Post, ["sessions", id, "messages"]) => api::sessions::send_message(ctx, req, id).await,
        (Post, ["sessions", id, "interrupt"]) => api::sessions::interrupt(ctx, id).await,
        (Post, ["sessions", id, "stop"]) => api::sessions::stop(ctx, id).await,
        (Post, ["sessions", id, "resume"]) => api::sessions::resume(ctx, req, id).await,
        (Get, ["sessions", id, "events"]) => api::sessions::events(ctx, req, id).await,
        (Get, ["sessions", id, "retrospectives"]) => {
            api::retrospectives::session_list(ctx, id).await
        }
        (Get, ["sessions", id, "ws"]) => api::sessions::websocket(ctx, req, id).await,
        (Post, ["sessions", id, "attachments"]) => {
            api::attachments::upload_to_session(ctx, req, id).await
        }
        (Get, ["sessions", id, "attachments", file]) => {
            api::attachments::download(ctx, req, id, file).await
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
        // Two-factor sign-in
        (Get, ["me", "two-factor"]) => api::two_factor::get(ctx).await,
        (Patch, ["me", "two-factor"]) => api::two_factor::update(ctx, req).await,
        (Post, ["me", "two-factor", "authenticator"]) => {
            api::two_factor::start_authenticator(ctx, req).await
        }
        (Post, ["me", "two-factor", "authenticator", "confirm"]) => {
            api::two_factor::confirm_authenticator(ctx, req).await
        }
        (Post, ["me", "two-factor", "authenticator", "remove"]) => {
            api::two_factor::remove_authenticator(ctx, req).await
        }
        (Post, ["me", "two-factor", "recovery-codes"]) => {
            api::two_factor::new_recovery_codes(ctx, req).await
        }
        // Passkeys
        (Post, ["me", "passkeys", "options"]) => api::passkeys::creation_options(ctx, req).await,
        (Post, ["me", "passkeys"]) => api::passkeys::add(ctx, req).await,
        (Patch, ["me", "passkeys", id]) => api::passkeys::rename(ctx, req, id).await,
        (Delete, ["me", "passkeys", id]) => api::passkeys::remove(ctx, id).await,
        (Patch, ["me", "emails", email]) => {
            api::emails::update(ctx, req, &http::decode(email)).await
        }
        (Delete, ["me", "emails", email]) => api::emails::remove(ctx, &http::decode(email)).await,
        (Put, ["me", "workspace-order"]) => api::workspaces::order(ctx, req).await,
        (Get, ["me", "export"]) => api::account::export(ctx).await,
        (Get, ["me", "deletion"]) => api::account::deletion(ctx).await,
        (Post, ["me", "delete"]) => api::account::delete(ctx, req).await,
        _ => Err(AppError(ApiError::not_found("Endpoint"))),
    }
}
