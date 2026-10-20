import { X } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { useHoldPage } from "@/lib/hold-page";

/** From here up (Tailwind's `md`) a sheet comes in from the right, not up from the bottom. */
const SIDE = "(min-width: 48rem)";
/** How long it takes to slide away at most (`.sheet[data-closing]` in app.css), and a margin. */
const CLOSING_MS = 400;

/**
 * A finger that lets go of a sheet pulled down this far (a fraction of its
 * height), or moving down this fast (px/ms) over its last moments, puts it
 * away; otherwise it settles back.
 */
const PULL_AWAY = 0.25;
const FLICK = 0.4;
/** The last moments, in ms. */
const RECENT_MS = 100;

/**
 * A finger pulling a sheet down, from where it touched it: how far it has
 * pulled, and whether letting go puts the sheet away. It takes measurements
 * rather than reading the page, so that tests can drive it.
 */
export class Pull {
  private moves: { y: number; at: number }[];
  constructor(
    private from: number,
    at: number,
  ) {
    this.moves = [{ y: from, at }];
  }

  /** The finger is at `y` (px) at `at` (ms). */
  moved(y: number, at: number) {
    this.moves.push({ y, at });
    // Enough to measure its speed over its last moments.
    while (this.moves.length > 2 && at - (this.moves[1]?.at ?? at) > RECENT_MS) this.moves.shift();
  }

  /** How far down the sheet is pulled (px): a finger above where it started doesn't lift it. */
  get distance(): number {
    return Math.max(0, (this.moves[this.moves.length - 1]?.y ?? this.from) - this.from);
  }

  /** Whether a finger that lets go at `at` (ms) puts away a sheet `height` px tall. */
  away(at: number, height: number): boolean {
    const recent = this.moves.filter((m) => at - m.at <= RECENT_MS);
    const first = recent[0];
    const last = recent[recent.length - 1];
    const speed = first && last && last.at > first.at ? (last.y - first.y) / (last.at - first.at) : 0;
    return this.distance > height * PULL_AWAY || (speed > FLICK && this.distance > 16);
  }
}

/**
 * A panel over the page, for what the page has no room for, on the native
 * modal <dialog> (focus stays in it; Escape closes it). On phones it slides
 * up from the bottom, as tall as what it holds, and a finger pulls it back
 * down by its top, or by anything in it once that is scrolled to its top.
 * From 768 px it slides in from the right. The X or a click beside it
 * closes it too, and focus goes back to what opened it.
 *
 * Its owner shows it by rendering it, and stops rendering it on `onClose`,
 * once it has slid away.
 */
