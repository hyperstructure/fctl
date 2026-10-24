//! SearchObject: one Durable Object per workspace, the search index of its
//! sessions (`fctl_domain::search`).
//!
//! Each session's SessionObject puts its documents here as they come (its
//! title and description, people's messages and the agent's replies), and
//! takes them out when its project is deleted. A search looks among the
//! sessions the Worker says the caller sees: those of some projects, and
//! some shared with them. The index is SQLite's FTS5 with the `trigram`
//! tokenizer, which finds any text of three characters or more, case and
//! accents aside; shorter searches look row by row. It lives here rather
//! than in D1, which can't export a database with virtual tables (ADR 0056),
//! and holds nothing that can't be indexed again from the sessions.

use crate::internal::{self, ForgetSession, IndexHit, IndexHits, PutDocs, SearchIndex};
use fctl_domain::search::{self, Doc, DocKind};
use fctl_proto::session::UserRef;
use fctl_proto::{ApiError, ErrorCode};
use serde::Deserialize;
use std::collections::HashMap;
use worker::{
    DurableObject, Env, Request, Response, Result, SqlStorage, SqlStorageValue, State,
    durable_object,
};

#[durable_object]
pub struct SearchObject {
    sql: SqlStorage,
}

fn sv(s: &str) -> SqlStorageValue {
    SqlStorageValue::from(s)
}

fn iv(n: i64) -> SqlStorageValue {
    SqlStorageValue::from(n)
}

fn create_schema(sql: &SqlStorage) {
    for stmt in [
        // A session's documents: `seq` 0 holds its title (`part` 0) and
        // description (1), other events a message or a reply each.
        "CREATE TABLE IF NOT EXISTS docs (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, project_id TEXT NOT NULL, \
         seq INTEGER NOT NULL, part INTEGER NOT NULL, kind TEXT NOT NULL, ts INTEGER NOT NULL, author TEXT, text TEXT NOT NULL)",
        "CREATE UNIQUE INDEX IF NOT EXISTS docs_at ON docs (session_id, seq, part)",
        // The index of their text, which the triggers keep in step.
        "CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(text, content = 'docs', content_rowid = 'id', \
         tokenize = 'trigram remove_diacritics 1')",
        "CREATE TRIGGER IF NOT EXISTS docs_added AFTER INSERT ON docs BEGIN \
         INSERT INTO docs_fts (rowid, text) VALUES (new.id, new.text); END",
        "CREATE TRIGGER IF NOT EXISTS docs_removed AFTER DELETE ON docs BEGIN \
         INSERT INTO docs_fts (docs_fts, rowid, text) VALUES ('delete', old.id, old.text); END",
        "CREATE TRIGGER IF NOT EXISTS docs_changed AFTER UPDATE OF text ON docs BEGIN \
         INSERT INTO docs_fts (docs_fts, rowid, text) VALUES ('delete', old.id, old.text); \
         INSERT INTO docs_fts (rowid, text) VALUES (new.id, new.text); END",
    ] {
        if let Err(e) = sql.exec(stmt, None) {
            worker::console_error!("search schema: {e}");
        }
    }
}

#[derive(Deserialize)]
struct SessionRow {
    session_id: String,
    titled: f64,
    last: f64,
    n: f64,
}

#[derive(Deserialize)]
struct MatchRow {
    session_id: String,
    seq: f64,
    kind: String,
    ts: f64,
    author: Option<String>,
    text: String,
}

impl DurableObject for SearchObject {
    fn new(state: State, _env: Env) -> Self {
        let sql = state.storage().sql();
        create_schema(&sql);
        Self { sql }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let bad = |e: worker::Error| ApiError::new(ErrorCode::BadRequest, e.to_string());
        let result = match (req.method(), req.path().as_str()) {
            (worker::Method::Post, "/put") => match req.json::<PutDocs>().await {
                Ok(b) => self.put(&b).map(|()| serde_json::json!({ "ok": true })),
                Err(e) => Err(bad(e)),
            },
            (worker::Method::Post, "/forget") => match req.json::<ForgetSession>().await {
                Ok(b) => self
                    .forget(b.session_id.as_str())
                    .map(|()| serde_json::json!({ "ok": true })),
                Err(e) => Err(bad(e)),
            },
            (worker::Method::Post, "/search") => match req.json::<SearchIndex>().await {
                Ok(b) => self.search(&b).map(|h| serde_json::json!(h)),
                Err(e) => Err(bad(e)),
            },
            _ => Err(ApiError::not_found("Search endpoint")),
        };
        match result {
            Ok(v) => internal::do_json(&v),
            Err(e) => internal::do_error(&e),
        }
    }
}

fn failed(e: worker::Error) -> ApiError {
    ApiError::new(ErrorCode::Internal, format!("search index: {e}"))
}

impl SearchObject {
    /// Adds a session's documents, or changes them: the same event's
    /// documents again replace those of before.
    fn put(&self, b: &PutDocs) -> std::result::Result<(), ApiError> {
        let session = b.session_id.as_str();
        if let Some(about) = &b.about {
            // A description taken out goes from the index too.
            self.sql
                .exec(
                    "DELETE FROM docs WHERE session_id = ? AND seq = 0 AND part >= ?",
                    Some(vec![sv(session), iv(about.len() as i64)]),
                )
                .map_err(failed)?;
        }
        for doc in b.about.iter().flatten().chain(&b.docs) {
            self.upsert(session, b.project_id.as_str(), doc)?;
        }
        Ok(())
    }

