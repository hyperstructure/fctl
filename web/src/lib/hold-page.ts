// Overlays over the page (the lightbox, sheets, the diffs) hold it still behind them.

import { useEffect } from "react";

/** How many overlays hold the page, and its style from before the first of them. */
let holders = 0;
let before: { overflow: string; scrollbarGutter: string } | undefined;

/**
 * Holds the page still while the component that calls it is there: it
 * doesn't scroll behind it. A scrollbar that takes room (not an overlay one)
 * leaves its room behind, so that the page doesn't shift.
 *
 * Overlays opened from one another share the hold, and the page scrolls
 * again once the last of them goes, whatever the order: when a sheet and a
 * dialog opened from it go together, React lets go of the sheet first.
 */
export function useHoldPage() {
  useEffect(() => {
    const html = document.documentElement;
    if (holders++ === 0) {
      before = { overflow: html.style.overflow, scrollbarGutter: html.style.scrollbarGutter };
      if (window.innerWidth > html.clientWidth) html.style.scrollbarGutter = "stable";
      html.style.overflow = "hidden";
    }
    return () => {
      if (--holders > 0 || !before) return;
      html.style.overflow = before.overflow;
      html.style.scrollbarGutter = before.scrollbarGutter;
      before = undefined;
    };
  }, []);
}
