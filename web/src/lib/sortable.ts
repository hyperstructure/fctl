// Lists people put in order by dragging, as the sidebar's workspaces and
// projects. A mouse drags once it moves a few pixels, so a click still
// opens what it clicks; a finger holds still for a moment first, so a
// swipe still scrolls. Alt (Option) with ↑ or ↓ moves the focused one. The
// others make room as it passes them, and a dropped one settles into its
// place.

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

// How far a mouse moves before dragging, how long a finger holds still
// first and how far it may wander meanwhile, the band at a scrolling list's
// edges that scrolls it, and how fast.
const MOUSE_SLOP = 4;
const HOLD_MS = 320;
const TOUCH_SLOP = 8;
const EDGE = 48;
const MAX_SCROLL = 14;
const SETTLE = "transform 220ms var(--ease-out)";

/** Where an item is, top to bottom (pixels, as laid out without transforms). */
export interface Box {
  top: number;
  height: number;
}

const middle = (b: Box) => b.top + b.height / 2;

/**
 * Where the item at `from` lands with its top at `top`: past each neighbour
 * whose middle its leading edge has crossed.
 */
export function targetIndex(boxes: readonly Box[], from: number, top: number): number {
  const moving = boxes[from];
  if (!moving) return from;
  let to = from;
  if (top > moving.top) {
    const bottom = top + moving.height;
    for (let next = boxes[to + 1]; next && bottom > middle(next); next = boxes[to + 1]) to++;
  } else {
    for (let prev = boxes[to - 1]; prev && top < middle(prev); prev = boxes[to - 1]) to--;
  }
  return to;
}

/** The space between two items: what a moved one takes up besides its height. */
export function gapOf(boxes: readonly Box[]): number {
  const [a, b] = boxes;
  return a && b ? Math.max(0, b.top - (a.top + a.height)) : 0;
}

/** How far the item at `i` moves aside while the one at `from` is over `to`. */
export function shiftOf(boxes: readonly Box[], from: number, to: number, i: number): number {
  const moving = boxes[from];
  if (!moving || i === from) return 0;
  const room = moving.height + gapOf(boxes);
  if (from < i && i <= to) return -room;
  if (to <= i && i < from) return room;
  return 0;
}

/** How far the item at `from` may move, up (negative) and down, staying within the list. */
export function limitsOf(boxes: readonly Box[], from: number): [number, number] {
  const moving = boxes[from];
  const first = boxes[0];
  const last = boxes[boxes.length - 1];
  if (!moving || !first || !last) return [0, 0];
  return [first.top - moving.top, last.top + last.height - moving.height - moving.top];
}

/**
 * How fast a list scrolls (pixels a frame, up negative) with the pointer at
 * `y`, near its edges at `top` and `bottom`: toward where the item is
 * dragged from `startY`, while it can go further that way (`offset`, its
 * distance from its place, within `limits`).
 */
export function scrollSpeed(
  y: number,
  startY: number,
  top: number,
  bottom: number,
  offset: number,
  [min, max]: readonly [number, number],
): number {
  if (y < top + EDGE && y < startY && offset > min) return -Math.min(1, (top + EDGE - y) / EDGE) * MAX_SCROLL;
  if (y > bottom - EDGE && y > startY && offset < max)
    return Math.min(1, (y - (bottom - EDGE)) / EDGE) * MAX_SCROLL;
  return 0;
}

/** `list` with the item at `from` moved to `to`. */
export function moveItem<T>(list: readonly T[], from: number, to: number): T[] {
  const next = list.slice();
  const [item] = next.splice(from, 1);
  if (item !== undefined) next.splice(to, 0, item);
  return next;
}

const reducedMotion = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

/** The nearest ancestor that scrolls vertically. */
function scrollParent(el: HTMLElement): HTMLElement {
  for (let p = el.parentElement; p; p = p.parentElement) {
    const { overflowY } = getComputedStyle(p);
    if (overflowY === "auto" || overflowY === "scroll") return p;
  }
  return (document.scrollingElement ?? document.documentElement) as HTMLElement;
}

// Screen readers hear where a moved item went, from a live region that is
// in the page before anything is said in it (else they may not hear it).
let region: HTMLElement | undefined;
function liveRegion(): HTMLElement {
  if (!region?.isConnected) {
    region = document.createElement("div");
    region.setAttribute("aria-live", "assertive");
    region.className = "sr-only";
    document.body.append(region);
  }
  return region;
}
const announce = (text: string) => {
  liveRegion().textContent = text;
};

