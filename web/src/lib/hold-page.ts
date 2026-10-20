// Overlays over the page (the lightbox, sheets) hold it still behind them.

import { useEffect } from "react";

/**
 * Holds the page still while the component that calls it is there: it
 * doesn't scroll behind it. A scrollbar that takes room (not an overlay one)
 * leaves its room behind, so that the page doesn't shift.
 */
export function useHoldPage() {
  useEffect(() => {
    const html = document.documentElement;
    const { overflow, scrollbarGutter } = html.style;
    if (window.innerWidth > html.clientWidth) html.style.scrollbarGutter = "stable";
    html.style.overflow = "hidden";
    return () => {
      html.style.overflow = overflow;
      html.style.scrollbarGutter = scrollbarGutter;
    };
  }, []);
}
