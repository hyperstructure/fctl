import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { byDirectory, DiffDialog, type DiffGroup, type DiffTarget, modeChange } from "./diff-dialog";

// Diffs come from the cache, or all at once from `sessionDiff`; one by one they never come.
const api = vi.hoisted(() => ({
  sessionDiff: vi.fn(),
  fileDiff: vi.fn(() => new Promise(() => {})),
}));
vi.mock("@/lib/api", () => ({ api, ApiFailure: class extends Error {} }));
vi.mock("@/lib/highlight", () => ({ tokenize: () => Promise.resolve(undefined) }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
beforeEach(() => {
  // Every file shows its lines at once, as files near the view do.
  vi.stubGlobal("IntersectionObserver", undefined);
});
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  localStorage.clear();
  vi.unstubAllGlobals();
  api.sessionDiff.mockReset();
  api.fileDiff.mockClear();
});

const file = (path: string, status = "modified", a = 1, d = 1): G.FileChange => ({
  path,
  status,
  additions: a,
  deletions: d,
  digest: `d-${path}`,
});

const diff = (path: string, d: Partial<G.FileDiff>): G.FileDiff => ({
  path,
  status: "modified",
  digest: `d-${path}`,
  ...d,
});

const groups: DiffGroup[] = [
  {
    title: "acme/web",
    files: [
      file("README.md"),
      file("assets/logo.png"),
      file("package-lock.json", "modified", 900, 300),
      file("scripts/deploy", "modified", 0, 0),
      file("scripts/link", "modified", 1, 0),
      file("src/a.rs"),
      file("src/b.rs", "renamed", 0, 0),
      // From an older agent, which sends no diffs.
      { path: "src/old.rs", status: "modified", additions: 1, deletions: 1 },
    ],
  },
  { repo: "acme/docs", title: "acme/docs", files: [file("guide.md", "added", 2, 0)] },
];

const diffs: G.FileDiff[] = [
  diff("README.md", { patch: "@@ -1 +1 @@\n-# Web\n+# Web app" }),
  diff("assets/logo.png", { binary: true }),
  diff("package-lock.json", { too_large: true }),
  diff("scripts/deploy", { old_mode: "100644", new_mode: "100755" }),
  // A binary file that became a link: its link's line.
  diff("scripts/link", {
    binary: true,
    old_mode: "100644",
    new_mode: "120000",
    patch: "@@ -0,0 +1 @@\n+target",
  }),
  diff("src/a.rs", { patch: "@@ -1,2 +1,2 @@\n fn a() {\n-    1\n+    2" }),
  diff("src/b.rs", { status: "renamed", old_path: "src/b_old.rs" }),
  diff("guide.md", { repo: "acme/docs", status: "added", patch: "@@ -0,0 +1,2 @@\n+# Guide\n+Hi app" }),
];

function open(at: DiffTarget, { cached = true }: { cached?: boolean } = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  if (cached) {
    for (const g of groups) {
      for (const f of g.files) {
        const d = diffs.find((x) => x.path === f.path && x.repo === g.repo);
        if (d) client.setQueryData(keys.fileDiff("ses_1", g.repo ?? "", f.path, f.digest ?? ""), d);
      }
    }
  }
  const onClose = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <DiffDialog session="ses_1" cliRef="web#12" groups={groups} initial={at} onClose={onClose} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  // The newest: a test may open several.
  const dialog = [...document.querySelectorAll("dialog")].pop() as HTMLDialogElement;
  const key = (k: string, target: Element = dialog, init: KeyboardEventInit = {}) => {
    const e = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init });
    act(() => {
      target.dispatchEvent(e);
    });
    return e;
  };
  const text = () => dialog.textContent ?? "";
  const nav = () => dialog.querySelector("nav")?.textContent;
  const inView = () => dialog.querySelector('aside [aria-current="true"]')?.getAttribute("title");
  const section = (path: string) =>
    [...dialog.querySelectorAll("section")].find(
      (s) => s.querySelector("h3")?.getAttribute("title") === path,
    );
  return { dialog, onClose, key, text, nav, inView, section };
}

