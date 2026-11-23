//! What runners do with session snapshots
//! (docs/architecture.md#session-snapshots), with their runner token, for
//! the sessions they run: upload a snapshot's parts as their agent saved
//! it, make it the session's, and download the session's for an agent that
//! restores it. The parts stay in the session's SessionDO, sealed: the
//! control plane never opens them.

use crate::api::runners;
use crate::ctx::Ctx;
use crate::error::{AppError, AppResult, not_found};
use crate::{http, internal};
use fctl_proto::ApiError;
use fctl_proto::api::{Ok as Done, SnapshotCommit};
use fctl_proto::runner::SNAPSHOT_PART_BYTES;
use worker::{Request, Response};

/// The most parts one snapshot may have: past the largest limit an
/// instance may set.
const MAX_PARTS: u32 =
    (fctl_proto::api::MAX_SNAPSHOT_MB as u64 * 1024 * 1024 / SNAPSHOT_PART_BYTES as u64) as u32 + 1;

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// A snapshot's id as agents make them: `snp_` and letters and digits.
fn check_id(id: &str) -> AppResult<()> {
    let ok = id.len() <= 64
        && id.strip_prefix("snp_").is_some_and(|rest| {
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric())
        });
    if ok {
        Ok(())
    } else {
        Err(AppError(ApiError::bad_request(
            "That isn't a snapshot's id",
        )))
    }
}

fn check_part(n: &str) -> AppResult<u32> {
    n.parse::<u32>()
        .ok()
        .filter(|n| *n < MAX_PARTS)
        .ok_or_else(|| AppError(ApiError::bad_request("That isn't a part of a snapshot")))
}

/// `PUT /api/v1/runner/sessions/{id}/snapshots/{snapshot}/parts/{n}`: part
/// `n` of a snapshot of the session its runner runs, at most
/// `SNAPSHOT_PART_BYTES`, as `application/octet-stream`.
pub async fn put_part(
    ctx: &Ctx,
    req: &mut Request,
    session_id: &str,
    snapshot: &str,
    n: &str,
) -> AppResult<Response> {
    let r = runners::runner_auth(ctx, req).await?;
    crate::spec::check_runs(&ctx.db, &r.id, session_id).await?;
    check_id(snapshot)?;
    let n = check_part(n)?;
    // Only for a session whose specs carry a key, while the instance and
    // its workspace keep snapshots: what would be refused isn't stored.
    let settings = crate::instance::settings(&ctx.db).await?;
    let refused = match crate::snapshots::row(&ctx.db, session_id).await? {
        None => Some("This session saves no snapshots".to_owned()),
        Some(_) => crate::snapshots::refusal(&ctx.db, &settings, session_id, 0).await?,
    };
    if let Some(why) = refused {
        return Err(AppError(ApiError::forbidden(why)));
    }
    if http::header(req, "Content-Length")
        .and_then(|l| l.parse::<usize>().ok())
        .is_some_and(|l| l > SNAPSHOT_PART_BYTES)
    {
        return Err(AppError(ApiError::bad_request(
            "A snapshot's part is too large",
        )));
    }
    // The body goes through unread; the object counts it.
    let body = req
        .inner()
        .body()
        .map(wasm_bindgen::JsValue::from)
        .unwrap_or_else(|| js_sys::Uint8Array::new_with_length(0).into());
    let stub = internal::session_stub(&ctx.env, session_id)?;
    let path = format!("/snapshot/part?id={}&n={n}", enc(snapshot));
    let _: serde_json::Value = internal::post_body(&stub, &path, body).await?;
    // An upload that never commits goes after a day, by the hourly job.
    if n == 0 {
        crate::snapshots::uploading(&ctx.db, session_id, ctx.now).await?;
    }
    http::ok(&Done { ok: true })
}

