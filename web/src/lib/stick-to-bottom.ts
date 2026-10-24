// The session page keeps its bottom in view while the agent's work comes in,
// as long as the reader is there. Scrolling up lets go at once, however small
// the step (the wheel, a trackpad, the keys, the scroll bar, a finger), and
// scrolling back down to the bottom sticks again. The page doesn't move under
// a pressed button or finger, and what the reader opens in the transcript (a
// tool call's details) stays where they opened it.

import { type RefObject, useCallback, useLayoutEffect, useRef } from "react";

/**
 * How close (px) to the bottom scrolling down has to come to stick again: a
 * couple of lines, as the agent may add one while the page gets there.
 */
export const NEAR_BOTTOM = 64;
/** Rounding (px): a page this close to its bottom is at it. */
const AT_BOTTOM = 2;

/**
 * Whether the page sticks to its bottom, from where it scrolls and what the
 * reader asks for. It takes measurements rather than reading the page, so
 * that tests can drive it.
 */
export class Stick {
  /** The page keeps its bottom in view as it grows. */
  stuck = true;
  /** Where the page was scrolled to, and how far above its bottom, last we knew. */
  private top?: number;
  private gap = 0;

  /** The page scrolled to `top`, `gap` px above its bottom: by the reader, the browser or us. */
  scrolled(top: number, gap: number) {
    if (this.top !== undefined) {
      // The reader moves the page against its bottom. The browser moves it along
      // with its bottom when it keeps what is in view in place as something above
      // changes size, and up to a new bottom when the page gets shorter there.
      if (top < this.top && gap > this.gap && gap > AT_BOTTOM) this.stuck = false;
      else if (top > this.top && gap < this.gap && gap <= NEAR_BOTTOM) this.stuck = true;
    }
    this.top = top;
    this.gap = gap;
  }

  /**
   * The reader turned the wheel or pressed a key to scroll the page `dir`
   * (up < 0 < down, Infinity all the way down), with the page at `top`, `gap`
   * px above its bottom. Up lets go before the page moves, so that a step to
   * the bottom can't cut the scroll short. Down at the bottom, where the page
   * can't move, sticks, and so does all the way down: the browser's scroll
   * there may stop short when the page grows on the way.
   */
  asked(dir: number, top: number, gap: number) {
    if (dir < 0 && top > 0) this.stuck = false;
    else if (dir === Number.POSITIVE_INFINITY || (dir > 0 && gap <= NEAR_BOTTOM)) this.stuck = true;
  }

  /** We scrolled the page to its bottom, at `top`: its scroll event isn't the reader's. */
  pinned(top: number) {
    this.top = top;
    this.gap = 0;
  }
}

/**
 * Keeps the page's bottom in view while `content` grows, or anything else on
 * the page, as long as the reader is at the bottom. A new `key` (another
 * session) starts at the bottom again. Returns what lets go of the bottom
 * before the page moves elsewhere for the reader, as finding in the
 * transcript does: what that opens grows the page, which would otherwise
 * take it back down.
 */
