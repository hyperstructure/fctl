import { describe, expect, it } from "vitest";
import { parsePatch } from "./diff";
import { firstLive, type LiveDiff, type LiveItem, nextLive, settle, splitLive } from "./diff-motion";

const hunks = (...lines: string[]) => parsePatch(lines.join("\n"));

/** Each row: `@@` for a hunk's line, else its mark and text, then how it came. */
const rows = (d: LiveDiff) =>
  d.items.map((i) =>
    i.type === "hunk"
      ? `@@ ${i.hunk.oldStart} ${i.how}`
      : `${{ add: "+", del: "-", context: " " }[i.line.kind]}${i.line.text} ${i.how}${i.was ? ` (was ${i.was.text})` : ""}`,
  );

const ids = (d: LiveDiff) =>
  new Map(d.items.map((i) => [i.type === "hunk" ? `@@${i.hunk.oldStart}` : i.line.text, i.id]));

/** The version after, from a settled one before. */
const step = (before: string[], after: string[]) => {
  const a = firstLive(hunks(...before));
  return { a, b: nextLive(a, hunks(...after)) };
};

describe("a diff that changes as it shows", () => {
  it("shows its first version as it is", () => {
    const d = firstLive(hunks("@@ -1,2 +1,2 @@", " a", "-b", "+c"));
    expect(rows(d)).toEqual(["@@ 1 kept", " a kept", "-b kept", "+c kept"]);
    expect(new Set(d.items.map((i) => i.id)).size).toBe(d.items.length);
  });

  it("keeps the rows of lines that stay, and opens the new ones", () => {
    const { a, b } = step(
      ["@@ -1,3 +1,4 @@", " fn a() {", "+    one();", "+    two();", " }"],
      ["@@ -1,3 +1,5 @@", " fn a() {", "+    one();", "+    between();", "+    two();", " }"],
    );
    expect(rows(b)).toEqual([
      "@@ 1 kept",
      " fn a() { kept",
      "+    one(); kept",
      "+    between(); entered",
      "+    two(); kept",
      " } kept",
    ]);
    // A line that stays is the same row, under the same id.
    for (const text of ["fn a() {", "    one();", "    two();", "}"]) {
      expect(ids(b).get(text)).toBe(ids(a).get(text));
    }
    expect(ids(b).get("@@1")).toBe(ids(a).get("@@1"));
    expect(ids(b).get("    between();")).not.toBe(undefined);
    expect([...ids(a).values()]).not.toContain(ids(b).get("    between();"));
  });

  it("marks a line taken out that showed unchanged, and unmarks one put back", () => {
    const before = ["@@ -1,3 +1,3 @@", " a", " b", "-c", "+C"];
    const after = ["@@ -1,3 +1,2 @@", " a", "-b", "-c", "+C"];
    const { a, b } = step(before, after);
    expect(rows(b)).toEqual(["@@ 1 kept", " a kept", "-b marked", "-c kept", "+C kept"]);
    expect(ids(b).get("b")).toBe(ids(a).get("b"));
    const back = nextLive(settle(b), hunks(...before));
    expect(rows(back)).toEqual(["@@ 1 kept", " a kept", " b unmarked", "-c kept", "+C kept"]);
  });

  it("rewrites a line it added before, on the same row", () => {
    const { a, b } = step(
      ["@@ -1,2 +1,2 @@", " let a = 1;", "-let b = 2;", "+let b = 3;"],
      ["@@ -1,2 +1,2 @@", " let a = 1;", "-let b = 2;", "+let b = a + 3;"],
    );
    expect(rows(b)).toEqual([
      "@@ 1 kept",
      " let a = 1; kept",
      "-let b = 2; kept",
      "+let b = a + 3; rewritten (was let b = 3;)",
    ]);
    expect(ids(b).get("let b = a + 3;")).toBe(ids(a).get("let b = 3;"));
  });

  it("keeps an added line as the change around it takes out a line more above it", () => {
    const { a, b } = step(
      ["@@ -1,4 +1,4 @@", " a", " b", "-c", "+C", " d"],
      ["@@ -1,4 +1,3 @@", " a", "-b", "-c", "+C", " d"],
    );
    expect(rows(b)).toEqual(["@@ 1 kept", " a kept", "-b marked", "-c kept", "+C kept", " d kept"]);
    expect(ids(b).get("C")).toBe(ids(a).get("C"));
  });

  it("moves an added line past the lines taken out after it, as git writes those first", () => {
    const { b } = step(
      ["@@ -1,6 +1,6 @@", " a", " b", "-c", "+C", " d", " e", " f"],
      ["@@ -1,6 +1,5 @@", " a", "-b", "-c", "-d", "+C", "+D", " e", " f"],
    );
    expect(rows(b)).toEqual([
      "@@ 1 kept",
      " a kept",
      "-b marked",
      "-c kept",
      "+C gone",
      "-d marked",
      "+C moved",
      "+D entered",
      " e kept",
      " f kept",
    ]);
  });

  it("opens a new hunk with its `@@` line, and closes one that goes", () => {
    const one = ["@@ -1,2 +1,2 @@", " a", "-b", "+B"];
    const two = ["@@ -40,2 +40,2 @@", " x", "-y", "+Y"];
    const { a, b } = step(one, [...one, ...two]);
    expect(rows(b)).toEqual([
      "@@ 1 kept",
      " a kept",
      "-b kept",
      "+B kept",
      "@@ 40 entered",
      " x entered",
      "-y entered",
      "+Y entered",
    ]);
    // Back again: the second goes, `@@` line and all.
    const c = nextLive(settle(b), hunks(...one));
    expect(rows(c)).toEqual([
      "@@ 1 kept",
      " a kept",
      "-b kept",
      "+B kept",
      "@@ 40 gone",
      " x gone",
      "-y gone",
      "+Y gone",
    ]);
    expect(ids(c).get("@@1")).toBe(ids(a).get("@@1"));
    // Rows that go are no place in the hunks.
    expect(c.items.filter((i) => i.how === "gone").every((i) => i.h === -1)).toBe(true);
  });

  it("joins two hunks into one, the lines between them coming into view", () => {
    const { b } = step(
      ["@@ -1,2 +1,2 @@", " a", "-b", "+B", "@@ -5,2 +5,2 @@", " e", "-f", "+F"],
      ["@@ -1,6 +1,6 @@", " a", "-b", "+B", " c", " d", " e", "-f", "+F"],
    );
    expect(rows(b)).toEqual([
      "@@ 1 kept",
      " a kept",
      "-b kept",
      "+B kept",
      " c entered",
      " d entered",
      "@@ 5 gone",
      " e kept",
      "-f kept",
      "+F kept",
    ]);
  });

  it("splits a hunk in two, a new `@@` line between them", () => {
    const { b } = step(
      ["@@ -1,7 +1,7 @@", " a", "-b", "+B", " c", "-d", "+D", " e", "-f", "+F"],
      ["@@ -1,3 +1,3 @@", " a", "-b", "+B", " c", "@@ -5,2 +5,2 @@", " e", "-f", "+F"],
    );
    expect(rows(b)).toEqual([
      "@@ 1 kept",
      " a kept",
      "-b kept",
      "+B kept",
      " c kept",
      "-d gone",
      "+D gone",
      "@@ 5 entered",
      " e kept",
      "-f kept",
      "+F kept",
    ]);
  });

  it("follows a new file's lines by what they say", () => {
    const { b } = step(
      ["@@ -0,0 +1,4 @@", "+# Notes", "+", "+One", "+Two"],
      ["@@ -0,0 +1,5 @@", "+# Notes", "+", "+One", "+One and a half", "+Three"],
    );
    expect(rows(b)).toEqual([
      "@@ 0 kept",
      "+# Notes kept",
      "+ kept",
      "+One kept",
      "+One and a half rewritten (was Two)",
      "+Three entered",
    ]);
  });

  it("takes rows still going from the version before out at once", () => {
    const { b } = step(["@@ -1,2 +1,2 @@", " a", "-b", "+B"], ["@@ -1,2 +1,1 @@", " a", "-b"]);
    expect(rows(b)).toContain("+B gone");
    // Not settled: the next version starts from what shows, without the row that went.
    const c = nextLive(b, hunks("@@ -1,2 +1,1 @@", " a", "-b"));
    expect(rows(c)).toEqual(["@@ 1 kept", " a kept", "-b kept"]);
  });

  it("settles a version: what went is left out, and the rest is as it is", () => {
    const { b } = step(["@@ -1,2 +1,2 @@", " a", "-b", "+B"], ["@@ -1,2 +1,3 @@", " a", "-b", "+C", "+D"]);
    const s = settle(b);
    expect(rows(s)).toEqual(["@@ 1 kept", " a kept", "-b kept", "+C kept", "+D kept"]);
    expect(s.items.map((i) => i.id)).toEqual(b.items.map((i) => i.id));
    expect(settle(s)).toBe(s);
  });

  it("shows anew what is too different to follow", () => {
    const many = (n: number, mark: string) => Array.from({ length: n }, (_, i) => `${mark}${mark}${i}`);
    const before = firstLive(hunks("@@ -0,0 +1,12000 @@", ...many(12000, "+")));
    const after = nextLive(before, hunks("@@ -1,12000 +0,0 @@", ...many(12000, "-")));
    expect(after.items.every((i) => i.how === "kept")).toBe(true);
    expect(after.version).toBe(1);
  });

  it("lays the rows side by side, a line that goes on its side", () => {
    const { b } = step(
      ["@@ -1,3 +1,3 @@", " a", "-b", "+B", " c", "-d", "+D"],
      ["@@ -1,3 +1,3 @@", " a", "-b", "+B2", " c", "-d"],
    );
    const side = (l?: LiveItem) => (l && l.type === "line" ? `${l.line.text}:${l.how}` : "");
    expect(
      splitLive(b.items).map((r) => (r.type === "hunk" ? "@@" : `${side(r.left)} | ${side(r.right)}`)),
    ).toEqual([
      "@@",
      "a:kept | a:kept",
      "b:kept | B2:rewritten",
      "c:kept | c:kept",
      "d:kept | ",
      " | D:gone",
    ]);
  });
});
