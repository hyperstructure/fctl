//! FilesObject: one Durable Object per project, holding the files people
//! attach to messages.
//!
//! An upload is stored in chunks (SQLite rows hold at most 2 MB) and waits
//! for a message in one of the project's sessions to claim it; uploads no
//! message claimed are deleted after a day. Claimed files are read through
//! the Worker by the session's viewers and by the runner that runs it, and
//! deleted with the project.

use crate::ctx::now_ms;
use crate::internal::{self, ClaimFiles, ClaimedFiles, StoreFile};
use fctl_domain::files;
use fctl_proto::event::Attachment;
use fctl_proto::{ApiError, AttachmentId, ErrorCode};
use serde::Deserialize;
use worker::{
    DurableObject, Env, Headers, Request, Response, Result, SqlStorage, SqlStorageValue, State,
    durable_object,
};

#[durable_object]
pub struct FilesObject {
    state: State,
    sql: SqlStorage,
}

#[derive(Deserialize)]
struct FileRow {
    id: String,
    name: String,
    media_type: String,
    size: f64,
    width: Option<f64>,
    height: Option<f64>,
    uploaded_by: String,
    session_id: Option<String>,
}

impl FileRow {
    fn attachment(&self) -> Attachment {
        Attachment {
            id: AttachmentId(self.id.clone()),
            name: self.name.clone(),
            media_type: self.media_type.clone(),
            size: self.size as u64,
            width: self.width.map(|w| w as u32),
            height: self.height.map(|h| h as u32),
        }
    }
}

#[derive(Deserialize)]
struct Oldest {
    created_at: Option<f64>,
}

fn sv(s: &str) -> SqlStorageValue {
    SqlStorageValue::from(s)
}

fn iv(n: i64) -> SqlStorageValue {
    SqlStorageValue::from(n)
}

fn api_err(code: ErrorCode, message: &str) -> ApiError {
    ApiError::new(code, message)
}

fn create_schema(sql: &SqlStorage) {
    for stmt in [
        "CREATE TABLE IF NOT EXISTS files (id TEXT PRIMARY KEY, name TEXT NOT NULL, media_type TEXT NOT NULL, size INTEGER NOT NULL, \
         width INTEGER, height INTEGER, uploaded_by TEXT NOT NULL, created_at INTEGER NOT NULL, session_id TEXT, claimed_at INTEGER)",
        "CREATE INDEX IF NOT EXISTS files_unclaimed ON files (created_at) WHERE session_id IS NULL",
        "CREATE TABLE IF NOT EXISTS chunks (file_id TEXT NOT NULL, n INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (file_id, n))",
    ] {
        if let Err(e) = sql.exec(stmt, None) {
            worker::console_error!("files schema: {e}");
        }
    }
}

impl DurableObject for FilesObject {
    fn new(state: State, _env: Env) -> Self {
        let sql = state.storage().sql();
        create_schema(&sql);
        Self { state, sql }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        let result = match (req.method(), path.as_str()) {
            (worker::Method::Post, "/store") => self.store(&mut req).await,
            (worker::Method::Post, "/claim") => match req.json::<ClaimFiles>().await {
                Ok(b) => self.claim(b),
                Err(e) => Err(api_err(ErrorCode::BadRequest, &e.to_string())),
            },
            (worker::Method::Get, "/file") => return self.file(&req),
            (worker::Method::Post, "/purge") => self.purge().await,
            _ => Err(api_err(ErrorCode::NotFound, "Unknown files endpoint")),
        };
        match result {
            Ok(v) => internal::do_json(&v),
            Err(e) => internal::do_error(&e),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        self.expire_unclaimed(now_ms()).await;
        Response::ok("ok")
    }
}

impl FilesObject {
    fn row(&self, id: &str) -> Option<FileRow> {
        self.sql
            .exec(
                "SELECT id, name, media_type, size, width, height, uploaded_by, session_id FROM files WHERE id = ?",
                Some(vec![sv(id)]),
            )
            .and_then(|c| c.to_array::<FileRow>())
            .ok()?
            .into_iter()
            .next()
    }

