import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { parsePatch } from "@/lib/diff";
import { DiffLines, type LiveView } from "./diff";
import { light, SETTLE_MS } from "./diff-play";

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

/** What shows a live diff: here, all of it in view, unless `band` says otherwise. */
const shower = (band: { top: number; bottom: number } = { top: -1, bottom: 10_000 }) => ({
  hold: vi.fn(),
  moving: vi.fn(),
  band: () => band,
  unseen: vi.fn(),
});

/** A live diff, drawn again with each version. */
function live(view: LiveView, { entering = false }: { entering?: boolean } = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  // Diffs that change together play together, once the commit is done: a microtask later.
  const draw = (...lines: string[]) =>
    act(async () => {
      root.render(
        <DiffLines hunks={parsePatch(lines.join("\n"))} lang="text" live={view} entering={entering} />,
      );
    });
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const row = (text: string) =>
    [...container.querySelectorAll<HTMLElement>(".diff-line")].find(
      (r) => r.querySelector(".diff-code")?.textContent === text,
    );
  return { container, draw, row };
}

describe("a live diff's lines", () => {
  const V1 = ["@@ -1,3 +1,4 @@", " fn a() {", "+    one();", "+    two();", " }"];

  it("keep their rows as the file changes, and those that come play in", async () => {
    const view = shower();
    const { draw, row } = live(view);
    await draw(...V1);
    const kept = ["fn a() {", "    one();", "    two();", "}"].map(row);
    expect(kept.every(Boolean)).toBe(true);
    // Nothing plays as the diff first shows.
    expect(view.moving).not.toHaveBeenCalled();
    await draw("@@ -1,3 +1,5 @@", " fn a() {", "+    one();", "+    between();", "+    two();", " }");
    expect(["fn a() {", "    one();", "    two();", "}"].map(row)).toEqual(kept);
    const added = row("    between();");
    expect(added?.dataset.open).toBe("in");
    expect(added?.dataset.how).toBe("entered");
    // Written in where it shows, behind a caret as wide as its code.
    expect(added?.dataset.play).toBe("entered");
    expect(added?.style.getPropertyValue("--diff-chars")).toBe("14");
    expect(kept.every((r) => !r?.dataset.play)).toBe(true);
    expect(view.hold).toHaveBeenCalled();
    expect(view.moving).toHaveBeenCalled();
    expect(view.unseen).not.toHaveBeenCalled();
    expect(added?.dataset.l).toBe("0:2");
  });

  it("close the rows of lines that went, then take them out", async () => {
    vi.useFakeTimers();
    try {
      const { container, draw, row } = live(shower());
      await draw(...V1);
      await draw("@@ -1,3 +1,3 @@", " fn a() {", "+    one();", " }");
      const gone = row("    two();");
      // Still there as it closes, for the eyes only: no place to find in, nothing to read out.
      expect(gone?.dataset.open).toBe("out");
      expect(gone?.getAttribute("aria-hidden")).toBe("true");
      expect(gone?.dataset.l).toBeUndefined();
      act(() => vi.advanceTimersByTime(SETTLE_MS + 10));
      expect(row("    two();")).toBeUndefined();
      expect(container.querySelector("[data-open], [data-how]")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("rewrite a line added before on its row, over what it said", async () => {
    const { draw, row } = live(shower());
    await draw("@@ -1,2 +1,2 @@", " let a = 1;", "-let b = 2;", "+let b = 3;");
    const before = row("let b = 3;");
    await draw("@@ -1,2 +1,2 @@", " let a = 1;", "-let b = 2;", "+let b = a + 3;");
    const after = row("let b = a + 3;");
    expect(after).toBe(before);
    expect(after?.dataset.play).toBe("rewritten");
    expect(after?.querySelector<HTMLElement>(".diff-code")?.dataset.was).toBe("let b = 3;");
    // What it said before shows from its pseudo-element: not in its text, which finding reads.
    expect(after?.querySelector(".diff-code")?.textContent).toBe("let b = a + 3;");
  });

  it("turn a line red that the agent took out", async () => {
    const { draw, row } = live(shower());
    await draw("@@ -1,3 +1,3 @@", " a", " b", "-c", "+C");
    await draw("@@ -1,3 +1,2 @@", " a", "-b", "-c", "+C");
    expect(row("b")?.dataset.play).toBe("marked");
    expect(row("b")?.dataset.kind).toBe("del");
  });

  it("say which changes came out of sight, each a run of rows, instead of playing them", async () => {
    const view = shower({ top: -10, bottom: -5 });
    const { draw, row } = live(view);
    await draw(...V1);
    await draw("@@ -1,3 +1,6 @@", " fn a() {", "+    zero();", "+    one();", "+    two();", " }", "+// end");
    expect(view.unseen).toHaveBeenCalledTimes(1);
    const changes = view.unseen.mock.calls[0]?.[0] as HTMLElement[][];
    expect(changes.map((c) => c.map((r) => r.querySelector(".diff-code")?.textContent))).toEqual([
      ["    zero();"],
      ["// end"],
    ]);
    expect(row("    zero();")?.dataset.play).toBeUndefined();
  });

  it("light up as the reader comes to them though the agent changed them again while they were lit", async () => {
    // In view as the first change plays, out of sight as the next comes.
    const band = { top: -1, bottom: 10_000 };
    const view = { ...shower(), band: () => band };
    const { draw, row } = live(view);
    await draw("@@ -1,2 +1,2 @@", " a", "-b", "+B");
    await draw("@@ -1,2 +1,2 @@", " a", "-b", "+B2");
    expect(row("B2")?.dataset.play).toBe("rewritten");
    band.top = -10;
    band.bottom = -5;
    await draw("@@ -1,2 +1,2 @@", " a", "-b", "+B3");
    const again = row("B3") as HTMLElement;
    expect(again.dataset.play).toBeUndefined();
    expect(view.unseen).toHaveBeenLastCalledWith([[again]]);
    // The reader comes to it: it lights up, whatever its write would have lit it for.
    light([again]);
    expect(again.dataset.play).toBe("seen");
  });

  it("come in from nothing, as a file's lines do when the agent changed it first", async () => {
    const { container, draw } = live(shower(), { entering: true });
    await draw("@@ -0,0 +1,2 @@", "+# Notes", "+Hi");
    expect([...container.querySelectorAll(".diff-line")].map((r) => (r as HTMLElement).dataset.how)).toEqual([
      "entered",
      "entered",
    ]);
  });

  it("play together with the other diffs that change with them, once they all have", async () => {
    const view = shower();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const draw = (a: string[], b: string[]) =>
      act(async () => {
        root.render(
          <>
            <DiffLines hunks={parsePatch(a.join("\n"))} lang="text" live={view} />
            <DiffLines hunks={parsePatch(b.join("\n"))} lang="text" live={view} />
          </>,
        );
      });
    roots.push(() => {
      act(() => root.unmount());
      container.remove();
    });
    await draw(V1, ["@@ -1 +1 @@", "-x", "+y"]);
    await draw([...V1, "+    three();"], ["@@ -1 +1,2 @@", "-x", "+y", "+z"]);
    // One pass for both: the place held once, one wave down them.
    expect(view.hold).toHaveBeenCalledTimes(1);
    expect(view.moving).toHaveBeenCalledTimes(1);
    const played = [...container.querySelectorAll<HTMLElement>("[data-play]")];
    expect(played.map((r) => r.querySelector(".diff-code")?.textContent)).toEqual(["    three();", "z"]);
  });

  it("wait for the colors of a new version before it shows", async () => {
    let color: (t: unknown) => void = () => {};
    highlight.tokenize.mockImplementation(
      (lines: string[]) =>
        new Promise((r) => {
          if (lines.some((l) => l.includes("between"))) color = r;
          else r(lines.map((l) => [{ content: l, style: { "--shiki-dark": "#abcdef" } }]));
        }),
    );
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const view = shower();
    const draw = (...lines: string[]) =>
      act(() => root.render(<DiffLines hunks={parsePatch(lines.join("\n"))} lang="rust" live={view} />));
    roots.push(() => {
      act(() => root.unmount());
      container.remove();
    });
    await draw(...V1);
    await act(() => new Promise((r) => setTimeout(r, 20)));
    await draw("@@ -1,3 +1,5 @@", " fn a() {", "+    one();", "+    between();", "+    two();", " }");
    await act(() => new Promise((r) => setTimeout(r, 20)));
    const codes = () => [...container.querySelectorAll(".diff-code")].map((c) => c.textContent);
    // The version before, colored, until the new one's colors come.
    expect(codes()).not.toContain("    between();");
    await act(async () => {
      color(
        ["fn a() {", "    one();", "    between();", "    two();", "}"].map((l) => [
          { content: l, style: { "--shiki-dark": "#abcdef" } },
        ]),
      );
      await new Promise((r) => setTimeout(r, 20));
    });
    expect(codes()).toContain("    between();");
    expect(container.querySelector('[data-how="entered"] .diff-code span[style]')).not.toBeNull();
  });
});
