// On phones a finger drags the sidebar's drawer: in from the left edge of the
// screen while it is closed, and back out by the drawer or the shade while it
// is open. It follows the finger, and letting go opens or closes it.

import { type RefObject, useCallback, useEffect, useRef } from "react";
import { flushSync } from "react-dom";

/** From here up (Tailwind's `md`) the sidebar stays beside the page: there is no drawer. */
const WIDE = "(min-width: 48rem)";
/**
 * How far from the left edge of the screen (px) a finger may touch to drag
 * the drawer in. On an iPhone the app keeps the band at the very edge, which
 * the browser would take for going back (./edge-swipes.ts); Android takes it
 * for its own back gesture whatever the page does, so the band is wider than
 * that, for a finger to start just inside it.
 */
export const EDGE = 32;
/** How far a finger moves (px) before it drags: less, and it taps. */
export const SLOP = 8;
/**
 * A finger that lets go of the drawer dragged this far open (a fraction of
 * its width), or moving this fast (px/ms) over its last moments, leaves it
 * open; otherwise it closes.
 */
const HALF = 0.5;
const FLICK = 0.4;
/** The last moments, in ms. */
const RECENT_MS = 100;

/**
 * A finger dragging the drawer sideways, from where it touched: how far open
 * the drawer is, and whether letting go leaves it open. It takes
 * measurements rather than reading the page, so that tests can drive it.
 */
export class Slide {
  private moves: { x: number; at: number }[];
  constructor(
    /** The drawer was open when the finger touched. */
    private open: boolean,
    /** Its width (px). */
    private width: number,
    private from: number,
    at: number,
  ) {
    this.moves = [{ x: from, at }];
  }

  /** The finger is at `x` (px) at `at` (ms). */
  moved(x: number, at: number) {
    this.moves.push({ x, at });
    // Enough to measure its speed over its last moments.
    while (this.moves.length > 2 && at - (this.moves[1]?.at ?? at) > RECENT_MS) this.moves.shift();
  }

  /** Where the drawer is (px): 0 when open, -width when closed, as far as the finger took it. */
  get offset(): number {
    const dx = (this.moves[this.moves.length - 1]?.x ?? this.from) - this.from;
    return Math.min(0, Math.max(-this.width, (this.open ? 0 : -this.width) + dx));
  }

  /** How far open it is, from 0 to 1. */
  get shown(): number {
    return this.width > 0 ? 1 + this.offset / this.width : 0;
  }

  /**
   * Whether a finger that lets go at `at` (ms) leaves it open. Its speed is
   * measured up to then: a finger that stopped before it let go flicks nothing.
   */
  opens(at: number): boolean {
    const recent = this.moves.filter((m) => at - m.at <= RECENT_MS);
    const first = recent[0];
    const last = recent[recent.length - 1];
    const speed = first && last && at > first.at ? (last.x - first.x) / (at - first.at) : 0;
    if (Math.abs(speed) > FLICK) return speed > 0;
    return this.shown > HALF;
  }
}

/**
 * Whether a finger that has moved `dx`, `dy` (px) from where it touched drags
 * a drawer that is `open` or not: sideways more than up or down, toward
 * where the drawer goes (right to open it, left to close it). `undefined`
 * while it hasn't moved far enough to tell.
 */
export function drags(open: boolean, dx: number, dy: number): boolean | undefined {
  if (Math.hypot(dx, dy) < SLOP) return undefined;
  return Math.abs(dx) > Math.abs(dy) && (open ? dx < 0 : dx > 0);
}

/**
 * Lets a finger drag the drawer. `page` holds the drawer and the page beside
 * it, but not what shows over them (the palette, sheets, menus): a closed
 * drawer comes in only from its left edge. `open` says whether it is open;
 * `onDrag` is called as a finger takes it, to show it (and the shade) for
 * dragging, at once (in `flushSync`), and `onLetGo` as the finger lets it go,
 * open or not.
 * Meanwhile the drawer and the shade follow the finger through their inline
 * styles, and slide on from there, at their own pace, once it lets go.
 * Returns whether a finger drags the drawer now.
 */
