//! FilesObject: one Durable Object per project, holding the files people
//! attach to messages and the screenshots its agents publish.
//!
//! An upload is stored in chunks (SQLite rows hold at most 2 MB) and waits
//! for a message in one of the project's sessions to claim it; uploads no
//! message claimed are deleted after a day. Claimed files are read through
//! the Worker by the session's viewers and by the runner that runs it, and
//! deleted with the project.
//!
//! A published screenshot is stored the same way, with the time it
//! expires; the Worker serves it to anyone with its link until then (see
//! `api/published.rs`), and the object's alarm deletes it after.

use crate::ctx::now_ms;
use crate::internal::{
    self, ClaimFiles, ClaimedFiles, PublishFile, PublishedBytes, StoreFile, Unpublish,
};
use fctl_domain::{files, published};
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
struct Due {
    at: Option<f64>,
}

#[derive(Deserialize)]
struct PublishedRow {
    media_type: String,
    size: f64,
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
        // Their bytes are in `chunks` too, under their id.
        "CREATE TABLE IF NOT EXISTS published (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, name TEXT NOT NULL, media_type TEXT NOT NULL, \
         size INTEGER NOT NULL, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL)",
        "CREATE INDEX IF NOT EXISTS published_expiry ON published (expires_at)",
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
            (worker::Method::Post, "/publish") => self.publish(&mut req).await,
            (worker::Method::Get, "/published") => return self.published(&req),
            (worker::Method::Post, "/unpublish") => match req.json::<Unpublish>().await {
                Ok(b) => Ok(self.unpublish(&b.id)),
                Err(e) => Err(api_err(ErrorCode::BadRequest, &e.to_string())),
            },
            (worker::Method::Post, "/purge") => self.purge().await,
            _ => Err(api_err(ErrorCode::NotFound, "Unknown files endpoint")),
        };
        match result {
            Ok(v) => internal::do_json(&v),
            Err(e) => internal::do_error(&e),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        self.expire(now_ms()).await;
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
        let bytes = crate::http::read_upto(req, files::MAX_FILE_BYTES as usize)
            .await
            .map_err(|e| {
                api_err(
                    ErrorCode::BadRequest,
                    &format!("Reading the upload: {}", e.0.message),
                )
            })?;
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
        self.write_chunks(att.id.as_str(), &bytes).map_err(failed)?;
        if let Err(e) = self.schedule(now + files::UNSENT_TTL_MS, now).await {
            worker::console_error!("scheduling the files alarm: {e}");
        }
        serde_json::to_value(&att).map_err(|e| api_err(ErrorCode::Internal, &e.to_string()))
    }

    fn write_chunks(&self, id: &str, bytes: &[u8]) -> Result<()> {
        for (n, chunk) in bytes.chunks(files::CHUNK_BYTES).enumerate() {
            self.sql.exec(
                "INSERT INTO chunks (file_id, n, data) VALUES (?, ?, ?)",
                Some(vec![
                    sv(id),
                    iv(n as i64),
                    SqlStorageValue::Blob(chunk.to_vec()),
                ]),
            )?;
        }
        Ok(())
    }

    /// A file's bytes, if they are all there.
    fn read_chunks(&self, id: &str, size: u64) -> Result<Option<Vec<u8>>> {
        let mut bytes = Vec::with_capacity(size as usize);
        let cursor = self.sql.exec(
            "SELECT data FROM chunks WHERE file_id = ? ORDER BY n",
            Some(vec![sv(id)]),
        )?;
        for r in cursor.raw() {
            if let Some(SqlStorageValue::Blob(chunk)) = r?.into_iter().next() {
                bytes.extend_from_slice(&chunk);
            }
        }
        if bytes.len() as u64 != size {
            worker::console_error!("file {id} has {} of {size} bytes", bytes.len());
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    /// Bytes `start..=end` of a file, from only the chunks that hold them.
    fn read_range(&self, id: &str, start: u64, end: u64) -> Result<Option<Vec<u8>>> {
        let (first, last, offset) = files::chunk_span(start, end);
        let mut bytes = Vec::new();
        let cursor = self.sql.exec(
            "SELECT data FROM chunks WHERE file_id = ? AND n BETWEEN ? AND ? ORDER BY n",
            Some(vec![sv(id), iv(first as i64), iv(last as i64)]),
        )?;
        for r in cursor.raw() {
            if let Some(SqlStorageValue::Blob(chunk)) = r?.into_iter().next() {
                bytes.extend_from_slice(&chunk);
            }
        }
        let part = bytes.get(offset..offset + (end - start) as usize + 1);
        if part.is_none() {
            worker::console_error!("file {id} lacks bytes {start}-{end}");
        }
        Ok(part.map(<[u8]>::to_vec))
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

    /// A claimed file's bytes (`?id=&session=`): all of them, or the range
    /// a `Range` header asks for, which video players need.
    fn file(&self, req: &Request) -> Result<Response> {
        let id = crate::http::query(req, "id").unwrap_or_default();
        let session = crate::http::query(req, "session").unwrap_or_default();
        let Some(row) = self
            .row(&id)
            .filter(|r| r.session_id.as_deref() == Some(session.as_str()))
        else {
            return internal::do_error(&ApiError::not_found("File"));
        };
        let att = row.attachment();
        let range = files::byte_range(crate::http::header(req, "Range").as_deref(), att.size);
        let h = Headers::new();
        h.set("Accept-Ranges", "bytes")?;
        let (status, bytes) = match range {
            files::ByteRange::Whole => (200, self.read_chunks(&id, att.size)?),
            files::ByteRange::Part { start, end } => {
                h.set(
                    "Content-Range",
                    &format!("bytes {start}-{end}/{}", att.size),
                )?;
                (206, self.read_range(&id, start, end)?)
            }
            files::ByteRange::Unsatisfiable => {
                h.set("Content-Range", &format!("bytes */{}", att.size))?;
                return Ok(Response::empty()?.with_status(416).with_headers(h));
            }
        };
        let Some(bytes) = bytes else {
            return internal::do_error(&api_err(ErrorCode::Internal, "The file is incomplete"));
        };
        let inline = files::shows_inline(&att.media_type);
        h.set("Content-Type", files::served_type(&att.media_type))?;
        h.set("Content-Length", &bytes.len().to_string())?;
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
        Ok(Response::from_bytes(bytes)?
            .with_status(status)
            .with_headers(h))
    }

    /// Deletes uploads no message claimed in time and published screenshots
    /// that expired, then waits for the next of either.
    async fn expire(&self, now: i64) {
        let cutoff = now - files::UNSENT_TTL_MS;
        for (stmt, at) in [
            (
                "DELETE FROM chunks WHERE file_id IN (SELECT id FROM files WHERE session_id IS NULL AND created_at < ?)",
                cutoff,
            ),
            (
                "DELETE FROM files WHERE session_id IS NULL AND created_at < ?",
                cutoff,
            ),
            (
                "DELETE FROM chunks WHERE file_id IN (SELECT id FROM published WHERE expires_at <= ?)",
                now,
            ),
            ("DELETE FROM published WHERE expires_at <= ?", now),
        ] {
            if let Err(e) = self.sql.exec(stmt, Some(vec![iv(at)])) {
                worker::console_error!("expiring files: {e}");
            }
        }
        let next = [
            self.due(&format!(
                "SELECT MIN(created_at) + {} AS at FROM files WHERE session_id IS NULL",
                files::UNSENT_TTL_MS
            )),
            self.due("SELECT MIN(expires_at) AS at FROM published"),
        ]
        .into_iter()
        .flatten()
        .min();
        // The alarm that runs this is spent: set the next one outright.
        if let Some(at) = next {
            let due = (at + 60_000 - now).max(60_000);
            let _ = self.state.storage().set_alarm(due).await;
        }
    }

    /// The time a query's `at` column names, if any.
    fn due(&self, query: &str) -> Option<i64> {
        self.sql
            .exec(query, None)
            .and_then(|c| c.to_array::<Due>())
            .ok()
            .and_then(|r| r.into_iter().next())
            .and_then(|d| d.at)
            .map(|t| t as i64)
    }

    /// Wakes the object a minute after `at` (but no sooner than a minute
    /// from now), unless it already wakes before then. Not for the alarm
    /// handler, whose own alarm is spent.
    async fn schedule(&self, at: i64, now: i64) -> Result<()> {
        let storage = self.state.storage();
        let due = (at + 60_000 - now).max(60_000);
        match storage.get_alarm().await? {
            Some(set) if set <= now + due => Ok(()),
            _ => storage.set_alarm(due).await,
        }
    }

    /// Stores a screenshot to publish (the request body) until it expires,
    /// if it is an image GitHub shows.
    async fn publish(&self, req: &mut Request) -> std::result::Result<serde_json::Value, ApiError> {
        let meta: PublishFile = crate::http::query(req, "meta")
            .and_then(|m| serde_json::from_str(&m).ok())
            .ok_or_else(|| api_err(ErrorCode::BadRequest, "No upload metadata"))?;
        let bytes = crate::http::read_upto(req, published::MAX_BYTES as usize)
            .await
            .map_err(|e| {
                api_err(
                    ErrorCode::BadRequest,
                    &format!("Reading the upload: {}", e.0.message),
                )
            })?;
        let image = published::check(&bytes)?;
        let now = now_ms();
        let failed = |e: worker::Error| {
            worker::console_error!("publishing a file: {e}");
            api_err(
                ErrorCode::Unavailable,
                "The screenshot could not be stored. Try again.",
            )
        };
        // No await in between: the rows are written together or not at all.
        self.sql
            .exec(
                "INSERT INTO published (id, session_id, name, media_type, size, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                Some(vec![
                    sv(&meta.id),
                    sv(meta.session_id.as_str()),
                    sv(&meta.name),
                    sv(image.media_type),
                    iv(bytes.len() as i64),
                    iv(now),
                    iv(meta.expires_at),
                ]),
            )
            .map_err(failed)?;
        self.write_chunks(&meta.id, &bytes).map_err(failed)?;
        if let Err(e) = self.schedule(meta.expires_at, now).await {
            worker::console_error!("scheduling the files alarm: {e}");
        }
        serde_json::to_value(PublishedBytes {
            media_type: image.media_type.to_owned(),
            size: bytes.len() as u64,
            width: image.width,
            height: image.height,
        })
        .map_err(|e| api_err(ErrorCode::Internal, &e.to_string()))
    }

    /// A published screenshot's bytes and type (`?id=`), until it expires.
    /// The Worker gives the response the headers of a public image.
    fn published(&self, req: &Request) -> Result<Response> {
        let id = crate::http::query(req, "id").unwrap_or_default();
        let row = self
            .sql
            .exec(
                "SELECT media_type, size FROM published WHERE id = ? AND expires_at > ?",
                Some(vec![sv(&id), iv(now_ms())]),
            )
            .and_then(|c| c.to_array::<PublishedRow>())
            .ok()
            .and_then(|r| r.into_iter().next());
        let Some(row) = row else {
            return internal::do_error(&ApiError::not_found("Screenshot"));
        };
        let Some(bytes) = self.read_chunks(&id, row.size as u64)? else {
            return internal::do_error(&api_err(
                ErrorCode::Internal,
                "The screenshot is incomplete",
            ));
        };
        let h = Headers::new();
        h.set("Content-Type", &row.media_type)?;
        Ok(Response::from_bytes(bytes)?.with_headers(h))
    }

    /// Deletes a published screenshot now.
    fn unpublish(&self, id: &str) -> serde_json::Value {
        for stmt in [
            "DELETE FROM chunks WHERE file_id IN (SELECT id FROM published WHERE id = ?)",
            "DELETE FROM published WHERE id = ?",
        ] {
            if let Err(e) = self.sql.exec(stmt, Some(vec![sv(id)])) {
                worker::console_error!("unpublishing {id}: {e}");
            }
        }
        serde_json::json!({ "ok": true })
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
