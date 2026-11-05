// Finding in a session's diffs, as the diff dialog does (⌘F there): what
// was typed, as code editors find it (literally, regardless of case unless
// asked, as a whole word or a regular expression when asked), in the lines
// of every changed file, in the order the dialog shows them.

import type { PatchLine } from "./diff";

/** How to find: as code editors' toggles say. */
export interface FindOptions {
  /** Upper and lower case are different letters. */
  matchCase: boolean;
  /** Only where it isn't part of a longer word. */
  wholeWord: boolean;
  /** What was typed is a regular expression. */
  regex: boolean;
}

/** An occurrence: in the `file`th file, on its line `h:j` ([`PatchLine`]), from `start` to `end` of the line's text. */
export interface DiffMatch {
  file: number;
  h: number;
  j: number;
  start: number;
  end: number;
}

/** A place in the diffs: a file and a line of it. */
export interface DiffPlace {
  file: number;
  h: number;
  j: number;
}

/** Past this many matches, finding stops counting. */
export const MOST_MATCHES = 10_000;

const literal = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * The expression that finds `query`; none for an empty one, and what is
 * wrong with a regular expression that isn't one.
 */
export function pattern(query: string, o: FindOptions): RegExp | { error: string } | undefined {
  if (!query) return undefined;
  let source = o.regex ? query : literal(query);
  // Letters of any script make words; a typed expression keeps the meaning it has without
  // the `u` flag that needs, as code editors give it.
  const unicode = o.wholeWord && !o.regex;
  if (o.wholeWord) {
    const word = unicode ? "[\\p{L}\\p{N}_]" : "\\w";
    source = `(?<!${word})(?:${source})(?!${word})`;
  }
  try {
    return new RegExp(source, `g${o.matchCase ? "" : "i"}${unicode ? "u" : ""}`);
  } catch (e) {
    const message = e instanceof Error ? e.message : "";
    // "Invalid regular expression: /(/gi: Unterminated group": what is wrong, alone.
    return { error: message.split(": ").pop() || "Not a regular expression" };
  }
}

/** Where `re` finds something in `text`, empty matches left out. */
export function occurrences(re: RegExp, text: string): [number, number][] {
  const out: [number, number][] = [];
  re.lastIndex = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m[0].length) out.push([m.index, m.index + m[0].length]);
    // An empty match (`^`, `a*`) would find itself forever.
    else re.lastIndex++;
    if (re.lastIndex > text.length) break;
  }
  return out;
}

/**
 * Every match of `re` in the files' lines, file after file, line after line,
 * up to `most`: `capped` says it stopped there.
 */
export function findIn(
  files: readonly (readonly PatchLine[] | undefined)[],
  re: RegExp,
  most = MOST_MATCHES,
): { matches: DiffMatch[]; capped: boolean } {
  const matches: DiffMatch[] = [];
  for (const [file, lines] of files.entries()) {
    for (const l of lines ?? []) {
      for (const [start, end] of occurrences(re, l.text)) {
        if (matches.length >= most) return { matches, capped: true };
        matches.push({ file, h: l.h, j: l.j, start, end });
      }
    }
  }
  return { matches, capped: false };
}

/** Which of two places comes first in the dialog: negative for `a`, positive for `b`. */
export const compare = (a: DiffPlace, b: DiffPlace) => a.file - b.file || a.h - b.h || a.j - b.j;

/**
 * The match to go to from `at`: the first at or after it (`forward`), or
 * the last before it, around to the other end when there is none that way.
 */
export function nearest(matches: readonly DiffMatch[], at: DiffPlace, forward = true): number {
  if (!matches.length) return -1;
  // The first match not before `at`.
  let lo = 0;
  let hi = matches.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (compare(matches[mid] as DiffMatch, at) < 0) lo = mid + 1;
    else hi = mid;
  }
  if (forward) return lo < matches.length ? lo : 0;
  return lo > 0 ? lo - 1 : matches.length - 1;
}

/** A match's key, which stays the same while the file's diff does. */
export const matchKey = (m: DiffMatch) => `${m.file}:${m.h}:${m.j}:${m.start}`;