export function useDrawerDrag({
  page,
  drawer,
  shade,
  open,
  onDrag,
  onLetGo,
}: {
  page: RefObject<HTMLElement | null>;
  drawer: RefObject<HTMLElement | null>;
  shade: RefObject<HTMLElement | null>;
  open: boolean;
  onDrag: () => void;
  onLetGo: (open: boolean) => void;
}): () => boolean {
  const latest = useRef({ open, onDrag, onLetGo });
  latest.current = { open, onDrag, onLetGo };
  const dragged = useRef<() => boolean>(() => false);

  useEffect(() => {
    const off = new AbortController();
    // The finger that touched, until it lets go, and the drawer's slide once it drags it.
    let touched: { id: number; x: number; y: number; at: number; open: boolean } | undefined;
    let slide: Slide | undefined;
    let stop: (() => void) | undefined;
    dragged.current = () => slide !== undefined;

    const place = (s: Slide) => {
      const d = drawer.current;
      if (d) d.style.translate = `${s.offset}px 0`;
      if (shade.current) shade.current.style.opacity = String(s.shown);
    };
    // Back to their own styles: from where the finger left them, they slide at their own pace.
    const unstyle = () => {
      for (const el of [drawer.current, shade.current]) {
        el?.style.removeProperty("transition");
        el?.style.removeProperty("translate");
        el?.style.removeProperty("opacity");
      }
    };
    const release = (opens: boolean) => {
      unstyle();
      flushSync(() => latest.current.onLetGo(opens));
    };
    const finger = (e: TouchEvent) =>
      touched ? [...e.touches].find((t) => t.identifier === touched?.id) : undefined;

    const move = (e: TouchEvent) => {
      const t = finger(e);
      if (!touched || !t) return;
      // A second finger, or a row of the sidebar lifted to be moved (lib/sortable.ts): not a drag.
      if (e.touches.length > 1 || (!slide && document.documentElement.dataset.dragging !== undefined)) {
        return abandon();
      }
      if (!slide) {
        const takes = drags(touched.open, t.clientX - touched.x, t.clientY - touched.y);
        if (takes === undefined) return;
        if (!takes) return abandon();
        const d = drawer.current;
        if (!d) return abandon();
        // Shown where it is, and following the finger from there.
        for (const el of [d, shade.current]) el?.style.setProperty("transition", "none");
        d.style.translate = touched.open ? "0 0" : "-100% 0";
        flushSync(() => latest.current.onDrag());
        if (shade.current) {
          shade.current.style.transition = "none";
          shade.current.style.opacity = touched.open ? "1" : "0";
        }
        slide = new Slide(touched.open, d.offsetWidth, touched.x, touched.at);
      }
      // The page doesn't scroll under the finger meanwhile.
      if (e.cancelable) e.preventDefault();
      slide.moved(t.clientX, e.timeStamp);
      place(slide);
    };

    const end = (e: TouchEvent) => {
      // Another finger let go: this one is still down.
      if (finger(e)) return;
      const s = slide;
      const was = touched?.open ?? false;
      touched = undefined;
      slide = undefined;
      stop?.();
      stop = undefined;
      if (!s) return;
      // The touch that dragged clicks nothing as it ends.
      if (e.type === "touchend" && e.cancelable) e.preventDefault();
      release(e.type === "touchend" ? s.opens(e.timeStamp) : was);
    };

    // Whether a closed drawer comes in under a finger that touches `target`
    // at `x`: at the left edge of the page (past the notch, in landscape), not
    // of a dialog over it, nor of something scrolled sideways, which the
    // finger scrolls back.
    const edge = (target: Element, x: number) => {
      const inset = drawer.current ? Number.parseFloat(getComputedStyle(drawer.current).paddingLeft) || 0 : 0;
      if (x > EDGE + inset || !page.current?.contains(target) || target.closest("dialog")) return false;
      for (let el: Element | null = target; el && el !== page.current; el = el.parentElement) {
        if (el.scrollLeft > 0) return false;
      }
      return true;
    };

    const start = (e: TouchEvent) => {
      const t = e.touches[0];
      // The only finger down: any other this followed has let go, though its end
      // never came (what it touched left the page, in some browsers).
      if (touched && e.touches.length === 1) abandon();
      if (touched || !t || e.touches.length > 1 || matchMedia(WIDE).matches) return;
      const open = latest.current.open;
      const target = e.target instanceof Element ? e.target : null;
      if (!target) return;
      // Open: by the drawer or the shade. Closed: from the left edge of the page.
      const takes = open
        ? !!drawer.current?.contains(target) || !!shade.current?.contains(target)
        : edge(target, t.clientX);
      if (!takes) return;
      touched = { id: t.identifier, x: t.clientX, y: t.clientY, at: e.timeStamp, open };
      // Only while a finger may drag the drawer: a touchmove listener that can
      // cancel makes the browser wait for it before it scrolls. On what it
      // touched, which gets the touch's events even once it leaves the page
      // (a list re-rendered under the finger), when the document no longer does.
      // (Element's types know no touch events: all its kinds of elements get them.)
      const during = new AbortController();
      const listen = (type: string, f: (e: TouchEvent) => void, passive: boolean) =>
        target.addEventListener(type, f as EventListener, { passive, signal: during.signal });
      listen("touchmove", move, false);
      listen("touchend", end, false);
      listen("touchcancel", end, true);
      stop = () => during.abort();
    };
    // Lets go of a touch whose end never came, the drawer back where it was.
    const abandon = () => {
      const was = touched?.open ?? false;
      const s = slide;
      touched = undefined;
      slide = undefined;
      stop?.();
      stop = undefined;
      if (s) release(was);
    };

    document.addEventListener("touchstart", start, { passive: true, signal: off.signal });
    return () => {
      off.abort();
      stop?.();
      // Unmounted mid-drag: the drawer goes back to its own styles.
      if (slide) unstyle();
      dragged.current = () => false;
    };
  }, [page, drawer, shade]);
  return useCallback(() => dragged.current(), []);
}
