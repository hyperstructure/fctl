//! Images agents read: each session keeps those in the results of its
//! agent's tools for its viewers, for a week (`SessionObject`,
//! `fctl_domain::kept_images`), and the transcript names them where the
//! agent read them.

use crate::access;
use crate::ctx::Ctx;
use crate::error::{AppResult, not_found};
use crate::internal;
use fctl_proto::KeptImageId;
use worker::Response;

/// `GET /api/v1/sessions/{id}/images/{image}`: an image the session's agent
/// read, for anyone who can see the session, until it expires.
pub async fn download(ctx: &Ctx, session_id: &str, image_id: &str) -> AppResult<Response> {
    let a = access::session(ctx, session_id).await?;
    if !KeptImageId::is_valid(image_id) {
        return Err(not_found("Image"));
    }
    let stub = internal::session_stub(&ctx.env, &a.row.id)?;
    // A valid id needs no escaping: letters, digits and `_`.
    internal::fetch(&stub, &format!("/image?id={image_id}")).await
}