    /// Stores an upload (the request body) and returns it as an attachment.
    async fn store(&self, req: &mut Request) -> std::result::Result<serde_json::Value, ApiError> {
        let meta: StoreFile = crate::http::query(req, "meta")
            .and_then(|m| serde_json::from_str(&m).ok())
            .ok_or_else(|| api_err(ErrorCode::BadRequest, "No upload metadata"))?;
        let bytes = req
            .bytes()
            .await
            .map_err(|e| api_err(ErrorCode::BadRequest, &format!("Reading the upload: {e}")))?;
        if bytes.len() as u64 > files::MAX_FILE_BYTES {
            return Err(ApiError::validation(
                "file",
                format!(
                    "Files can be at most {}",
                    files::human_size(files::MAX_FILE_BYTES)
                ),
            ));
        }
        let image = files::sniff_image(&bytes);
        let att = Attachment {
            id: meta.id,
            name: meta.name,
            media_type: image.map_or(meta.media_type, |i| i.media_type.to_owned()),
            size: bytes.len() as u64,
            width: image.map(|i| i.width),
            height: image.map(|i| i.height),
        };
        let now = now_ms();
        let failed = |e: worker::Error| {
            worker::console_error!("storing a file: {e}");
            api_err(
                ErrorCode::Unavailable,
                "The file could not be stored. Try again.",
            )
        };
        // No await in between: the rows are written together or not at all.
        self.sql
            .exec(
                "INSERT INTO files (id, name, media_type, size, width, height, uploaded_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                Some(vec![
                    sv(att.id.as_str()),
                    sv(&att.name),
                    sv(&att.media_type),
                    iv(att.size as i64),
                    att.width.map_or(SqlStorageValue::Null, |w| iv(i64::from(w))),
                    att.height.map_or(SqlStorageValue::Null, |h| iv(i64::from(h))),
                    sv(&meta.uploaded_by),
                    iv(now),
                ]),
            )
            .map_err(failed)?;
        for (n, chunk) in bytes.chunks(files::CHUNK_BYTES).enumerate() {
            self.sql
                .exec(
                    "INSERT INTO chunks (file_id, n, data) VALUES (?, ?, ?)",
                    Some(vec![
                        sv(att.id.as_str()),
                        iv(n as i64),
                        SqlStorageValue::Blob(chunk.to_vec()),
                    ]),
                )
                .map_err(failed)?;
        }
        if let Ok(None) = self.state.storage().get_alarm().await {
            let _ = self
                .state
                .storage()
                .set_alarm(files::UNSENT_TTL_MS + 60_000)
                .await;
        }
        serde_json::to_value(&att).map_err(|e| api_err(ErrorCode::Internal, &e.to_string()))
    }

    /// Hands uploads to a message: each must be the sender's and not sent in
    /// another session.
    fn claim(&self, b: ClaimFiles) -> std::result::Result<serde_json::Value, ApiError> {
        let mut out = Vec::with_capacity(b.ids.len());
        for id in &b.ids {
            let row = self
                .row(id.as_str())
                .filter(|r| r.uploaded_by == b.user_id)
                .ok_or_else(|| {
                    ApiError::validation(
                        "attachments",
                        "A file is no longer available (unsent uploads are deleted after a day). Attach it again.",
                    )
                })?;
            match row.session_id.as_deref() {
                Some(s) if s != b.session_id.as_str() => {
                    return Err(ApiError::validation(
                        "attachments",
                        format!(
                            "{} was already sent in another session. Attach it again.",
                            row.name
                        ),
                    ));
                }
                _ => {}
            }
            out.push(row.attachment());
        }
        let now = now_ms();
        for id in &b.ids {
            let _ = self.sql.exec(
                "UPDATE files SET session_id = ?, claimed_at = ? WHERE id = ? AND session_id IS NULL",
                Some(vec![sv(b.session_id.as_str()), iv(now), sv(id.as_str())]),
            );
        }
        serde_json::to_value(ClaimedFiles { attachments: out })
            .map_err(|e| api_err(ErrorCode::Internal, &e.to_string()))
    }

    /// A claimed file's bytes, as a complete response (`?id=&session=`).
    fn file(&self, req: &Request) -> Result<Response> {
        let id = crate::http::query(req, "id").unwrap_or_default();
        let session = crate::http::query(req, "session").unwrap_or_default();
        let Some(row) = self
            .row(&id)
            .filter(|r| r.session_id.as_deref() == Some(session.as_str()))
        else {
            return internal::do_error(&ApiError::not_found("File"));
        };
        let mut bytes = Vec::with_capacity(row.size as usize);
        let cursor = self.sql.exec(
            "SELECT data FROM chunks WHERE file_id = ? ORDER BY n",
            Some(vec![sv(&id)]),
        )?;
        for r in cursor.raw() {
            if let Some(SqlStorageValue::Blob(chunk)) = r?.into_iter().next() {
                bytes.extend_from_slice(&chunk);
            }
        }
        if bytes.len() as u64 != row.size as u64 {
            worker::console_error!("file {id} has {} of {} bytes", bytes.len(), row.size);
            return internal::do_error(&api_err(ErrorCode::Internal, "The file is incomplete"));
        }
        let att = row.attachment();
        let inline = files::shows_inline(&att.media_type);
        let h = Headers::new();
        h.set("Content-Type", &att.media_type)?;
        h.set("Content-Length", &att.size.to_string())?;
        h.set(
            "Content-Disposition",
            &files::content_disposition(&att.name, inline),
        )?;
        // Files never change; only people who can see the session get them.
        h.set("Cache-Control", "private, max-age=604800, immutable")?;
        h.set("X-Content-Type-Options", "nosniff")?;
        h.set(
            "Content-Security-Policy",
            "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; sandbox",
        )?;
        h.set("Cross-Origin-Resource-Policy", "same-origin")?;
        h.set("Referrer-Policy", "same-origin")?;
        Ok(Response::from_bytes(bytes)?.with_headers(h))
    }

    /// Deletes uploads no message claimed in time, then waits for the next.
    async fn expire_unclaimed(&self, now: i64) {
        let cutoff = now - files::UNSENT_TTL_MS;
        for stmt in [
            "DELETE FROM chunks WHERE file_id IN (SELECT id FROM files WHERE session_id IS NULL AND created_at < ?)",
            "DELETE FROM files WHERE session_id IS NULL AND created_at < ?",
        ] {
            if let Err(e) = self.sql.exec(stmt, Some(vec![iv(cutoff)])) {
                worker::console_error!("expiring files: {e}");
            }
        }
        let oldest = self
            .sql
            .exec(
                "SELECT MIN(created_at) AS created_at FROM files WHERE session_id IS NULL",
                None,
            )
            .and_then(|c| c.to_array::<Oldest>())
            .ok()
            .and_then(|r| r.into_iter().next())
            .and_then(|o| o.created_at);
        if let Some(t) = oldest {
            let due = (t as i64 + files::UNSENT_TTL_MS + 60_000 - now).max(60_000);
            let _ = self.state.storage().set_alarm(due).await;
        }
    }

    /// Deletes every file: the project was deleted (`housekeeping::purge_files`).
    async fn purge(&self) -> std::result::Result<serde_json::Value, ApiError> {
        let failed = |e: worker::Error| api_err(ErrorCode::Internal, &format!("purge: {e}"));
        let storage = self.state.storage();
        storage.delete_alarm().await.map_err(failed)?;
        storage.delete_all().await.map_err(failed)?;
        create_schema(&self.sql);
        Ok(serde_json::json!({ "ok": true }))
    }
}
