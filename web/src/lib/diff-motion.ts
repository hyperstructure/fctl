// A diff that changes while it shows, as a session's do while its agent
// works: each version's lines matched to those of the version before, so
// that a line that stays keeps its row (and its place on the screen), a
// new one opens in, and one that went closes. The file as it was before
// the session doesn't change, so a line of it (unchanged, or taken out) is
// the same line in both versions, by its number there. A line added is
// the same when it reads the same and sits by the same unchanged line, on
// one side at least. A line the agent added before and wrote over since is
// one row, rewritten.

import { diffArrays } from "diff";
import { align, type DiffLine, type Hunk, hiddenBefore } from "./diff";

/** How a row came to be as it shows, against the version before. */
export type How =
  /** As it was; its numbers may have moved. */
  | "kept"
  /** New: what the agent changed, or the unchanged lines around it. */
  | "entered"
  /** In the version before, not in this one: it closes, then goes. */
  | "gone"
  /** A line added before, written over. */
  | "rewritten"
  /** A line that was unchanged, taken out now. */
  | "marked"
  /** A line taken out before, unchanged again. */
  | "unmarked"
  /**
   * A line added before that the lines taken out beside it moved past, as git
   * writes those first: it opens where it is now as it closes where it was.
   */
  | "moved";

/** A hunk's `@@` line. */
export interface LiveHunk {
  type: "hunk";
  id: string;
  /** Its place among the hunks; -1 for one that goes. */
  h: number;
  hunk: Hunk;
  /** The unchanged lines hidden before it. */
  hidden: number;
  how: How;
}

/** A line, with the place in the hunks (`h:j`) that finding goes by; -1 for one that goes. */
export interface LiveLine {
  type: "line";
  id: string;
  h: number;
  j: number;
  line: DiffLine;
  how: How;
  /** What a line written over said before. */
  was?: DiffLine;
}

export type LiveItem = LiveHunk | LiveLine;

/** A version of a diff as it shows: its hunks, and its rows with how each came. */
export interface LiveDiff {
  hunks: Hunk[];
  items: LiveItem[];
  /** Counts the versions, for the ids of new rows. */
  version: number;
}

/** Past this many lines in the two versions, or edits between them, the new one shows anew. */
const MOST_MATCHED = 20_000;
const MOST_EDITS = 4_000;

/** A diff's first version: every row as it is. */
export function firstLive(hunks: Hunk[], version = 0): LiveDiff {
  const items: LiveItem[] = [];
  let n = 0;
  for (const [h, hunk] of hunks.entries()) {
    items.push({
      type: "hunk",
      id: `${version}.${n++}`,
      h,
      hunk,
      hidden: hiddenBefore(hunks, h),
      how: "kept",
    });
    for (const [j, line] of hunk.lines.entries()) {
      items.push({ type: "line", id: `${version}.${n++}`, h, j, line, how: "kept" });
    }
  }
  return { hunks, items, version };
}

/**
 * Where each line of a hunk sits in the file before: after which unchanged
 * line (`prev`) and before which (`next`), those just outside the hunk at
 * its ends.
 */
function bounds(hunk: Hunk): { prev: number; next: number }[] {
  const out: { prev: number; next: number }[] = [];
  let prev = hunk.oldStart - 1;
  for (const l of hunk.lines) {
    if (l.kind === "context" && l.old !== undefined) prev = l.old;
    out.push({ prev, next: 0 });
  }
  let next = hunk.oldStart + hunk.oldLines;
  for (let j = hunk.lines.length - 1; j >= 0; j--) {
    const l = hunk.lines[j] as DiffLine;
    if (l.kind === "context" && l.old !== undefined) next = l.old;
    (out[j] as { next: number }).next = next;
  }
  return out;
}

interface Key {
  line: DiffLine;
  prev: number;
  next: number;
}

/** Whether two lines of two versions are the same line. */
function same(a: Key, b: Key): boolean {
  if (a.line.kind === "add" || b.line.kind === "add") {
    return (
      a.line.kind === b.line.kind && a.line.text === b.line.text && (a.prev === b.prev || a.next === b.next)
    );
  }
  // Unchanged or taken out: the same line of the file before, whichever it is now.
  return a.line.old === b.line.old && a.line.text === b.line.text;
}

/** A line on its way into the next version, with the one before it came from. */
type Merged = LiveLine & { from?: number };

/**
 * The next version of a diff that shows: `hunks`, its rows matched to those
 * of `before` (rows still going from the version before go at once).
 */
