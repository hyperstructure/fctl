// Diffs as the session page shows them: git's hunks (a changed file's,
// `GET /sessions/{id}/diff`), Claude Code's (an Edit's result), or what an
// edit's strings make; each line numbered, and the words that changed
// between a line taken out and the one that took its place marked.

import { diffLines, diffWordsWithSpace } from "diff";

export type LineKind = "context" | "add" | "del";

export interface DiffLine {
  kind: LineKind;
  /** Its number in the file before (context and taken out) and after (context and added), when known. */
  old?: number;
  new?: number;
  text: string;
  /** The file ends with this line, without a newline. */
  noNewline?: boolean;
  /** What changed in it, as [start, end) offsets into `text`, against the line it pairs with. */
  changed?: [number, number][];
}

export interface Hunk {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  /** What git says the hunk is in: the function or heading above it. */
  section?: string;
  lines: DiffLine[];
}

/** Lines added and taken out. */
export function counts(hunks: Hunk[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const h of hunks) {
    for (const l of h.lines) {
      if (l.kind === "add") added++;
      else if (l.kind === "del") removed++;
    }
  }
  return { added, removed };
}

const HEADER = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@ ?(.*)$/;

/** The hunks of a diff as git writes it, from its first `@@` line on. */
export function parsePatch(patch: string): Hunk[] {
  const hunks: Hunk[] = [];
  let hunk: Hunk | undefined;
  let oldN = 0;
  let newN = 0;
  for (const raw of patch.split("\n")) {
    const m = HEADER.exec(raw);
    if (m) {
      const [, os = "1", ol, ns = "1", nl, section] = m;
      hunk = {
        oldStart: Number(os),
        oldLines: ol === undefined ? 1 : Number(ol),
        newStart: Number(ns),
        newLines: nl === undefined ? 1 : Number(nl),
        section: section?.trim() || undefined,
        lines: [],
      };
      hunks.push(hunk);
      oldN = hunk.oldStart;
      newN = hunk.newStart;
      continue;
    }
    if (!hunk) continue;
    const mark = raw[0];
    // A CRLF file's lines end with a carriage return: not part of what shows.
    const text = raw.slice(1).replace(/\r$/, "");
    if (mark === " ") hunk.lines.push({ kind: "context", old: oldN++, new: newN++, text });
    else if (mark === "+") hunk.lines.push({ kind: "add", new: newN++, text });
    else if (mark === "-") hunk.lines.push({ kind: "del", old: oldN++, text });
    else if (mark === "\\") {
      const last = hunk.lines[hunk.lines.length - 1];
      if (last) last.noNewline = true;
    }
  }
  for (const h of hunks) markChanges(h.lines);
  return hunks;
}

/** A hunk of Claude Code's `structuredPatch` (an Edit's or a Write's result). */
interface StructuredHunk {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: string[];
}

/** The hunks of Claude Code's `structuredPatch`, numbered as the file is, when the result has one. */
export function fromStructured(structured: unknown): Hunk[] | undefined {
  const patch = (structured as { structuredPatch?: unknown } | null | undefined)?.structuredPatch;
  if (!Array.isArray(patch) || patch.length === 0) return undefined;
  const ok = (h: unknown): h is StructuredHunk => {
    const s = h as StructuredHunk;
    return (
      typeof s?.oldStart === "number" &&
      typeof s.newStart === "number" &&
      Array.isArray(s.lines) &&
      s.lines.every((l) => typeof l === "string")
    );
  };
  if (!patch.every(ok)) return undefined;
  const text = patch
    .map((h) => [`@@ -${h.oldStart},${h.oldLines} +${h.newStart},${h.newLines} @@`, ...h.lines].join("\n"))
    .join("\n");
  return parsePatch(text);
}

/**
 * What changed between two strings, as one hunk of all their lines: an
 * edit's `old_string` and `new_string`, whose lines in the file aren't known.
 */
