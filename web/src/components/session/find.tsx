// Finding in a session's transcript (⌘F on its page): a bar under the header
// that finds what was typed in all the session said, steps from match to
// match, the newest first, opens what folds the match it gets to, loads
// earlier activity to look in it too, and lights the matches up on the page
// (lib/find.ts says what it looks in and how).

import { ChevronDown, ChevronUp, Search, X } from "lucide-react";
import {
  createContext,
  type RefObject,
  useCallback,
  useContext,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { IconButton, Spinner } from "@/components/ui";
import { cn } from "@/lib/cn";
import { findables, type Reveal as How, type Match, needle, rangesIn, revealing, search } from "@/lib/find";
import type { LiveSession } from "@/lib/live";
import type { Item } from "@/lib/transcript";

/** What finding wants open in the transcript, by key ([`How`]): it opens what it gets to. */
export const Reveal = createContext<ReadonlyMap<string, How[1]>>(new Map());

/**
 * Calls `open` once finding wants the line `key` open, for a match in what it
 * opens into. What the line shows stays its own: the reader closes it again.
 */
export function useRevealed(key: string | undefined, open: () => void) {
  const wanted = useContext(Reveal).get(key ?? "") === "open";
  const latest = useRef(open);
  latest.current = open;
  useLayoutEffect(() => {
    if (wanted) latest.current();
  }, [wanted]);
}

/** On Apple's systems shortcuts take ⌘; elsewhere, Ctrl (lib/hotkeys.ts). */
const apple = () => /Macintosh|iPhone|iPad|iPod/.test(navigator.userAgent);
export const findShortcut = () => (apple() ? "⌘F" : "Ctrl+F");

/** Pages of earlier activity finding loads by itself, a few at a time: 40,000 events. Further back, a step past the oldest match loads more. */
const AUTO_PAGES = 100;
const PAGES_AT_ONCE = 5;

const NONE: ReadonlyMap<string, How[1]> = new Map();

/** How long typing pauses before the page goes to the match it found, opening what folds it (ms). */
const TYPING_PAUSE_MS = 350;

/** Highlights for the matches, and the one the reader is at (styles/app.css). */
const HIGHLIGHT = "fctl-find";
const HIGHLIGHT_CURRENT = "fctl-find-current";

function registry(): HighlightRegistry | undefined {
  return typeof CSS !== "undefined" && "highlights" in CSS && typeof Highlight === "function"
    ? CSS.highlights
    : undefined;
}

const byId = (root: ParentNode, attr: string, id: string) =>
  root.querySelector(`[${attr}="${CSS.escape(id)}"]`);

/**
 * Finding in the transcript of the session `store` holds: what the bar shows
 * and does, and what finding wants open (`reveal`, for the transcript's
 * [`Reveal`]). `transcript` holds the transcript; `header` and `footer` are
 * what covers the page above and below it. `landing`: what a search of all
 * sessions found here, and the event it found it in.
 */
export function useFind({
  store,
  items,
  cwd,
  hasMoreBefore,
  loadingOlder,
  transcript,
  header,
  footer,
  landing,
}: {
  store: LiveSession;
  items: Item[];
  cwd?: string;
  hasMoreBefore: boolean;
  loadingOlder: boolean;
  transcript: RefObject<HTMLElement | null>;
  header: RefObject<HTMLElement | null>;
  footer: RefObject<HTMLElement | null>;
  landing?: { query: string; seq?: number };
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const typed = useDeferredValue(query);
  const [current, setCurrent] = useState<string>();
  // The match the page went to, and whether the reader stepped to the one the bar is at.
  const [gone, setGone] = useState<string>();
  const stepped = useRef(false);
  const [reveal, setReveal] = useState(NONE);
  const input = useRef<HTMLInputElement>(null);
  const bar = useRef<HTMLElement>(null);
  // Where the focus was before the bar took it, to go back to.
  const before = useRef<Element | null>(null);
  // Pages finding loaded by itself this time, and a step past the oldest match that waits for more.
  const pages = useRef(0);
  const older = useRef(false);
  // The event a search of all sessions found the text in, until the bar gets there.
  const target = useRef<number | undefined>(undefined);

  const n = needle(typed);
  const regions = useMemo(() => (open ? findables(items, cwd) : []), [open, items, cwd]);
  const matches = useMemo(() => (n ? search(regions, typed) : []), [regions, typed, n]);
  const index = useMemo(() => new Map(matches.map((m, i) => [m.key, i])), [matches]);
  const at = current === undefined ? -1 : (index.get(current) ?? -1);
  const match = matches[at];

  const focus = useCallback(() => {
    requestAnimationFrame(() => {
      input.current?.focus();
      input.current?.select();
    });
  }, []);

  const show = useCallback(() => {
    if (document.activeElement !== input.current) before.current = document.activeElement;
    // Words selected on the page are what to find, as in a browser or an editor.
    const selected = window.getSelection()?.toString().trim() ?? "";
    if (selected && !selected.includes("\n") && selected.length <= 200) setQuery(selected);
    setOpen(true);
    focus();
  }, [focus]);

  const close = useCallback(() => {
    setOpen(false);
    setCurrent(undefined);
    setGone(undefined);
    setReveal(NONE);
    older.current = false;
    target.current = undefined;
    pages.current = 0;
    const back = before.current;
    before.current = null;
    if (back instanceof HTMLElement && back.isConnected) back.focus({ preventScroll: true });
  }, []);

  // Another session: the bar closes, unless a search of all sessions opened it.
  useEffect(() => {
    if (store) close();
  }, [store, close]);

  // A search of all sessions opened the page: the bar finds what it found, where it found it,
  // and takes the keys, for Enter to go on to the next match (not on touch screens, where the
  // keyboard would cover the page).
  useEffect(() => {
    if (!landing?.query) return;
    setOpen(true);
    setQuery(landing.query);
    target.current = landing.seq;
    if (window.matchMedia?.("(pointer: fine)").matches) focus();
  }, [landing, focus]);

  // The match nearest the reader: the newest one that starts above the bottom of the view, else the oldest.
  const nearest = useCallback(
    (list: Match[]): Match | undefined => {
      const root = transcript.current;
      if (!root || !list.length) return list[list.length - 1];
      // Where the composer starts, or the view ends.
      const bottom = footer.current?.getBoundingClientRect().top || window.innerHeight;
      let best: Match | undefined;
      const tops = new Map<string, number>();
      for (const m of list) {
        const part = m.findable.part;
        let top = tops.get(part);
        if (top === undefined) {
          top = byId(root, "data-part", part)?.getBoundingClientRect().top ?? Number.NEGATIVE_INFINITY;
          tops.set(part, top);
        }
        if (top >= bottom) break;
        best = m;
      }
      return best ?? list[0];
    },
    [transcript, footer],
  );

  // What was typed changed: the bar goes to the match nearest the reader.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only a new search moves it there
  useEffect(() => {
    if (!open || target.current !== undefined) return;
    older.current = false;
    setCurrent(nearest(matches)?.key);
  }, [open, typed]);

  // Matches came (earlier activity loaded), or the transcript changed under the match the bar
  // was at (what it was in is gone): the nearest one.
  useEffect(() => {
    if (!open || target.current !== undefined || !matches.length) return;
    if (current === undefined || !index.has(current)) setCurrent(nearest(matches)?.key);
  }, [open, current, index, matches, nearest]);

  // Landing from a search of all sessions: the first match in the event it found, once it loaded.
  useEffect(() => {
    const seq = target.current;
    // Once the transcript is there.
    if (!open || seq === undefined || !n || !items.length) return;
    const found = matches.find((m) => m.findable.seq === seq);
    if (found) {
      target.current = undefined;
      stepped.current = true;
      setCurrent(found.key);
      return;
    }
    const first = items[0]?.seq ?? 0;
    if (hasMoreBefore && (first === 0 || first > seq)) {
      if (!loadingOlder) void store.loadEarlier({ until: seq, pages: 50 });
      return;
    }
    // Not where the search found it (its text changed, or finding reads it another way): the nearest event's.
    target.current = undefined;
    stepped.current = true;
    const after = matches.find((m) => m.findable.seq >= seq);
    setCurrent((after ?? matches[matches.length - 1])?.key);
  }, [open, matches, n, items, hasMoreBefore, loadingOlder, store]);

  // Earlier activity, to look in it too, a few pages at a time while there is something to find.
  useEffect(() => {
    if (!open || !n || !hasMoreBefore || loadingOlder || pages.current >= AUTO_PAGES) return;
    const t = setTimeout(() => {
      pages.current += PAGES_AT_ONCE;
      void store.loadEarlier({ pages: PAGES_AT_ONCE });
    }, 250);
    return () => clearTimeout(t);
  }, [open, n, hasMoreBefore, loadingOlder, store]);

  // A step past the oldest match waited for earlier activity: the next match back, or around to the newest.
  useEffect(() => {
    if (!older.current || loadingOlder) return;
    if (at > 0) {
      older.current = false;
      setCurrent(matches[at - 1]?.key);
    } else if (!hasMoreBefore) {
      older.current = false;
      setCurrent(matches[matches.length - 1]?.key);
    } else void store.loadEarlier({ pages: PAGES_AT_ONCE });
  }, [at, matches, hasMoreBefore, loadingOlder, store]);

  /** To the match before (`older`: up the page), or after; around at either end. */
  const step = useCallback(
    (dir: "older" | "newer") => {
      stepped.current = true;
      if (!matches.length) {
        // Nothing yet, and finding stopped loading by itself: further back.
        if (dir === "older" && n && hasMoreBefore) {
          older.current = true;
          if (!loadingOlder) void store.loadEarlier({ pages: PAGES_AT_ONCE });
        }
        return;
      }
      if (dir === "newer") {
        setCurrent(matches[at < 0 || at >= matches.length - 1 ? 0 : at + 1]?.key);
        return;
      }
      if (at > 0) {
        setCurrent(matches[at - 1]?.key);
        return;
      }
      if (hasMoreBefore) {
        older.current = true;
        if (!loadingOlder) void store.loadEarlier({ pages: PAGES_AT_ONCE });
        return;
      }
      setCurrent(matches[matches.length - 1]?.key);
    },
    [matches, at, n, hasMoreBefore, loadingOlder, store],
  );

  // The match the page goes to: the one the bar is at, once the reader
  // stops typing, so that what folds the matches of what they typed so far
  // stays folded; at once when they step to it.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the bar moving is what counts
  useEffect(() => {
    if (!match) return;
    if (stepped.current) {
      stepped.current = false;
      setGone(match.key);
      return;
    }
    const t = setTimeout(() => setGone(match.key), TYPING_PAUSE_MS);
    return () => clearTimeout(t);
  }, [match?.key]);

  // What folds it opens, and stays open.
  useEffect(() => {
    const m = gone === undefined ? undefined : matches[index.get(gone) ?? -1];
    if (m?.findable.reveal.length) setReveal((r) => revealing(r, m));
  }, [gone, matches, index]);

  // It comes into view: at once where the page shows it already, else once what folds it opened.
  // biome-ignore lint/correctness/useExhaustiveDependencies: it moves the page only when the bar moves
  useEffect(() => {
    const root = transcript.current;
    if (!open || !root || !match) return;
    const there = gone === match.key;
    if (!there && !byId(root, "data-find", match.findable.id)) return;
    let frames = 0;
    let raf = 0;
    const go = () => {
      const el = byId(root, "data-find", match.findable.id);
      if (!el && there && frames++ < 12) {
        raf = requestAnimationFrame(go);
        return;
      }
      const ranges = el ? rangesIn(el, n) : [];
      const into =
        ranges[match.at] ?? ranges[ranges.length - 1] ?? el ?? byId(root, "data-part", match.findable.part);
      if (into) bringIntoView(into, bar.current ?? header.current, footer.current);
    };
    go();
    return () => cancelAnimationFrame(raf);
  }, [open, match?.key, gone, n]);

  // The matches light up on the page, the one the bar is at most, as it changes.
  useEffect(() => {
    const root = transcript.current;
    const reg = registry();
    if (!root || !reg) return;
    if (!open || !n || !matches.length) {
      reg.delete(HIGHLIGHT);
      reg.delete(HIGHLIGHT_CURRENT);
      return;
    }
    const ids = new Set(matches.map((m) => m.findable.id));
    let raf = 0;
    const paint = () => {
      raf = 0;
      const all: Range[] = [];
      let here: Range | undefined;
      for (const el of root.querySelectorAll("[data-find]")) {
        const id = el.getAttribute("data-find") ?? "";
        if (!ids.has(id)) continue;
        const ranges = rangesIn(el, n);
        if (id === match?.findable.id) here = ranges[match.at] ?? ranges[ranges.length - 1];
        for (const r of ranges) if (r !== here) all.push(r);
      }
      reg.set(HIGHLIGHT, new Highlight(...all));
      if (here) reg.set(HIGHLIGHT_CURRENT, new Highlight(here));
      else reg.delete(HIGHLIGHT_CURRENT);
    };
    paint();
    // Again as what shows changes: lines that open, code that colors in, earlier activity.
    const watch = new MutationObserver((records) => {
      if (raf || !records.some(touchesRegion)) return;
      raf = requestAnimationFrame(paint);
    });
    watch.observe(root, { subtree: true, childList: true, characterData: true });
    return () => {
      watch.disconnect();
      cancelAnimationFrame(raf);
    };
  }, [open, n, matches, match, transcript]);

  useEffect(
    () => () => {
      registry()?.delete(HIGHLIGHT);
      registry()?.delete(HIGHLIGHT_CURRENT);
    },
    [],
  );

  // ⌘F opens the bar, or takes the focus back to it; once more there, the browser's own find opens.
  // ⌘G and ⌘⇧G step to the next match and back, as browsers do.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = apple() ? e.metaKey : e.ctrlKey;
      const key = e.key.toLowerCase();
      if (mod && !e.altKey && key === "f" && !e.shiftKey) {
        if (open && document.activeElement === input.current) return;
        e.preventDefault();
        show();
      } else if (open && mod && !e.altKey && key === "g") {
        e.preventDefault();
        step(e.shiftKey ? "newer" : "older");
      } else if (open && e.key === "F3") {
        e.preventDefault();
        step(e.shiftKey ? "newer" : "older");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, show, step]);

  return {
    open,
    show,
    close,
    reveal,
    bar: {
      bar,
      input,
      query,
      onQuery: setQuery,
      position: at < 0 ? undefined : matches.length - at,
      total: matches.length,
      searched: !!n,
      more: hasMoreBefore,
      searching: loadingOlder && !!n,
      onOlder: () => step("older"),
      onNewer: () => step("newer"),
      onClose: close,
    },
  };
}

/** Whether a change on the page may change what finding lights up: something in a region, or one that came or went. */
function touchesRegion(r: MutationRecord): boolean {
  const target = r.target.nodeType === 1 ? (r.target as Element) : r.target.parentElement;
  if (target?.closest("[data-find]")) return true;
  const hasRegion = (n: Node) =>
    n.nodeType === 1 &&
    ((n as Element).matches("[data-find]") || !!(n as Element).querySelector("[data-find]"));
  return [...r.addedNodes].some(hasRegion) || [...r.removedNodes].some(hasRegion);
}

/**
 * Brings a match into view: in what scrolls inside the page (a long
 * command, a prompt), then on the page, in the middle of what covers its
 * top (the header, and the find bar under it) and its footer leave of the
 * view, unless it is well in view already. Far, the page jumps there, as a
 * browser's find does; near, it glides.
 */
function bringIntoView(to: Range | Element, header: HTMLElement | null, footer: HTMLElement | null) {
  const start = to instanceof Range ? to.startContainer : to;
  for (
    let box = start.nodeType === 1 ? (start as Element) : start.parentElement;
    box && box !== document.body && box !== document.documentElement;
    box = box.parentElement
  ) {
    const style = getComputedStyle(box);
    const x = /auto|scroll/.test(style.overflowX) && box.scrollWidth > box.clientWidth;
    const y = /auto|scroll/.test(style.overflowY) && box.scrollHeight > box.clientHeight;
    if (!x && !y) continue;
    const r = to.getBoundingClientRect();
    const b = box.getBoundingClientRect();
    if (x && (r.left < b.left || r.right > b.right))
      box.scrollLeft += r.left - b.left - (b.width - r.width) / 2;
    if (y && (r.top < b.top || r.bottom > b.bottom))
      box.scrollTop += r.top - b.top - (b.height - r.height) / 2;
  }
  const top = header?.getBoundingClientRect().bottom ?? 0;
  const bottom = footer?.getBoundingClientRect().top ?? window.innerHeight;
  const r = to.getBoundingClientRect();
  const margin = 32;
  if (r.top >= top + margin && r.bottom <= bottom - margin) return;
  const middle = top + (bottom - top) / 2;
  const by = r.top + r.height / 2 - middle;
  const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  window.scrollTo({
    top: window.scrollY + by,
    behavior: reduce || Math.abs(by) > (bottom - top) * 1.5 ? "instant" : "smooth",
  });
}

/** The bar: what to find, where the reader is among the matches, and steps to the others. */
export function FindBar({
  bar,
  input,
  query,
  onQuery,
  position,
  total,
  searched,
  more,
  searching,
  onOlder,
  onNewer,
  onClose,
}: ReturnType<typeof useFind>["bar"]) {
  const count = !searched
    ? ""
    : total === 0
      ? searching
        ? "Looking further back…"
        : more
          ? "None so far"
          : "No matches"
      : `${position ?? 0} of ${total}${more ? "+" : ""}`;
  return (
    <search
      ref={bar}
      aria-label="Find in this session"
      className="flex animate-fade items-center gap-1 md:gap-1.5"
    >
      <label
        className={cn(
          "flex h-9 min-w-0 flex-1 items-center gap-2 rounded-lg border border-line-strong bg-sunken/60 px-2.5 transition-[border-color,box-shadow,background-color] duration-150",
          "focus-within:border-accent/70 focus-within:bg-sunken focus-within:shadow-[0_0_0_3px_var(--c-accent-soft)]",
        )}
      >
        <Search aria-hidden="true" className="size-4 shrink-0 text-ink-4" />
        <input
          ref={input}
          type="search"
          value={query}
          onChange={(e) => onQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            if (e.key === "Escape") {
              e.preventDefault();
              onClose();
            } else if (e.key === "Enter" || e.key === "ArrowUp" || e.key === "ArrowDown") {
              e.preventDefault();
              if (e.key === "ArrowDown" || (e.key === "Enter" && e.shiftKey)) onNewer();
              else onOlder();
            }
          }}
          placeholder="Find in this session"
          aria-label="Find in this session"
          aria-describedby="find-count"
          enterKeyHint="search"
          autoComplete="off"
          autoCorrect="off"
          spellCheck={false}
          className="h-full min-w-0 flex-1 bg-transparent text-[14px] text-ink outline-none placeholder:text-ink-4 [&::-webkit-search-cancel-button]:hidden"
        />
        <span
          id="find-count"
          aria-live="polite"
          className={cn(
            "flex shrink-0 items-center gap-1.5 text-[12px] whitespace-nowrap tabular-nums",
            total ? "text-ink-3" : "text-ink-4",
          )}
        >
          {searching && <Spinner className="size-3 text-ink-4" />}
          {count}
        </span>
      </label>
      <IconButton
        label={!total && more ? "Look further back" : "Earlier match (Enter)"}
        onClick={onOlder}
        disabled={!searched || (!total && !more)}
        className="max-md:size-11"
      >
        <ChevronUp className="size-4.5" />
      </IconButton>
      <IconButton
        label="Later match (Shift+Enter)"
        onClick={onNewer}
        disabled={!total}
        className="max-md:size-11"
      >
        <ChevronDown className="size-4.5" />
      </IconButton>
      <IconButton label="Close (Esc)" onClick={onClose} className="max-md:size-11">
        <X className="size-4" />
      </IconButton>
    </search>
  );
}
