// How wide the sidebar is and whether it is folded into its rail, as each person leaves it, kept
// in this browser.

import { useCallback, useSyncExternalStore } from "react";
import { shortcutText } from "./shortcuts";

/** Its width at first, and at least and at most when dragged (CSS pixels). */
export const SIDEBAR_WIDTH = { initial: 288, min: 240, max: 420 } as const;
/** The rail it folds into. */
export const RAIL_WIDTH = 60;
/** What folds and unfolds it, as its tooltips say it (lib/shortcuts.ts). */
export const foldKeys = () => shortcutText("fold");

const WIDTH = "fctl:sidebar-width";
const FOLDED = "fctl:sidebar-folded";
const listeners = new Set<() => void>();

function get(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function put(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // Private browsing without storage: it lasts the page.
  }
  for (const l of listeners) l();
}

function subscribe(fn: () => void) {
  listeners.add(fn);
  window.addEventListener("storage", fn);
  return () => {
    listeners.delete(fn);
    window.removeEventListener("storage", fn);
  };
}

const clamp = (w: number) => Math.round(Math.min(SIDEBAR_WIDTH.max, Math.max(SIDEBAR_WIDTH.min, w)));

/**
 * Where dragging its edge to `w` px leaves it: folded into its rail while the edge is nearer the
 * rail than its least width, else open that wide, within bounds.
 */
export const draggedTo = (w: number): number | "folded" =>
  w < (RAIL_WIDTH + SIDEBAR_WIDTH.min) / 2 ? "folded" : clamp(w);

/** Its width, and a setter that keeps a new one within bounds (none: back to the initial one). */
export function useSidebarWidth(): [number, (w: number | null) => void] {
  const raw = useSyncExternalStore(
    subscribe,
    () => get(WIDTH),
    () => null,
  );
  const n = Number(raw);
  const width = raw && Number.isFinite(n) ? clamp(n) : SIDEBAR_WIDTH.initial;
  const set = useCallback((w: number | null) => put(WIDTH, w === null ? null : String(clamp(w))), []);
  return [width, set];
}

/** Whether it is folded into its rail (on wide screens). */
export function useSidebarFolded(): [boolean, (folded: boolean) => void] {
  const raw = useSyncExternalStore(
    subscribe,
    () => get(FOLDED),
    () => null,
  );
  return [raw === "1", useCallback((f: boolean) => put(FOLDED, f ? "1" : null), [])];
}
