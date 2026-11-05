//! Avatars: the pictures people upload for themselves, and workspace admins
//! for their workspaces (`api::workspaces`), kept in D1 as base64; the
//! picture linking Google copies for someone who has none; and their
//! Gravatar, copied while they have no picture of their own.

use crate::auth;
use crate::ctx::Ctx;
use crate::db::{batch, n, q, s};
use crate::error::{AppError, AppResult, not_found, validation};
use crate::http;
use crate::model::UserRow;
use base64::Engine;
use fctl_domain::avatar as policy;
use fctl_proto::ApiError;
use fctl_proto::api::{AvatarSource, InstanceSettings};
use serde::Deserialize;
use worker::{D1Database, Fetch, Headers, Request, RequestInit, Response};

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
    let bytes = http::read_upto(req, policy::MAX_BYTES).await?;
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
    store(
        &ctx.db,
        ctx.now,
        &id,
        &data,
        media_type,
        AvatarSource::Upload,
    )
    .await?;
    me(ctx, &id).await
}

/// `DELETE /api/v1/me/avatar`: your initials again, and not your Gravatar
/// either.
pub async fn remove_mine(ctx: &mut Ctx) -> AppResult<Response> {
    let id = ctx.user()?.id.clone();
    batch(&ctx.db, remove(ctx, &id)?).await?;
    me(ctx, &id).await
}

/// What forgets someone's picture, uploaded or not, and turns their
/// Gravatar off, which would otherwise take its place.
pub fn remove(ctx: &Ctx, user_id: &str) -> AppResult<Vec<crate::db::Query>> {
    Ok(vec![
        q(
            &ctx.db,
            "DELETE FROM user_avatars WHERE user_id = ?",
            &[s(user_id)],
        )?,
        q(
            &ctx.db,
            "UPDATE users SET avatar_url = NULL, gravatar = 0, updated_at = ? WHERE id = ?",
            &[n(ctx.now), s(user_id)],
        )?,
    ])
}

/// Keeps a picture as someone's avatar: an upload always, Google's only
/// while they have none, and their Gravatar only while they have none of
/// their own and still let it.
async fn store(
    db: &D1Database,
    now: i64,
    user_id: &str,
    data: &str,
    media_type: &str,
    source: AvatarSource,
) -> AppResult<()> {
    // Both statements check it: the first one's row makes it hold for the
    // second, which a picture of their own keeps from holding for either.
    let only = match source {
        AvatarSource::Upload => "",
        AvatarSource::Google => " AND avatar_url IS NULL",
        AvatarSource::Gravatar => {
            " AND gravatar = 1 AND (avatar_url IS NULL OR id IN \
             (SELECT user_id FROM user_avatars WHERE source = 'gravatar'))"
        }
    };
    batch(
        db,
        vec![
            q(
                db,
                &format!(
                    "INSERT INTO user_avatars (user_id, image, media_type, updated_at, source) \
                     SELECT id, ?, ?, ?, ? FROM users WHERE id = ?{only} \
                     ON CONFLICT (user_id) DO UPDATE SET image = excluded.image, \
                     media_type = excluded.media_type, updated_at = excluded.updated_at, \
                     source = excluded.source"
                ),
                &[
                    s(data),
                    s(media_type),
                    n(now),
                    s(source.as_str()),
                    s(user_id),
                ],
            )?,
            q(
                db,
                &format!("UPDATE users SET avatar_url = ?, updated_at = ? WHERE id = ?{only}"),
                &[s(&policy::user_path(user_id, now)), n(now), s(user_id)],
            )?,
        ],
    )
    .await
}

/// What forgets someone's picture if it is their Gravatar.
pub fn remove_gravatar(
    db: &D1Database,
    now: i64,
    user_id: &str,
) -> AppResult<Vec<crate::db::Query>> {
    Ok(vec![
        q(
            db,
            "UPDATE users SET avatar_url = NULL, updated_at = ? WHERE id = ? AND id IN \
             (SELECT user_id FROM user_avatars WHERE source = 'gravatar')",
            &[n(now), s(user_id)],
        )?,
        q(
            db,
            "DELETE FROM user_avatars WHERE user_id = ? AND source = 'gravatar'",
            &[s(user_id)],
        )?,
    ])
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
    let (data, media_type) = fetch(&url).await?.ok_or("HTTP 404")?;
    store(
        &ctx.db,
        ctx.now,
        user_id,
        &data,
        media_type,
        AvatarSource::Google,
    )
    .await
    .map_err(|e| e.0.message)
}

