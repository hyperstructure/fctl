import { describe, expect, it } from "vitest";
import {
  counts,
  type DiffLine,
  firstLine,
  fromStrings,
  fromStructured,
  hiddenBefore,
  markChanges,
  parsePatch,
  splitRows,
} from "./diff";

const PATCH = [
  "@@ -3,4 +3,5 @@ fn main() {",
  "     let a = 1;",
  "-    let b = foo(a, 2);",
  "+    let b = foo(a, 3);",
  "+    let c = b;",
  "     println!();",
  " }",
  "@@ -40 +41,2 @@",
  "-old",
  "\\ No newline at end of file",
  "+new\r",
  "+more",
  "\\ No newline at end of file",
].join("\n");

describe("git's hunks", () => {
  it("number each line as the file has it, before and after", () => {
    const hunks = parsePatch(PATCH);
    expect(hunks).toHaveLength(2);
    const [h1, h2] = hunks;
    expect(h1).toMatchObject({ oldStart: 3, oldLines: 4, newStart: 3, newLines: 5, section: "fn main() {" });
    expect(h1?.lines.map((l) => [l.kind, l.old, l.new])).toEqual([
      ["context", 3, 3],
      ["del", 4, undefined],
      ["add", undefined, 4],
      ["add", undefined, 5],
      ["context", 5, 6],
      ["context", 6, 7],
    ]);
    // `@@ -40 +41,2 @@`: a count left out is one.
    expect(h2).toMatchObject({ oldStart: 40, oldLines: 1, newStart: 41, newLines: 2, section: undefined });
    expect(h2?.lines.map((l) => [l.kind, l.text, !!l.noNewline])).toEqual([
      ["del", "old", true],
      ["add", "new", false],
      ["add", "more", true],
    ]);
    expect(counts(hunks)).toEqual({ added: 4, removed: 2 });
  });

  it("mark the words that changed in a line and the one that took its place", () => {
    const [h] = parsePatch(PATCH);
    const del = h?.lines[1];
    const add = h?.lines[2];
    expect(del?.text.slice(...(del.changed?.[0] ?? [0, 0]))).toBe("2");
    expect(add?.text.slice(...(add.changed?.[0] ?? [0, 0]))).toBe("3");
    expect(del?.changed).toHaveLength(1);
    // An added line with nothing to pair with has nothing marked.
    expect(h?.lines[3]?.changed).toBeUndefined();
  });

  it("mark what changed in one piece when only spaces lie between", () => {
    const lines: DiffLine[] = [
      { kind: "del", text: "export function DayPicker({ days, value, onChange }: Props) {" },
      { kind: "add", text: "export function DayPicker({ days, value, onChange, skipped = [] }: Props) {" },
    ];
    markChanges(lines);
    const add = lines[1];
    expect(add?.changed?.map(([a, b]) => add.text.slice(a, b))).toEqual([", skipped = []"]);
  });

  it("leave lines that share little unmarked", () => {
    const lines = parsePatch("@@ -1 +1 @@\n-const config = load();\n+export default {};").flatMap(
      (h) => h.lines,
    );
    expect(lines.map((l) => l.changed)).toEqual([undefined, undefined]);
    const same: DiffLine[] = [
      { kind: "del", text: "  return a + b;" },
      { kind: "add", text: "  return a - b;" },
    ];
    markChanges(same);
    expect(same.map((l) => l.changed)).toEqual([[[11, 12]], [[11, 12]]]);
  });

  it("count the unchanged lines between them, and know the file's first line", () => {
    const hunks = parsePatch(PATCH);
    expect(hiddenBefore(hunks, 0)).toBe(2);
    expect(hiddenBefore(hunks, 1)).toBe(33);
    expect(firstLine(hunks)).toBeUndefined();
    // A file that became a link: its old lines out, then its new one in.
    const changed = parsePatch("@@ -1,2 +0,0 @@\n-one\n-two\n@@ -0,0 +1 @@\n+target");
    expect(hiddenBefore(changed, 1)).toBe(0);
    expect(firstLine(parsePatch("@@ -0,0 +1,2 @@\n+#!/bin/sh\n+echo hi"))).toBe("#!/bin/sh");
  });

  it("ignore what comes before the first hunk", () => {
    expect(parsePatch("")).toEqual([]);
    expect(parsePatch("garbage\n+x")).toEqual([]);
  });
});