export function nextLive(before: LiveDiff, hunks: Hunk[]): LiveDiff {
  const version = before.version + 1;
  let made = 0;
  const fresh = () => `${version}.${made++}`;

  const oldBounds = before.hunks.map(bounds);
  const was = before.items.filter((i): i is LiveLine => i.type === "line" && i.how !== "gone" && i.h >= 0);
  const a: Key[] = was.map((i) => ({ line: i.line, ...(oldBounds[i.h]?.[i.j] ?? { prev: -2, next: -2 }) }));
  const now = hunks.flatMap((hunk, h) => hunk.lines.map((line, j) => ({ h, j, line })));
  const newBounds = hunks.map(bounds);
  const b: Key[] = now.map(({ h, j, line }) => ({ line, ...(newBounds[h]?.[j] ?? { prev: -2, next: -2 }) }));
  const changes =
    a.length + b.length > MOST_MATCHED
      ? undefined
      : diffArrays(a, b, { comparator: same, maxEditLength: MOST_EDITS, timeout: 100 });
  // Too far apart to tell what became what: the new version shows as it is.
  if (!changes) return firstLive(hunks, version);

  const merged: Merged[] = [];
  let i = 0;
  let k = 0;
  for (const c of changes) {
    const n = c.count ?? c.value.length;
    for (let t = 0; t < n; t++) {
      if (c.removed) {
        const o = was[i] as LiveLine;
        merged.push({ ...o, h: -1, j: -1, how: "gone", was: undefined, from: i });
        i++;
      } else if (c.added) {
        merged.push({ type: "line", id: "", ...(now[k] as (typeof now)[number]), how: "entered" });
        k++;
      } else {
        const o = was[i] as LiveLine;
        const nw = now[k] as (typeof now)[number];
        const how: How =
          o.line.kind === nw.line.kind ? "kept" : nw.line.kind === "del" ? "marked" : "unmarked";
        merged.push({ type: "line", id: o.id, ...nw, how, from: i });
        i++;
        k++;
      }
    }
  }

  // Lines added before and gone now, beside lines added now in their place: written over.
  const dropped = new Set<Merged>();
  for (let s = 0; s < merged.length; ) {
    if ((merged[s] as Merged).how !== "gone" && (merged[s] as Merged).how !== "entered") {
      s++;
      continue;
    }
    let e = s;
    while (
      e < merged.length &&
      ((merged[e] as Merged).how === "gone" || (merged[e] as Merged).how === "entered")
    )
      e++;
    const run = merged.slice(s, e);
    const gone = run.filter((m) => m.how === "gone" && m.line.kind === "add");
    const came = run.filter((m) => m.how === "entered" && m.line.kind === "add");
    for (let p = 0; p < Math.min(gone.length, came.length); p++) {
      const g = gone[p] as Merged;
      const m = came[p] as Merged;
      m.id = g.id;
      m.from = g.from;
      if (g.line.text === m.line.text) m.how = "kept";
      else {
        m.how = "rewritten";
        m.was = g.line;
      }
      dropped.add(g);
    }
    s = e;
  }
  const lines = merged.filter((m) => !dropped.has(m));
  // A line added that only moved past lines taken out (or put back) beside it: the same line.
  for (const [p, g] of lines.entries()) {
    if (g.how !== "gone" || g.line.kind !== "add") continue;
    for (const dir of [1, -1]) {
      let q = p + dir;
      while (lines[q] && (lines[q] as Merged).line.kind === "del" && (lines[q] as Merged).how !== "gone")
        q += dir;
      const m = lines[q];
      if (m && m.how === "entered" && m.line.kind === "add" && m.line.text === g.line.text && q !== p + dir) {
        m.how = "moved";
        break;
      }
    }
  }
  for (const m of lines) if (!m.id) m.id = fresh();

  // Each hunk takes the `@@` line of the hunk its first line that stayed was in, unless a hunk
  // before it took that one; those of the hunks before that none took go.
  const oldHeaders = new Map<number, LiveHunk>();
  for (const it of before.items) {
    if (it.type === "hunk" && it.how !== "gone" && it.h >= 0) oldHeaders.set(it.h, it);
  }
  const oldHunk = (m: Merged) => (m.from === undefined ? undefined : was[m.from]?.h);
  const took = new Map<number, number>();
  const taken = new Set<number>();
  for (let h = 0; h < hunks.length; h++) {
    const first = lines.find((m) => m.h === h && m.from !== undefined);
    const o = first && oldHunk(first);
    if (o !== undefined && oldHeaders.has(o) && !taken.has(o)) {
      took.set(h, o);
      taken.add(o);
    }
  }
  const tookBy = new Map([...took].map(([h, o]) => [o, h]));
  // Where each hunk's lines end, so that a line that goes opens no hunk before the one above ends.
  const last = new Map<number, number>();
  for (const [p, m] of lines.entries()) if (m.h >= 0) last.set(m.h, p);

  const items: LiveItem[] = [];
  const headed = new Set<number>();
  const ended = new Set<number>();
  for (const [p, m] of lines.entries()) {
    const o = oldHunk(m);
    if (o !== undefined && !ended.has(o)) {
      ended.add(o);
      const header = oldHeaders.get(o);
      if (header && !taken.has(o)) items.push({ ...header, h: -1, how: "gone" });
    }
    // A line's own hunk opens with it; so does the hunk that took the `@@` line of a line that
    // goes, above the lines it had, unless the hunk before has lines further down.
    let opens: number | undefined = m.h >= 0 ? m.h : undefined;
    if (opens === undefined && o !== undefined) {
      const h = tookBy.get(o);
      if (h !== undefined && (h === 0 || (last.get(h - 1) ?? -1) < p)) opens = h;
    }
    if (opens !== undefined && !headed.has(opens)) {
      headed.add(opens);
      const o2 = took.get(opens);
      const header = o2 === undefined ? undefined : oldHeaders.get(o2);
      items.push({
        type: "hunk",
        id: header?.id ?? fresh(),
        h: opens,
        hunk: hunks[opens] as Hunk,
        hidden: hiddenBefore(hunks, opens),
        how: header ? "kept" : "entered",
      });
    }
    const { from: _, ...line } = m;
    items.push(line);
  }
  return { hunks, items, version };
}