export function fromStrings(before: string, after: string): Hunk[] {
  const lines: DiffLine[] = [];
  for (const part of diffLines(before, after)) {
    const kind: LineKind = part.added ? "add" : part.removed ? "del" : "context";
    for (const text of part.value.replace(/\n$/, "").split("\n")) lines.push({ kind, text });
  }
  if (!lines.length) return [];
  markChanges(lines);
  const n = (k: LineKind) => lines.filter((l) => l.kind === "context" || l.kind === k).length;
  return [{ oldStart: 1, oldLines: n("del"), newStart: 1, newLines: n("add"), lines }];
}

/** Lines longer than this aren't compared word by word. */
const LONGEST_COMPARED = 1000;

/**
 * Marks the words that changed between lines taken out and those added
 * right after them, pair by pair, when the pair has enough in common for
 * that to help: two lines that share nothing are just different.
 */
export function markChanges(lines: DiffLine[]) {
  let i = 0;
  while (i < lines.length) {
    if (lines[i]?.kind !== "del") {
      i++;
      continue;
    }
    const dels: DiffLine[] = [];
    while (lines[i]?.kind === "del") dels.push(lines[i++] as DiffLine);
    const adds: DiffLine[] = [];
    while (lines[i]?.kind === "add") adds.push(lines[i++] as DiffLine);
    for (let k = 0; k < Math.min(dels.length, adds.length); k++) {
      const del = dels[k] as DiffLine;
      const add = adds[k] as DiffLine;
      if (del.text.length > LONGEST_COMPARED || add.text.length > LONGEST_COMPARED) continue;
      const parts = diffWordsWithSpace(del.text, add.text);
      const kept = parts.filter((p) => !p.added && !p.removed).reduce((n, p) => n + p.value.trim().length, 0);
      const longest = Math.max(del.text.trim().length, add.text.trim().length);
      // Mostly different: marking nearly all of it says nothing.
      if (!longest || kept / longest < 0.4) continue;
      const ranges = (side: "added" | "removed") => {
        const out: [number, number][] = [];
        let at = 0;
        for (const p of parts) {
          const other = side === "added" ? p.removed : p.added;
          if (other) continue;
          const end = at + p.value.length;
          if (p[side] && p.value.trim()) {
            const last = out[out.length - 1];
            if (last && last[1] === at) last[1] = end;
            else out.push([at, end]);
          }
          at = end;
        }
        return out;
      };
      const removed = ranges("removed");
      const added = ranges("added");
      if (removed.length) del.changed = removed;
      if (added.length) add.changed = added;
    }
  }
}

/** A row of a side-by-side diff: the line before on the left, after on the right. */
export interface SplitRow {
  left?: DiffLine;
  right?: DiffLine;
}

/**
 * A hunk's lines side by side: context on both sides, and lines taken out
 * beside those added in their place.
 */
export function splitRows(lines: DiffLine[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i] as DiffLine;
    if (line.kind === "context") {
      rows.push({ left: line, right: line });
      i++;
      continue;
    }
    const dels: DiffLine[] = [];
    while (lines[i]?.kind === "del") dels.push(lines[i++] as DiffLine);
    const adds: DiffLine[] = [];
    while (lines[i]?.kind === "add") adds.push(lines[i++] as DiffLine);
    for (let k = 0; k < Math.max(dels.length, adds.length); k++) rows.push({ left: dels[k], right: adds[k] });
  }
  return rows;
}

/** How many unchanged lines lie between a hunk and the one before it (or the file's start). */
export function hiddenBefore(hunks: Hunk[], i: number): number {
  const h = hunks[i];
  if (!h) return 0;
  const prev = hunks[i - 1];
  const from = prev ? prev.newStart + prev.newLines : 1;
  return Math.max(0, h.newStart - from);
}

/** The first line of the file as it is now, when a hunk starts at its top. */
export function firstLine(hunks: Hunk[]): string | undefined {
  const h = hunks[0];
  if (!h || h.newStart > 1) return undefined;
  return h.lines.find((l) => l.kind !== "del")?.text;
}
