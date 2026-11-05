import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { parsePatch } from "@/lib/diff";
import { DiffLines } from "./diff";

// No Shiki in tests: lines show as plain text, unless a test colors them.
const highlight = vi.hoisted(() => ({
  tokenize: vi.fn((_lines: string[], _lang: string): Promise<unknown> => Promise.resolve(undefined)),
}));
vi.mock("@/lib/highlight", () => highlight);

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  highlight.tokenize.mockReset();
  highlight.tokenize.mockImplementation(() => Promise.resolve(undefined));
});

function render(node: React.ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(node));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

const PATCH = [
  "@@ -3,3 +3,4 @@ fn main() {",
  "     let a = 1;",
  "-    let b = foo(a, 2);",
  "+    let b = foo(a, 3);",
  "+    let c = b;",
  " }",
  "@@ -40 +41 @@",
  "-old",
  "+new",
  "\\ No newline at end of file",
].join("\n");

const rows = (c: HTMLElement) =>
  [...c.querySelectorAll(".diff-line")].map((r) =>
    [...r.querySelectorAll(".diff-number"), r.querySelector(".diff-mark"), r.querySelector(".diff-code")].map(
      (e) => e?.textContent ?? "",
    ),
  );

describe("a diff's lines", () => {
  it("are numbered as the file has them, before and after, each marked", () => {
    const c = render(<DiffLines hunks={parsePatch(PATCH)} lang="rust" />);
    expect(rows(c)).toEqual([
      ["3", "3", "", "    let a = 1;"],
      ["4", "", "−", "    let b = foo(a, 2);"],
      ["", "4", "+", "    let b = foo(a, 3);"],
      ["", "5", "+", "    let c = b;"],
      ["5", "6", "", "}"],
      ["40", "", "−", "old"],
      ["", "41", "+", "new"],
    ]);
    // What a mark means, for assistive technology.
    expect([...c.querySelectorAll(".diff-mark[role=img]")].map((m) => m.getAttribute("aria-label"))).toEqual([
      "Removed",
      "Added",
      "Added",
      "Removed",
      "Added",
    ]);
    // The words that changed between lines with much in common, marked once each.
    expect([...c.querySelectorAll(".diff-word")].map((w) => w.textContent)).toEqual(["2", "3"]);
    // The file ends without a newline after the last line.
    expect(c.querySelectorAll('[aria-label="No newline at end of file"]')).toHaveLength(1);
  });

  it("say above each hunk where it is and what it is in", () => {
    const c = render(<DiffLines hunks={parsePatch(PATCH)} lang="rust" />);
    expect([...c.querySelectorAll("[data-hunk]")].map((h) => h.textContent)).toEqual([
      "⋯@@ −3,3 +3,4 @@fn main() {2 unchanged lines before",
      "⋯@@ −40 +41 @@34 unchanged lines before",
    ]);
  });

  it("leave out the hunk's line for a whole file, added or deleted", () => {
    const c = render(<DiffLines hunks={parsePatch("@@ -0,0 +1,2 @@\n+a\n+b")} lang="text" />);
    expect(c.querySelector("[data-hunk]")).toBeNull();
    expect(rows(c).map((r) => r[1])).toEqual(["1", "2"]);
  });

  it("go side by side, taken out beside added, a hatch where a side has none", () => {
    const c = render(<DiffLines hunks={parsePatch(PATCH)} lang="rust" layout="split" />);
    const sides = [...c.querySelectorAll(".grid-cols-2")].map((row) =>
      [...row.children].map((s) =>
        s.classList.contains("diff-empty") ? null : s.querySelector(".diff-code")?.textContent,
      ),
    );
    expect(sides).toEqual([
      ["    let a = 1;", "    let a = 1;"],
      ["    let b = foo(a, 2);", "    let b = foo(a, 3);"],
      [null, "    let c = b;"],
      ["}", "}"],
      ["old", "new"],
    ]);
    // Side by side, long lines wrap.
    expect(c.querySelector(".diff")?.classList.contains("diff-wrap")).toBe(true);
  });

  it("keep their colors when their hunks are made again with the same lines", async () => {
    highlight.tokenize.mockImplementation((lines: string[]) =>
      Promise.resolve(lines.map((l) => [{ content: l, style: { "--shiki-dark": "#abcdef" } }])),
    );
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const draw = () => act(() => root.render(<DiffLines hunks={parsePatch(PATCH)} lang="rust" />));
    draw();
    roots.push(() => {
      act(() => root.unmount());
      container.remove();
    });
    const colored = () => container.querySelectorAll(".diff-code span[style]").length;
    // Shiki's tokens come a few turns of the event loop later.
    for (let i = 0; i < 50 && colored() === 0; i++) {
      await act(() => new Promise((r) => setTimeout(r, 10)));
    }
    expect(colored()).toBeGreaterThan(0);
    const asked = highlight.tokenize.mock.calls.length;
    // A transcript's edit makes its hunks again at each render.
    draw();
    expect(colored()).toBeGreaterThan(0);
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(highlight.tokenize.mock.calls.length).toBe(asked);
  });

  it("go without numbers when nobody knows them", () => {
    const c = render(<DiffLines hunks={parsePatch(PATCH)} lang="rust" numbers={false} headers={false} />);
    expect(c.querySelector(".diff-number")).toBeNull();
    expect(c.querySelector("[data-hunk]")).toBeNull();
  });
});