/// An avatar image from another site (base64, with its media type), or
/// `None` when it answers 404.
async fn fetch(url: &str) -> Result<Option<(String, &'static str)>, String> {
    let headers = Headers::new();
    headers
        .set("Accept", "image/png,image/jpeg,image/webp,image/gif")
        .map_err(|e| e.to_string())?;
    let mut init = RequestInit::new();
    init.with_headers(headers);
    let mut res = Fetch::Request(Request::new_with_init(url, &init).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    match res.status_code() {
        200 => {}
        404 => return Ok(None),
        status => return Err(format!("HTTP {status}")),
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    let media_type = policy::check(&bytes).map_err(|e| e.message)?;
    Ok(Some((
        base64::engine::general_purpose::STANDARD.encode(&bytes),
        media_type,
    )))
}

/// When to look up someone's Gravatar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// At most once a day ([`policy::gravatar_due`]): as they open fctl.
    IfDue,
    /// Now, if they have no picture of their own: they just signed up, or
    /// turned their Gravatar on.
    Now,
}

/// Whether [`gravatar`] would look up `user`'s Gravatar.
pub fn gravatar_wanted(settings: &InstanceSettings, user: &UserRow, now: i64, look: Look) -> bool {
    let checked_at = match look {
        Look::IfDue => user.gravatar_checked_at,
        Look::Now => None,
    };
    user.is_active()
        && policy::gravatar_due(
            settings.gravatar,
            user.gravatar,
            user.picture(),
            checked_at,
            now,
        )
}

/// Looks up `user`'s Gravatar by their primary address, where the instance
/// and they let it and they have no picture of their own: keeps a copy as
/// their picture, replaces the copy when their Gravatar changed, and
/// forgets it when they have none any more. Says whether their picture
/// changed. Gravatar's failures leave it as it is, and are logged.
pub async fn gravatar(
    db: &D1Database,
    now: i64,
    settings: &InstanceSettings,
    user: &UserRow,
    look: Look,
) -> AppResult<bool> {
    if !gravatar_wanted(settings, user, now, look) {
        return Ok(false);
    }
    // One look a day, however many requests ask at once.
    let claimed = q(
        db,
        "UPDATE users SET gravatar_checked_at = ?1 WHERE id = ?2 \
         AND (?3 OR gravatar_checked_at IS NULL OR gravatar_checked_at <= ?4 OR gravatar_checked_at > ?1)",
        &[
            n(now),
            s(&user.id),
            crate::db::b(look == Look::Now),
            n(now - policy::GRAVATAR_EVERY_MS),
        ],
    )?
    .run()
    .await?;
    if claimed == 0 {
        return Ok(false);
    }
    let copied = user.picture().source == Some(AvatarSource::Gravatar);
    match fetch(&policy::gravatar_url(&user.email)).await {
        Ok(Some((data, media_type))) => {
            #[derive(Deserialize)]
            struct Image {
                image: String,
            }
            // The same picture keeps its address, which browsers cache.
            let same: Option<Image> = q(
                db,
                "SELECT image FROM user_avatars WHERE user_id = ? AND source = 'gravatar'",
                &[s(&user.id)],
            )?
            .first()
            .await?;
            if copied && same.is_some_and(|i| i.image == data) {
                return Ok(false);
            }
            store(db, now, &user.id, &data, media_type, AvatarSource::Gravatar).await?;
            Ok(true)
        }
        Ok(None) if copied => {
            batch(db, remove_gravatar(db, now, &user.id)?).await?;
            Ok(true)
        }
        Ok(None) => Ok(false),
        Err(e) => {
            worker::console_log!("gravatar not copied: {e}");
            Ok(false)
        }
    }
}