const settle = () => act(() => new Promise((r) => setTimeout(r, 0)));

const type = (input: HTMLInputElement, value: string) =>
  act(() => {
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    set?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });

describe("the session's changes over the page", () => {
  it("shows every changed file's lines, one after the other, from the file opened", async () => {
    const { dialog, nav, inView, section } = open({ path: "src/a.rs" });
    await settle();
    expect(dialog.open).toBe(true);
    expect(dialog.querySelector("h2")?.textContent).toBe("Changes");
    expect(dialog.querySelector("header")?.textContent).toContain("9 files+907 −304");
    expect([...dialog.querySelectorAll("section h3")].map((h) => h.getAttribute("title"))).toEqual([
      "README.md",
      "package-lock.json",
      "assets/logo.png",
      "scripts/deploy",
      "scripts/link",
      "src/a.rs",
      "src/b.rs",
      "src/old.rs",
      "guide.md",
    ]);
    expect(nav()).toBe("6/9");
    expect(inView()).toBe("src/a.rs");
    const a = section("src/a.rs");
    expect(a?.textContent).toContain("Modified");
    expect([...(a?.querySelectorAll(".diff-code") ?? [])].map((c) => c.textContent)).toEqual([
      "fn a() {",
      "    1",
      "    2",
    ]);
    expect(section("README.md")?.querySelector(".diff-code")?.textContent).toBe("# Web");
    // Each line says where it is, for finding and stepping to it.
    expect([...(a?.querySelectorAll("[data-l]") ?? [])].map((l) => l.getAttribute("data-l"))).toEqual([
      "0:0",
      "0:1",
      "0:2",
    ]);
  });

  it("lists the changed files of each repository under their directories, the one in view marked", () => {
    const { dialog } = open({ path: "src/a.rs" });
    const list = dialog.querySelector("aside") as HTMLElement;
    const lines = [...list.querySelectorAll("ul ul > li, ul > li > div")].map((e) => e.textContent?.trim());
    expect(lines).toContain("assets");
    expect(lines).toContain("src");
    expect(list.textContent).toContain("acme/docs");
    const current = list.querySelector('[aria-current="true"]');
    expect(current?.textContent).toContain("a.rs");
    // Tab stops at it alone: the arrow keys move within the list.
    expect(
      [...list.querySelectorAll("button[data-row]")].filter((b) => b.getAttribute("tabindex") === "0"),
    ).toEqual([current]);
  });

  it("groups each directory's files once, those at the root first", () => {
    const paths = [
      "README.md",
      "assets/logo.png",
      "package-lock.json",
      "src/a.rs",
      "src/lib/b.rs",
      "src/c.rs",
    ];
    expect(
      byDirectory(paths.map((p) => file(p))).map((d) => [d.dir, d.files.map((f) => f.path.split("/").pop())]),
    ).toEqual([
      ["", ["README.md", "package-lock.json"]],
      ["assets/", ["logo.png"]],
      ["src/", ["a.rs", "c.rs"]],
      ["src/lib/", ["b.rs"]],
    ]);
  });

  it("goes to the next and previous files with j and k, across repositories", () => {
    const { key, nav, inView } = open({ path: "src/old.rs" });
    key("j");
    expect(inView()).toBe("guide.md");
    expect(nav()).toBe("9/9");
    // From the last file there is none further.
    key("j");
    expect(inView()).toBe("guide.md");
    key("k");
    key("k");
    expect(inView()).toBe("src/b.rs");
    expect(nav()).toBe("7/9");
  });

  it("goes from file to file with the arrow keys in the list, the focus with them", () => {
    const { dialog, key, inView } = open({ path: "README.md" });
    const row = (path: string) => dialog.querySelector(`aside button[title="${path}"]`) as HTMLButtonElement;
    act(() => row("README.md").click());
    act(() => row("README.md").focus());
    key("ArrowDown", row("README.md"));
    expect(inView()).toBe("package-lock.json");
    expect(document.activeElement).toBe(row("package-lock.json"));
    expect(row("package-lock.json").getAttribute("tabindex")).toBe("0");
    key("End", row("package-lock.json"));
    expect(inView()).toBe("guide.md");
    key("ArrowUp", row("guide.md"));
    expect(inView()).toBe("src/old.rs");
    // j and k in the list are its arrows.
    key("k", row("src/old.rs"));
    expect(document.activeElement).toBe(row("src/b.rs"));
    // A click goes to the file too.
    act(() => row("scripts/deploy").click());
    expect(inView()).toBe("scripts/deploy");
  });

  it("wraps lines with w and lays them side by side with s, and keeps the choice", async () => {
    const { dialog, key } = open({ path: "src/a.rs" });
    await settle();
    expect(dialog.querySelector(".diff")?.classList.contains("diff-scroll")).toBe(true);
    key("w");
    expect(dialog.querySelector(".diff")?.classList.contains("diff-wrap")).toBe(true);
    expect(localStorage.getItem("fctl.diff.wrap")).toBe("wrap");
    key("s");
    expect(localStorage.getItem("fctl.diff.layout")).toBe("split");
  });

  it("folds a file to its header, and opens it again", () => {
    const { section } = open({ path: "src/a.rs" });
    const fold = section("src/a.rs")?.querySelector("button[aria-expanded]") as HTMLButtonElement;
    act(() => fold.click());
    expect(fold.getAttribute("aria-expanded")).toBe("false");
    expect(section("src/a.rs")?.querySelector(".diff-code")).toBeNull();
    act(() => fold.click());
    expect(section("src/a.rs")?.querySelector(".diff-code")).not.toBeNull();
  });

  it("filters the list, and Escape clears the filter before it closes", () => {
    const { dialog, onClose, key } = open({ path: "src/a.rs" });
    const input = dialog.querySelector("aside input") as HTMLInputElement;
    type(input, "src");
    const names = () =>
      [...dialog.querySelectorAll("aside button[title]")].map((b) => b.getAttribute("title"));
    expect(names()).toEqual(["src/a.rs", "src/b.rs", "src/old.rs"]);
    const esc = key("Escape", input);
    expect(esc.defaultPrevented).toBe(true);
    expect(input.value).toBe("");
    expect(onClose).not.toHaveBeenCalled();
    // Letters in the filter are its own, not shortcuts.
    expect(key("j", input).defaultPrevented).toBe(false);
  });

  it("finds in every file's lines with Ctrl+F, steps with Enter, and Escape closes the bar first", async () => {
    const { dialog, key, text } = open({ path: "README.md" });
    await settle();
    expect(key("f", dialog, { ctrlKey: true }).defaultPrevented).toBe(true);
    const input = dialog.querySelector("search input") as HTMLInputElement;
    expect(input).not.toBeNull();
    expect(document.activeElement).toBe(input);
    type(input, "app");
    const count = () => dialog.querySelector("#diff-find-count")?.textContent;
    expect(count()).toBe("1 of 2");
    // The list says how many each file has, and dims those without.
    const row = (path: string) => dialog.querySelector(`aside button[title="${path}"]`) as HTMLElement;
    expect(row("README.md").textContent).toContain("1 match");
    expect(row("guide.md").textContent).toContain("1 match");
    expect(row("src/a.rs").className).toContain("opacity-45");
    key("Enter", input);
    expect(count()).toBe("2 of 2");
    key("Enter", input);
    expect(count()).toBe("1 of 2");
    key("Enter", input, { shiftKey: true });
    expect(count()).toBe("2 of 2");
    key("g", input, { ctrlKey: true });
    expect(count()).toBe("1 of 2");
    // Case, whole words and expressions, from the keyboard as from the toggles.
    key("c", input, { altKey: true, code: "KeyC" } as KeyboardEventInit);
    expect(localStorage.getItem("fctl.diff.find.case")).toBe("on");
    type(input, "App");
    expect(count()).toBe("No matches");
    key("r", input, { altKey: true, code: "KeyR" } as KeyboardEventInit);
    type(input, "(");
    expect(count()).toMatch(/group|\)/i);
    expect(input.getAttribute("aria-invalid")).toBe("true");
    key("Escape", input);
    expect(dialog.querySelector("search")).toBeNull();
    expect(dialog.open).toBe(true);
    expect(text()).not.toContain("match");
  });

  it("asks for every diff at once when it doesn't have them, not one by one", async () => {
    api.sessionDiff.mockResolvedValue({ files: diffs });
    const { section } = open({ path: "src/a.rs" }, { cached: false });
    expect(section("src/a.rs")?.querySelector('[aria-busy="true"]')).not.toBeNull();
    await settle();
    await settle();
    expect(api.sessionDiff).toHaveBeenCalledTimes(1);
    expect(api.fileDiff).not.toHaveBeenCalled();
    expect(section("src/a.rs")?.querySelector(".diff-code")?.textContent).toBe("fn a() {");
    expect(section("guide.md")?.querySelector(".diff-code")?.textContent).toBe("# Guide");
  });

  it("says why a file shows no lines", async () => {
    const { section } = open({ path: "README.md" });
    await settle();
    const say = (path: string) => section(path)?.textContent ?? "";
    expect(say("assets/logo.png")).toContain("A binary file");
    const large = say("package-lock.json");
    expect(large).toContain("This diff is too large to show here");
    expect(large).toContain("900 lines added and 300 taken out");
    expect(large).toContain("fctl diff 'web#12' package-lock.json");
    expect(say("scripts/deploy")).toContain("Only its mode changed100644 → 100755: executable now.");
    expect(say("src/b.rs")).toContain("Renamed, its lines as they wereFrom src/b_old.rs.");
    const link = say("scripts/link");
    expect(link).toContain("Mode 100644 → 120000: a symbolic link now");
    expect(link).toContain("target");
    expect(link).not.toContain("A binary file");
    expect(say("src/old.rs")).toContain("its runner runs an older version of fctl");
  });

  it("opens at the first file when the one opened changes no more", () => {
    const { nav, inView } = open({ path: "gone.rs" });
    expect(nav()).toBe("1/9");
    expect(inView()).toBe("README.md");
  });

  it("says what a change of mode means", () => {
    expect(modeChange("100644", "100755")).toBe("100644 → 100755: executable now");
    expect(modeChange("100755", "100644")).toBe("100755 → 100644: no longer executable");
    expect(modeChange("100644", "120000")).toBe("100644 → 120000: a symbolic link now");
    expect(modeChange("120000", "100755")).toBe("120000 → 100755: no longer a symbolic link");
    expect(modeChange("100644", "100664")).toBe("100644 → 100664");
  });

  it("closes with its X", () => {
    const { dialog, onClose } = open({ path: "src/a.rs" });
    act(() => (dialog.querySelector('button[aria-label="Close (Esc)"]') as HTMLButtonElement).click());
    expect(onClose).toHaveBeenCalled();
  });

  it("gives the focus back to what a click opened it from without the keyboard's ring, until it leaves", () => {
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();
    // A click focused it: without the keyboard's ring (which happy-dom gives every focus).
    const matches = opener.matches.bind(opener);
    vi.spyOn(opener, "matches").mockImplementation((s) => s !== ":focus-visible" && matches(s));
    const { dialog } = open({ path: "src/a.rs" });
    expect(opener.dataset.quietFocus).toBe("");
    act(() => dialog.close());
    for (const unmount of roots.splice(0)) unmount();
    opener.focus();
    act(() => opener.blur());
    expect(opener.dataset.quietFocus).toBeUndefined();
    opener.remove();
  });
});
