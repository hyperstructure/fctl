// On phones and tablets the app keeps the screen's size: it doesn't zoom in or
// out. index.html's viewport tag says so (user-scalable=no), and browsers on
// Android keep to it, but Safari on iOS and iPadOS lets people pinch anyway.
// Its pinches arrive as gesture events, which only Safari sends; cancelling
// them stops the zoom. The lightbox zooms in on its pictures by itself
// instead (`Zoom`).

const PINCH = ["gesturestart", "gesturechange"];

/** Stops pinch zoom on a touch screen (not on a Mac's trackpad). Returns what undoes it. */
export function stopPinchZoom(target: EventTarget = document, touch = navigator.maxTouchPoints > 0) {
  if (!touch) return () => {};
  const cancel = (e: Event) => e.preventDefault();
  for (const type of PINCH) target.addEventListener(type, cancel, { passive: false });
  return () => {
    for (const type of PINCH) target.removeEventListener(type, cancel);
  };
}

/** A point on the screen, or a move across it (px). */
export interface Point {
  x: number;
  y: number;
}

/**
 * How a picture shows: `scale` times the size it fits the window at, moved
 * `x` px to the right and `y` px down from where it fits. It zooms about its
 * middle.
 */
export interface View {
  scale: number;
  x: number;
  y: number;
}

/** A picture as it fits the window. */
export const FITS: View = { scale: 1, x: 0, y: 0 };

/** Fingers zoom a picture in this many times, or until it shows at its own size if that is further. */
const MAX_ZOOM = 4;
/** A double tap zooms a picture in this many times. */
const DOUBLE_TAP_ZOOM = 3;
/** A picture zoomed in less than this goes back to how it fits. */
const ZOOMED = 1.01;
/** How far a picture follows fingers that zoom or move it past where it may go. */
const RESIST = 0.3;
/** How much of its speed a gliding picture keeps for each ms. */
const GLIDE = 0.995;
/** A gliding picture this slow (px/ms) stops. */
const STILL = 0.02;
/** The last moments of a finger's moves, in ms. */
const RECENT_MS = 100;

/**
 * A picture that fingers zoom in on and move about in the window: where it
 * fits, and where pinches, moves, double taps and glides take it from
 * there. It takes measurements rather than reading the page, so that tests
 * can drive it.
 */
export class Zoom {
  /** How many times its fitting size it zooms in to at most. */
  readonly max: number;
  private middle: Point;

  constructor(
    /** The window's size (px). */
    private window: { width: number; height: number },
    /** The picture where it fits: its left and top edges in the window, and its size (px). */
    private fit: { left: number; top: number; width: number; height: number },
    /** How wide it is in its own pixels. */
    natural: number,
  ) {
    this.max = Math.max(MAX_ZOOM, natural / fit.width);
    this.middle = { x: fit.left + fit.width / 2, y: fit.top + fit.height / 2 };
  }

  /**
   * How far the picture may move at `scale`, left to right and top to bottom
   * (px): one larger than the window covers it, a smaller one stays in it,
   * and either may stay where it fits, zoomed about its middle.
   */
  bounds(scale: number): { x: [number, number]; y: [number, number] } {
    const axis = (window: number, middle: number, size: number): [number, number] => {
      // Its near edge on the window's, and its far edge on the window's.
      const near = (size * scale) / 2 - middle;
      const far = window - middle - (size * scale) / 2;
      return [Math.min(near, far, 0), Math.max(near, far, 0)];
    };
    return {
      x: axis(this.window.width, this.middle.x, this.fit.width),
      y: axis(this.window.height, this.middle.y, this.fit.height),
    };
  }

  /**
   * The view after two fingers moved from `from` to `to`, from `start`: the
   * spot between them stays between them. It resists zooming out past the
   * fitting size, or in past the most.
   */
  pinch(start: View, from: [Point, Point], to: [Point, Point]): View {
    const scale = resist(start.scale * (apart(to) / Math.max(apart(from), 1)), 1, this.max);
    return this.about(start, scale, between(from), between(to));
  }

  /** The view after a finger moved `by` from `start`. It resists moving past the picture's edges. */
  pan(start: View, by: Point): View {
    const { x, y } = this.bounds(start.scale);
    return { scale: start.scale, x: resist(start.x + by.x, ...x), y: resist(start.y + by.y, ...y) };
  }

  /**
   * Where a view comes to rest once the fingers let go: zoomed in no further
   * than the most, about `at`, where they were, and moved no further than
   * the picture's edges; or as it fits, if it was zoomed out that far.
   */
  settle(view: View, at: Point): View {
    if (view.scale < ZOOMED) return FITS;
    const v = view.scale > this.max ? this.about(view, this.max, at) : view;
    const { x, y } = this.bounds(v.scale);
    return { scale: v.scale, x: clamp(v.x, ...x), y: clamp(v.y, ...y) };
  }

  /** A double tap at `at`: zooms in about it, or back out to the fitting size. */
  toggle(view: View, at: Point): View {
    return view.scale > 1 ? FITS : this.settle(this.about(view, DOUBLE_TAP_ZOOM, at), at);
  }

  /**
   * A gliding picture `ms` later, from `view` at `speed` (px/ms): where it is
   * then, and how fast it goes on. It slows down, and stops at its edges;
   * once it is still, its speed is 0.
   */
  glide(view: View, speed: Point, ms: number): { view: View; speed: Point } {
    if (Math.hypot(speed.x, speed.y) < STILL) return { view, speed: { x: 0, y: 0 } };
    const { x, y } = this.bounds(view.scale);
    const to = { x: view.x + speed.x * ms, y: view.y + speed.y * ms };
    const at = { x: clamp(to.x, ...x), y: clamp(to.y, ...y) };
    const keep = GLIDE ** ms;
    return {
      view: { scale: view.scale, ...at },
      speed: { x: at.x === to.x ? speed.x * keep : 0, y: at.y === to.y ? speed.y * keep : 0 },
    };
  }

  /** `view` zoomed to `scale`, with the spot at `from` moved to `to`. */
  private about(view: View, scale: number, from: Point, to = from): View {
    const k = scale / view.scale;
    const { x, y } = this.middle;
    return { scale, x: to.x - x - (from.x - x - view.x) * k, y: to.y - y - (from.y - y - view.y) * k };
  }
}

/** Where a finger moving a picture has been lately, for the speed it lets go at. */
export class Trail {
  private moves: (Point & { at: number })[] = [];

  /** The finger is at `p` at `at` (ms). */
  moved(p: Point, at: number) {
    this.moves.push({ x: p.x, y: p.y, at });
    // Enough to measure its speed over its last moments.
    while (this.moves.length > 2 && at - (this.moves[1]?.at ?? at) > RECENT_MS) this.moves.shift();
  }

  /** Its speed (px/ms) over its last moments before `at` (ms): none if it held still. */
  speed(at: number): Point {
    const recent = this.moves.filter((m) => at - m.at <= RECENT_MS);
    const first = recent[0];
    const last = recent[recent.length - 1];
    if (!first || !last || last.at <= first.at) return { x: 0, y: 0 };
    const ms = last.at - first.at;
    return { x: (last.x - first.x) / ms, y: (last.y - first.y) / ms };
  }
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
/** `v`, or only RESIST as far past `lo` or `hi`. */
const resist = (v: number, lo: number, hi: number) => {
  const c = clamp(v, lo, hi);
  return c + (v - c) * RESIST;
};
const apart = ([a, b]: [Point, Point]) => Math.hypot(b.x - a.x, b.y - a.y);
const between = ([a, b]: [Point, Point]): Point => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });
