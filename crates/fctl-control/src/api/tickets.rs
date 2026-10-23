//! `GET /api/v1/projects/{id}/tickets`: a project's ticket tracker as it
//! works now (`crate::tickets::status`), for its Tickets tab and `fctl
//! projects tickets`.

use crate::access;
use crate::ctx::Ctx;
use crate::error::AppResult;
use crate::http;
use worker::Response;

pub async fn get(ctx: &Ctx, id: &str) -> AppResult<Response> {
    let p = access::project(ctx, id).await?;
    http::ok(&crate::tickets::status(ctx, &p.row).await?)
}
