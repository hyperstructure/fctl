// The diff dialog follows the agent (session/diff-dialog.tsx): as it changes
// the files again, their diffs change where the reader looks
// (components/diff-play.ts), and the reader stays where they are. The line
// at the top of the view, under its file's header, holds its place while
// rows come and go above it and files above it take their room, at the
// very top of the diffs too; what changes below it pushes the rest down.
// What the agent changed out of sight waits, counted on a button at the
// edge it is past, until the reader comes to it, when it lights up.

import { ArrowDown, ArrowUp } from "lucide-react";
import { type CSSProperties, type RefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type LiveView, light } from "@/components/diff-play";
import { cn } from "@/lib/cn";

/** Where the reader's place is: a row (or a file's section, before it shows its rows), and how far down the diffs it is. */
interface Anchor {
  el: Element;
  /** Its file's section, and its line's place in its hunks, to find it again once it was drawn anew. */
  section: Element;
  l?: string;
  /** Its top, from the top of the diffs (px). */
  top: number;
}

/** A change out of sight: its rows, or its file's section when the file doesn't show its rows. */
type Change = HTMLElement[];

/** The first of `list` whose bottom is past `y`. */
function firstPast<T extends Element>(list: ArrayLike<T>, y: number): T | undefined {
  let lo = 0;
  let hi = list.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((list[mid]?.getBoundingClientRect().bottom ?? 0) <= y) lo = mid + 1;
    else hi = mid;
  }
  return list[lo];
}

/**
 * Keeps the reader's place in `body`, which scrolls the files' sections in
 * `column` under headers `header` px tall, and follows what the agent
 * changes out of sight. `pinned` is the section of the file the reader went
 * to, which stays at the top until they scroll (as the dialog opens, too:
 * it gets its size after it scrolled there). `view` is for the diffs, `pick`
 * to call after the dialog scrolled them itself.
 */
