"use client";

import { useSyncExternalStore } from "react";

/**
 * Whether the screen is short: a phone in landscape, or with its keyboard
 * open, which shrinks the visual viewport and not the window.
 */
export function useShortViewport(): boolean {
  return useSyncExternalStore(
    (change) => {
      window.visualViewport?.addEventListener("resize", change);
      return () => window.visualViewport?.removeEventListener("resize", change);
    },
    () => (window.visualViewport?.height ?? window.innerHeight) < 480,
    () => false,
  );
}
