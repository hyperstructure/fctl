// The sessions someone hid from the sidebar, remembered in this browser: a session that waits for
// a message, stopped or failed leaves the sidebar on its own an hour after it ends, and people who
// are done with it sooner hide it. It stays hidden while it rests, and comes back once its agent
// works again: a run starts, it asks something, or it ends another turn. Signing out forgets them.

import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";
import type * as G from "@/gen";

const KEY = "fctl:hidden-sessions";
// How long a hidden session is remembered: long after it left the sidebar on its own. The privacy
// policy (hyperstructure/fctl-site's legal/privacy.md) says so.
const KEPT_MS = 7 * 24 * 3_600_000;

/**
 * A hidden session: how many turns it had ended when it was hidden, the time of the summary it was
 * hidden from (the control plane's `updated_at`), and when it was hidden (this browser's time).
 */
interface Hidden {
  turns: number;
  updated: number;
  at: number;
}

type Store = Record<string, Hidden>;

type Session = Pick<G.Session, "id" | "status" | "question" | "updated_at"> & {
  stats: Pick<G.SessionStats, "turns">;
};

/** Whether `s` can be hidden: its agent rests (it waits for a message, stopped or failed) and asks nothing. */
export const hideable = (s: Session) =>
  (s.status === "idle" || s.status === "stopped" || s.status === "failed") && !s.question;

/**
 * Whether `s`, hidden as `h`, stays hidden: its agent still rests (on its way to stopping too) and
 * asks nothing, and it has ended no turn since.
 */
export const staysHidden = (s: Session, h: Hidden | undefined) =>
  !!h && (hideable(s) || (s.status === "stopping" && !s.question)) && s.stats.turns <= h.turns;

/**
 * Whether `s`, hidden as `h`, came back for good: it doesn't stay hidden, and this summary of it is
 * newer than the one it was hidden from. Another tab whose lists are behind may still see it at
 * work on the turn it rested, stopped or failed after, with as many turns ended.
 */
export const cameBack = (s: Session, h: Hidden | undefined) =>
  !!h && !staysHidden(s, h) && s.updated_at > h.updated;

const listeners = new Set<() => void>();

function read(): string {
  try {
    return localStorage.getItem(KEY) ?? "{}";
  } catch {
    return "{}";
  }
}

function parse(raw: string): Store {
  let d: unknown;
  try {
    d = JSON.parse(raw);
  } catch {
    return {};
  }
  if (typeof d !== "object" || d === null || Array.isArray(d)) return {};
  const out: Store = {};
  for (const [id, v] of Object.entries(d)) {
    if (typeof v !== "object" || v === null) continue;
    const { turns, updated, at } = v as Record<string, unknown>;
    if (typeof turns === "number" && typeof updated === "number" && typeof at === "number")
      out[id] = { turns, updated, at };
  }
  return out;
}

/** Keeps `store`, without the sessions hidden longer ago than anyone needs. */
function write(store: Store, now = Date.now()) {
  const kept = Object.fromEntries(Object.entries(store).filter(([, h]) => now - h.at < KEPT_MS));
  try {
    localStorage.setItem(KEY, JSON.stringify(kept));
  } catch {
    // Private browsing without storage: hiding lasts the page.
    memory = JSON.stringify(kept);
  }
  for (const l of listeners) l();
}

// Where the store lives when this browser keeps nothing.
let memory: string | undefined;

const snapshot = () => memory ?? read();

/** Forgets the sessions hidden (signing out). */
export function forgetHiddenSessions() {
  memory = undefined;
  try {
    localStorage.removeItem(KEY);
  } catch {}
  for (const l of listeners) l();
}

/**
 * The sessions of `sessions` hidden from the sidebar, and how to hide one. A hidden session whose
 * agent works again is forgotten, so that it shows when it rests again.
 */
export function useHiddenSessions(sessions: readonly Session[] | undefined): {
  hidden: ReadonlySet<string>;
  hide: (s: Session) => void;
} {
  const raw = useSyncExternalStore(
    (fn) => {
      listeners.add(fn);
      window.addEventListener("storage", fn);
      return () => {
        listeners.delete(fn);
        window.removeEventListener("storage", fn);
      };
    },
    snapshot,
    () => "{}",
  );
  const store = useMemo(() => parse(raw), [raw]);
  const hidden = useMemo(
    () => new Set((sessions ?? []).filter((s) => staysHidden(s, store[s.id])).map((s) => s.id)),
    [sessions, store],
  );
  // The hidden ones whose agents work again, as summaries newer than the hide say: another tab
  // whose lists are behind, where one still works on the turn it was hidden after, leaves it.
  const back = useMemo(
    () => (sessions ?? []).filter((s) => cameBack(s, store[s.id])).map((s) => s.id),
    [sessions, store],
  );
  // Those, and those hidden longer ago than anyone needs, go.
  useEffect(() => {
    const now = Date.now();
    const old = Object.values(store).some((h) => now - h.at >= KEPT_MS);
    if (!back.length && !old) return;
    const next = parse(snapshot());
    for (const id of back) delete next[id];
    write(next, now);
  }, [back, store]);
  const hide = useCallback((s: Session) => {
    write({ ...parse(snapshot()), [s.id]: { turns: s.stats.turns, updated: s.updated_at, at: Date.now() } });
  }, []);
  return { hidden, hide };
}
