//! Readings and each pot's calibration, in SQLite.

use rusqlite::{Connection, OptionalExtension, params};

pub fn open(path: &str) -> rusqlite::Result<Connection> {
    let db = Connection::open(path)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS readings (board TEXT NOT NULL, raw INTEGER NOT NULL, at INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS readings_board_at ON readings (board, at);
         CREATE TABLE IF NOT EXISTS calibrations (board TEXT PRIMARY KEY, dry INTEGER, wet INTEGER);",
    )?;
    Ok(db)
}

/// A pot's raw readings with its soil dry and just after watering.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Calibration {
    pub dry: Option<u16>,
    pub wet: Option<u16>,
}

pub fn record(db: &Connection, board: &str, raw: u16, at: i64) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO readings (board, raw, at) VALUES (?1, ?2, ?3)",
        params![board, raw, at],
    )?;
    Ok(())
}

pub fn calibration(db: &Connection, board: &str) -> rusqlite::Result<Calibration> {
    Ok(db
        .query_row(
            "SELECT dry, wet FROM calibrations WHERE board = ?1",
            params![board],
            |r| Ok(Calibration { dry: r.get(0)?, wet: r.get(1)? }),
        )
        .optional()?
        .unwrap_or_default())
}

/// Takes the pot's latest reading as its dry (or wet) one.
pub fn calibrate(db: &Connection, board: &str, wet: bool) -> rusqlite::Result<()> {
    let column = if wet { "wet" } else { "dry" };
    db.execute(
        &format!(
            "INSERT INTO calibrations (board, {column}) \
             VALUES (?1, (SELECT raw FROM readings WHERE board = ?1 ORDER BY at DESC LIMIT 1)) \
             ON CONFLICT (board) DO UPDATE SET {column} = excluded.{column}"
        ),
        params![board],
    )?;
    Ok(())
}