export function useLive(
  body: RefObject<HTMLElement | null>,
  column: RefObject<HTMLElement | null>,
  header: number,
  pinned: () => HTMLElement | null | undefined,
) {
  const anchor = useRef<Anchor | undefined>(undefined);
  // Read where it's called: a new list of files makes a new `pinned`, and the place must not be
  // picked anew because of it (see the effect below).
  const pinnedNow = useRef(pinned);
  pinnedNow.current = pinned;

  const pick = useCallback(() => {
    const b = body.current;
    if (!b) return;
    const v = b.getBoundingClientRect();
    const y = v.top + header + 1;
    const s = firstPast(b.querySelectorAll<HTMLElement>("section[data-file]"), y);
    if (!s) {
      anchor.current = undefined;
      return;
    }
    const el = firstPast(s.querySelectorAll<HTMLElement>(".diff-line, .diff-hunk"), y) ?? s;
    anchor.current = {
      el,
      section: s,
      l: (el as HTMLElement).dataset.l,
      top: el.getBoundingClientRect().top - v.top + b.scrollTop,
    };
  }, [body, header]);

  const hold = useCallback(() => {
    const b = body.current;
    if (!b) return;
    const p = pinnedNow.current();
    if (p) {
      if (Math.abs(b.scrollTop - p.offsetTop) > 1) b.scrollTop = p.offsetTop;
      pick();
      return;
    }
    const a = anchor.current;
    if (!a) return;
    if (!a.el.isConnected) {
      // Drawn anew (laid out another way): the same line of the same file, if it still shows.
      const again =
        a.l !== undefined && a.section.isConnected ? a.section.querySelector(`[data-l="${a.l}"]`) : undefined;
      if (!again) {
        pick();
        return;
      }
      a.el = again;
    }
    const top = a.el.getBoundingClientRect().top - b.getBoundingClientRect().top + b.scrollTop;
    const by = top - a.top;
    a.top = top;
    if (Math.abs(by) >= 0.5) b.scrollTop += by;
  }, [body, pick]);

  // The reader's place is where they scroll to; it holds as what is above it changes size. Set up
  // once: picking the place again as it starts over would take a layout that changed and wasn't
  // held yet for where the reader was.
  useEffect(() => {
    const b = body.current;
    const c = column.current;
    if (!b || !c) return;
    b.addEventListener("scroll", pick, { passive: true });
    const ro = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(hold);
    ro?.observe(c);
    pick();
    return () => {
      b.removeEventListener("scroll", pick);
      ro?.disconnect();
    };
  }, [body, column, pick, hold]);

  const band = useCallback(() => {
    const v = body.current?.getBoundingClientRect();
    return v && { top: v.top + header, bottom: v.bottom };
  }, [body, header]);
  const where = useCallback(
    (el: Element): "above" | "in" | "below" => {
      const v = band();
      if (!v) return "in";
      const r = el.getBoundingClientRect();
      return r.bottom <= v.top ? "above" : r.top >= v.bottom ? "below" : "in";
    },
    [band],
  );

  // What changed out of sight, until the reader comes to it.
  const waiting = useRef(new Set<Change>());
  const [counts, setCounts] = useState({ above: 0, below: 0 });
  const looking = useRef(0);
  /** The rows of a change that show, or nearly: a long one (a new file's) lights up no further. */
  const near = useCallback(
    (rows: HTMLElement[]) => {
      const v = band();
      if (!v || rows.length < 60) return rows;
      return rows.filter((el) => {
        const r = el.getBoundingClientRect();
        return r.bottom > v.top - 200 && r.top < v.bottom + 600;
      });
    },
    [band],
  );
  // Rows that move (open or close) are no sign that the reader saw what they move past.
  const frame = useRef(0);
  const look = useCallback(() => {
    looking.current = 0;
    let above = 0;
    let below = 0;
    for (const change of waiting.current) {
      const first = change.find((el) => el.isConnected);
      if (!first) {
        waiting.current.delete(change);
        continue;
      }
      const w = where(first);
      if (w === "in") {
        if (frame.current) continue;
        waiting.current.delete(change);
        if (first.tagName !== "SECTION") light(near(change));
      } else if (w === "above") above++;
      else below++;
    }
    setCounts((c) => (c.above === above && c.below === below ? c : { above, below }));
  }, [where, near]);
  const lookSoon = useCallback(() => {
    if (!looking.current) looking.current = requestAnimationFrame(look);
  }, [look]);
  useEffect(() => {
    const b = body.current;
    if (!b) return;
    b.addEventListener("scroll", lookSoon, { passive: true });
    return () => {
      b.removeEventListener("scroll", lookSoon);
      cancelAnimationFrame(looking.current);
    };
  }, [body, lookSoon]);

  const unseen = useCallback(
    (changes: Change[]) => {
      for (const c of changes) if (c.length) waiting.current.add(c);
      lookSoon();
    },
    [lookSoon],
  );

  // While rows open and close, it holds at each frame; then what came into view as they did is seen.
  const until = useRef(0);
  const moving = useCallback(
    (ms: number) => {
      until.current = Math.max(until.current, performance.now() + ms);
      if (frame.current) return;
      const tick = () => {
        hold();
        if (performance.now() < until.current) frame.current = requestAnimationFrame(tick);
        else {
          frame.current = 0;
          lookSoon();
        }
      };
      frame.current = requestAnimationFrame(tick);
    },
    [hold, lookSoon],
  );
  useEffect(() => () => cancelAnimationFrame(frame.current), []);

  /** The change out of sight nearest the view, above it or below it. */
  const nearest = useCallback(
    (dir: "above" | "below"): HTMLElement | undefined => {
      let best: { el: HTMLElement; d: number } | undefined;
      const b = body.current;
      if (!b) return undefined;
      const v = b.getBoundingClientRect();
      for (const change of waiting.current) {
        const first = change.find((el) => el.isConnected);
        if (!first || where(first) !== dir) continue;
        const r = first.getBoundingClientRect();
        const d = dir === "above" ? v.top - r.bottom : r.top - v.bottom;
        if (!best || d < best.d) best = { el: first, d };
      }
      return best?.el;
    },
    [body, where],
  );

  const view: LiveView = useMemo(() => ({ hold, moving, band, unseen }), [hold, moving, band, unseen]);
  return { view, pick, counts, nearest, where, look: lookSoon };
}

