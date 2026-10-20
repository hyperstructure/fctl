// Earlier activity loads above what the reader is looking at, and that stays
// where it is, as in a chat. Browsers wouldn't keep it there: Safari has no
// scroll anchoring, and the others anchor to the first thing in view, the
// "Show earlier activity" button the older items go in under. Either way the
// reader would land on the oldest of them, far above their place. So the
// transcript keeps it, with the browser's anchoring off in it meanwhile, so
// that only one of them moves the page.

import { type RefObject, useCallback, useLayoutEffect, useRef } from "react";

/** How many items in view to measure from: the first ones may merge with older ones. */
const MARKS = 8;

/** Where an element is on the page, from its top. */
const pageTop = (el: Element) => el.getBoundingClientRect().top + window.scrollY;

/**
 * Keeps the reader's place in `list` while older items load at its start
 * (its items are the children with `data-item`). Call the function it returns
 * before asking for them: it notes where the items in view are. Once
 * `loading` is over, the page scrolls by what went in above them, before the
 * browser paints.
 */
export function useKeepPlace(list: RefObject<HTMLElement | null>, loading: boolean): () => void {
  const marks = useRef<{ el: Element; top: number }[]>(undefined);
  useLayoutEffect(() => {
    const held = marks.current;
    if (loading || !held) return;
    marks.current = undefined;
    list.current?.style.removeProperty("overflow-anchor");
    const mark = held.find((m) => m.el.isConnected);
    const moved = mark ? pageTop(mark.el) - mark.top : 0;
    if (moved) window.scrollBy({ top: moved, behavior: "instant" });
  });
  return useCallback(() => {
    const el = list.current;
    if (!el) return;
    const items = [...el.children].filter((c) => c.hasAttribute("data-item"));
    const first = items.findIndex((c) => c.getBoundingClientRect().bottom > 0);
    if (first < 0) return;
    marks.current = items.slice(first, first + MARKS).map((c) => ({ el: c, top: pageTop(c) }));
    el.style.overflowAnchor = "none";
  }, [list]);
}
