//! Screenshots agents publish (`fctl publish`) so that pull requests can
//! show them: GitHub takes no uploads from a session's GitHub App token.
//!
//! A session's host uploads one with its runner token. Anyone with its link
//! can open it until it expires, while the instance and the project allow
//! publishing; people who manage its session can delete it sooner. The
//! bytes live in the project's FilesObject (`do_files.rs`); D1's
//! `published_files` finds them by link, and keeps only the SHA-256 of each
//! link's token.

use crate::access;
use crate::api::runners;
use crate::ctx::Ctx;
use crate::db::{Count, n, q, s};
use crate::error::{AppError, AppResult, conflict, forbidden, not_found, validation};
use crate::http;
use crate::internal::{self, PublishFile, PublishedBytes, Unpublish};
use fctl_domain::tokens::hash_secret;
use fctl_domain::{files, published};
use fctl_proto::api::Ok;
use fctl_proto::event::PublishedFile;
use fctl_proto::session::ProjectSettings;
use fctl_proto::{ApiError, SessionId};
use serde::Deserialize;
use worker::{Headers, Request, Response};

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Why a link shows nothing. The same for every reason, so a link says
/// nothing about what it was.
fn gone() -> AppError {
    not_found("Screenshot")
}

/// `POST /api/v1/runner/sessions/{id}/published?name=<file name>` with the
/// image as an `application/octet-stream` body, from the host the session
/// runs on. Answers with the [`PublishedFile`] and its link.
pub async fn runner_publish(
    ctx: &mut Ctx,
    req: &mut Request,
    session_id: &str,
) -> AppResult<Response> {
    let r = runners::runner_auth(ctx, req).await?;
    #[derive(Deserialize)]
    struct Row {
        project_id: String,
        runner_id: Option<String>,
        settings: String,
        #[serde(default)]
        workspace_id: Option<String>,
    }
    let row: Option<Row> = q(
        &ctx.db,
        "SELECT s.project_id, s.runner_id, p.settings, p.workspace_id FROM sessions s JOIN projects p ON p.id = s.project_id WHERE s.id = ?",
        &[s(session_id)],
    )?
    .first()
    .await?;
    let row = row.ok_or_else(|| not_found("Session"))?;
    if row.runner_id.as_deref() != Some(r.id.as_str()) {
        return Err(forbidden("This session is not assigned to this runner"));
    }
    if !crate::spec::runner_serves(&ctx.db, &r.id, session_id).await? {
        return Err(crate::spec::not_served());
    }
    let settings: ProjectSettings = serde_json::from_str(&row.settings).unwrap_or_default();
    if !crate::instance::settings(&ctx.db)
        .await?
        .publish_screenshots
    {
        return Err(forbidden(
            "This fctl instance doesn't publish screenshots. Its admins can allow it in Admin → Settings.",
        ));
    }
    if !settings.publish_screenshots {
        return Err(forbidden(
            "This project doesn't publish screenshots. Its maintainers can turn it on in the project's settings.",
        ));
    }
    let published_before: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM published_files WHERE session_id = ?",
        &[s(session_id)],
    )?
    .first()
    .await?;
    if published_before.map_or(0, |c| c.n) >= i64::from(published::MAX_PER_SESSION) {
        return Err(conflict(&format!(
            "A session can publish at most {} screenshots",
            published::MAX_PER_SESSION
        )));
    }
    let ct = http::header(req, "Content-Type").unwrap_or_default();
    if !ct
        .to_ascii_lowercase()
        .starts_with("application/octet-stream")
    {
        return Err(AppError(ApiError::bad_request(
            "Send the screenshot as application/octet-stream",
        )));
    }
    if http::header(req, "Content-Length")
        .and_then(|l| l.parse::<u64>().ok())
        .is_some_and(|l| l > published::MAX_BYTES)
    {
        return Err(validation(
            "file",
            &format!(
                "Screenshots can be at most {}: GitHub doesn't show larger images",
                files::human_size(published::MAX_BYTES)
            ),
        ));
    }
    let name = files::sanitize_name(&http::query(req, "name").unwrap_or_default());
    let id = fctl_domain::ids::new_id(published::ID_PREFIX, ctx.now);
    let token = published::new_token();
    let expires_at = published::expires_at(ctx.now, published::days(&settings));
    let meta = PublishFile {
        id: id.clone(),
        session_id: SessionId(session_id.to_owned()),
        name: name.clone(),
        expires_at,
    };
    // The body goes through unread; the object checks that it is an image.
    let body = req
        .inner()
        .body()
        .map(wasm_bindgen::JsValue::from)
        .unwrap_or_else(|| js_sys::Uint8Array::new_with_length(0).into());
    let stub = internal::files_stub(&ctx.env, &row.project_id)?;
    let path = format!("/publish?meta={}", enc(&serde_json::to_string(&meta)?));
    let stored: PublishedBytes = internal::post_body(&stub, &path, body).await?;
    let inserted = async {
        q(
            &ctx.db,
            "INSERT INTO published_files (id, token_hash, project_id, session_id, name, media_type, size, width, height, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                s(&id),
                s(&hash_secret(&token)),
                s(&row.project_id),
                s(session_id),
                s(&name),
                s(&stored.media_type),
                n(stored.size as i64),
                n(i64::from(stored.width)),
                n(i64::from(stored.height)),
                n(ctx.now),
                n(expires_at),
            ],
        )?
        .run()
        .await
    }
    .await;
    if let Err(e) = inserted {
        // Without its row no link reaches the file: don't keep it.
        let _ =
            internal::post::<_, serde_json::Value>(&stub, "/unpublish", &Unpublish { id }).await;
        return Err(e);
    }
    let file = PublishedFile {
        url: ctx
            .cfg
            .url(&published::path(&token, &name, &stored.media_type)),
        name,
        media_type: stored.media_type,
        size: stored.size,
        width: Some(stored.width),
        height: Some(stored.height),
        expires_at,
    };
    // In the workspace's log: what its sessions publish is its own.
    ctx.audit_in(
        row.workspace_id.as_deref(),
        "screenshot.published",
        Some(session_id),
        Some(serde_json::json!({
            "project_id": row.project_id,
            "runner_id": r.id,
            "name": file.name,
            "size": file.size,
            "expires_at": expires_at,
        })),
    );
    http::created(&file)
}

