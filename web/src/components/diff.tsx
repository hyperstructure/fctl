// A diff's lines, highlighted as the file's language is: numbered as the
// file has them when that is known, the words that changed marked, one
// below the other (unified) or before and after side by side (split).
// Shared by the session's changes (session/diff-dialog.tsx) and the
// transcript's edits (session/tool.tsx). The session's changes are live:
// as the agent changes the file again, the lines that stay keep their
// rows, and those it changed come in (components/diff-play.ts).

import { CircleSlash } from "lucide-react";
import {
  type CSSProperties,
  memo,
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { cn } from "@/lib/cn";
import type { DiffLine, Hunk } from "@/lib/diff";
import {
  firstLive,
  type How,
  type LiveDiff,
  type LiveHunk,
  type LiveLine,
  nextLive,
  settle,
  splitLive,
} from "@/lib/diff-motion";
import { type Token, tokenize } from "@/lib/highlight";
import { type LiveView, play, SETTLE_MS } from "./diff-play";

export type { LiveView };

/** Past this many lines a diff shows without highlighting, which would hold the page up. */
const MOST_HIGHLIGHTED = 6000;

/** How long a live diff's new version waits for its colors before it shows without them (ms). */
const WAIT_FOR_COLORS = 450;

/**
 * Each line's tokens once Shiki has them, by hunk and line: the lines before
 * (context and taken out) and after (context and added) of each hunk are
 * highlighted as pieces of code, so that what spans lines (a comment, a
 * string) shows as it does in the file. None until then, and for plain
 * text. Hunks made again with the same lines, as a transcript's edit's are
 * at each render, keep theirs. `pending`: they are on their way.
 */
export function useTokens(hunks: Hunk[], lang: string): { lines?: Token[][][]; pending: boolean } {
  const [tokens, setTokens] = useState<{ key: string; lines: Token[][][] }>();
  const key = useMemo(
    () =>
      `${lang}\u0000${hunks.map((h) => h.lines.map((l) => l.kind[0] + l.text).join("\n")).join("\u0001")}`,
    [hunks, lang],
  );
  const total = useMemo(() => hunks.reduce((n, h) => n + h.lines.length, 0), [hunks]);
  const highlights = lang !== "text" && total > 0 && total <= MOST_HIGHLIGHTED;
  const latest = useRef(hunks);
  latest.current = hunks;
  useEffect(() => {
    const hunks = latest.current;
    if (!highlights) return;
    let alive = true;
    (async () => {
      const found: Token[][][] = [];
      for (const h of hunks) {
        const before = h.lines.flatMap((l, i) => (l.kind !== "add" ? [i] : []));
        const after = h.lines.flatMap((l, i) => (l.kind !== "del" ? [i] : []));
        const text = (at: number[]) => at.map((i) => h.lines[i]?.text ?? "");
        const [old, now] = await Promise.all([tokenize(text(before), lang), tokenize(text(after), lang)]);
        if (!alive) return;
        // A language Shiki doesn't know: plain text, as it will stay.
        if (!old || !now) {
          setTokens({ key, lines: [] });
          return;
        }
        const lines: Token[][] = [];
        for (const [k, i] of before.entries()) lines[i] = old[k] ?? [];
        // Context lines take their colors from the file as it is now.
        for (const [k, i] of after.entries()) lines[i] = now[k] ?? [];
        found.push(lines);
        // Let the page breathe between hunks.
        await new Promise((r) => setTimeout(r));
      }
      if (alive) setTokens({ key, lines: found });
    })().catch(() => {
      if (alive) setTokens({ key, lines: [] });
    });
    return () => {
      alive = false;
    };
  }, [key, lang, highlights]);
  const lines = tokens?.key === key ? tokens.lines : undefined;
  return { lines: lines?.length ? lines : undefined, pending: highlights && !lines };
}

/** No hunks: what a diff whose lines come in shows before they do. */
const NONE: Hunk[] = [];

/**
 * The version a live diff shows: the newest once its colors came (or after
 * a moment without them), so that its new lines come in colored; until
 * then the one before, with its own (none, for lines that come in). Not
 * live: always the newest.
 */
function useShown(hunks: Hunk[], lang: string, live: boolean, entering: boolean) {
  const { lines, pending } = useTokens(hunks, lang);
  const [late, setLate] = useState<Hunk[]>();
  const before = useRef<{ hunks: Hunk[]; tokens?: Token[][][] } | undefined>(
    live && entering ? { hunks: NONE } : undefined,
  );
  const ready = !live || !pending || late === hunks || !before.current;
  const shown = ready
    ? { hunks, tokens: lines }
    : (before.current as { hunks: Hunk[]; tokens?: Token[][][] });
  useLayoutEffect(() => {
    before.current = shown;
  });
  useEffect(() => {
    if (ready) return;
    const t = setTimeout(() => setLate(hunks), WAIT_FOR_COLORS);
    return () => clearTimeout(t);
  }, [ready, hunks]);
  return shown;
}

/**
 * The rows a diff shows. Live, each version's rows are matched to those of
 * the version before (lib/diff-motion.ts), which settle once they came and
 * went; `entering`, its first version comes in as new lines do. Not live,
 * each version shows as it is.
 */
function useView(hunks: Hunk[], live: boolean, entering: boolean): LiveDiff {
  const [view, setView] = useState<LiveDiff>(() => firstLive(live && entering ? NONE : hunks));
  const fixed = useMemo(() => (live ? undefined : firstLive(hunks)), [live, hunks]);
  let current = view;
  if (!fixed && view.hunks !== hunks) {
    current = nextLive(view, hunks);
    setView(current);
  }
  useEffect(() => {
    if (fixed || current.items.every((i) => i.how === "kept")) return;
    const t = setTimeout(() => setView((v) => (v === current ? settle(v) : v)), SETTLE_MS);
    return () => clearTimeout(t);
  }, [fixed, current]);
  return fixed ?? current;
}

/** A piece of a line: its text, its colors, and whether it changed. */
interface Piece {
  content: string;
  style?: Record<string, string>;
  changed: boolean;
}

/** A line's pieces: its tokens (or its text), cut where what changed starts and ends. */
function pieces(line: DiffLine, tokens: Token[] | undefined): Piece[] {
  const source: Token[] = tokens?.length ? tokens : [{ content: line.text }];
  const ranges = line.changed ?? [];
  if (!ranges.length) return source.map((t) => ({ content: t.content, style: t.style, changed: false }));
  const cuts = new Set<number>();
  for (const [a, b] of ranges) {
    cuts.add(a);
    cuts.add(b);
  }
  const inside = (at: number) => ranges.some(([a, b]) => at >= a && at < b);
  const out: Piece[] = [];
  let at = 0;
  for (const t of source) {
    let start = 0;
    for (let i = 1; i <= t.content.length; i++) {
      if (i === t.content.length || cuts.has(at + i)) {
        const content = t.content.slice(start, i);
        if (content) out.push({ content, style: t.style, changed: inside(at + start) });
        start = i;
      }
    }
    at += t.content.length;
  }
  return out;
}

/** A line's code: highlighted pieces, the words that changed marked. */
const Code = memo(function Code({ line, tokens }: { line: DiffLine; tokens?: Token[] }) {
  // Each run of what changed is one mark, whatever tokens it spans.
  const runs: { changed: boolean; pieces: Piece[] }[] = [];
  for (const p of pieces(line, tokens)) {
    const last = runs[runs.length - 1];
    if (last && last.changed === p.changed) last.pieces.push(p);
    else runs.push({ changed: p.changed, pieces: [p] });
  }
  const spans = (ps: Piece[]) =>
    ps.map((p, i) => (
      <span key={i} style={p.style as CSSProperties | undefined}>
        {p.content}
      </span>
    ));
  return (
    <>
      {runs.map((r, i) =>
        r.changed ? (
          <span key={i} className="diff-word">
            {spans(r.pieces)}
          </span>
        ) : (
          spans(r.pieces)
        ),
      )}
      {line.noNewline && (
        <span
          role="img"
          title="No newline at end of file"
          aria-label="No newline at end of file"
          className="ml-1.5 inline-flex translate-y-[2px] text-ink-4"
        >
          <CircleSlash className="size-3" />
        </span>
      )}
    </>
  );
});

const MARK: Record<DiffLine["kind"], string> = { add: "+", del: "−", context: " " };

/** How the lines go: below each other, or before and after side by side. */
export type DiffLayout = "unified" | "split";

/**
 * A diff's hunks. `numbers`: the file's line numbers, unknown for an edit's
 * strings. `wrap`: long lines wrap rather than scroll; side by side they
 * always wrap. `live`: the diff changes as it shows, and what shows it
 * keeps the reader's place (session/diff-dialog.tsx); `entering`: its
 * lines come in as the first ones.
 */
export function DiffLines({
  hunks,
  lang,
  layout = "unified",
  wrap = false,
  numbers = true,
  headers = true,
  className,
  live,
  entering = false,
}: {
  hunks: Hunk[];
  /** Shiki's id of the file's language (`langOf`). */
  lang: string;
  layout?: DiffLayout;
  wrap?: boolean;
  numbers?: boolean;
  /** Each hunk's `@@` line: the lines it covers, and what it is in. */
  headers?: boolean;
  className?: string;
  live?: LiveView;
  entering?: boolean;
}) {
  const shown = useShown(hunks, lang, !!live, entering);
  const view = useView(shown.hunks, !!live, entering);
  // Each line's tokens: those of the version it is in, and of the one before for a line that
  // goes. Read in the render after the one that showed them.
  const tokensOf = useRef(new Map<string, Token[]>());
  const tokens = useMemo(() => {
    const byId = new Map<string, Token[]>();
    for (const it of view.items) {
      if (it.type !== "line") continue;
      const t = it.how === "gone" ? tokensOf.current.get(it.id) : shown.tokens?.[it.h]?.[it.j];
      if (t) byId.set(it.id, t);
    }
    return byId;
  }, [view.items, shown.tokens]);
  useLayoutEffect(() => {
    tokensOf.current = tokens;
  }, [tokens]);
  // The gutters are as wide as the longest number.
  const digits = useMemo(() => {
    let most = 0;
    for (const h of view.hunks) most = Math.max(most, h.oldStart + h.oldLines, h.newStart + h.newLines);
    return Math.max(2, String(most).length);
  }, [view.hunks]);
  const split = layout === "split";
  const pairs = useMemo(() => (split ? splitLive(view.items) : undefined), [split, view.items]);
  // One hunk from the file's top, as a new or deleted file's: its `@@` line says nothing the numbers don't.
  const first = view.hunks[0];
  const wholeFile =
    view.hunks.length === 1 && !!first && first.oldStart <= 1 && first.newStart <= 1 && !first.section;
  const showHeaders = headers && !wholeFile;

  // The rows of each new version come in and go where the reader sees them.
  const box = useRef<HTMLDivElement>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: once for each version
  useLayoutEffect(() => {
    if (live && box.current && view.items.some((i) => i.how !== "kept")) play(box.current, live);
  }, [view.version]);

  const style = { "--diff-digits": `${digits}ch` } as CSSProperties;
  const header = (it: LiveHunk) =>
    showHeaders && <HunkHeader key={it.id} item={it} hidden={it.hidden} numbers={numbers} split={split} />;
  return (
    <div
      ref={box}
      className={cn(
        "diff font-mono text-[12.5px] leading-5",
        split || wrap ? "diff-wrap" : "diff-scroll",
        className,
      )}
      style={style}
    >
      <div className={cn(!split && !wrap && "w-max min-w-full")}>
        {pairs
          ? pairs.map((row) =>
              row.type === "hunk" ? (
                header(row)
              ) : (
                <div key={row.id} className="grid grid-cols-2" data-open={pairOpen(row.left, row.right)}>
                  <Side
                    item={row.left}
                    side="old"
                    tokens={row.left && tokens.get(row.left.id)}
                    numbers={numbers}
                  />
                  <Side
                    item={row.right}
                    side="new"
                    tokens={row.right && tokens.get(row.right.id)}
                    numbers={numbers}
                    divided
                  />
                </div>
              ),
            )
          : view.items.map((it) =>
              it.type === "hunk" ? (
                header(it)
              ) : (
                <Row key={it.id} item={it} tokens={tokens.get(it.id)} numbers={numbers} />
              ),
            )}
      </div>
    </div>
  );
}

/** Whether a row opens as it comes or closes as it goes: one that comes is new, one that goes is gone. */
function opens(how: How): "in" | "out" | undefined {
  return how === "entered" || how === "moved" ? "in" : how === "gone" ? "out" : undefined;
}

/** A row side by side opens or closes when each line in it does. */
function pairOpen(left?: LiveLine, right?: LiveLine) {
  const l = left && opens(left.how);
  const r = right && opens(right.how);
  return left && right ? (l === r ? l : undefined) : (l ?? r);
}

/**
 * How a line's code comes, for what plays it: the agent's changes, not the
 * unchanged lines around them. Code it added is written in; a line it took
 * out, which showed or not, turns red.
 */
function written(item: LiveLine): string | undefined {
  const { how, line } = item;
  if (how === "entered") return line.kind === "add" ? "entered" : line.kind === "del" ? "marked" : undefined;
  if (how === "rewritten" || how === "marked" || how === "unmarked") return how;
  return undefined;
}

/** A hunk's `@@` line, which says where its lines are, how many lie hidden before it, and what they are in. */
function HunkHeader({
  item,
  hidden,
  numbers,
  split,
}: {
  item: LiveHunk;
  hidden: number;
  numbers: boolean;
  split: boolean;
}) {
  const { hunk } = item;
  const range = (start: number, count: number) => (count === 1 ? `${start}` : `${start},${count}`);
  const gone = item.how === "gone";
  return (
    <div
      className="diff-hunk flex min-h-7 items-center"
      data-hunk={gone ? undefined : ""}
      data-open={opens(item.how)}
      aria-hidden={gone || undefined}
    >
      {numbers && !split && (
        <span className="diff-gutter diff-gutter-two flex shrink-0 items-center justify-center select-none">
          <span className="text-[11px] tracking-[0.2em]" aria-hidden="true">
            ⋯
          </span>
        </span>
      )}
      <span className="flex min-w-0 items-baseline gap-2.5 px-3 py-1 text-[11.5px]">
        <span className="shrink-0 text-ink-4">
          @@ −{range(hunk.oldStart, hunk.oldLines)} +{range(hunk.newStart, hunk.newLines)} @@
        </span>
        {hunk.section && <span className="truncate text-ink-3">{hunk.section}</span>}
        {hidden > 0 && (
          <span className="sr-only">
            {hidden} unchanged {hidden === 1 ? "line" : "lines"} before
          </span>
        )}
      </span>
    </div>
  );
}

/** A line's code, in a box that a line written over shows what it said before in, as it changes. */
function CodeBox({ item, tokens, pad }: { item: LiveLine; tokens?: Token[]; pad: string }) {
  return (
    <span className={cn("diff-code min-w-0 flex-1", pad)} data-was={item.was?.text}>
      <span className="diff-text">
        <Code line={item.line} tokens={tokens} />
      </span>
    </span>
  );
}

/** A line below the others: its numbers before and after, its mark, its code. */
function Row({ item, tokens, numbers }: { item: LiveLine; tokens?: Token[]; numbers: boolean }) {
  const { line } = item;
  const gone = item.how === "gone";
  return (
    <div
      className="diff-line flex"
      data-kind={line.kind}
      // Its hunk and its place in it, `h:j`, which finding in the diff goes by.
      data-l={gone ? undefined : `${item.h}:${item.j}`}
      data-open={opens(item.how)}
      data-how={written(item)}
      aria-hidden={gone || undefined}
    >
      {numbers && (
        <span className="diff-gutter diff-gutter-two flex shrink-0 select-none">
          <LineNumber n={line.old} />
          <LineNumber n={line.new} />
        </span>
      )}
      <Mark kind={line.kind} />
      <CodeBox item={item} tokens={tokens} pad="pr-4" />
    </div>
  );
}

/** One side of a line side by side: before (`old`) or after (`new`), empty where that side has none. */
function Side({
  item,
  side,
  tokens,
  numbers,
  divided,
}: {
  item?: LiveLine;
  side: "old" | "new";
  tokens?: Token[];
  numbers: boolean;
  divided?: boolean;
}) {
  if (!item) return <div className={cn("diff-line diff-empty", divided && "diff-divided")} />;
  const { line } = item;
  const gone = item.how === "gone";
  return (
    <div
      className={cn("diff-line flex min-w-0", divided && "diff-divided")}
      data-kind={line.kind}
      data-l={gone ? undefined : `${item.h}:${item.j}`}
      data-how={written(item)}
      aria-hidden={gone || undefined}
    >
      {numbers && (
        <span className="diff-gutter flex shrink-0 select-none">
          <LineNumber n={side === "old" ? line.old : line.new} />
        </span>
      )}
      <Mark kind={line.kind} />
      <CodeBox item={item} tokens={tokens} pad="pr-3" />
    </div>
  );
}

function LineNumber({ n }: { n?: number }): ReactNode {
  return <span className="diff-number">{n ?? ""}</span>;
}

/** A line's + or −, which says to assistive technology what it is. */
function Mark({ kind }: { kind: DiffLine["kind"] }) {
  return kind === "context" ? (
    <span className="diff-mark w-5 shrink-0 select-none" aria-hidden="true" />
  ) : (
    <span
      role="img"
      aria-label={kind === "add" ? "Added" : "Removed"}
      className="diff-mark w-5 shrink-0 text-center select-none"
    >
      {MARK[kind]}
    </span>
  );
}