export function useStickToBottom(content: RefObject<HTMLElement | null>, key: unknown): () => void {
  const letGo = useRef<() => void>(undefined);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `key` starts it over
  useLayoutEffect(() => {
    const el = content.current;
    if (!el) return;
    const page = document.documentElement;
    const stick = new Stick();
    const gap = () => page.scrollHeight - window.scrollY - window.innerHeight;
    // A button or finger is down (selecting text, dragging the scroll bar).
    let held = false;
    // A button in the transcript was just clicked: it may have opened something.
    let clicked = false;
    let height = 0;

    // While the page sticks, its scrolling is ours and the browser's scroll
    // anchoring is off: at the bottom it can hold on to a stale position and
    // move the page down along with it, under a pressed button too. Scrolled
    // up, anchoring keeps what the reader reads in place as things above change.
    let anchoring: boolean | undefined;
    const syncAnchoring = () => {
      if (anchoring === !stick.stuck) return;
      anchoring = !stick.stuck;
      page.style.overflowAnchor = anchoring ? "" : "none";
    };
    const toBottom = () => {
      if (!stick.stuck || held) return;
      window.scrollTo({ top: page.scrollHeight, behavior: "instant" });
      stick.pinned(window.scrollY);
    };
    const asked = (dir: number, target: EventTarget | null) => {
      if (dir && !inDialog(target) && !scrollsInside(target, dir)) {
        stick.asked(dir, window.scrollY, gap());
      }
      syncAnchoring();
    };
    const grown = new ResizeObserver((entries) => {
      for (const entry of entries) {
        if (entry.target !== el) continue;
        const h = entry.contentRect.height;
        // What a click opened grows the transcript where the reader is: stay there.
        if (clicked && h > height) stick.stuck = false;
        height = h;
      }
      syncAnchoring();
      toBottom();
    });
    grown.observe(el);
    // And the rest of the page, like a header whose status wraps to another line.
    grown.observe(page);
    // When the page gets shorter at its bottom, the browser moves it up to the new
    // bottom, and the page may grow again before the scroll event: that would look
    // like the reader scrolling up. Seen as soon as the page changes, the move is
    // the browser's. Only what can make the page shorter is looked at: reading the
    // scroll position lays the page out, and text that streams in only grows.
    const changed = new MutationObserver((records) => {
      if (!stick.stuck || !mayShrink(records)) return;
      stick.scrolled(window.scrollY, gap());
      syncAnchoring();
    });
    changed.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      characterData: true,
      attributeOldValue: true,
      characterDataOldValue: true,
    });
    syncAnchoring();
    letGo.current = () => {
      stick.stuck = false;
      syncAnchoring();
    };

    const off = new AbortController();
    const on = { signal: off.signal };
    const passive = { signal: off.signal, passive: true };
    const release = () => {
      held = false;
    };
    window.addEventListener(
      "scroll",
      () => {
        stick.scrolled(window.scrollY, gap());
        syncAnchoring();
      },
      passive,
    );
    window.addEventListener("resize", toBottom, on);
    window.addEventListener(
      "wheel",
      (e) => {
        // A pinch on a trackpad comes as a wheel with ctrlKey: it zooms.
        if (!e.ctrlKey) asked(Math.sign(e.deltaY), e.target);
      },
      passive,
    );
    window.addEventListener("keydown", (e) => asked(keyDirection(e), e.target), on);
    window.addEventListener(
      "pointerdown",
      (e) => {
        if (e.button === 0) held = true;
      },
      { signal: off.signal, capture: true },
    );
    window.addEventListener("pointerup", release, { signal: off.signal, capture: true });
    window.addEventListener("pointercancel", release, { signal: off.signal, capture: true });
    window.addEventListener("blur", release, on);
    el.addEventListener(
      "click",
      (e) => {
        if (!(e.target instanceof Element && e.target.closest("button"))) return;
        clicked = true;
        requestAnimationFrame(() =>
          requestAnimationFrame(() => {
            clicked = false;
          }),
        );
      },
      { signal: off.signal, capture: true },
    );
    return () => {
      off.abort();
      grown.disconnect();
      changed.disconnect();
      page.style.overflowAnchor = "";
      letGo.current = undefined;
    };
  }, [content, key]);
  return useCallback(() => letGo.current?.(), []);
}

/**
 * Whether changes may have made the page shorter: something taken out, text
 * shortened or an attribute changed. Text that streams in only adds to it, and
 * React sets some attributes at every render, back to the value they had.
 */
export function mayShrink(records: MutationRecord[]): boolean {
  const attributes = new Map<Node, Set<string>>();
  for (const r of records) {
    if (r.type === "childList") {
      if (r.removedNodes.length) return true;
    } else if (r.type === "characterData") {
      if ((r.target.nodeValue?.length ?? 0) < (r.oldValue?.length ?? 0)) return true;
    } else {
      // The first change to an attribute holds its value from before them all.
      const name = r.attributeName ?? "";
      const seen = attributes.get(r.target) ?? new Set<string>();
      if (seen.has(name)) continue;
      attributes.set(r.target, seen.add(name));
      if (!(r.target instanceof Element) || r.target.getAttribute(name) !== r.oldValue) return true;
    }
  }
  return false;
}

/** Which way a key scrolls the page: up (-1), down (1), all the way down (Infinity) or not at all (0). */
export function keyDirection(e: KeyboardEvent): number {
  const t = e.target;
  const typing =
    t instanceof HTMLElement &&
    (t.isContentEditable || t.matches("input, textarea, select, [role=combobox]"));
  if (e.defaultPrevented || typing) return 0;
  switch (e.key) {
    case "ArrowUp":
    case "PageUp":
    case "Home":
      return -1;
    case "End":
      return Number.POSITIVE_INFINITY;
    case "ArrowDown":
      // ⌘↓ on a Mac is End.
      return e.metaKey ? Number.POSITIVE_INFINITY : 1;
    case "PageDown":
      return 1;
    case " ":
      // On a button it presses the button instead.
      return t === document.body ? (e.shiftKey ? -1 : 1) : 0;
    default:
      return 0;
  }
}

/**
 * Whether an event comes from a dialog over the page, like the lightbox
 * that holds the page still: its wheel and keys aren't the reader's on the
 * page.
 */
export function inDialog(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest("dialog[open]") !== null;
}

/**
 * Whether scrolling `dir` (up < 0 < down) from `target` moves a box inside
 * the page, like a tool's long output, rather than the page.
 */
export function scrollsInside(target: EventTarget | null, dir: number): boolean {
  for (
    let el = target instanceof Element ? target : null;
    el && el !== document.body && el !== document.documentElement;
    el = el.parentElement
  ) {
    const room = dir < 0 ? el.scrollTop : el.scrollHeight - el.clientHeight - el.scrollTop;
    if (room >= 1 && /auto|scroll/.test(getComputedStyle(el).overflowY)) return true;
  }
  return false;
}
