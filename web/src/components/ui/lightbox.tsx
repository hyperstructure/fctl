import { ChevronLeft, ChevronRight, Download, X } from "lucide-react";
import { type MouseEvent, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal, flushSync } from "react-dom";
import { playable } from "@/lib/attachments";
import { cn } from "@/lib/cn";
import { bytes } from "@/lib/format";
import { useHoldPage } from "@/lib/hold-page";
import { FITS, type Point, Trail, type View, Zoom } from "@/lib/zoom";

/** What a lightbox shows. Attachments and published screenshots are these. */
export interface LightboxFile {
  name: string;
  /** A video type plays in a player; anything else shows as an image. */
  media_type?: string;
  /** Bytes. */
  size?: number;
  width?: number;
  height?: number;
}

/** A file in a lightbox, and where it comes from. */
export interface LightboxItem {
  file: LightboxFile;
  src: string;
  /**
   * Offers the file as a download, except on touch screens, where iOS
   * answers a download with a sheet of its own.
   */
  download?: boolean;
}

/**
 * Whether a click on a link to a file may show it in a lightbox: a plain
 * click. With a modifier key, the browser opens the link in a new tab or
 * window as usual.
 */
export function plainClick(
  e: Pick<MouseEvent, "button" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">,
): boolean {
  return e.button === 0 && !e.metaKey && !e.ctrlKey && !e.shiftKey && !e.altKey;
}

/**
 * A finger that lets go of a picture swiped this far aside (a fraction of
 * the window's width), or moving aside this fast (px/ms) over its last
 * moments, brings in the next or previous one; otherwise it settles back.
 */
const SWIPE_AWAY = 0.25;
const FLICK = 0.4;
/** The last moments, in ms. */
const RECENT_MS = 100;
/** How far a finger moves (px) before it swipes: less, and it taps. */
const SLOP = 10;
/** How far a picture follows a finger toward a side with no more pictures. */
const RESIST = 0.3;
/**
 * How long a swiped picture takes to slide into place at most, or a zoomed
 * one to settle (`.lightbox-track` and `.lightbox-picture` in app.css), and a
 * margin.
 */
const SLIDE_MS = 400;
/** A finger that lets go sooner than this (ms) after it touched, having hardly moved, taps. */
const TAP_MS = 300;
/** Two taps this close together, in time (ms) and on the screen (px), are a double tap. */
const DOUBLE_TAP_MS = 300;
const DOUBLE_TAP_PX = 50;
/** A frame, in ms, at 60 frames a second. */
const FRAME_MS = 16;

const spot = (t: Touch): Point => ({ x: t.clientX, y: t.clientY });
const reducedMotion = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

/**
 * A finger swiping pictures sideways, from where it started to: how far it
 * has moved, and which picture letting go brings in. It takes measurements
 * rather than reading the page, so that tests can drive it.
 */