/// `POST /api/v1/runner/sessions/{id}/snapshots/{snapshot}`: once every
/// part is uploaded, makes the snapshot the session's, in place of the one
/// before. Refused, and its parts deleted, for a session whose specs carry
/// no snapshot key, where the instance or the workspace keeps no
/// snapshots, or past the size the instance allows.
pub async fn commit(
    ctx: &Ctx,
    req: &mut Request,
    session_id: &str,
    snapshot: &str,
) -> AppResult<Response> {
    let body: SnapshotCommit = http::body(req).await?;
    let r = runners::runner_auth(ctx, req).await?;
    crate::spec::check_runs(&ctx.db, &r.id, session_id).await?;
    check_id(snapshot)?;
    if body.parts == 0 || body.parts > MAX_PARTS {
        return Err(AppError(ApiError::bad_request(
            "That isn't a snapshot's parts",
        )));
    }
    let stub = internal::session_stub(&ctx.env, session_id)?;
    let parts = internal::SnapshotParts {
        id: snapshot.to_owned(),
        parts: body.parts,
        size: body.saved.size,
    };
    let settings = crate::instance::settings(&ctx.db).await?;
    // Only a session whose spec let it save one has a key, and a row.
    let refused = match crate::snapshots::row(&ctx.db, session_id).await? {
        None => Some("This session saves no snapshots".to_owned()),
        Some(_) => {
            crate::snapshots::refusal(&ctx.db, &settings, session_id, body.saved.size).await?
        }
    };
    if let Some(why) = refused {
        let _: AppResult<serde_json::Value> =
            internal::post(&stub, "/snapshot/delete", &parts).await;
        return Err(AppError(ApiError::forbidden(why)));
    }
    // Whole in SessionDO, then the session's in D1, and only then does the
    // one before go: D1 never names a snapshot whose parts are gone.
    let _: serde_json::Value = internal::post(&stub, "/snapshot/commit", &parts).await?;
    // A D1 that fails keeps the parts, so that the host's retry commits
    // them; D1 still names the one before, whose parts are still there.
    crate::snapshots::keep(
        &ctx.db,
        session_id,
        snapshot,
        &body,
        (&r.id, &r.name),
        ctx.now,
    )
    .await?;
    let pruned: AppResult<serde_json::Value> =
        internal::post(&stub, "/snapshot/prune", &parts).await;
    if let Err(e) = pruned {
        // The next commit lets go of it, or the session's purge.
        worker::console_error!(
            "letting go of {session_id}'s older snapshot: {}",
            e.0.message
        );
    }
    http::ok(&Done { ok: true })
}

/// `GET /api/v1/runner/sessions/{id}/snapshots/{snapshot}/parts/{n}`: part
/// `n` of the session's snapshot, for the runner that runs it now. Only
/// the session's current one, while its workspace keeps it.
pub async fn get_part(
    ctx: &Ctx,
    req: &Request,
    session_id: &str,
    snapshot: &str,
    n: &str,
) -> AppResult<Response> {
    let r = runners::runner_auth(ctx, req).await?;
    crate::spec::check_runs(&ctx.db, &r.id, session_id).await?;
    check_id(snapshot)?;
    let n = check_part(n)?;
    let current = crate::snapshots::row(&ctx.db, session_id)
        .await?
        .and_then(|row| row.reference(ctx.now))
        .filter(|s| s.id == snapshot && n < s.parts);
    if current.is_none() {
        return Err(not_found("Snapshot"));
    }
    let stub = internal::session_stub(&ctx.env, session_id)?;
    internal::fetch(&stub, &format!("/snapshot/part?id={}&n={n}", enc(snapshot))).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_parts_are_what_agents_make() {
        assert!(check_id("snp_01M84ABCDEF").is_ok());
        for bad in [
            "snp_",
            "snp_../x",
            "ses_1",
            "snp_a b",
            &format!("snp_{}", "a".repeat(80)),
        ] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
        assert_eq!(check_part("0").ok(), Some(0));
        assert!(check_part("-1").is_err());
        assert!(check_part(&MAX_PARTS.to_string()).is_err());
    }
}
