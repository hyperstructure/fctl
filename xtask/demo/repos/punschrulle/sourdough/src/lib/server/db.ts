import Database from "better-sqlite3";
import type { Bake } from "$lib/plan";

export const db = new Database(process.env.DATABASE_PATH ?? "sourdough.db");
db.pragma("journal_mode = WAL");

db.exec(`
  CREATE TABLE IF NOT EXISTS bakes (
    id INTEGER PRIMARY KEY,
    recipe TEXT NOT NULL,
    ready_at TEXT NOT NULL
  );
  CREATE TABLE IF NOT EXISTS feedings (
    id INTEGER PRIMARY KEY,
    fed_at TEXT NOT NULL,
    flour TEXT NOT NULL,
    grams INTEGER NOT NULL,
    photo TEXT
  );
`);

/** The bakes still to come, soonest first. */
export function upcomingBakes(now = new Date()): Bake[] {
  const rows = db
    .prepare("SELECT id, recipe, ready_at FROM bakes WHERE ready_at > ? ORDER BY ready_at")
    .all(now.toISOString()) as { id: number; recipe: string; ready_at: string }[];
  return rows.map((r) => ({ id: r.id, recipe: r.recipe, readyAt: new Date(r.ready_at) }));
}
