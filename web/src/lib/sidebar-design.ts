// Which sidebar the app shows (a design prototype): today's, or one of the proposals, chosen in
// Settings → Appearance or with ?sidebar=<design> in the address, and remembered in this
// browser with how wide each is and whether it is folded into its rail.

import { useCallback, useSyncExternalStore } from "react";

export type SidebarDesign = "classic" | "tree" | "rail" | "focus";

export const SIDEBAR_DESIGNS: { design: SidebarDesign; label: string; hint: string }[] = [
  {
    design: "classic",
    label: "Today's",
    hint: "The sidebar as it is: workspaces, their projects, and sessions at work.",
  },
  {
    design: "tree",
    label: "Tree",
    hint: "Wider, with project icons, what waits for you at the top, and the open project's own links.",
  },
  {
    design: "rail",
    label: "Workspace rail",
    hint: "Workspaces in a rail; the one you are in fills a panel with its pull requests, tasks and runners.",
  },
  {
    design: "focus",
    label: "Focus",
    hint: "What needs you first, then the agents at work, then every project as an icon to jump to.",
  },
];

/** How wide each design is at first, and at least and at most when dragged (CSS pixels). */
export const WIDTHS: Record<SidebarDesign, { initial: number; min: number; max: number }> = {
  classic: { initial: 244, min: 244, max: 244 },
  tree: { initial: 288, min: 240, max: 400 },
  rail: { initial: 332, min: 300, max: 420 },
  focus: { initial: 300, min: 260, max: 420 },
};
/** A folded sidebar's rail. */
export const RAIL = 60;

const DESIGN = "fctl:sidebar-design";
const FOLDED = "fctl:sidebar-folded";
const width = (d: SidebarDesign) => `fctl:sidebar-width:${d}`;
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
  } catch {}
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

const isDesign = (v: unknown): v is SidebarDesign => SIDEBAR_DESIGNS.some((d) => d.design === v);

// ?sidebar=<design> in the address chooses one, as Settings does.
if (typeof window !== "undefined") {
  const asked = new URLSearchParams(window.location.search).get("sidebar");
  if (isDesign(asked)) put(DESIGN, asked);
}

export function useSidebarDesign(): [SidebarDesign, (d: SidebarDesign) => void] {
  const raw = useSyncExternalStore(
    subscribe,
    () => get(DESIGN),
    () => null,
  );
  return [isDesign(raw) ? raw : "tree", useCallback((d: SidebarDesign) => put(DESIGN, d), [])];
}

/** How wide `design` is, and a setter that keeps a new width within its bounds (none: its initial). */
export function useSidebarWidth(design: SidebarDesign): [number, (w: number | null) => void] {
  const raw = useSyncExternalStore(
    subscribe,
    () => get(width(design)),
    () => null,
  );
  const { initial, min, max } = WIDTHS[design];
  const n = Number(raw);
  const w = raw && Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : initial;
  const set = useCallback(
    (next: number | null) =>
      put(width(design), next === null ? null : String(Math.round(Math.min(max, Math.max(min, next))))),
    [design, min, max],
  );
  return [w, set];
}

/** Whether the sidebar is folded into its rail (on wide screens). */
export function useSidebarFolded(): [boolean, (folded: boolean) => void] {
  const raw = useSyncExternalStore(
    subscribe,
    () => get(FOLDED),
    () => null,
  );
  return [raw === "1", useCallback((f: boolean) => put(FOLDED, f ? "1" : null), [])];
}
