// The sessions someone hid from the sidebar, remembered in this browser: a session that waits for
// a message, stopped or failed leaves the sidebar on its own an hour after it ends, and people who
// are done with it sooner hide it. It stays hidden while it rests, and comes back once its agent
// works again: a run starts, it asks something, or it ends another turn. Signing out forgets them.

import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";
import type * as G from "@/gen";

const KEY = "fctl:hidden-sessions";
// How long a hidden session is remembered: long after it left the sidebar on its own.
const KEPT_MS = 7 * 24 * 3_600_000;

/** A hidden session: how many turns it had ended when it was hidden, and when (this browser's time). */
interface Hidden {
  turns: number;
  at: number;
}

type Store = Record<string, Hidden>;

type Session = Pick<G.Session, "id" | "status" | "question"> & { stats: Pick<G.SessionStats, "turns"> };

/** Whether `s` can be hidden: its agent rests (it waits for a message, stopped or failed) and asks nothing. */
export const hideable = (s: Session) =>
  (s.status === "idle" || s.status === "stopped" || s.status === "failed") && !s.question;

/**
 * Whether `s`, hidden as `h`, stays hidden: its agent still rests (on its way to stopping too) and
 * asks nothing, and it has ended no turn since.
 */
export const staysHidden = (s: Session, h: Hidden | undefined) =>
  !!h && (hideable(s) || (s.status === "stopping" && !s.question)) && s.stats.turns <= h.turns;

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
    const { turns, at } = v as Record<string, unknown>;
    if (typeof turns === "number" && typeof at === "number") out[id] = { turns, at };
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
  // The hidden ones whose agents work again.
  const back = useMemo(
    () => (sessions ?? []).filter((s) => store[s.id] && !hidden.has(s.id)).map((s) => s.id),
    [sessions, store, hidden],
  );
  useEffect(() => {
    if (!back.length) return;
    const next = parse(snapshot());
    for (const id of back) delete next[id];
    write(next);
  }, [back]);
  const hide = useCallback((s: Session) => {
    write({ ...parse(snapshot()), [s.id]: { turns: s.stats.turns, at: Date.now() } });
  }, []);
  return { hidden, hide };
}
