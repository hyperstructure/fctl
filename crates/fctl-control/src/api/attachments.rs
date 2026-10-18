//! Files people attach to messages: uploads, and reading them back.
//!
//! The bytes live in the project's FilesObject (`do_files.rs`). A message,
//! new session or resume sends uploads by id, which hands them to the
//! session; from then on its viewers and its runner can read them.

use crate::access;
use crate::api::runners;
use crate::ctx::Ctx;
use crate::db::{q, s};
use crate::error::{AppError, AppResult, forbidden, not_found, validation};
use crate::http;
use crate::internal::{self, StoreFile};
use fctl_domain::files;
use fctl_proto::event::Attachment;
use fctl_proto::{ApiError, AttachmentId};
use worker::{Request, Response};

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// `POST /api/v1/projects/{id}/attachments?name=<file name>&type=<media type>`
/// with the file as an `application/octet-stream` body. Answers with the
/// [`Attachment`]. Uploads no message sends are deleted after a day.
pub async fn upload(ctx: &mut Ctx, req: &mut Request, project_id: &str) -> AppResult<Response> {
    let p = access::project(ctx, project_id).await?;
    p.require_editor()?;
    // Browsers cannot send this type cross-site without asking first, which
    // keeps uploads as safe from forged requests as JSON bodies.
    let ct = http::header(req, "Content-Type").unwrap_or_default();
    if !ct
        .to_ascii_lowercase()
        .starts_with("application/octet-stream")
    {
        return Err(AppError(ApiError::bad_request(
            "Send the file as application/octet-stream",
        )));
    }
    let too_large = || {
        validation(
            "file",
            &format!(
                "Files can be at most {}",
                files::human_size(files::MAX_FILE_BYTES)
            ),
        )
    };
    if http::header(req, "Content-Length")
        .and_then(|l| l.parse::<u64>().ok())
        .is_some_and(|l| l > files::MAX_FILE_BYTES)
    {
        return Err(too_large());
    }
    let name = files::sanitize_name(&http::query(req, "name").unwrap_or_default());
    let meta = StoreFile {
        id: fctl_domain::ids::attachment_id(ctx.now),
        media_type: files::media_type(http::query(req, "type").as_deref(), &name),
        name,
        uploaded_by: ctx.user()?.id.clone(),
    };
    // The body goes through unread; the object checks its size.
    let body = req
        .inner()
        .body()
        .map(wasm_bindgen::JsValue::from)
        .unwrap_or_else(|| js_sys::Uint8Array::new_with_length(0).into());
    let stub = internal::files_stub(&ctx.env, &p.row.id)?;
    let path = format!("/store?meta={}", enc(&serde_json::to_string(&meta)?));
    let att: Attachment = internal::post_body(&stub, &path, body).await?;
    http::created(&att)
}

/// `GET /api/v1/sessions/{id}/attachments/{attachment}`: a file sent in the
/// session, for anyone who can see the session.
pub async fn download(ctx: &Ctx, session_id: &str, attachment_id: &str) -> AppResult<Response> {
    let a = access::session(ctx, session_id).await?;
    file(ctx, &a.row.project_id, &a.row.id, attachment_id).await
}

/// `GET /api/v1/runner/sessions/{id}/attachments/{attachment}`: the same for
/// the runner the session is assigned to, so its host can hand the file to
/// the agent.
pub async fn runner_download(
    ctx: &Ctx,
    req: &Request,
    session_id: &str,
    attachment_id: &str,
) -> AppResult<Response> {
    let r = runners::runner_auth(ctx, req).await?;
    #[derive(serde::Deserialize)]
    struct Assigned {
        project_id: String,
        runner_id: Option<String>,
    }
    let row: Option<Assigned> = q(
        &ctx.db,
        "SELECT project_id, runner_id FROM sessions WHERE id = ?",
        &[s(session_id)],
    )?
    .first()
    .await?;
    let assigned = row.ok_or_else(|| not_found("Session"))?;
    if assigned.runner_id.as_deref() != Some(r.id.as_str()) {
        return Err(forbidden("This session is not assigned to this runner"));
    }
    file(ctx, &assigned.project_id, session_id, attachment_id).await
}

async fn file(
    ctx: &Ctx,
    project_id: &str,
    session_id: &str,
    attachment_id: &str,
) -> AppResult<Response> {
    if !AttachmentId::is_valid(attachment_id) {
        return Err(not_found("File"));
    }
    let stub = internal::files_stub(&ctx.env, project_id)?;
    internal::fetch(
        &stub,
        &format!(
            "/file?id={}&session={}",
            enc(attachment_id),
            enc(session_id)
        ),
    )
    .await
}
