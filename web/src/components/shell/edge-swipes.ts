// Safari goes back when a finger swipes in from the left edge of the screen,
// and forward from the right, and so does every other browser on an iPhone
// (all of them are WebKit). On a phone the app keeps those swipes: from the
// left edge a finger drags the sidebar's drawer in (./drawer-drag.ts), and
// from neither edge does it leave the page. A page can stop them only by
// cancelling the touch as it starts, which takes away everything else the
// browser would have done with it. So for a touch that starts at either edge,
// the app does that itself: a tap clicks what it touched, and a swipe scrolls
// what is under it, gliding on once the finger lets go.
//
// Other browsers keep their swipes. Android's own back gesture takes the
// touch before the page could cancel it, and Chrome's swipe from the edge
// stops at `overscroll-behavior-x: none` (styles/app.css), which costs nothing.

import { useEffect, useRef } from "react";
import { type Point, Trail } from "@/lib/zoom";
import { SLOP } from "./drawer-drag";

/** From here up both ways (a tablet's or a computer's screen), edge swipes stay the browser's. */
const ROOMY = "(min-width: 48rem) and (min-height: 48rem)";
/**
 * How far from the left or right edge of the screen (px) a touch starts that
 * the app keeps: as far in as Safari takes one for going back or forward,
 * about 25 px by others' measures.
 */
export const GUARD = 24;
/** How much of its speed something scrolled keeps each ms as it glides on: as on an iPhone. */
const DECAY = 0.998;
/** Something gliding this slow (px/ms) stops. */
const STILL = 0.02;
/** The longest step (ms) a glide takes between two frames, after the page stalled. */
const STEP_MS = 50;

/** What the browser draws controls in, which only its own handling of a touch works. */
const NATIVE = "video, audio, input[type='range']";

/** The way a finger scrolls: sideways or up and down. */
type Axis = "x" | "y";

/** Whether a touch at `x` (px), on a screen `width` px wide, starts at its left or right edge. */
export const atEdge = (x: number, width: number) => x < GUARD || x > width - GUARD;

/**
 * How far (px) something gliding at `speed` (px/ms) goes over the next `ms`,
 * and how fast it goes on then: it slows down as an iPhone's scroll views do,
 * and once it is still, its speed is 0.
 */
export function glide(speed: number, ms: number): { by: number; speed: number } {
  if (Math.abs(speed) < STILL) return { by: 0, speed: 0 };
  const keep = DECAY ** ms;
  return { by: (speed * (keep - 1)) / Math.log(DECAY), speed: speed * keep };
}

/** Whether a `touch-action` lets a finger pan along `axis`. */
const pans = (touchAction: string, axis: Axis) =>
  !touchAction ||
  touchAction === "auto" ||
  touchAction === "manipulation" ||
  touchAction.includes(`pan-${axis}`);

/**
 * What a finger moving along `axis` from `el` scrolls, as the browser would
 * have: the nearest thing that scrolls that way, or else the page. Never the
 * page under a dialog or a popover, and nothing where `touch-action` keeps
 * fingers to what the page does with them itself.
 */
export function scrollerOf(el: Element, axis: Axis): Element | undefined {
  for (let at: Element | null = el; at && at !== document.documentElement; at = at.parentElement) {
    const style = getComputedStyle(at);
    if (!pans(style.touchAction, axis)) return undefined;
    const overflow = axis === "x" ? style.overflowX : style.overflowY;
    const more = axis === "x" ? at.scrollWidth > at.clientWidth : at.scrollHeight > at.clientHeight;
    if ((overflow === "auto" || overflow === "scroll") && more) return at;
    if (at.tagName === "DIALOG" || at.hasAttribute("popover")) return undefined;
  }
  // The page never scrolls sideways: what is wider scrolls in its own row.
  return axis === "y" ? (document.scrollingElement ?? undefined) : undefined;
}

const position = (el: Element, axis: Axis) => (axis === "x" ? el.scrollLeft : el.scrollTop);
const put = (el: Element, axis: Axis, to: number) =>
  el.scrollTo(axis === "x" ? { left: to, behavior: "instant" } : { top: to, behavior: "instant" });

/** Inputs that take no typing, which a tap doesn't focus. */
const UNTYPED = new Set([
  "button",
  "checkbox",
  "color",
  "file",
  "hidden",
  "image",
  "radio",
  "range",
  "reset",
  "submit",
]);

/**
 * A tap on `el` at `at`, as the browser's own would have been: a text field
 * takes the focus (and brings the keyboard up), and it clicks. It clicks as a
 * finger would, at once, so that what a click opens may do what only a
 * person's action lets it.
 */
export function tap(el: Element, at: Point) {
  const field = el.closest("input, textarea, select, [contenteditable]:not([contenteditable='false'])");
  if (field instanceof HTMLElement && !(field instanceof HTMLInputElement && UNTYPED.has(field.type))) {
    field.focus();
  }
  const init = {
    bubbles: true,
    cancelable: true,
    composed: true,
    view: window,
    // From a pointer: a click without one (0) comes from the keys or assistive technology.
    detail: 1,
    button: 0,
    clientX: at.x,
    clientY: at.y,
  };
  el.dispatchEvent(
    typeof PointerEvent === "function"
      ? new PointerEvent("click", { ...init, pointerType: "touch", isPrimary: true })
      : new MouseEvent("click", init),
  );
}