    fn upsert(&self, session: &str, project: &str, d: &Doc) -> std::result::Result<(), ApiError> {
        let author = d
            .author
            .as_ref()
            .and_then(|a| serde_json::to_string(a).ok());
        self.sql
            .exec(
                "INSERT INTO docs (session_id, project_id, seq, part, kind, ts, author, text) VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (session_id, seq, part) DO UPDATE SET kind = excluded.kind, ts = excluded.ts, \
                 author = excluded.author, text = excluded.text WHERE text != excluded.text OR ts != excluded.ts",
                Some(vec![
                    sv(session),
                    sv(project),
                    iv(d.seq),
                    iv(i64::from(d.part)),
                    sv(d.kind.as_str()),
                    iv(d.ts),
                    author.as_deref().map_or(SqlStorageValue::Null, sv),
                    sv(&d.text),
                ]),
            )
            .map_err(failed)?;
        Ok(())
    }

    fn forget(&self, session: &str) -> std::result::Result<(), ApiError> {
        self.sql
            .exec(
                "DELETE FROM docs WHERE session_id = ?",
                Some(vec![sv(session)]),
            )
            .map_err(failed)?;
        Ok(())
    }

    /// The sessions among those `b` names whose documents contain `b.q`:
    /// those with it in their title first, then the others, the most
    /// recently mentioned first, each with where it matched.
    fn search(&self, b: &SearchIndex) -> std::result::Result<IndexHits, ApiError> {
        let indexed = search::indexable(&b.q);
        // An index of trigrams finds three characters or more; shorter
        // searches go through the documents one by one.
        let (from, matching, pattern) = if indexed {
            (
                "docs_fts JOIN docs d ON d.id = docs_fts.rowid",
                "docs_fts MATCH ?",
                search::fts_phrase(&b.q),
            )
        } else {
            (
                "docs d",
                "d.text LIKE ? ESCAPE '\\'",
                search::like_pattern(&b.q),
            )
        };
        let json = |ids: &[String]| serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
        let sessions: Vec<SessionRow> = self
            .sql
            .exec(
                &format!(
                    "SELECT d.session_id AS session_id, MAX(d.kind = 'title') AS titled, MAX(d.ts) AS last, COUNT(*) AS n \
                     FROM {from} WHERE {matching} \
                     AND (d.project_id IN (SELECT value FROM json_each(?)) OR d.session_id IN (SELECT value FROM json_each(?))) \
                     GROUP BY d.session_id ORDER BY titled DESC, last DESC LIMIT ?"
                ),
                Some(vec![
                    sv(&pattern),
                    sv(&json(&b.projects)),
                    sv(&json(&b.sessions)),
                    iv(i64::from(b.limit) + 1),
                ]),
            )
            .and_then(|c| c.to_array())
            .map_err(failed)?;
        if sessions.is_empty() {
            return Ok(IndexHits {
                sessions: Vec::new(),
            });
        }
        // Where each matched: its title and description first, then its
        // newest messages and replies. A few more than it shows, for those
        // that say the same as another (a task that is the title still).
        let ids: Vec<String> = sessions.iter().map(|s| s.session_id.clone()).collect();
        let rows: Vec<MatchRow> = self
            .sql
            .exec(
                &format!(
                    "SELECT session_id, seq, kind, ts, author, text FROM (\
                     SELECT d.session_id AS session_id, d.seq AS seq, d.kind AS kind, d.ts AS ts, d.author AS author, d.text AS text, \
                     ROW_NUMBER() OVER (PARTITION BY d.session_id ORDER BY d.seq != 0, d.part, d.seq DESC) AS r \
                     FROM {from} WHERE {matching} AND d.session_id IN (SELECT value FROM json_each(?))) \
                     WHERE r <= ? ORDER BY r"
                ),
                Some(vec![
                    sv(&pattern),
                    sv(&json(&ids)),
                    iv(search::MATCHES_SHOWN as i64 + 2),
                ]),
            )
            .and_then(|c| c.to_array())
            .map_err(failed)?;
        let mut matches: HashMap<String, Vec<fctl_proto::api::SearchMatch>> = HashMap::new();
        let mut said: HashMap<String, Vec<String>> = HashMap::new();
        for r in rows {
            let Some(kind) = DocKind::parse(&r.kind) else {
                continue;
            };
            let texts = said.entry(r.session_id.clone()).or_default();
            if texts.len() >= search::MATCHES_SHOWN || texts.contains(&r.text) {
                continue;
            }
            texts.push(r.text.clone());
            let author = r
                .author
                .as_deref()
                .and_then(|a| serde_json::from_str::<UserRef>(a).ok());
            matches
                .entry(r.session_id)
                .or_default()
                .push(search::to_match(
                    kind,
                    r.seq as i64,
                    r.ts as i64,
                    author,
                    &r.text,
                    &b.q,
                ));
        }
        Ok(IndexHits {
            sessions: sessions
                .into_iter()
                .map(|s| IndexHit {
                    matches: matches.remove(&s.session_id).unwrap_or_default(),
                    session_id: s.session_id,
                    titled: s.titled > 0.0,
                    last: s.last as i64,
                    count: s.n as u32,
                })
                .collect(),
        })
    }
}
