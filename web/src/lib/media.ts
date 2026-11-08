// What a CSS media query says about the window, as it changes.

import { useCallback, useSyncExternalStore } from "react";

/**
 * Whether the window matches `query` (`"(min-width: 48rem)"`), again each time that changes;
 * `fallback` where the browser can't say (tests without `matchMedia`).
 */
export function useMedia(query: string, fallback = true): boolean {
  const subscribe = useCallback(
    (changed: () => void) => {
      const q = window.matchMedia?.(query);
      q?.addEventListener?.("change", changed);
      return () => q?.removeEventListener?.("change", changed);
    },
    [query],
  );
  return useSyncExternalStore(
    subscribe,
    () => window.matchMedia?.(query)?.matches ?? fallback,
    () => fallback,
  );
}