export function Sheet({
  title,
  description,
  onClose,
  children,
}: {
  title: ReactNode;
  description?: ReactNode;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const titleId = useId();
  // A press that starts in the sheet and ends beside it (selecting text) doesn't close it.
  const pressedOutside = useRef(false);
  // It focuses itself as it opens, not the X: Safari would ring the X after a
  // tap. Tab reaches the X. (React doesn't set `autofocus` from `autoFocus`.)
  useEffect(() => {
    ref.current?.setAttribute("autofocus", "");
    ref.current?.showModal();
  }, []);
  useHoldPage();

  // It slides away, then the dialog closes, which hands focus back and tells the owner.
  const close = useCallback(() => {
    const d = ref.current;
    if (!d?.open || d.dataset.closing !== undefined) return;
    d.dataset.closing = "";
    // From wherever a finger left it.
    d.style.removeProperty("translate");
    d.style.removeProperty("transition");
    d.style.removeProperty("--sheet-pulled");
    const done = () => {
      clearTimeout(timer);
      d.removeEventListener("transitionend", ended);
      d.close();
    };
    const ended = (e: TransitionEvent) => {
      if (e.target === d && e.propertyName === "translate") done();
    };
    d.addEventListener("transitionend", ended);
    const timer = setTimeout(done, CLOSING_MS);
  }, []);

  // A finger pulls it down. Touch events, not pointer events: only they can
  // keep the browser from scrolling what the sheet holds meanwhile.
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    let touched: { x: number; y: number; at: number; inside: boolean } | undefined;
    let pull: Pull | undefined;
    const settle = () => {
      delete d.dataset.pulling;
      d.style.removeProperty("transition");
      d.style.removeProperty("translate");
      d.style.removeProperty("--sheet-pulled");
    };
    const start = (e: TouchEvent) => {
      const t = e.touches[0];
      pull = undefined;
      touched =
        t && e.touches.length === 1 && d.dataset.closing === undefined && !matchMedia(SIDE).matches
          ? {
              x: t.clientX,
              y: t.clientY,
              at: e.timeStamp,
              inside: e.target instanceof Node && !!body.current?.contains(e.target),
            }
          : undefined;
    };
    const move = (e: TouchEvent) => {
      const t = e.touches[0];
      if (!touched || !t) return;
      if (!pull) {
        // Down more than sideways, by the top of the sheet or from the top of
        // what it holds: anything else scrolls that, or isn't a pull.
        const dx = t.clientX - touched.x;
        const dy = t.clientY - touched.y;
        if (dy <= 0 || Math.abs(dx) > dy || (touched.inside && (body.current?.scrollTop ?? 0) > 0)) {
          touched = undefined;
          return;
        }
        pull = new Pull(touched.y, touched.at);
        d.dataset.pulling = "";
        d.style.transition = "none";
      }
      e.preventDefault();
      pull.moved(t.clientY, e.timeStamp);
      d.style.translate = `0 ${pull.distance}px`;
      d.style.setProperty("--sheet-pulled", String(Math.min(1, pull.distance / d.offsetHeight)));
    };
    const end = (e: TouchEvent) => {
      const p = pull;
      touched = undefined;
      pull = undefined;
      if (!p) return;
      if (e.type === "touchend" && p.away(e.timeStamp, d.offsetHeight)) {
        delete d.dataset.pulling;
        close();
      } else {
        // Back up, at the sheet's own pace.
        settle();
      }
    };
    const off = new AbortController();
    d.addEventListener("touchstart", start, { passive: true, signal: off.signal });
    d.addEventListener("touchmove", move, { passive: false, signal: off.signal });
    d.addEventListener("touchend", end, { signal: off.signal });
    d.addEventListener("touchcancel", end, { signal: off.signal });
    return () => off.abort();
  }, [close]);

  return createPortal(
    // biome-ignore lint/a11y/useKeyWithClickEvents: the click closes beside the sheet only; Escape is native to <dialog>.
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      onCancel={(e) => {
        // Escape: it slides away first.
        e.preventDefault();
        close();
      }}
      onClose={onClose}
      onPointerDown={(e) => {
        pressedOutside.current = e.target === ref.current;
      }}
      onClick={(e) => {
        if (pressedOutside.current && e.target === ref.current) close();
      }}
      className={
        "sheet m-0 flex-col overflow-hidden border-line-strong bg-surface p-0 text-ink outline-none open:flex " +
        // Phones: along the bottom, as tall as what it holds, up to most of the screen.
        "inset-x-0 top-auto bottom-0 max-h-[min(88dvh,calc(100dvh-2.5rem))] w-full max-w-none rounded-t-[18px] border-t shadow-[0_-16px_60px_rgb(0_0_0/0.35)] " +
        // From 768 px: along the right, the screen's height.
        "md:top-0 md:right-0 md:left-auto md:h-dvh md:max-h-none md:w-[340px] md:rounded-none md:border-t-0 md:border-l md:shadow-[-16px_0_60px_rgb(0_0_0/0.3)]"
      }
    >
      <div className="relative shrink-0 border-b border-line px-5 pt-6 pb-4 md:pt-5">
        {/* What a finger pulls it down by, with the title. */}
        <span
          aria-hidden="true"
          className="absolute top-2 left-1/2 h-[5px] w-9 -translate-x-1/2 rounded-full bg-ink-4/50 md:hidden"
        />
        <div className="flex items-start gap-3">
          <div className="min-w-0 flex-1 [overflow-wrap:anywhere]">
            <h2
              id={titleId}
              className="font-display text-[19px] leading-snug font-medium tracking-[-0.01em] text-ink"
            >
              {title}
            </h2>
            {description && <p className="mt-1 text-[13.5px] leading-relaxed text-ink-3">{description}</p>}
          </div>
          <button
            type="button"
            aria-label="Close"
            onClick={close}
            className="group -mt-1.5 -mr-2.5 flex size-11 shrink-0 items-center justify-center rounded-full"
          >
            <span className="flex size-7 items-center justify-center rounded-full bg-hover text-ink-3 transition-colors duration-150 group-hover:bg-line-strong group-hover:text-ink">
              <X className="size-4" />
            </span>
          </button>
        </div>
      </div>
      <div
        ref={body}
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain pb-[env(safe-area-inset-bottom)]"
      >
        {children}
      </div>
    </dialog>,
    document.body,
  );
}
