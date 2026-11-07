// Finding in the diff dialog's diffs (⌘F there): a bar over the diffs that
// finds what was typed in every changed file's lines, from where the reader
// is, steps from match to match (Enter, Shift+Enter, ⌘G), and lights the
// matches up as they show (lib/diff-find.ts says how it finds).

import { CaseSensitive, ChevronDown, ChevronUp, Regex, Search, WholeWord, X } from "lucide-react";
import {
  type RefObject,
  useCallback,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { IconButton, Spinner } from "@/components/ui";
import { cn } from "@/lib/cn";
import type { PatchLine } from "@/lib/diff";
import {
  type DiffMatch,
  type DiffPlace,
  type FindOptions,
  findIn,
  matchKey,
  nearest,
  pattern,
} from "@/lib/diff-find";
import { apple } from "@/lib/hotkeys";

/** Highlights for the matches, and the one the bar is at (styles/app.css): not the transcript's, which the page behind keeps. */
const HIGHLIGHT = "fctl-diff-find";
const HIGHLIGHT_CURRENT = "fctl-diff-find-current";

function registry(): HighlightRegistry | undefined {
  return typeof CSS !== "undefined" && "highlights" in CSS && typeof Highlight === "function"
    ? CSS.highlights
    : undefined;
}

/** The toggles, each with its key (Alt and the letter, as in code editors) and the option it turns. */
export const TOGGLES = [
  { option: "matchCase", code: "KeyC", label: "Match case", Icon: CaseSensitive },
  { option: "wholeWord", code: "KeyW", label: "Match whole word", Icon: WholeWord },
  { option: "regex", code: "KeyR", label: "Use regular expression", Icon: Regex },
] as const;

/** Alt (⌥ on Apple's systems) and a letter, as shortcuts say it. */
export const altKey = (letter: string) => (apple() ? `⌥${letter}` : `Alt+${letter}`);

/** The text of a line's code (`.diff-code`), and where each of its text nodes starts in it. */
function codeText(code: Element): { node: Text; at: number }[] {
  const nodes: { node: Text; at: number }[] = [];
  const walk = code.ownerDocument.createTreeWalker(code, NodeFilter.SHOW_TEXT);
  let at = 0;
  for (let n = walk.nextNode() as Text | null; n; n = walk.nextNode() as Text | null) {
    nodes.push({ node: n, at });
    at += n.data.length;
  }
  return nodes;
}

/** The range of `[start, end)` of a line's code. */
function rangeIn(code: Element, nodes: { node: Text; at: number }[], start: number, end: number) {
  const point = (to: number, isEnd: boolean): [Text, number] | undefined => {
    for (const { node, at } of nodes) {
      const len = node.data.length;
      if (to < at + len || (isEnd && to === at + len)) return [node, to - at];
    }
    const last = nodes[nodes.length - 1];
    return last ? [last.node, last.node.data.length] : undefined;
  };
  const a = point(start, false);
  const b = point(end, true);
  if (!a || !b) return undefined;
  const r = code.ownerDocument.createRange();
  try {
    r.setStart(a[0], a[1]);
    r.setEnd(b[0], b[1]);
  } catch {
    return undefined;
  }
  return r.collapsed ? undefined : r;
}

/** A file's section of the dialog, by its place in the list. */
export const sectionOf = (root: ParentNode, file: number) =>
  root.querySelector<HTMLElement>(`section[data-file="${file}"]`);

/** Where a match shows: in each side its line is on (a line kept shows on both side by side). */
export function matchRanges(root: ParentNode, m: DiffMatch): Range[] {
  const section = sectionOf(root, m.file);
  if (!section) return [];
  const out: Range[] = [];
  for (const line of section.querySelectorAll(`[data-l="${m.h}:${m.j}"]`)) {
    const code = line.querySelector(".diff-code");
    const r = code && rangeIn(code, codeText(code), m.start, m.end);
    if (r) out.push(r);
  }
  return out;
}

/**
 * Finding in the dialog's diffs. `lines`: each file's, as the dialog lists
 * them (none while its diff hasn't come); `body`: what scrolls them;
 * `placeAtTop`: the line at the top of the view; `inView`: whether a line
 * shows; `goTo`: brings a match into view, opening what folds it.
 */
export function useDiffFind({
  lines,
  body,
  options,
  placeAtTop,
  inView,
  goTo,
}: {
  lines: readonly (readonly PatchLine[] | undefined)[];
  body: RefObject<HTMLElement | null>;
  options: FindOptions;
  placeAtTop: () => DiffPlace | undefined;
  inView: (at: DiffPlace) => boolean;
  goTo: (m: DiffMatch) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const typed = useDeferredValue(query);
  const [current, setCurrent] = useState<string>();
  const input = useRef<HTMLInputElement>(null);

  const re = useMemo(() => (open ? pattern(typed, options) : undefined), [open, typed, options]);
  const { matches, capped } = useMemo(
    () => (re instanceof RegExp ? findIn(lines, re) : { matches: [] as DiffMatch[], capped: false }),
    [re, lines],
  );
  const index = useMemo(() => new Map(matches.map((m, i) => [matchKey(m), i])), [matches]);
  const at = current === undefined ? -1 : (index.get(current) ?? -1);
  const match = matches[at];
  // The place of the match the bar was at, for finding again from there.
  const last = useRef<DiffPlace | undefined>(undefined);
  if (match) last.current = match;
  // The match the page goes to, each time the reader steps or types (not when diffs come):
  // a new ask each time, gone to once, even when it is the match the bar is at already.
  const [goal, setGoal] = useState<{ key: string; ask: number }>();
  const asks = useRef(0);
  const goToKey = useCallback((key: string) => {
    setCurrent(key);
    setGoal({ key, ask: ++asks.current });
  }, []);
  // The reader scrolled since the page went to the match: finding goes on from where they are.
  const away = useRef(false);
  useEffect(() => {
    const b = body.current;
    if (!b) return;
    const leave = () => {
      away.current = true;
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.target === b && /^(Arrow|Page|Home$|End$| $)/.test(e.key)) leave();
    };
    b.addEventListener("wheel", leave, { passive: true });
    b.addEventListener("touchmove", leave, { passive: true });
    b.addEventListener("pointerdown", leave);
    b.addEventListener("keydown", onKey);
    return () => {
      b.removeEventListener("wheel", leave);
      b.removeEventListener("touchmove", leave);
      b.removeEventListener("pointerdown", leave);
      b.removeEventListener("keydown", onKey);
    };
  }, [body]);
  /** Whether the reader is at a match: the page went there and they stayed, or it shows. */
  const here = useCallback((at: DiffPlace) => !away.current || inView(at), [inView]);

  /** Matches by file, for the list to count. */
  const byFile = useMemo(() => {
    const out = new Map<number, number>();
    for (const m of matches) out.set(m.file, (out.get(m.file) ?? 0) + 1);
    return out;
  }, [matches]);

  // The field takes the keys, its text selected, as the bar renders after each ⌘F: before the
  // next key comes, so that none of what the reader types right after ⌘F goes elsewhere, and
  // once, so that what they type next isn't selected again.
  const [focusAsk, setFocusAsk] = useState(0);
  useLayoutEffect(() => {
    if (!focusAsk) return;
    input.current?.focus();
    input.current?.select();
  }, [focusAsk]);

  const show = useCallback(() => {
    // Words selected in a diff are what to find, as in an editor.
    const selected = window.getSelection()?.toString() ?? "";
    if (selected.trim() && !selected.includes("\n") && selected.length <= 200) setQuery(selected);
    if (!open) last.current = undefined;
    setOpen(true);
    setFocusAsk((n) => n + 1);
  }, [open]);

  const close = useCallback(() => {
    // What the bar was at stays selected, as a browser's find leaves it.
    const there = match && body.current ? matchRanges(body.current, match)[0] : undefined;
    setOpen(false);
    setCurrent(undefined);
    body.current?.focus({ preventScroll: true });
    if (there) {
      const sel = window.getSelection();
      sel?.removeAllRanges();
      sel?.addRange(there);
    }
  }, [match, body]);

  // The reader typed, or changed how to find: the match nearest them, from the one the bar was
  // at while it shows, else from the top of the view. Diffs that came or changed under the match
  // the bar was at: the nearest one, without moving the page.
  const searched = useRef<unknown>(undefined);
  useEffect(() => {
    if (!open) {
      searched.current = undefined;
      return;
    }
    const origin = { file: 0, h: 0, j: 0 };
    if (searched.current !== re) {
      searched.current = re;
      const prev = last.current;
      const i = nearest(matches, prev && here(prev) ? prev : (placeAtTop() ?? prev ?? origin));
      if (i < 0) setCurrent(undefined);
      else goToKey(matchKey(matches[i] as DiffMatch));
      return;
    }
    if (!matches.length || (current !== undefined && index.has(current))) return;
    const i = nearest(matches, last.current ?? placeAtTop() ?? origin);
    setCurrent(matchKey(matches[i] as DiffMatch));
  }, [open, re, matches, index, current, placeAtTop, here, goToKey]);

  /** To the next match, or back: from the one the bar is at while it shows, else from the top of the view. */
  const step = useCallback(
    (forward: boolean) => {
      if (!matches.length) return;
      let i: number;
      if (match && here(match)) {
        i = forward ? (at + 1) % matches.length : (at - 1 + matches.length) % matches.length;
      } else {
        const top = placeAtTop();
        i = top ? nearest(matches, top, forward) : forward ? 0 : matches.length - 1;
      }
      // The same match again (the only one) comes back into view all the same.
      goToKey(matchKey(matches[i] as DiffMatch));
    },
    [matches, match, at, here, placeAtTop, goToKey],
  );

  // Each ask, once its match is there.
  const gone = useRef(0);
  useLayoutEffect(() => {
    if (!goal || gone.current === goal.ask) return;
    const m = matches[index.get(goal.key) ?? -1];
    if (!m) return;
    gone.current = goal.ask;
    away.current = false;
    goTo(m);
  }, [goal, matches, index, goTo]);

  // The matches light up as they show, the one the bar is at most, as it changes.
  useEffect(() => {
    const root = body.current;
    const reg = registry();
    if (!root || !reg) return;
    if (!open || !matches.length) {
      reg.delete(HIGHLIGHT);
      reg.delete(HIGHLIGHT_CURRENT);
      return;
    }
    // By file, then line.
    const wanted = new Map<number, Map<string, DiffMatch[]>>();
    for (const m of matches) {
      let file = wanted.get(m.file);
      if (!file) {
        file = new Map();
        wanted.set(m.file, file);
      }
      const k = `${m.h}:${m.j}`;
      const list = file.get(k);
      if (list) list.push(m);
      else file.set(k, [m]);
    }
    const here = match && matchKey(match);
    let raf = 0;
    const paint = () => {
      raf = 0;
      const all: Range[] = [];
      const now: Range[] = [];
      for (const section of root.querySelectorAll<HTMLElement>("section[data-file]")) {
        const file = wanted.get(Number(section.dataset.file));
        if (!file) continue;
        for (const line of section.querySelectorAll<HTMLElement>("[data-l]")) {
          const list = file.get(line.dataset.l ?? "");
          const code = list && line.querySelector(".diff-code");
          if (!list || !code) continue;
          const nodes = codeText(code);
          for (const m of list) {
            const r = rangeIn(code, nodes, m.start, m.end);
            if (r) (matchKey(m) === here ? now : all).push(r);
          }
        }
      }
      reg.set(HIGHLIGHT, new Highlight(...all));
      if (now.length) reg.set(HIGHLIGHT_CURRENT, new Highlight(...now));
      else reg.delete(HIGHLIGHT_CURRENT);
    };
    paint();
    // Again as what shows changes: files that come into view, code that colors in, a layout.
    const watch = new MutationObserver(() => {
      if (!raf) raf = requestAnimationFrame(paint);
    });
    watch.observe(root, { subtree: true, childList: true, characterData: true });
    return () => {
      watch.disconnect();
      cancelAnimationFrame(raf);
    };
  }, [open, matches, match, body]);

  useEffect(
    () => () => {
      registry()?.delete(HIGHLIGHT);
      registry()?.delete(HIGHLIGHT_CURRENT);
    },
    [],
  );

  return {
    open,
    show,
    close,
    step,
    input,
    query,
    setQuery,
    /** What finding found where, for the list of files: none while it isn't finding. */
    byFile: open && matches.length ? byFile : undefined,
    finding: open && !!typed,
    position: at < 0 ? undefined : at + 1,
    total: matches.length,
    capped,
    error: re && !(re instanceof RegExp) ? re.error : undefined,
  };
}

/** The bar: what to find, how, where the reader is among the matches, and steps to the others. */
export function DiffFindBar({
  find,
  options,
  onToggle,
  loading,
}: {
  find: ReturnType<typeof useDiffFind>;
  options: FindOptions;
  onToggle: (option: keyof FindOptions) => void;
  /** Diffs are still coming: there may be more to find. */
  loading: boolean;
}) {
  const { input, query, setQuery, position, total, capped, error, finding } = find;
  const count = error
    ? error
    : !finding
      ? ""
      : total === 0
        ? loading
          ? "None so far"
          : "No matches"
        : `${position ?? 0} of ${total.toLocaleString()}${capped ? "+" : ""}`;
  const countId = "diff-find-count";
  return (
    <search
      aria-label="Find in the diffs"
      className="flex shrink-0 animate-fade items-center gap-1 border-b border-line bg-surface py-1.5 pr-1.5 pl-2.5"
    >
      <label
        className={cn(
          "flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md border bg-sunken/60 pr-1 pl-2.5 transition-[border-color,box-shadow,background-color] duration-150",
          error
            ? "border-danger/60"
            : "border-line-strong focus-within:border-accent/70 focus-within:bg-sunken focus-within:shadow-[0_0_0_3px_var(--c-accent-soft)]",
        )}
      >
        <Search aria-hidden="true" className="size-3.5 shrink-0 text-ink-4" />
        <input
          ref={input}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            if (e.key === "Escape") {
              e.preventDefault();
              e.stopPropagation();
              find.close();
            } else if (e.key === "Enter") {
              e.preventDefault();
              find.step(!e.shiftKey);
            }
          }}
          placeholder="Find in the diffs"
          aria-label="Find in the diffs"
          aria-describedby={countId}
          aria-invalid={!!error}
          enterKeyHint="search"
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          className="h-full min-w-0 flex-1 bg-transparent font-mono text-[12.5px] text-ink outline-none placeholder:font-sans placeholder:text-ink-4"
        />
        <span
          id={countId}
          aria-live="polite"
          className={cn(
            "flex max-w-[45%] shrink-0 items-center gap-1.5 truncate text-[11.5px] whitespace-nowrap tabular-nums",
            error ? "text-danger" : total ? "text-ink-3" : "text-ink-4",
          )}
        >
          {loading && finding && <Spinner className="size-3 text-ink-4" />}
          {count}
        </span>
        <span className="flex shrink-0 items-center gap-px">
          {TOGGLES.map(({ option, code, label, Icon }) => (
            <button
              key={option}
              type="button"
              aria-pressed={options[option]}
              aria-label={label}
              title={`${label} (${altKey(code.slice(3))})`}
              // The focus stays in the field, for typing on.
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => onToggle(option)}
              className={cn(
                "inline-flex size-6 items-center justify-center rounded-[5px] transition-colors focus-visible:outline-offset-0",
                options[option]
                  ? "bg-accent-soft text-ink shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
                  : "text-ink-4 hover:bg-hover hover:text-ink-2",
              )}
            >
              <Icon className="size-3.5" />
            </button>
          ))}
        </span>
      </label>
      <IconButton
        label="Previous match (Shift+Enter)"
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => find.step(false)}
        disabled={!total}
      >
        <ChevronUp className="size-4" />
      </IconButton>
      <IconButton
        label="Next match (Enter)"
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => find.step(true)}
        disabled={!total}
      >
        <ChevronDown className="size-4" />
      </IconButton>
      <IconButton label="Close the search (Esc)" onClick={find.close}>
        <X className="size-4" />
      </IconButton>
    </search>
  );
}