describe("lines taken out and those added in their place", () => {
  it("pair up by what they have in common, not by where they are", () => {
    const [h] = parsePatch(
      [
        "@@ -1,3 +1,4 @@",
        "-    pub fn location(&self) -> Location {",
        "-        Stop::Depot(l) => *l,",
        "+    /// Where it is: a break has no place of its own.",
        "+    pub fn location(&self) -> Option<Location> {",
        "+        Stop::Depot(l) => Some(*l),",
      ].join("\n"),
    );
    const lines = h?.lines ?? [];
    const marked = (l?: DiffLine) => l?.changed?.map(([a, b]) => l.text.slice(a, b));
    // Nothing of the old lines went; what came in around it is marked.
    expect(marked(lines[0])).toBeUndefined();
    expect(marked(lines[3])).toEqual(["Option<", ">"]);
    expect(marked(lines[1])).toBeUndefined();
    expect(marked(lines[4])).toEqual(["Some(", ")"]);
    expect(lines[2]?.changed).toBeUndefined();
    // Side by side too: the comment on its own, each line beside its new self.
    expect(splitRows(lines).map((r) => [r.left?.text.trim() ?? null, r.right?.text.trim() ?? null])).toEqual([
      [null, "/// Where it is: a break has no place of its own."],
      ["pub fn location(&self) -> Location {", "pub fn location(&self) -> Option<Location> {"],
      ["Stop::Depot(l) => *l,", "Stop::Depot(l) => Some(*l),"],
    ]);
  });

  it("go side by side as they come when nothing pairs them", () => {
    const lines = parsePatch("@@ -1,2 +1,2 @@\n-alpha one\n-beta two\n+gamma\n+delta").flatMap(
      (h) => h.lines,
    );
    expect(splitRows(lines).map((r) => [r.left?.text ?? null, r.right?.text ?? null])).toEqual([
      ["alpha one", "gamma"],
      ["beta two", "delta"],
    ]);
  });
});

describe("side by side", () => {
  it("puts lines taken out beside those added in their place", () => {
    const [h] = parsePatch(PATCH);
    const rows = splitRows(h?.lines ?? []);
    expect(rows.map((r) => [r.left?.text ?? null, r.right?.text ?? null])).toEqual([
      ["    let a = 1;", "    let a = 1;"],
      ["    let b = foo(a, 2);", "    let b = foo(a, 3);"],
      [null, "    let c = b;"],
      ["    println!();", "    println!();"],
      ["}", "}"],
    ]);
  });
});

describe("an edit's diff", () => {
  it("takes Claude Code's hunks, numbered as the file is", () => {
    const hunks = fromStructured({
      filePath: "/w/a.rs",
      structuredPatch: [{ oldStart: 10, oldLines: 2, newStart: 10, newLines: 2, lines: [" a", "-b", "+c"] }],
    });
    expect(hunks?.[0]?.lines.map((l) => [l.kind, l.old, l.new])).toEqual([
      ["context", 10, 10],
      ["del", 11, undefined],
      ["add", undefined, 11],
    ]);
    expect(fromStructured(undefined)).toBeUndefined();
    expect(fromStructured({ structuredPatch: [] })).toBeUndefined();
    expect(fromStructured({ structuredPatch: [{ lines: [1] }] })).toBeUndefined();
  });

  it("makes one of the strings it replaced, without numbers", () => {
    const [h] = fromStrings("a\nb\nc\n", "a\nB\nc\n");
    expect(h?.lines.map((l) => [l.kind, l.text, l.old ?? null])).toEqual([
      ["context", "a", null],
      ["del", "b", null],
      ["add", "B", null],
      ["context", "c", null],
    ]);
    expect(fromStrings("", "")).toEqual([]);
  });
});
