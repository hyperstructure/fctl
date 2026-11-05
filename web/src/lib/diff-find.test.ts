import { describe, expect, it } from "vitest";
import { changeStarts, parsePatch, patchLines } from "./diff";
import { type DiffMatch, findIn, matchKey, nearest, occurrences, pattern } from "./diff-find";

const plain = { matchCase: false, wholeWord: false, regex: false };

const find = (query: string, text: string, o = plain) => {
  const re = pattern(query, o);
  if (!(re instanceof RegExp)) throw new Error("no expression");
  return occurrences(re, text).map(([a, b]) => text.slice(a, b));
};

describe("finding in diffs", () => {
  it("finds what was typed literally, regardless of case", () => {
    expect(find("foo(", "Foo(a) + foo(b)")).toEqual(["Foo(", "foo("]);
    expect(find("a.b", "axb a.b")).toEqual(["a.b"]);
    expect(find("  ", "x    y")).toEqual(["  ", "  "]);
  });

  it("tells case apart, finds whole words and reads regular expressions when asked", () => {
    expect(find("foo", "Foo foo", { ...plain, matchCase: true })).toEqual(["foo"]);
    expect(find("id", "id ids _id uid id_x (id)", { ...plain, wholeWord: true })).toEqual(["id", "id"]);
    expect(find("café", "café cafés", { ...plain, wholeWord: true })).toEqual(["café"]);
    expect(find("fn \\w+", "pub fn main() { fn a", { ...plain, regex: true })).toEqual(["fn main", "fn a"]);
    expect(find("\\d+", "a1 b22", { ...plain, regex: true, wholeWord: true })).toEqual([]);
    // Empty matches are none, and don't loop.
    expect(find("x*", "axxb", { ...plain, regex: true })).toEqual(["xx"]);
    expect(find("^", "abc", { ...plain, regex: true })).toEqual([]);
  });

  it("says what is wrong with an expression, and finds nothing for nothing typed", () => {
    expect(pattern("(", { ...plain, regex: true })).toEqual({ error: expect.stringMatching(/group/i) });
    expect(pattern("(", plain)).toBeInstanceOf(RegExp);
    expect(pattern("", plain)).toBeUndefined();
  });

  it("goes file after file, line after line, keyed as the dialog numbers the lines", () => {
    const a = patchLines("@@ -1,2 +1,2 @@\n let x = 1;\n-let y = x;\n+let y = x + 1;\n@@ -9 +9 @@\n-x\n+y");
    const b = patchLines("@@ -0,0 +1 @@\n+x");
    const re = pattern("x", plain) as RegExp;
    const { matches, capped } = findIn([a, undefined, b], re);
    expect(capped).toBe(false);
    expect(matches.map((m) => [m.file, `${m.h}:${m.j}`, m.start])).toEqual([
      [0, "0:0", 4],
      [0, "0:1", 8],
      [0, "0:2", 8],
      [0, "1:0", 0],
      [2, "0:0", 0],
    ]);
    expect(findIn([a, b], re, 2)).toMatchObject({ capped: true, matches: { length: 2 } });
    expect(matchKey(matches[1] as DiffMatch)).toBe("0:0:1:8");
  });

  it("starts from where the reader is, and goes around at either end", () => {
    const m = (file: number, h: number, j: number): DiffMatch => ({ file, h, j, start: 0, end: 1 });
    const matches = [m(0, 0, 3), m(0, 1, 0), m(2, 0, 5), m(2, 0, 9)];
    expect(nearest(matches, { file: 0, h: 0, j: 0 })).toBe(0);
    expect(nearest(matches, { file: 0, h: 1, j: 0 })).toBe(1);
    expect(nearest(matches, { file: 1, h: 0, j: 0 })).toBe(2);
    expect(nearest(matches, { file: 2, h: 0, j: 10 })).toBe(0);
    expect(nearest(matches, { file: 2, h: 0, j: 5 }, false)).toBe(1);
    expect(nearest(matches, { file: 0, h: 0, j: 0 }, false)).toBe(3);
    expect(nearest([], { file: 0, h: 0, j: 0 })).toBe(-1);
  });
});

describe("a diff's lines, for every file at once", () => {
  const PATCH = [
    "@@ -3,4 +3,5 @@ fn main() {",
    "     let a = 1;",
    "-    let b = foo(a, 2);",
    "+    let b = foo(a, 3);",
    "+    let c = b;",
    "     println!();",
    "@@ -40 +41,2 @@",
    "-old",
    "\\ No newline at end of file",
    "+new\r",
    "",
  ].join("\n");

  it("are where parsePatch puts them", () => {
    const lines = patchLines(PATCH);
    const hunks = parsePatch(PATCH);
    expect(lines.map((l) => [l.h, l.j, l.kind, l.text])).toEqual(
      hunks.flatMap((h, i) => h.lines.map((l, j) => [i, j, l.kind, l.text])),
    );
    expect(lines.at(-1)?.text).toBe("new");
  });

  it("say where each change starts", () => {
    expect(changeStarts(patchLines(PATCH)).map((l) => `${l.h}:${l.j}`)).toEqual(["0:1", "1:0"]);
  });
});
