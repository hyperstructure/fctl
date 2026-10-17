//! D1 helpers: parameter binding and typed queries.
//!
//! Queries are plain SQL with `?` placeholders. Bind values with [`Val`]:
//! `q(&db, "SELECT * FROM users WHERE id = ?", &[s(id)]).first::<UserRow>()`.

use crate::error::AppResult;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement};

/// A bound value.
pub type Val = JsValue;

pub fn s(v: &str) -> Val {
    JsValue::from_str(v)
}

pub fn os(v: Option<&str>) -> Val {
    v.map_or(JsValue::NULL, JsValue::from_str)
}

/// Integers. D1 numbers are f64, exact up to 2^53, which covers timestamps.
pub fn n(v: i64) -> Val {
    JsValue::from_f64(v as f64)
}

pub fn on(v: Option<i64>) -> Val {
    v.map_or(JsValue::NULL, |x| JsValue::from_f64(x as f64))
}

pub fn b(v: bool) -> Val {
    JsValue::from_f64(if v { 1.0 } else { 0.0 })
}

pub fn null() -> Val {
    JsValue::NULL
}

pub struct Query(D1PreparedStatement);

pub fn q(db: &D1Database, sql: &str, params: &[Val]) -> AppResult<Query> {
    Ok(Query(db.prepare(sql).bind(params)?))
}

impl Query {
    pub async fn first<T: DeserializeOwned>(self) -> AppResult<Option<T>> {
        Ok(self.0.first::<T>(None).await?)
    }

    pub async fn all<T: DeserializeOwned>(self) -> AppResult<Vec<T>> {
        Ok(self.0.all().await?.results::<T>()?)
    }

    /// Runs a statement; returns the number of changed rows.
    pub async fn run(self) -> AppResult<usize> {
        let r = self.0.run().await?;
        Ok(r.meta()?.and_then(|m| m.changes).unwrap_or(0))
    }

    pub fn statement(self) -> D1PreparedStatement {
        self.0
    }
}

/// Runs statements in one transaction.
pub async fn batch(db: &D1Database, statements: Vec<Query>) -> AppResult<()> {
    db.batch(statements.into_iter().map(Query::statement).collect())
        .await?;
    Ok(())
}

/// A single integer column, for `SELECT COUNT(*) AS n`.
#[derive(serde::Deserialize)]
pub struct Count {
    pub n: i64,
}

/// SQLite stores booleans as integers.
pub fn int_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v: serde_json::Value = serde::Deserialize::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        _ => false,
    })
}

/// D1 returns numbers as floats; accept both.
pub fn int<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    let v: serde_json::Value = serde::Deserialize::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        _ => 0,
    })
}

pub fn opt_int<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let v: serde_json::Value = serde::Deserialize::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        _ => None,
    })
}

/// Another handle to the same database (D1Database is not `Clone`).
pub fn handle(db: &D1Database) -> D1Database {
    use wasm_bindgen::JsCast;
    let js: &JsValue = db.as_ref();
    js.clone().unchecked_into()
}