/**
 * Keeps swipes from the left and right edges of the screen in the app, on a
 * phone in WebKit. `dragged` says whether a finger drags the drawer: that
 * touch is the drawer's.
 */
export function useEdgeSwipes(dragged: () => boolean) {
  const latest = useRef(dragged);
  latest.current = dragged;

  useEffect(() => {
    // Only WebKit sends gesture events, and only a touch screen swipes.
    if (!("GestureEvent" in window) || !navigator.maxTouchPoints) return;
    const off = new AbortController();
    // The touch the app took, until it lets go: what it touched, where it started and where it is
    // now. Once it moves further than a tap, the way it scrolls, and what it scrolls, if anything,
    // from where.
    let touch:
      | {
          id: number;
          target: Element;
          from: Point;
          at: Point;
          scroll?: { axis: Axis; el?: Element; start: number; from: Point; trail: Trail };
        }
      | undefined;
    let stop: (() => void) | undefined;
    let frame = 0;

    const finish = () => {
      touch = undefined;
      stop?.();
      stop = undefined;
    };
    const finger = (e: TouchEvent) =>
      touch ? [...e.touches].find((t) => t.identifier === touch?.id) : undefined;
    // What the page does with the touch itself: the drawer, a row lifted to be moved
    // (lib/sortable.ts), or what cancelled it, as the lightbox and sheets do.
    const taken = (e: TouchEvent) =>
      e.defaultPrevented || latest.current() || document.documentElement.dataset.dragging !== undefined;

    const glideOn = (el: Element, axis: Axis, speed: number) => {
      let then = performance.now();
      const step = (now: number) => {
        const g = glide(speed, Math.min(STEP_MS, Math.max(0, now - then)));
        then = now;
        speed = g.speed;
        const was = position(el, axis);
        put(el, axis, was - g.by);
        // Until it is still, or can't go further.
        if (speed && position(el, axis) !== was) frame = requestAnimationFrame(step);
      };
      frame = requestAnimationFrame(step);
    };

    const move = (e: TouchEvent) => {
      const t = finger(e);
      if (!touch || !t) return;
      if (e.touches.length > 1 || taken(e)) return finish();
      const at = { x: t.clientX, y: t.clientY };
      touch.at = at;
      if (!touch.scroll) {
        const dx = at.x - touch.from.x;
        const dy = at.y - touch.from.y;
        if (Math.hypot(dx, dy) < SLOP) return;
        const axis = Math.abs(dx) > Math.abs(dy) ? "x" : "y";
        const el = scrollerOf(touch.target, axis);
        // From here, so that what it scrolls doesn't jump by as far as the finger went first.
        touch.scroll = { axis, el, start: el ? position(el, axis) : 0, from: at, trail: new Trail() };
      }
      const s = touch.scroll;
      s.trail.moved(at, e.timeStamp);
      if (s.el) put(s.el, s.axis, s.start - (at[s.axis] - s.from[s.axis]));
    };

    const end = (e: TouchEvent) => {
      // Another finger let go: this one is still down.
      if (!touch || finger(e)) return;
      const { target, at, scroll } = touch;
      finish();
      if (e.type === "touchcancel" || taken(e)) return;
      if (!scroll) {
        const el = target.isConnected ? target : document.elementFromPoint(at.x, at.y);
        if (el) tap(el, at);
      } else if (scroll.el) {
        glideOn(scroll.el, scroll.axis, scroll.trail.speed(e.timeStamp)[scroll.axis]);
      }
    };

    const start = (e: TouchEvent) => {
      // A finger on the screen stops what glides, as it stops the browser's own scrolling.
      cancelAnimationFrame(frame);
      // A second finger, or a touch whose end never came (what it touched left the page).
      if (touch) finish();
      const t = e.touches[0];
      const target = e.target instanceof Element ? e.target : null;
      // A touch that starts while the browser scrolls is the browser's whatever it is: it can't
      // be cancelled.
      if (!t || !target || e.touches.length > 1 || !e.cancelable || e.defaultPrevented) return;
      if (!atEdge(t.clientX, window.innerWidth) || matchMedia(ROOMY).matches) return;
      // The browser's own controls (a video's, a slider) need the touch itself.
      if (target.closest(NATIVE)) return;
      e.preventDefault();
      touch = {
        id: t.identifier,
        target,
        from: { x: t.clientX, y: t.clientY },
        at: { x: t.clientX, y: t.clientY },
      };
      // Only while it lasts, and passive: what the page does with the touch comes first. On the
      // document, which hears its events after everything on the way had its say, and on what it
      // touched, which alone hears them once it leaves the page (a list re-rendered under the
      // finger). (Element's types know no touch events: all its kinds of elements get them.)
      const during = new AbortController();
      const on = { passive: true, signal: during.signal };
      for (const [type, f] of [
        ["touchmove", move],
        ["touchend", end],
        ["touchcancel", end],
      ] as const) {
        document.addEventListener(type, f, on);
        target.addEventListener(type, ((ev: TouchEvent) => target.isConnected || f(ev)) as EventListener, on);
      }
      stop = () => during.abort();
    };

    // Not passive: only a touch cancelled as it starts doesn't go back or forward. The browser
    // waits for it before it scrolls.
    document.addEventListener("touchstart", start, { passive: false, signal: off.signal });
    return () => {
      off.abort();
      finish();
      cancelAnimationFrame(frame);
    };
  }, []);
}