/**
 * Says how many changes the agent made past an edge of the view, which the
 * reader hasn't seen, and goes to the nearest of them.
 */
export function UnseenChanges({
  dir,
  count,
  onGo,
}: {
  dir: "above" | "below";
  count: number;
  onGo: () => void;
}) {
  // It stays a moment as it goes, to fade.
  const [shown, setShown] = useState(count);
  const [leaving, setLeaving] = useState(false);
  useEffect(() => {
    if (count > 0) {
      setShown(count);
      setLeaving(false);
      return;
    }
    setLeaving(true);
    const t = setTimeout(() => setShown(0), 200);
    return () => clearTimeout(t);
  }, [count]);
  if (!shown) return null;
  const Arrow = dir === "above" ? ArrowUp : ArrowDown;
  const what = `${shown} new ${shown === 1 ? "change" : "changes"} ${dir}`;
  return (
    <div
      className={cn(
        "pointer-events-none absolute inset-x-0 z-[4] flex justify-center",
        dir === "above" ? "top-[52px]" : "bottom-4",
      )}
    >
      <button
        type="button"
        onClick={onGo}
        onMouseDown={(e) => e.preventDefault()}
        data-dir={dir}
        data-leaving={leaving || undefined}
        className="diff-unseen pointer-events-auto inline-flex h-7 items-center gap-2 rounded-full border border-line-strong bg-raised pr-3 pl-2 text-[12px] font-medium text-ink shadow-[0_8px_28px_rgb(0_0_0/0.32)] transition-colors hover:bg-[color-mix(in_srgb,var(--c-raised),var(--c-ink)_6%)]"
      >
        <span className="relative flex size-4 items-center justify-center rounded-full bg-highlight text-highlight-ink">
          <Arrow className="size-3" strokeWidth={2.5} aria-hidden="true" />
        </span>
        <span key={shown} className="tabular-nums">
          {what}
        </span>
      </button>
    </div>
  );
}

/**
 * How many times `value` changed since it was first given (a file's
 * digest: the agent changed the file), and whether it did in the last `ms`.
 */
export function useChanged(value: string | undefined, ms = 2600): { times: number; fresh: boolean } {
  const seen = useRef(value);
  const [times, setTimes] = useState(0);
  const [fresh, setFresh] = useState(false);
  useEffect(() => {
    if (value === undefined) return;
    const before = seen.current;
    seen.current = value;
    if (before === undefined || before === value) return;
    setTimes((n) => n + 1);
    setFresh(true);
    const t = setTimeout(() => setFresh(false), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return { times, fresh };
}

/** A number that rolls to its new value as it changes: up as it grows, down as it shrinks. */
export function Rolling({ value }: { value: number }) {
  const [shown, setShown] = useState(value);
  const [roll, setRoll] = useState<{ from: number; dir: 1 | -1 }>();
  if (shown !== value) {
    setShown(value);
    setRoll({ from: shown, dir: value > shown ? 1 : -1 });
  }
  const style = roll ? ({ "--roll": roll.dir } as CSSProperties) : undefined;
  return (
    <span className="inline-grid overflow-hidden align-top">
      {roll && (
        <span
          key={`${roll.from}-out`}
          aria-hidden="true"
          style={style}
          className="col-start-1 row-start-1 animate-roll-out"
        >
          {roll.from}
        </span>
      )}
      <span
        key={value}
        style={style}
        className={cn("col-start-1 row-start-1", roll && "animate-roll-in")}
        onAnimationEnd={() => setRoll(undefined)}
      >
        {value}
      </span>
    </span>
  );
}
