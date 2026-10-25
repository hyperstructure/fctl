//! The SQLite files of a stopped `wrangler dev`: D1's, and each Durable
//! Object's. Node's own `node:sqlite` reads and writes them (Node 22.13 and
//! later have it), so xtask needs no SQLite of its own.

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Runs each statement with its parameters in one transaction, and prints
/// what the queries among them return.
const SCRIPT: &str = r#"
const { DatabaseSync } = require('node:sqlite');
let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (d) => (input += d));
process.stdin.on('end', () => {
  const ops = JSON.parse(input);
  const db = new DatabaseSync(process.argv[1]);
  const out = [];
  db.exec('BEGIN');
  try {
    for (const op of ops) {
      const st = db.prepare(op.sql);
      if (op.query) out.push(st.all(...op.params));
      else out.push(st.run(...op.params).changes);
    }
    db.exec('COMMIT');
  } catch (e) {
    db.exec('ROLLBACK');
    console.error(String(e && e.message ? e.message : e));
    process.exit(1);
  }
  db.close();
  process.stdout.write(JSON.stringify(out));
});
"#;

/// A database file, and the statements waiting to run on it.
pub struct Db {
    pub path: PathBuf,
    ops: Vec<Value>,
}

/// A row, by its columns' names.
pub type Row = Map<String, Value>;

impl Db {
    pub fn open(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            ops: Vec::new(),
        }
    }

    /// Runs `sql` now, and returns its rows.
    pub fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let out = run(
            &self.path,
            &[json!({"sql": sql, "params": params, "query": true})],
        )?;
        let rows = out
            .into_iter()
            .next()
            .and_then(|v| match v {
                Value::Array(rows) => Some(rows),
                _ => None,
            })
            .unwrap_or_default();
        Ok(rows
            .into_iter()
            .filter_map(|r| match r {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .collect())
    }

    /// Runs `sql` with the others in [`Db::commit`].
    pub fn exec(&mut self, sql: &str, params: Vec<Value>) {
        self.ops
            .push(json!({"sql": sql, "params": params, "query": false}));
    }

    /// Runs what [`Db::exec`] gathered, in one transaction.
    pub fn commit(&mut self) -> Result<()> {
        if self.ops.is_empty() {
            return Ok(());
        }
        let ops = std::mem::take(&mut self.ops);
        run(&self.path, &ops).map(|_| ())
    }
}

fn run(path: &Path, ops: &[Value]) -> Result<Vec<Value>> {
    let attempt = |flag: bool| -> Result<std::process::Output> {
        let mut cmd = Command::new("node");
        if flag {
            cmd.arg("--experimental-sqlite");
        }
        let mut child = cmd
            .arg("--no-warnings")
            .arg("-e")
            .arg(SCRIPT)
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("starting node, which edits the stopped stack's SQLite files")?;
        if let Some(mut stdin) = child.stdin.take() {
            // Node that can't load node:sqlite exits before it reads: what
            // it said decides, below.
            match stdin.write_all(serde_json::to_string(ops)?.as_bytes()) {
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
                written => written?,
            }
        }
        Ok(child.wait_with_output()?)
    };
    let mut out = attempt(false)?;
    // Node 22 before 22.13 has node:sqlite behind a flag.
    if !out.status.success() && String::from_utf8_lossy(&out.stderr).contains("node:sqlite") {
        out = attempt(true)?;
    }
    if !out.status.success() {
        bail!(
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    serde_json::from_slice(&out.stdout)
        .with_context(|| format!("reading what node said of {}", path.display()))
}

/// The SQLite files of a Durable Object class under `wrangler dev
/// --persist-to <state>`: one for each object.
pub fn objects(state: &Path, class: &str) -> Result<Vec<PathBuf>> {
    let dir = state.join("v3/do").join(format!("fctl-control-{class}"));
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "sqlite")
                && p.file_name().is_some_and(|n| n != "metadata.sqlite")
        })
        .collect();
    files.sort();
    Ok(files)
}

/// D1's SQLite file under `wrangler dev --persist-to <state>`: the one with
/// the control plane's tables.
pub fn d1(state: &Path) -> Result<PathBuf> {
    let dir = state.join("v3/d1/miniflare-D1DatabaseObject");
    for e in std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
    {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "sqlite")
            || path.file_name().is_some_and(|n| n == "metadata.sqlite")
        {
            continue;
        }
        let tables = Db::open(&path).query(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
            &[],
        )?;
        if !tables.is_empty() {
            return Ok(path);
        }
    }
    bail!(
        "no D1 database with the control plane's tables in {}",
        dir.display()
    )
}
