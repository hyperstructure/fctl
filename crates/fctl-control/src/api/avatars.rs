//! Avatars: the pictures people upload for themselves, and workspace admins
//! for their workspaces (`api::workspaces`), kept in D1 as base64; and the
//! picture linking Google copies for someone who has none.

use crate::auth;
use crate::ctx::Ctx;
use crate::db::{batch, n, q, s};
use crate::error::{AppError, AppResult, not_found, validation};
use crate::http;
use base64::Engine;
use fctl_domain::avatar as policy;
use fctl_proto::ApiError;
use serde::Deserialize;
use worker::{Fetch, Headers, Request, RequestInit, Response};

/// The avatar in a request's body, which must be `application/octet-stream`:
/// base64, with its media type.
pub async fn read(req: &mut Request) -> AppResult<(String, &'static str)> {
    let ct = http::header(req, "Content-Type").unwrap_or_default();
    if !ct
        .to_ascii_lowercase()
        .starts_with("application/octet-stream")
    {
        return Err(AppError(ApiError::bad_request(
            "Send the image as application/octet-stream",
        )));
    }
    if http::header(req, "Content-Length")
        .and_then(|l| l.parse::<usize>().ok())
        .is_some_and(|l| l > policy::MAX_BYTES)
    {
        return Err(validation("avatar", "Use a smaller image"));
    }
    let bytes = req.bytes().await?;
    let media_type = policy::check(&bytes)?;
    Ok((
        base64::engine::general_purpose::STANDARD.encode(&bytes),
        media_type,
    ))
}

/// An avatar image (base64) as the response. Its URL changes with it, so
/// browsers keep it.
pub fn image(data: &str, media_type: &str) -> AppResult<Response> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(crate::error::internal)?;
    let h = Headers::new();
    h.set("Content-Type", media_type)?;
    h.set("Cache-Control", "private, max-age=31536000, immutable")?;
    h.set("X-Content-Type-Options", "nosniff")?;
    h.set("Content-Security-Policy", "default-src 'none'; sandbox")?;
    h.set("Cross-Origin-Resource-Policy", "same-origin")?;
    Ok(Response::from_bytes(bytes)?.with_headers(h))
}

/// `PUT /api/v1/me/avatar`: your picture, as an `application/octet-stream`
/// body. It replaces the one you had.
pub async fn set_mine(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let id = ctx.user()?.id.clone();
    let (data, media_type) = read(req).await?;
    store(ctx, &id, &data, media_type, false).await?;
    me(ctx, &id).await
}

/// `DELETE /api/v1/me/avatar`: your initials again.
pub async fn remove_mine(ctx: &mut Ctx) -> AppResult<Response> {
    let id = ctx.user()?.id.clone();
    batch(&ctx.db, remove(ctx, &id)?).await?;
    me(ctx, &id).await
}

/// What forgets someone's picture, uploaded or not.
pub fn remove(ctx: &Ctx, user_id: &str) -> AppResult<Vec<crate::db::Query>> {
    Ok(vec![
        q(
            &ctx.db,
            "DELETE FROM user_avatars WHERE user_id = ?",
            &[s(user_id)],
        )?,
        q(
            &ctx.db,
            "UPDATE users SET avatar_url = NULL, updated_at = ? WHERE id = ?",
            &[n(ctx.now), s(user_id)],
        )?,
    ])
}

/// Keeps a picture as someone's avatar; with `if_none`, only while they
/// have none, uploaded or not.
async fn store(
    ctx: &Ctx,
    user_id: &str,
    data: &str,
    media_type: &str,
    if_none: bool,
) -> AppResult<()> {
    let only = if if_none {
        " AND avatar_url IS NULL"
    } else {
        ""
    };
    batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                &format!(
                    "INSERT INTO user_avatars (user_id, image, media_type, updated_at) \
                     SELECT id, ?, ?, ? FROM users WHERE id = ?{only} \
                     ON CONFLICT (user_id) DO UPDATE SET image = excluded.image, \
                     media_type = excluded.media_type, updated_at = excluded.updated_at"
                ),
                &[s(data), s(media_type), n(ctx.now), s(user_id)],
            )?,
            q(
                &ctx.db,
                &format!("UPDATE users SET avatar_url = ?, updated_at = ? WHERE id = ?{only}"),
                &[
                    s(&policy::user_path(user_id, ctx.now)),
                    n(ctx.now),
                    s(user_id),
                ],
            )?,
        ],
    )
    .await
}

async fn me(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let user = auth::user_by_id(ctx, id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::ok(&auth::me(ctx, &user).await?)
}

/// `GET /api/v1/users/{id}/avatar`: for anyone signed in, since their
/// avatar shows wherever their name does, and for the person themselves
/// while their sign-in waits for its second factor.
pub async fn user(ctx: &Ctx, req: &Request, id: &str) -> AppResult<Response> {
    if ctx.auth.is_none() {
        let theirs = crate::api::two_factor::waiting_sign_in(ctx, req)
            .await
            .is_ok_and(|(_, user)| user == id);
        if !theirs {
            return Err(AppError(ApiError::unauthorized()));
        }
    }
    #[derive(Deserialize)]
    struct Row {
        image: String,
        media_type: String,
    }
    let row: Option<Row> = q(
        &ctx.db,
        "SELECT image, media_type FROM user_avatars WHERE user_id = ?",
        &[s(id)],
    )?
    .first()
    .await?;
    let row = row.ok_or_else(|| not_found("Avatar"))?;
    image(&row.image, &row.media_type)
}

/// Copies a Google account's picture (its ID token's `picture`) as the
/// avatar of someone who has none, as they link the account or sign up
/// with it, so that the people who see it load it from the instance rather
/// than from Google. Says why when it doesn't.
pub async fn adopt_google(ctx: &Ctx, user_id: &str, picture: &str) -> Result<(), String> {
    let url = policy::google_picture(picture).ok_or("not a Google picture address")?;
    let headers = Headers::new();
    headers
        .set("Accept", "image/png,image/jpeg,image/webp,image/gif")
        .map_err(|e| e.to_string())?;
    let mut init = RequestInit::new();
    init.with_headers(headers);
    let mut res = Fetch::Request(Request::new_with_init(&url, &init).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if res.status_code() != 200 {
        return Err(format!("HTTP {}", res.status_code()));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    let media_type = policy::check(&bytes).map_err(|e| e.message)?;
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    store(ctx, user_id, &data, media_type, true)
        .await
        .map_err(|e| e.0.message)
}
