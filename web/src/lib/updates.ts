// Runners that run an older fctl than the control plane and don't update
// themselves: the home page tells the people who registered them, with the
// command that updates them, until they skip that version in this browser.

import { useCallback, useSyncExternalStore } from "react";
import type * as G from "@/gen";

/** `0.16.0` (a leading `v` is fine) as numbers to compare, as `fctl self update` reads it. */
export function parseVersion(v: string | undefined): [number, number, number] | undefined {
  const m = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(v?.trim() ?? "");
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : undefined;
}

/** Whether version `a` comes before `b`: never when either isn't one. */
export function older(a: string | undefined, b: string | undefined): boolean {
  const x = parseVersion(a);
  const y = parseVersion(b);
  if (!x || !y) return false;
  const [a1, a2, a3] = x;
  const [b1, b2, b3] = y;
  return a1 !== b1 ? a1 < b1 : a2 !== b2 ? a2 < b2 : a3 < b3;
}

/**
 * Your runners online now with an older fctl than `version`, the control
 * plane's: the ones you registered, whose machines you can update. Those
 * whose hosts update themselves are left out: they update within minutes.
 */
export function outdated(runners: readonly G.Runner[], version: string | undefined): G.Runner[] {
  return runners.filter(
    (r) => r.yours && r.status === "online" && !r.auto_update && older(r.version, version),
  );
}

// localStorage, after the person's id: the version of fctl they skipped the
// notice for in this browser. Without storage, it lasts the page.
const SKIPPED = "fctl:skipped-update:";
const here = new Map<string, string>();
const listeners = new Set<() => void>();

function subscribe(fn: () => void) {
  listeners.add(fn);
  window.addEventListener("storage", fn);
  return () => {
    listeners.delete(fn);
    window.removeEventListener("storage", fn);
  };
}

function readSkipped(user: string): string | undefined {
  try {
    const v = localStorage.getItem(SKIPPED + user);
    if (v !== null) return v;
  } catch {}
  return here.get(user);
}

/** The version of fctl whose notice `user` skipped in this browser, and how to skip one. */
export function useSkippedUpdate(user: string): [string | undefined, (version: string) => void] {
  const skipped = useSyncExternalStore(
    subscribe,
    () => readSkipped(user),
    () => undefined,
  );
  const skip = useCallback(
    (version: string) => {
      try {
        localStorage.setItem(SKIPPED + user, version);
      } catch {
        here.set(user, version);
      }
      for (const l of listeners) l();
    },
    [user],
  );
  return [skipped, skip];
}