export class Swipe {
  private moves: { x: number; at: number }[];
  constructor(
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

  /** How far the finger has moved (px): to the left < 0 < to the right. */
  get distance(): number {
    return (this.moves[this.moves.length - 1]?.x ?? this.from) - this.from;
  }

  /**
   * The picture a finger that lets go at `at` (ms), in a window `width` px
   * wide, brings in: the next one (1) after a swipe to the left, the
   * previous one (-1) after a swipe to the right, or none (0).
   */
  step(at: number, width: number): -1 | 0 | 1 {
    const recent = this.moves.filter((m) => at - m.at <= RECENT_MS);
    const first = recent[0];
    const last = recent[recent.length - 1];
    const speed = first && last && last.at > first.at ? (last.x - first.x) / (last.at - first.at) : 0;
    const d = this.distance;
    // A flick back toward where the finger started brings in none.
    if (Math.abs(speed) > FLICK) return speed < 0 && d < -16 ? 1 : speed > 0 && d > 16 ? -1 : 0;
    return d < -width * SWIPE_AWAY ? 1 : d > width * SWIPE_AWAY ? -1 : 0;
  }
}

/**
 * Screenshots or a video at full size over the page, on the native modal
 * <dialog>. The X at the top right, a click anywhere outside the picture
 * and its caption, or Escape closes it, and focus goes back to what opened
 * it. With several pictures it says which one of how many it shows, and
 * the arrow keys, the buttons at its sides or a swipe step to the previous
 * or next one. On a touch screen, two fingers zoom in on an image (the page
 * itself doesn't zoom), one moves it about, and a double tap zooms in on it
 * or back out.
 */
export function Lightbox({
  items,
  index = 0,
  onIndex,
  onClose,
}: {
  /** A file, or pictures in the order the page shows them. */
  items: LightboxItem[];
  /** The one it shows. */
  index?: number;
  /** Shows another of them: the one before or after. */
  onIndex?: (index: number) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  // The picture and the ones beside it, which a swipe slides along.
  const track = useRef<HTMLDivElement>(null);
  // A press that starts on the picture and ends outside it, selecting or
  // dragging, doesn't close it.
  const pressedOutside = useRef(false);
  // A swiped picture sliding into place: what puts it there at once.
  const sliding = useRef<() => void>(undefined);
  // The image fingers zoomed in on and how it shows, and what shows it as it
  // fits again.
  const zoomed = useRef<{ img: HTMLImageElement; view: View }>(undefined);
  const unzoom = useRef<() => void>(undefined);
  const latest = useRef({ index, count: items.length, onIndex });
  latest.current = { index, count: items.length, onIndex };
  const several = items.length > 1;
  // Fingers swipe between pictures and zoom in on images. On a video alone,
  // they are its player's.
  const takesFingers = several || !playable(items[index]?.file.media_type ?? "");
  // The dialog focuses itself as it opens, not the X: Safari shows a focus
  // ring on what a dialog focuses, even when a tap opened it, and Chrome,
  // which focuses the X anyway, rings it at the first arrow key. Tab reaches
  // the X. React doesn't set the `autofocus` attribute from `autoFocus`.
  useEffect(() => {
    ref.current?.setAttribute("autofocus", "");
    ref.current?.showModal();
    ref.current?.focus();
  }, []);
  useHoldPage();

  /** Shows the picture before (-1) or after (1) this one, if there is one. */
  const go = (by: -1 | 1) => {
    sliding.current?.();
    const { index, count, onIndex } = latest.current;
    if (index + by >= 0 && index + by < count) onIndex?.(index + by);
  };

  // A picture that another one takes the place of shows as it fits again.
  // biome-ignore lint/correctness/useExhaustiveDependencies: when the picture shown changes
  useLayoutEffect(() => () => unzoom.current?.(), [index]);

  // A finger swipes the picture aside, and the one beside it follows it in.
  // Two fingers zoom in on an image; then one moves it about, and past its
  // left or right edge swipes on. A double tap zooms in or back out. Touch
  // events, not pointer events: a mouse sends none, so that its drags stay
  // the browser's (dragging the picture out, selecting the caption).
  useEffect(() => {
    const d = ref.current;
    const t = track.current;
    if (!takesFingers || !d || !t) return;
    const off = new AbortController();
    let touched: Point | undefined;
    let swipe: Swipe | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // Into place, from where the finger left it: on the next (1) or previous
    // (-1) picture, or back (0).
    const slide = (by: -1 | 0 | 1) => {
      t.dataset.sliding = "";
      if (by) t.style.translate = `${-by * 100}% 0`;
      else t.style.removeProperty("translate");
      const done = () => {
        clearTimeout(timer);
        t.removeEventListener("transitionend", ended);
        sliding.current = undefined;
        // The picture it slid to takes the middle as the track goes back under it.
        const { index, onIndex } = latest.current;
        if (by) flushSync(() => onIndex?.(index + by));
        delete t.dataset.sliding;
        t.style.removeProperty("translate");
      };
      const ended = (e: TransitionEvent) => {
        if (e.target === t && e.propertyName === "translate") done();
      };
      t.addEventListener("transitionend", ended, { signal: off.signal });
      timer = setTimeout(done, SLIDE_MS);
      sliding.current = done;
    };
    // The pictures follow a finger `dx` px aside, and resist it toward a side with no more.
    const follow = (dx: number) => {
      const { index, count } = latest.current;
      const more = dx > 0 ? index > 0 : index < count - 1;
      t.style.translate = `${more ? dx : dx * RESIST}px 0`;
    };

    // A finger moving a zoomed image about, or two zooming it: the image, how
    // it showed and where they touched it.
    let pan: { img: HTMLImageElement; zoom: Zoom; view: View; from: Point; trail: Trail } | undefined;
    let pinch:
      | {
          img: HTMLImageElement;
          zoom: Zoom;
          view: View;
          ids: number[];
          from: [Point, Point];
          to: [Point, Point];
        }
      | undefined;
    // A finger tapping the image, and the tap before it, for a double tap.
    let tap: { img: HTMLImageElement; x: number; y: number; at: number } | undefined;
    let tapped: { x: number; y: number; at: number } | undefined;
    // A gliding image's next frame, and a settling one: what puts it in place at once.
    let frame = 0;
    let settling: (() => void) | undefined;
    let settleTimer: ReturnType<typeof setTimeout> | undefined;
    /** The image shown, once it has loaded. */
    const image = () => {
      const img = t.querySelector<HTMLImageElement>("figure:not([inert]) img");
      return img?.complete && img.naturalWidth ? img : undefined;
    };
    const viewOf = (img: HTMLImageElement) => (zoomed.current?.img === img ? zoomed.current.view : FITS);
    // Where it fits, in the figure that fills the window. Offsets leave its zoom out.
    const measure = (img: HTMLImageElement) =>
      new Zoom(
        { width: d.clientWidth, height: d.clientHeight },
        { left: img.offsetLeft, top: img.offsetTop, width: img.offsetWidth, height: img.offsetHeight },
        img.naturalWidth,
      );
    const show = (img: HTMLImageElement, view: View) => {
      zoomed.current = { img, view };
      img.style.translate = `${view.x}px ${view.y}px`;
      img.style.scale = String(view.scale);
    };
    // At rest, the browser draws it again at the size it shows at.
    const still = (img: HTMLImageElement) => {
      delete img.dataset.moving;
    };
    // Back as it fits.
    const restore = (img: HTMLImageElement) => {
      if (zoomed.current?.img === img) zoomed.current = undefined;
      still(img);
      delete img.dataset.settling;
      img.style.removeProperty("translate");
      img.style.removeProperty("scale");
    };
    // Into place, zoomed in or out, or back within its edges.
    const settle = (img: HTMLImageElement, view: View) => {
      cancelAnimationFrame(frame);
      settling?.();
      still(img);
      img.dataset.settling = "";
      show(img, view);
      const done = () => {
        clearTimeout(settleTimer);
        img.removeEventListener("transitionend", ended);
        settling = undefined;
        delete img.dataset.settling;
        if (view.scale === 1) restore(img);
      };
      const ended = (e: TransitionEvent) => {
        if (e.target === img) done();
      };
      img.addEventListener("transitionend", ended, { signal: off.signal });
      settleTimer = setTimeout(done, SLIDE_MS);
      settling = done;
    };
    // On from where the finger let go of it, slowing down.
    const glide = (img: HTMLImageElement, zoom: Zoom, speed: Point) => {
      if (!speed.x && !speed.y) return still(img);
      let last: number | undefined;
      const step = (now: number) => {
        const z = zoomed.current;
        if (z?.img !== img) return still(img);
        const next = zoom.glide(
          z.view,
          speed,
          last === undefined ? FRAME_MS : Math.min(now - last, 4 * FRAME_MS),
        );
        last = now;
        speed = next.speed;
        show(img, next.view);
        if (speed.x || speed.y) frame = requestAnimationFrame(step);
        else still(img);
      };
      frame = requestAnimationFrame(step);
    };
    unzoom.current = () => {
      cancelAnimationFrame(frame);
      settling?.();
      pan = undefined;
      pinch = undefined;
      tapped = undefined;
      if (zoomed.current) restore(zoomed.current.img);
    };

    // What the fingers on the screen do from now on: two zoom the image, and
    // one moves a zoomed image about, or swipes. An image fingers move is
    // drawn on a layer of its own meanwhile, which moves without drawing it
    // again.
    const begin = (e: TouchEvent) => {
      const a = e.touches[0];
      const b = e.touches[1];
      // A swipe that another finger joins settles back.
      if (swipe) slide(0);
      touched = undefined;
      swipe = undefined;
      pan = undefined;
      pinch = undefined;
      const img = image();
      if (a && b) {
        if (!img) return;
        // Nor is it a click outside the picture.
        pressedOutside.current = false;
        const from: [Point, Point] = [spot(a), spot(b)];
        pinch = {
          img,
          zoom: measure(img),
          view: viewOf(img),
          ids: [a.identifier, b.identifier],
          from,
          to: from,
        };
        img.dataset.moving = "";
      } else if (a) {
        const view = img ? viewOf(img) : FITS;
        if (img && view.scale > 1) {
          pan = { img, zoom: measure(img), view, from: spot(a), trail: new Trail() };
          img.dataset.moving = "";
        } else if (several) touched = spot(a);
      }
    };
    const start = (e: TouchEvent) => {
      sliding.current?.();
      cancelAnimationFrame(frame);
      settling?.();
      const f = e.touches[0];
      const img = image();
      tap =
        f && img && e.touches.length === 1 && e.target === img
          ? { img, x: f.clientX, y: f.clientY, at: e.timeStamp }
          : undefined;
      begin(e);
    };
    const move = (e: TouchEvent) => {
      const f = e.touches[0];
      if (!f) return;
      if (tap && Math.hypot(f.clientX - tap.x, f.clientY - tap.y) >= SLOP) tap = undefined;
      if (pinch) {
        const p = pinch;
        const [a, b] = p.ids.map((id) => Array.from(e.touches).find((t) => t.identifier === id));
        if (!a || !b) return;
        e.preventDefault();
        p.to = [spot(a), spot(b)];
        show(p.img, p.zoom.pinch(p.view, p.from, p.to));
      } else if (pan) {
        const p = pan;
        e.preventDefault();
        const by = { x: f.clientX - p.from.x, y: f.clientY - p.from.y };
        p.trail.moved(spot(f), e.timeStamp);
        // Nor is it a click outside the picture.
        if (Math.hypot(by.x, by.y) >= SLOP) pressedOutside.current = false;
        // Past the image's left or right edge, the finger swipes on.
        const [left, right] = p.zoom.bounds(p.view.scale).x;
        const x = p.view.x + by.x;
        const past = several ? x - Math.min(right, Math.max(left, x)) : 0;
        show(p.img, p.zoom.pan(p.view, { x: by.x - past, y: by.y }));
        if (past) {
          swipe ??= new Swipe(f.clientX - past, e.timeStamp);
          swipe.moved(f.clientX, e.timeStamp);
          follow(swipe.distance);
        } else if (swipe) {
          swipe = undefined;
          t.style.removeProperty("translate");
        }
      } else if (touched) {
        if (!swipe) {
          const dx = f.clientX - touched.x;
          const dy = f.clientY - touched.y;
          if (Math.abs(dx) < SLOP && Math.abs(dy) < SLOP) return;
          // Sideways more than up or down, or it isn't a swipe.
          if (Math.abs(dy) >= Math.abs(dx)) {
            touched = undefined;
            return;
          }
          swipe = new Swipe(f.clientX, e.timeStamp);
          // Nor is it a click outside the picture.
          pressedOutside.current = false;
        }
        e.preventDefault();
        swipe.moved(f.clientX, e.timeStamp);
        follow(swipe.distance);
      }
    };
    const end = (e: TouchEvent) => {
      const on = Array.from(e.touches);
      if (pinch) {
        const p = pinch;
        // Another finger let go: the two go on.
        if (p.ids.every((id) => on.some((t) => t.identifier === id))) return;
        pinch = undefined;
        const view = viewOf(p.img);
        // A finger left on goes on moving the image about, zoomed in as far as it may be.
        if (on.length && view.scale >= 1 && view.scale <= p.zoom.max) return begin(e);
        const [a, b] = p.to;
        settle(p.img, p.zoom.settle(view, { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }));
      }
      if (on.length) return;
      const tapping = tap;
      tap = undefined;
      if (pan) {
        const p = pan;
        pan = undefined;
        const view = viewOf(p.img);
        const rest = p.zoom.settle(view, p.from);
        if (rest.scale !== view.scale || rest.x !== view.x || rest.y !== view.y) settle(p.img, rest);
        else if (!swipe && e.type === "touchend" && !reducedMotion())
          glide(p.img, p.zoom, p.trail.speed(e.timeStamp));
        else still(p.img);
      }
      const s = swipe;
      touched = undefined;
      swipe = undefined;
      if (s) {
        const { index, count } = latest.current;
        const by = e.type === "touchend" ? s.step(e.timeStamp, t.offsetWidth) : 0;
        slide(index + by >= 0 && index + by < count ? by : 0);
      }
      // A double tap on the image zooms in on it, or back out.
      if (!tapping || e.type !== "touchend" || e.timeStamp - tapping.at > TAP_MS) tapped = undefined;
      else if (
        tapped &&
        tapping.at - tapped.at < DOUBLE_TAP_MS &&
        Math.hypot(tapping.x - tapped.x, tapping.y - tapped.y) < DOUBLE_TAP_PX
      ) {
        tapped = undefined;
        settle(tapping.img, measure(tapping.img).toggle(viewOf(tapping.img), tapping));
      } else tapped = { x: tapping.x, y: tapping.y, at: e.timeStamp };
    };
    d.addEventListener("touchstart", start, { passive: true, signal: off.signal });
    d.addEventListener("touchmove", move, { passive: false, signal: off.signal });
    d.addEventListener("touchend", end, { signal: off.signal });
    d.addEventListener("touchcancel", end, { signal: off.signal });
    // Turned, or resized: the image fits anew.
    window.addEventListener("resize", () => unzoom.current?.(), { signal: off.signal });
    return () => {
      off.abort();
      clearTimeout(timer);
      cancelAnimationFrame(frame);
      settling?.();
      sliding.current = undefined;
      unzoom.current = undefined;
      // Back in place, for a picture that stays alone.
      delete t.dataset.sliding;
      t.style.removeProperty("translate");
    };
  }, [several, takesFingers]);

  const current = items[index];
  if (!current) return null;
  // Through the dialog, which hands focus back; its `close` event tells the owner.
  const close = () => ref.current?.close();
  const outside = (target: EventTarget) =>
    target === ref.current ||
    target === track.current ||
    (target instanceof Element && target.parentElement === track.current);
  // The pictures beside this one wait out of sight, loaded for a swipe.
  const shown = several ? [index - 1, index, index + 1] : [index];
  return createPortal(
    <dialog
      ref={ref}
      tabIndex={-1}
      aria-label={current.file.name}
      onClose={onClose}
      onPointerDown={(e) => {
        pressedOutside.current = outside(e.target);
      }}
      onClick={(e) => {
        if (pressedOutside.current && outside(e.target)) close();
      }}
      onKeyDown={(e) => {
        const by = e.key === "ArrowLeft" ? -1 : e.key === "ArrowRight" ? 1 : 0;
        // With a modifier key, an arrow is the browser's (⌘← goes back) or selects.
        if (!by || !several || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
        e.preventDefault();
        go(by);
      }}
      className={cn(
        "m-0 size-full max-h-none max-w-none overflow-hidden bg-transparent p-0 outline-none backdrop:animate-fade backdrop:bg-black/75 backdrop:backdrop-blur-[3px]",
        // Fingers swipe and zoom from the start: the browser doesn't take them for a scroll.
        takesFingers && "touch-none",
      )}
    >
      <button
        type="button"
        onClick={close}
        aria-label="Close"
        className="group fixed top-3 right-3 z-10 rounded-full border border-line-strong bg-surface/90 p-1 backdrop-blur-xl"
      >
        <span className="flex size-7 items-center justify-center rounded-full text-ink-3 transition-colors duration-150 group-hover:bg-hover group-hover:text-ink">
          <X className="size-4" />
        </span>
      </button>
      {several && (
        <>
          <p
            aria-live="polite"
            className="fixed top-3 left-3 z-10 flex h-9.5 items-center rounded-full border border-line-strong bg-surface/90 px-3.5 font-mono text-[11.5px] text-ink-3 tabular-nums backdrop-blur-xl"
          >
            <span className="sr-only">{current.file.name}: </span>
            {index + 1} of {items.length}
          </p>
          <StepButton by={-1} disabled={index === 0} onClick={() => go(-1)} />
          <StepButton by={1} disabled={index === items.length - 1} onClick={() => go(1)} />
        </>
      )}
      <div ref={track} className="lightbox-track absolute inset-0 animate-zoom-in">
        {shown.map((i) => {
          const item = items[i];
          return item && <Slide key={`${i} ${item.src}`} item={item} offset={i - index} steps={several} />;
        })}
      </div>
    </dialog>,
    document.body,
  );
}

/** The button at a side of the lightbox that steps to the picture before (-1) or after (1). */
function StepButton({ by, disabled, onClick }: { by: -1 | 1; disabled: boolean; onClick: () => void }) {
  const label = by < 0 ? "Previous image" : "Next image";
  const Icon = by < 0 ? ChevronLeft : ChevronRight;
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      // Not `disabled`: the button keeps focus at the first or last picture.
      aria-disabled={disabled || undefined}
      aria-keyshortcuts={by < 0 ? "ArrowLeft" : "ArrowRight"}
      title={`${label} (${by < 0 ? "←" : "→"})`}
      className={cn(
        "group fixed top-1/2 z-10 -translate-y-1/2 rounded-full border border-line-strong bg-surface/90 p-1 backdrop-blur-xl aria-disabled:opacity-40",
        // Fingers swipe instead.
        "pointer-coarse:hidden",
        by < 0 ? "left-3" : "right-3",
      )}
    >
      <span className="flex size-7 items-center justify-center rounded-full text-ink-3 transition-colors duration-150 group-hover:bg-hover group-hover:text-ink group-aria-disabled:bg-transparent group-aria-disabled:text-ink-3">
        <Icon className="size-4" />
      </span>
    </button>
  );
}

/**
 * A file in a lightbox, with its caption: the one it shows, or one beside
 * it (`offset` windows to the side), out of sight and reach until a swipe
 * brings it in. With `steps`, it leaves room at its sides for the buttons.
 */
function Slide({ item, offset, steps }: { item: LightboxItem; offset: number; steps: boolean }) {
  const { file, src, download } = item;
  const [failed, setFailed] = useState(false);
  const video = playable(file.media_type ?? "");
  const facts = [
    file.width && file.height ? `${file.width}×${file.height}` : "",
    file.size === undefined ? "" : bytes(file.size),
  ]
    .filter(Boolean)
    .join(" · ");
  // Room at the sides for the buttons, where a mouse uses them.
  const wide = cn("max-w-[calc(100vw-2rem)]", steps && "not-pointer-coarse:max-w-[calc(100vw-8rem)]");
  // Room above for the X, and below for the caption.
  const fit = cn(
    "max-h-[calc(100dvh-8rem)] rounded-lg border border-line-strong shadow-[0_30px_120px_rgb(0_0_0/0.55)]",
    wide,
  );
  return (
    <figure
      inert={offset !== 0}
      style={offset ? { translate: `${offset * 100}% 0` } : undefined}
      className="absolute inset-0 flex flex-col items-center justify-center gap-3 px-4 pt-14 pb-4"
    >
      {failed ? (
        <p className="rounded-lg border border-line-strong bg-surface px-5 py-4 text-[13.5px] text-ink-2">
          {video ? `This browser can't play ${file.name}.` : `${file.name} didn't load.`}
        </p>
      ) : video ? (
        // biome-ignore lint/a11y/useMediaCaption: recordings people attach come without captions.
        <video
          src={src}
          controls
          autoPlay
          playsInline
          onError={() => setFailed(true)}
          className={cn(fit, "bg-black")}
        />
      ) : (
        <img
          src={src}
          alt={file.name}
          decoding="async"
          onError={() => setFailed(true)}
          className={cn(fit, "lightbox-picture h-auto w-auto bg-raised object-contain")}
        />
      )}
      {/* Over the picture, zoomed in. */}
      <figcaption
        className={cn(
          "relative flex h-9 items-center gap-3 rounded-full border border-line-strong bg-surface/90 pl-4 text-[12.5px] text-ink-2 backdrop-blur-xl",
          wide,
          download ? "pr-1 pointer-coarse:pr-4" : "pr-4",
        )}
      >
        <span className="truncate">{file.name}</span>
        {facts && <span className="shrink-0 font-mono text-[11px] text-ink-4">{facts}</span>}
        {download && (
          <a
            href={src}
            download={file.name}
            aria-label={`Download ${file.name}`}
            title="Download"
            className="flex size-7 shrink-0 items-center justify-center rounded-full text-ink-3 hover:bg-hover hover:text-ink pointer-coarse:hidden"
          >
            <Download className="size-3.5" />
          </a>
        )}
      </figcaption>
    </figure>
  );
}