interface Drag {
  id: string;
  from: number;
  to: number;
  boxes: Box[];
  els: HTMLElement[];
  scroller: HTMLElement;
  startY: number;
  startScroll: number;
  y: number;
  limits: [number, number];
  frame: number;
}

/** How far the pointer has taken the dragged item from its place, scrolling included. */
const offsetOf = (d: Drag) => d.y - d.startY + (d.scroller.scrollTop - d.startScroll);

/** Puts the dragged item under the pointer, and the others out of its way. */
function follow(d: Drag) {
  const dy = Math.min(d.limits[1], Math.max(d.limits[0], offsetOf(d)));
  const moving = d.els[d.from];
  const box = d.boxes[d.from];
  if (!moving || !box) return;
  moving.style.transform = `translateY(${dy}px)`;
  const to = targetIndex(d.boxes, d.from, box.top + dy);
  if (to === d.to) return;
  d.to = to;
  d.els.forEach((el, i) => {
    if (i === d.from) return;
    const shift = shiftOf(d.boxes, d.from, to, i);
    el.style.transform = shift ? `translateY(${shift}px)` : "";
  });
}

interface Press {
  id: string;
  pointer: number;
  touch: boolean;
  x: number;
  y: number;
  timer: number;
}

export interface Sortable {
  /** The id of the item being dragged, or settling into its place after a drop. */
  lifted: string | undefined;
  /** A ref for the element that moves: a row, or a workspace with its projects. */
  itemRef: (id: string) => (el: HTMLElement | null) => void;
  /** Props for where dragging starts: the row itself, or a workspace's header. */
  handleProps: (id: string) => {
    onPointerDown: (e: React.PointerEvent) => void;
    onKeyDown: (e: React.KeyboardEvent) => void;
    onClickCapture: (e: React.MouseEvent) => void;
    onContextMenu: (e: React.MouseEvent) => void;
    onDragStart: (e: React.DragEvent) => void;
    "aria-describedby"?: string;
  };
}

/**
 * Makes a list sortable. `ids` are its items in their order; `onReorder`
 * gets them in a new one, after a drop or a key press; `label` names an
 * item for screen readers; `describedBy` is the id of the text that says
 * how to move one.
 */
