import { ChevronLeft, ChevronRight, Download, X } from "lucide-react";
import { type MouseEvent, useEffect, useRef, useState } from "react";
import { createPortal, flushSync } from "react-dom";
import { playable } from "@/lib/attachments";
import { cn } from "@/lib/cn";
import { bytes } from "@/lib/format";
import { useHoldPage } from "@/lib/hold-page";

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
/** How long a swiped picture takes to slide into place at most (`.lightbox-track` in app.css), and a margin. */
const SLIDE_MS = 400;

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
 * or next one.
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
  const latest = useRef({ index, count: items.length, onIndex });
  latest.current = { index, count: items.length, onIndex };
  const several = items.length > 1;
  // The dialog focuses itself as it opens, not the X: Safari shows a focus
  // ring on what a dialog focuses, even when a tap opened it. Tab reaches
  // the X. (Chrome focuses the X anyway, with a ring only for the keyboard.)
  // React doesn't set the `autofocus` attribute from `autoFocus`.
  useEffect(() => {
    ref.current?.setAttribute("autofocus", "");
    ref.current?.showModal();
  }, []);
  useHoldPage();

  /** Shows the picture before (-1) or after (1) this one, if there is one. */
  const go = (by: -1 | 1) => {
    sliding.current?.();
    const { index, count, onIndex } = latest.current;
    if (index + by >= 0 && index + by < count) onIndex?.(index + by);
  };

  // A finger swipes the picture aside, and the one beside it follows it in.
  // Touch events, not pointer events: a mouse sends none, so that its drags
  // stay the browser's (dragging the picture out, selecting the caption).
  useEffect(() => {
    const d = ref.current;
    const t = track.current;
    if (!several || !d || !t) return;
    const off = new AbortController();
    let touched: { x: number; y: number } | undefined;
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
    const start = (e: TouchEvent) => {
      sliding.current?.();
      const f = e.touches[0];
      swipe = undefined;
      touched = f && e.touches.length === 1 ? { x: f.clientX, y: f.clientY } : undefined;
    };
    const move = (e: TouchEvent) => {
      const f = e.touches[0];
      if (!touched || !f) return;
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
      const { index, count } = latest.current;
      const by = swipe.distance;
      const more = by > 0 ? index > 0 : index < count - 1;
      t.style.translate = `${more ? by : by * RESIST}px 0`;
    };
    const end = (e: TouchEvent) => {
      const s = swipe;
      touched = undefined;
      swipe = undefined;
      if (!s) return;
      const { index, count } = latest.current;
      const by = e.type === "touchend" ? s.step(e.timeStamp, t.offsetWidth) : 0;
      slide(index + by >= 0 && index + by < count ? by : 0);
    };
    d.addEventListener("touchstart", start, { passive: true, signal: off.signal });
    d.addEventListener("touchmove", move, { passive: false, signal: off.signal });
    d.addEventListener("touchend", end, { signal: off.signal });
    d.addEventListener("touchcancel", end, { signal: off.signal });
    return () => {
      off.abort();
      clearTimeout(timer);
      sliding.current = undefined;
    };
  }, [several]);

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
        // A finger's moves are a swipe's from the start: the browser doesn't take them for a scroll.
        several && "touch-none",
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
          className={cn(fit, "h-auto w-auto bg-raised object-contain")}
        />
      )}
      <figcaption
        className={cn(
          "flex h-9 items-center gap-3 rounded-full border border-line-strong bg-surface/90 pl-4 text-[12.5px] text-ink-2 backdrop-blur-xl",
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
