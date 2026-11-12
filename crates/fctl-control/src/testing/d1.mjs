// A stand-in for D1 in fctl-control's tests (crate::testing): SQLite in
// memory through node:sqlite, with the control plane's migrations applied,
// and the part of D1's API that fctl-control calls (prepare, bind, first,
// all, run and batch). Like D1, it binds numbers as REAL, runs a batch in
// one transaction that a failing statement rolls back, and answers each
// call with a promise, so that two futures racing in a test interleave at
// each query.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";

class D1PreparedStatement {
  #db;
  #sql;
  #params;

  constructor(db, sql, params = []) {
    this.#db = db;
    this.#sql = sql;
    this.#params = params;
  }

  bind(...params) {
    return new D1PreparedStatement(this.#db, this.#sql, params);
  }

  /** Runs it now: what D1 answers for it. */
  execute() {
    const changed = () => this.#db.prepare("SELECT total_changes() AS n").get().n;
    const before = changed();
    const statement = this.#db.prepare(this.#sql);
    let results = [];
    if (statement.columns().length > 0) {
      results = statement.all(...this.#params);
    } else {
      statement.run(...this.#params);
    }
    const changes = changed() - before;
    return {
      success: true,
      results: results.map((row) => ({ ...row })),
      meta: { changes, changed_db: changes > 0, duration: 0 },
    };
  }

  /** Runs it now, failing as D1 does. */
  #answer() {
    try {
      return this.execute();
    } catch (e) {
      throw d1Error(e);
    }
  }

  async first(column) {
    const row = this.#answer().results[0] ?? null;
    if (column === undefined || row === null) {
      return row;
    }
    if (!(column in row)) {
      throw new Error(`D1_COLUMN_NOTFOUND: Column not found (${column})`);
    }
    return row[column];
  }

  async all() {
    return this.#answer();
  }

  async run() {
    return this.#answer();
  }
}

/** An error as D1 throws it. */
function d1Error(e) {
  return new Error(`D1_ERROR: ${e.message}`, { cause: e });
}

class D1Database {
  #db;

  constructor(db) {
    this.#db = db;
  }

  prepare(sql) {
    return new D1PreparedStatement(this.#db, sql);
  }

  async batch(statements) {
    this.#db.exec("BEGIN");
    try {
      const results = statements.map((s) => s.execute());
      this.#db.exec("COMMIT");
      return results;
    } catch (e) {
      this.#db.exec("ROLLBACK");
      throw d1Error(e);
    }
  }
}

/** A fresh database with the migrations in `migrations` applied in order. */
export function database(migrations) {
  const db = new DatabaseSync(":memory:");
  const names = readdirSync(migrations).filter((n) => n.endsWith(".sql"));
  for (const name of names.sort()) {
    db.exec(readFileSync(join(migrations, name), "utf8"));
  }
  return new D1Database(db);
}
