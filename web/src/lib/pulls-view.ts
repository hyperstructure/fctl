// How the pull requests page was left, kept in this browser, so that it is as it was when someone
// comes back to it. Mine, on until they turn it off, lasts until they change it, and the sidebar's
// counts follow it. The tab and the filter last as long as the browser tab, so that a new window
// starts on Open with nothing filtered.

import { useCallback, useMemo, useSyncExternalStore } from "react";
import { PULL_STATES, type PullState } from "./pull-list";

/** localStorage: "0" once Mine is turned off. */
const MINE = "fctl:pulls-mine";
/** sessionStorage: the tab and the filter the page last showed, as JSON. */
const LEFT = "fctl:pulls-left";

/** What the page showed last, besides its workspace or project, which its address says. */
export interface PullsLeft {
  state?: PullState;
  q?: string;
}

const listeners = new Set<() => void>();
/** What the browser didn't let us keep (private browsing without storage, or none left): for the page. */
const unkept = new Map<string, string | null>();

function get(storage: () => Storage, key: string): string | null {
  if (unkept.has(key)) return unkept.get(key) ?? null;
  try {
    return storage().getItem(key);
  } catch {
    return null;
  }
}

function put(storage: () => Storage, key: string, value: string | null) {
  try {
    if (value === null) storage().removeItem(key);
    else storage().setItem(key, value);
    unkept.delete(key);
  } catch {
    unkept.set(key, value);
  }
  for (const l of listeners) l();
}

function subscribe(fn: () => void) {
  listeners.add(fn);
  // Mine turned on or off in another tab.
  window.addEventListener("storage", fn);
  return () => {
    listeners.delete(fn);
    window.removeEventListener("storage", fn);
  };
}

const local = () => localStorage;
const session = () => sessionStorage;

/** Whether the page lists only the pull requests of sessions you started, and a setter. */
export function usePullsMine(): [boolean, (mine: boolean) => void] {
  const raw = useSyncExternalStore(
    subscribe,
    () => get(local, MINE),
    () => null,
  );
  return [raw !== "0", useCallback((mine: boolean) => put(local, MINE, mine ? null : "0"), [])];
}

/** The tab and the filter the page showed last in this browser tab. */
export function usePullsLeft(): PullsLeft {
  const raw = useSyncExternalStore(
    subscribe,
    () => get(session, LEFT),
    () => null,
  );
  return useMemo(() => parseLeft(raw), [raw]);
}

/** Keeps what the page shows, for when it is opened again. */
export function keepPullsLeft(left: PullsLeft) {
  const raw = JSON.stringify({ state: left.state, q: left.q || undefined });
  if (raw !== get(session, LEFT)) put(session, LEFT, raw);
}

export function parseLeft(raw: string | null): PullsLeft {
  let v: unknown;
  try {
    v = JSON.parse(raw ?? "null");
  } catch {
    return {};
  }
  if (!v || typeof v !== "object") return {};
  const { state, q } = v as Record<string, unknown>;
  return {
    state: PULL_STATES.find((s) => s === state),
    q: typeof q === "string" && q ? q : undefined,
  };
}