#[derive(Deserialize)]
struct LinkRow {
    id: String,
    project_id: String,
    session_id: String,
    name: String,
    media_type: String,
    #[serde(deserialize_with = "crate::db::int")]
    expires_at: i64,
    settings: String,
}

/// The published screenshot a link's token names, unless it expired.
async fn by_token(ctx: &Ctx, token: &str) -> AppResult<LinkRow> {
    if !published::is_token(token) {
        return Err(gone());
    }
    let row: Option<LinkRow> = q(
        &ctx.db,
        "SELECT f.id, f.project_id, f.session_id, f.name, f.media_type, f.expires_at, p.settings \
         FROM published_files f JOIN projects p ON p.id = f.project_id WHERE f.token_hash = ?",
        &[s(&hash_secret(token))],
    )?
    .first()
    .await?;
    row.filter(|r| r.expires_at > ctx.now).ok_or_else(gone)
}

/// `GET /api/v1/published/{token}/{name}`: a published screenshot, for
/// anyone with its link (GitHub's image proxy too), until it expires and
/// while the instance and its project allow publishing. The name is only
/// for people.
pub async fn serve(ctx: &Ctx, req: &Request, token: &str) -> AppResult<Response> {
    let row = by_token(ctx, token).await?;
    let settings: ProjectSettings = serde_json::from_str(&row.settings).unwrap_or_default();
    let instance = crate::instance::settings(&ctx.db).await?;
    if !published::allowed(instance.publish_screenshots, &settings) {
        return Err(gone());
    }
    // Caches ask again every time, so a deletion, an expiry or a switch
    // turned off takes the image offline at once.
    let etag = format!("\"{}\"", row.id);
    let h = Headers::new();
    h.set("ETag", &etag)?;
    h.set("Cache-Control", "public, no-cache")?;
    h.set("X-Content-Type-Options", "nosniff")?;
    // As for attachments: the browser's own page for an image opened in a
    // tab styles it inline, and nothing in it may run.
    h.set(
        "Content-Security-Policy",
        "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; sandbox",
    )?;
    // Pages elsewhere show it (GitHub through its image proxy).
    h.set("Cross-Origin-Resource-Policy", "cross-origin")?;
    h.set("Referrer-Policy", "no-referrer")?;
    h.set("X-Robots-Tag", "noindex, nofollow, noarchive")?;
    let fresh = http::header(req, "If-None-Match").is_some_and(|v| {
        v.split(',')
            .any(|t| t.trim().trim_start_matches("W/") == etag)
    });
    if fresh {
        return Ok(Response::empty()?.with_status(304).with_headers(h));
    }
    let stub = internal::files_stub(&ctx.env, &row.project_id)?;
    let mut res = internal::fetch(&stub, &format!("/published?id={}", enc(&row.id))).await?;
    if res.status_code() != 200 {
        return Err(gone());
    }
    let bytes = res.bytes().await?;
    h.set("Content-Type", &row.media_type)?;
    h.set("Content-Length", &bytes.len().to_string())?;
    h.set(
        "Content-Disposition",
        &files::content_disposition(&row.name, true),
    )?;
    Ok(Response::from_bytes(bytes)?.with_headers(h))
}

/// `DELETE /api/v1/published/{token}`: deletes a published screenshot
/// before it expires, for people who manage its session.
pub async fn delete(ctx: &mut Ctx, token: &str) -> AppResult<Response> {
    ctx.user()?;
    let row = by_token(ctx, token).await?;
    let a = access::session(ctx, &row.session_id).await?;
    if !a.role.can_manage() {
        return Err(forbidden(
            "Only people who manage this session can delete its screenshots",
        ));
    }
    let stub = internal::files_stub(&ctx.env, &row.project_id)?;
    let _: serde_json::Value =
        internal::post(&stub, "/unpublish", &Unpublish { id: row.id.clone() }).await?;
    q(
        &ctx.db,
        "DELETE FROM published_files WHERE id = ?",
        &[s(&row.id)],
    )?
    .run()
    .await?;
    ctx.audit_in(
        Some(a.workspace_id()),
        "screenshot.deleted",
        Some(&row.session_id),
        Some(serde_json::json!({ "project_id": row.project_id, "name": row.name })),
    );
    http::ok(&Ok { ok: true })
}
