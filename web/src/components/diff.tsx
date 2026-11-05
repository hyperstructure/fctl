// A diff's lines, highlighted as the file's language is: numbered as the
// file has them when that is known, the words that changed marked, one
// below the other (unified) or before and after side by side (split).
// Shared by the session's changes (session/diff-dialog.tsx) and the
// transcript's edits (session/tool.tsx).

import { CircleSlash } from "lucide-react";
import {
  type CSSProperties,
  Fragment,
  memo,
  type ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { cn } from "@/lib/cn";
import { type DiffLine, type Hunk, hiddenBefore, splitRows } from "@/lib/diff";
import { type Token, tokenize } from "@/lib/highlight";

/** Past this many lines a diff shows without highlighting, which would hold the page up. */
const MOST_HIGHLIGHTED = 6000;

/**
 * Each line's tokens once Shiki has them, by hunk and line: the lines before
 * (context and taken out) and after (context and added) of each hunk are
 * highlighted as pieces of code, so that what spans lines (a comment, a
 * string) shows as it does in the file. None until then, and for plain
 * text. Hunks made again with the same lines, as a transcript's edit's are
 * at each render, keep theirs.
 */
export function useTokens(hunks: Hunk[], lang: string): Token[][][] | undefined {
  const [tokens, setTokens] = useState<{ key: string; lines: Token[][][] }>();
  const key = useMemo(
    () =>
      `${lang}\u0000${hunks.map((h) => h.lines.map((l) => l.kind[0] + l.text).join("\n")).join("\u0001")}`,
    [hunks, lang],
  );
  const latest = useRef(hunks);
  latest.current = hunks;
  useEffect(() => {
    const hunks = latest.current;
    const total = hunks.reduce((n, h) => n + h.lines.length, 0);
    if (lang === "text" || !total || total > MOST_HIGHLIGHTED) return;
    let alive = true;
    (async () => {
      const found: Token[][][] = [];
      for (const h of hunks) {
        const before = h.lines.flatMap((l, i) => (l.kind !== "add" ? [i] : []));
        const after = h.lines.flatMap((l, i) => (l.kind !== "del" ? [i] : []));
        const text = (at: number[]) => at.map((i) => h.lines[i]?.text ?? "");
        const [old, now] = await Promise.all([tokenize(text(before), lang), tokenize(text(after), lang)]);
        if (!alive || !old || !now) return;
        const lines: Token[][] = [];
        for (const [k, i] of before.entries()) lines[i] = old[k] ?? [];
        // Context lines take their colors from the file as it is now.
        for (const [k, i] of after.entries()) lines[i] = now[k] ?? [];
        found.push(lines);
        // Let the page breathe between hunks.
        await new Promise((r) => setTimeout(r));
      }
      if (alive) setTokens({ key, lines: found });
    })().catch(() => {});
    return () => {
      alive = false;
    };
  }, [key, lang]);
  return tokens?.key === key ? tokens.lines : undefined;
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
 * always wrap.
 */
export function DiffLines({
  hunks,
  lang,
  layout = "unified",
  wrap = false,
  numbers = true,
  headers = true,
  className,
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
}) {
  const highlighted = useTokens(hunks, lang);
  // Each line's tokens, for the rows side by side as below each other.
  const tokens = useMemo(() => {
    if (!highlighted) return undefined;
    const byLine = new Map<DiffLine, Token[]>();
    for (const [i, h] of hunks.entries()) {
      for (const [j, l] of h.lines.entries()) {
        const t = highlighted[i]?.[j];
        if (t) byLine.set(l, t);
      }
    }
    return byLine;
  }, [highlighted, hunks]);
  // The gutters are as wide as the longest number.
  const digits = useMemo(() => {
    let most = 0;
    for (const h of hunks) most = Math.max(most, h.oldStart + h.oldLines, h.newStart + h.newLines);
    return Math.max(2, String(most).length);
  }, [hunks]);
  const split = layout === "split";
  // Side by side, each hunk's rows, with where their lines are in it (`h:j`).
  const sides = useMemo(
    () =>
      split
        ? hunks.map((h, i) => {
            const at = new Map(h.lines.map((l, j) => [l, `${i}:${j}`]));
            return splitRows(h.lines).map((r) => ({
              ...r,
              leftAt: r.left && at.get(r.left),
              rightAt: r.right && at.get(r.right),
            }));
          })
        : undefined,
    [hunks, split],
  );
  // One hunk from the file's top, as a new or deleted file's: its `@@` line says nothing the numbers don't.
  const first = hunks[0];
  const wholeFile =
    hunks.length === 1 && !!first && first.oldStart <= 1 && first.newStart <= 1 && !first.section;
  const style = { "--diff-digits": `${digits}ch` } as CSSProperties;
  return (
    <div
      className={cn(
        "diff font-mono text-[12.5px] leading-5",
        split || wrap ? "diff-wrap" : "diff-scroll",
        className,
      )}
      style={style}
    >
      <div className={cn(!split && !wrap && "w-max min-w-full")}>
        {hunks.map((h, i) => (
          <Fragment key={`${h.oldStart}:${h.newStart}:${i}`}>
            {headers && !wholeFile && (
              <HunkHeader hunk={h} hidden={hiddenBefore(hunks, i)} numbers={numbers} split={split} />
            )}
            {sides
              ? sides[i]?.map((row, j) => (
                  <div key={j} className="grid grid-cols-2">
                    <Side
                      line={row.left}
                      at={row.leftAt}
                      side="old"
                      tokens={row.left && tokens?.get(row.left)}
                      numbers={numbers}
                    />
                    <Side
                      line={row.right}
                      at={row.rightAt}
                      side="new"
                      tokens={row.right && tokens?.get(row.right)}
                      numbers={numbers}
                      divided
                    />
                  </div>
                ))
              : h.lines.map((l, j) => (
                  <Row key={j} line={l} at={`${i}:${j}`} tokens={tokens?.get(l)} numbers={numbers} />
                ))}
          </Fragment>
        ))}
      </div>
    </div>
  );
}

/** A hunk's `@@` line, which says where its lines are, how many lie hidden before it, and what they are in. */
function HunkHeader({
  hunk,
  hidden,
  numbers,
  split,
}: {
  hunk: Hunk;
  hidden: number;
  numbers: boolean;
  split: boolean;
}) {
  const range = (start: number, count: number) => (count === 1 ? `${start}` : `${start},${count}`);
  return (
    <div className="diff-hunk flex min-h-7 items-center" data-hunk="">
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

/** A line below the others: its numbers before and after, its mark, its code. */
function Row({
  line,
  at,
  tokens,
  numbers,
}: {
  line: DiffLine;
  /** Its hunk and its place in it, `h:j`, which finding in the diff goes by (`data-l`). */
  at: string;
  tokens?: Token[];
  numbers: boolean;
}) {
  return (
    <div className="diff-line flex" data-kind={line.kind} data-l={at}>
      {numbers && (
        <span className="diff-gutter diff-gutter-two flex shrink-0 select-none">
          <LineNumber n={line.old} />
          <LineNumber n={line.new} />
        </span>
      )}
      <Mark kind={line.kind} />
      <span className="diff-code min-w-0 flex-1 pr-4">
        <Code line={line} tokens={tokens} />
      </span>
    </div>
  );
}

/** One side of a line side by side: before (`old`) or after (`new`), empty where that side has none. */
function Side({
  line,
  at,
  side,
  tokens,
  numbers,
  divided,
}: {
  line?: DiffLine;
  at?: string;
  side: "old" | "new";
  tokens?: Token[];
  numbers: boolean;
  divided?: boolean;
}) {
  if (!line) return <div className={cn("diff-line diff-empty", divided && "diff-divided")} />;
  return (
    <div
      className={cn("diff-line flex min-w-0", divided && "diff-divided")}
      data-kind={line.kind}
      data-l={at}
    >
      {numbers && (
        <span className="diff-gutter flex shrink-0 select-none">
          <LineNumber n={side === "old" ? line.old : line.new} />
        </span>
      )}
      <Mark kind={line.kind} />
      <span className="diff-code min-w-0 flex-1 pr-3">
        <Code line={line} tokens={tokens} />
      </span>
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
