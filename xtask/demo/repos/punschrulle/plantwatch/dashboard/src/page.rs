//! The page: every pot, how wet it is, and which one is thirsty.

use crate::{db, thirst};
use axum::extract::{Path, State};
use axum::response::{Html, Redirect};
use axum::routing::{get, post};
use axum::Router;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

type Db = Arc<Mutex<Connection>>;

pub fn router(db: Db) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/pots/{board}/calibrate/{which}", post(calibrate))
        .with_state(db)
}

async fn index(State(db): State<Db>) -> Html<String> {
    let Ok(db) = db.lock() else { return Html("Busy".into()) };
    let mut rows = String::new();
    for board in ["living-room", "kitchen", "bedroom", "balcony"] {
        let raw: Option<u16> = db
            .query_row("SELECT raw FROM readings WHERE board = ?1 ORDER BY at DESC LIMIT 1", [board], |r| r.get(0))
            .ok();
        let cal = db::calibration(&db, board).unwrap_or_default();
        let wet = raw.and_then(|r| thirst::percent(r, cal));
        let note = match wet {
            Some(p) if thirst::thirsty(p) => format!("{p}%: thirsty"),
            Some(p) => format!("{p}%"),
            None => "needs calibrating".into(),
        };
        rows.push_str(&format!("<li><b>{board}</b> {note}</li>"));
    }
    Html(format!("<!doctype html><title>plantwatch</title><ul>{rows}</ul>"))
}

async fn calibrate(State(db): State<Db>, Path((board, which)): Path<(String, String)>) -> Redirect {
    if let Ok(db) = db.lock() {
        let _ = db::calibrate(&db, &board, which == "wet");
    }
    Redirect::to("/")
}