export function useSortable({
  ids,
  onReorder,
  label,
  describedBy,
}: {
  ids: readonly string[];
  onReorder: (ids: string[]) => void;
  label: (id: string) => string;
  describedBy?: string;
}): Sortable {
  const [lifted, setLifted] = useState<string>();
  const els = useRef(new Map<string, HTMLElement>());
  const refs = useRef(new Map<string, (el: HTMLElement | null) => void>());
  const press = useRef<Press | undefined>(undefined);
  const drag = useRef<Drag | undefined>(undefined);
  // Where each item showed before a reorder, for it to glide from there to its new place.
  const before = useRef<{ tops: Map<string, number>; focus: Element | null; fallback: number }>(undefined);
  const dropped = useRef(0);
  const latest = useRef({ ids, onReorder, label });
  // A glide that ends while another drag has begun leaves that one lifted.
  const settled = useCallback(() => {
    if (!drag.current) setLifted(undefined);
  }, []);
  latest.current = { ids, onReorder, label };
  const sortable = ids.length > 1;

  const itemRef = useCallback((id: string) => {
    let ref = refs.current.get(id);
    if (!ref) {
      ref = (el) => {
        if (el) els.current.set(id, el);
        else els.current.delete(id);
      };
      refs.current.set(id, ref);
    }
    return ref;
  }, []);

  /** Moves elements back to their places, gliding unless motion is reduced. */
  const release = useCallback((list: HTMLElement[], glide: boolean, done?: () => void) => {
    const moving = list.filter((el) => el.style.transform);
    for (const el of list) if (!el.style.transform) el.style.transition = "";
    if (!glide || reducedMotion() || !moving.length) {
      for (const el of moving) {
        el.style.transition = "";
        el.style.transform = "";
      }
      done?.();
      return;
    }
    for (const el of moving) {
      el.style.transition = SETTLE;
      el.style.transform = "";
    }
    // transitionend doesn't come when nothing moved far enough to animate.
    window.setTimeout(() => {
      for (const el of moving) if (!el.style.transform) el.style.transition = "";
      done?.();
    }, 240);
  }, []);

  /** Notes where each item shows now, then reorders: the layout effect below animates the change. */
  const commit = useCallback(
    (next: string[]) => {
      const tops = new Map<string, number>();
      for (const [id, el] of els.current) tops.set(id, el.getBoundingClientRect().top);
      window.clearTimeout(before.current?.fallback);
      // Should the list not change after all, everything goes back to its place.
      const fallback = window.setTimeout(() => {
        if (!before.current) return;
        before.current = undefined;
        release([...els.current.values()], true, settled);
      }, 1500);
      before.current = { tops, focus: document.activeElement, fallback };
      // A key pressed again before the list shows the new order moves on from it.
      latest.current = { ...latest.current, ids: next };
      latest.current.onReorder(next);
    },
    [release, settled],
  );

  const key = ids.join("\n");
  // biome-ignore lint/correctness/useExhaustiveDependencies: it runs when the order changes
  useLayoutEffect(() => {
    const b = before.current;
    if (!b) return;
    before.current = undefined;
    window.clearTimeout(b.fallback);
    // Inverted, then played: each item starts where it showed and glides to its new place.
    const items = [...els.current];
    for (const [, el] of items) {
      el.style.transition = "none";
      el.style.transform = "";
    }
    const shifts = items.map(([id, el]) => {
      const top = b.tops.get(id);
      return top === undefined ? 0 : top - el.getBoundingClientRect().top;
    });
    items.forEach(([, el], i) => {
      const dy = shifts[i] ?? 0;
      if (Math.abs(dy) >= 0.5) el.style.transform = `translateY(${dy}px)`;
    });
    // Moving a focused element in the document takes its focus away.
    if (b.focus instanceof HTMLElement && b.focus !== document.activeElement && b.focus.isConnected)
      b.focus.focus({ preventScroll: true });
    void document.body.offsetHeight; // the inverted positions take effect first
    release(
      items.map(([, el]) => el),
      true,
      settled,
    );
  }, [key, release, settled]);

  const scrollFrame = useCallback(() => {
    const d = drag.current;
    if (!d) return;
    const r = d.scroller.getBoundingClientRect();
    const top = Math.max(r.top, 0);
    const bottom = Math.min(r.bottom, window.innerHeight);
    const v = scrollSpeed(d.y, d.startY, top, bottom, offsetOf(d), d.limits);
    if (v) {
      const was = d.scroller.scrollTop;
      d.scroller.scrollTop = was + v;
      if (d.scroller.scrollTop !== was) follow(d);
    }
    d.frame = requestAnimationFrame(scrollFrame);
  }, []);

  const stopListening = useRef<() => void>(() => {});

  const end = useCallback(
    (drop: boolean) => {
      const p = press.current;
      if (p) window.clearTimeout(p.timer);
      press.current = undefined;
      stopListening.current();
      const d = drag.current;
      drag.current = undefined;
      if (!d) return;
      cancelAnimationFrame(d.frame);
      delete document.documentElement.dataset.dragging;
      dropped.current = performance.now();
      const order = latest.current.ids;
      // The list changed meanwhile (a project came or went): nothing moves.
      const same =
        order.length === d.els.length && d.els.every((el, i) => els.current.get(order[i] ?? "") === el);
      if (drop && same && d.to !== d.from) {
        const next = moveItem(order, d.from, d.to);
        announce(`Moved ${latest.current.label(d.id)} to ${d.to + 1} of ${next.length}`);
        commit(next);
      } else {
        release(d.els, true, settled);
      }
    },
    [commit, release, settled],
  );

  const start = useCallback(() => {
    const p = press.current;
    if (!p) return;
    const order = latest.current.ids;
    const from = order.indexOf(p.id);
    const list = order.map((id) => els.current.get(id));
    const moving = list[from];
    if (from < 0 || !moving || list.some((el) => !el)) return end(false);
    const elements = list as HTMLElement[];
    // A drop's glide that hasn't finished stops where it is.
    for (const el of elements) {
      el.style.transition = "none";
      el.style.transform = "";
    }
    const boxes = elements.map((el) => {
      const r = el.getBoundingClientRect();
      return { top: r.top, height: r.height };
    });
    // The others glide aside; the dragged one follows the pointer at once (its
    // classes may animate how it lifts, but not its transform).
    for (const el of elements) el.style.transition = el === moving ? "" : SETTLE;
    const scroller = scrollParent(moving);
    drag.current = {
      id: p.id,
      from,
      to: from,
      boxes,
      els: elements,
      scroller,
      startY: p.y,
      startScroll: scroller.scrollTop,
      y: p.y,
      limits: limitsOf(boxes, from),
      frame: requestAnimationFrame(scrollFrame),
    };
    press.current = undefined;
    setLifted(p.id);
    document.documentElement.dataset.dragging = "";
    if (p.touch) navigator.vibrate?.(8);
  }, [end, scrollFrame]);

  useEffect(() => {
    liveRegion();
    // Unmounted mid-drag (the drawer closed): let go.
    return () => end(false);
  }, [end]);

  const handleProps = useCallback(
    (id: string) => ({
      onPointerDown: (e: React.PointerEvent) => {
        if (!sortable || e.button !== 0 || e.ctrlKey || e.metaKey) return;
        end(false); // a press whose pointer never came up
        const touch = e.pointerType === "touch";
        const p: Press = {
          id,
          pointer: e.pointerId,
          touch,
          x: e.clientX,
          y: e.clientY,
          timer: touch ? window.setTimeout(start, HOLD_MS) : 0,
        };
        press.current = p;
        const move = (ev: PointerEvent) => {
          if (ev.pointerId !== p.pointer) return;
          if (!drag.current) {
            const far = Math.hypot(ev.clientX - p.x, ev.clientY - p.y);
            // A finger that moves before it lifts anything swipes: the list scrolls.
            if (touch) return far > TOUCH_SLOP && end(false);
            if (far <= MOUSE_SLOP) return;
            start();
          }
          const d = drag.current;
          if (!d) return;
          d.y = ev.clientY;
          follow(d);
        };
        const up = (ev: PointerEvent) => ev.pointerId === p.pointer && end(true);
        const cancel = (ev: PointerEvent) => ev.pointerId === p.pointer && end(false);
        const key = (ev: KeyboardEvent) => {
          if (ev.key !== "Escape" || !drag.current) return;
          ev.preventDefault();
          ev.stopPropagation();
          end(false);
        };
        // Once dragging, a finger moves the item instead of scrolling the page.
        const touchmove = (ev: TouchEvent) => {
          if (drag.current && ev.cancelable) ev.preventDefault();
        };
        const blur = () => end(false);
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", up);
        window.addEventListener("pointercancel", cancel);
        window.addEventListener("keydown", key, true);
        window.addEventListener("touchmove", touchmove, { passive: false });
        window.addEventListener("blur", blur);
        stopListening.current = () => {
          window.removeEventListener("pointermove", move);
          window.removeEventListener("pointerup", up);
          window.removeEventListener("pointercancel", cancel);
          window.removeEventListener("keydown", key, true);
          window.removeEventListener("touchmove", touchmove);
          window.removeEventListener("blur", blur);
          stopListening.current = () => {};
        };
      },
      onKeyDown: (e: React.KeyboardEvent) => {
        if (!sortable || !e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
        if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
        e.preventDefault();
        e.stopPropagation();
        const order = latest.current.ids;
        const from = order.indexOf(id);
        const to = from + (e.key === "ArrowUp" ? -1 : 1);
        if (from < 0 || to < 0 || to >= order.length || drag.current) return;
        announce(`Moved ${latest.current.label(id)} to ${to + 1} of ${order.length}`);
        commit(moveItem(order, from, to));
      },
      // The click a drop ends with opens nothing.
      onClickCapture: (e: React.MouseEvent) => {
        if (performance.now() - dropped.current > 400) return;
        dropped.current = 0;
        e.preventDefault();
        e.stopPropagation();
      },
      // A held finger lifts the item rather than opening the link's menu.
      onContextMenu: (e: React.MouseEvent) => {
        if (press.current?.touch || drag.current) e.preventDefault();
      },
      // The browser's own dragging of links and images would take the pointer away.
      onDragStart: (e: React.DragEvent) => e.preventDefault(),
      ...(describedBy && sortable ? { "aria-describedby": describedBy } : {}),
    }),
    [sortable, start, end, commit, describedBy],
  );

  return { lifted, itemRef, handleProps };
}
