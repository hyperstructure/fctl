// Which session page people see: the classic one, or the new one they can try beside it
// (routes/session-next.tsx), as each person picks it in this browser, as they do the theme. The
// classic one stays until people have compared the two.

import { useSyncExternalStore } from "react";

export type SessionView = "classic" | "new";

const KEY = "fctl:session-view";
const listeners = new Set<() => void>();

// Read once, and again when another tab changes it.
let picked: SessionView | undefined;

/** The session page this browser shows. */
export function sessionView(): SessionView {
  if (picked === undefined) {
    let saved: string | null = null;
    try {
      saved = localStorage.getItem(KEY);
    } catch {}
    picked = saved === "new" ? "new" : "classic";
  }
  return picked;
}

/** Shows the session page `view` from now on, in this browser; the classic one forgets the pick. */
export function setSessionView(view: SessionView) {
  const apply = () => {
    picked = view;
    try {
      if (view === "classic") localStorage.removeItem(KEY);
      else localStorage.setItem(KEY, view);
    } catch {}
    for (const l of listeners) l();
  };
  // The page changes under the reader as one picture fades into the other, as the theme does.
  const doc = document as Document & { startViewTransition?: (cb: () => void) => unknown };
  if (doc.startViewTransition && !matchMedia?.("(prefers-reduced-motion: reduce)").matches)
    doc.startViewTransition(apply);
  else apply();
}

/** The other one: what the switch between them shows next. */
export const otherView = (view: SessionView): SessionView => (view === "new" ? "classic" : "new");

function subscribe(fn: () => void) {
  listeners.add(fn);
  // Another tab picked one.
  const storage = (e: StorageEvent) => {
    if (e.key !== KEY && e.key !== null) return;
    picked = undefined;
    fn();
  };
  window.addEventListener("storage", storage);
  return () => {
    listeners.delete(fn);
    window.removeEventListener("storage", storage);
  };
}

export function useSessionView(): [SessionView, (view: SessionView) => void] {
  return [useSyncExternalStore(subscribe, sessionView), setSessionView];
}
