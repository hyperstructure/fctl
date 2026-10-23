// The sections a project's page lists its sessions in, as the Agent Floor
// lists everyone's: the ones at work, the ones that ended lately, and the rest.

import type * as G from "@/gen";

/** The statuses of sessions that haven't ended, which `?status=active` lists. */
export const ACTIVE: readonly G.SessionStatus[] = [
  "queued",
  "provisioning",
  "preparing",
  "running",
  "idle",
  "paused",
  "stopping",
];

/**
 * How long an ended session counts as recent: a day, while lists give its
 * time in minutes or hours (`agoShort`).
 */
export const RECENT_MS = 24 * 3_600_000;

/** When a session last moved, as lists show it. */
export const lastMoved = (s: Pick<G.Session, "last_activity_at" | "created_at">) =>
  s.last_activity_at ?? s.created_at;

export interface Sections {
  /** The sessions that haven't ended. */
  working: G.Session[];
  /** The ended sessions that moved in the last day. */
  recent: G.Session[];
  /** The other ended sessions. */
  older: G.Session[];
}

/**
 * The sessions of a page's lists in their sections: the ones at work newest
 * first, where they keep their places as they work, and the ended ones by
 * when they last moved, as their times read. One that ends or resumes moves
 * at once, as the feed marks it in the list that has it, and one in two
 * lists, as it may be while they come again, counts once, as its newest
 * summary has it.
 */
export function sections(lists: readonly (readonly G.Session[] | undefined)[], now: number): Sections {
  const byId = new Map<string, G.Session>();
  for (const list of lists)
    for (const s of list ?? []) {
      const seen = byId.get(s.id);
      if (!seen || s.updated_at > seen.updated_at) byId.set(s.id, s);
    }
  const out: Sections = { working: [], recent: [], older: [] };
  for (const s of [...byId.values()].sort((a, b) => b.created_at - a.created_at)) {
    if (ACTIVE.includes(s.status)) out.working.push(s);
    else if (now - lastMoved(s) < RECENT_MS) out.recent.push(s);
    else out.older.push(s);
  }
  const moved = (a: G.Session, b: G.Session) => lastMoved(b) - lastMoved(a);
  out.recent.sort(moved);
  out.older.sort(moved);
  return out;
}