/** The same version once its rows have come and gone: those that went left out, all of them as they are. */
export function settle(d: LiveDiff): LiveDiff {
  if (d.items.every((i) => i.how === "kept")) return d;
  return {
    ...d,
    items: d.items
      .filter((i) => i.how !== "gone")
      .map((i) => (i.type === "line" ? { ...i, how: "kept", was: undefined } : { ...i, how: "kept" })),
  };
}

/** A row side by side: the line before on the left, after on the right (the same line both sides when unchanged). */
export interface LivePair {
  type: "pair";
  id: string;
  left?: LiveLine;
  right?: LiveLine;
}

/**
 * The rows side by side: each unchanged line on both sides, the lines a
 * change took out beside those it added ([`align`]), and a line that goes
 * on its side of a row of its own.
 */
export function splitLive(items: LiveItem[]): (LiveHunk | LivePair)[] {
  const out: (LiveHunk | LivePair)[] = [];
  let i = 0;
  while (i < items.length) {
    const it = items[i] as LiveItem;
    if (it.type === "hunk") {
      out.push(it);
      i++;
      continue;
    }
    if (it.how === "gone" || it.line.kind === "context") {
      const kind = it.line.kind;
      out.push({
        type: "pair",
        id: it.id,
        left: kind !== "add" ? it : undefined,
        right: kind !== "del" ? it : undefined,
      });
      i++;
      continue;
    }
    // A change: its lines taken out beside those it added, those of it that go after them.
    const dels: LiveLine[] = [];
    const adds: LiveLine[] = [];
    const going: LiveLine[] = [];
    for (; i < items.length; i++) {
      const x = items[i] as LiveItem;
      if (x.type !== "line") break;
      if (x.how === "gone") going.push(x);
      else if (x.line.kind === "del" && !adds.length) dels.push(x);
      else if (x.line.kind === "add") adds.push(x);
      else break;
    }
    const of = new Map<DiffLine, LiveLine>([...dels, ...adds].map((x) => [x.line, x]));
    for (const row of align(
      dels.map((d) => d.line),
      adds.map((d) => d.line),
    )) {
      const left = row.left && of.get(row.left);
      const right = row.right && of.get(row.right);
      out.push({ type: "pair", id: (left ?? right)?.id ?? "", left, right });
    }
    for (const g of going) {
      out.push({
        type: "pair",
        id: g.id,
        left: g.line.kind !== "add" ? g : undefined,
        right: g.line.kind !== "del" ? g : undefined,
      });
    }
  }
  return out;
}
