import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { byDirectory, DiffDialog, type DiffGroup, type DiffTarget } from "./diff-dialog";

// Diffs come from the cache; the files beside the one shown load ahead, and never come.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: pending, ApiFailure: class extends Error {} };
});
vi.mock("@/lib/highlight", () => ({ tokenize: () => Promise.resolve(undefined) }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  localStorage.clear();
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
  diff("src/a.rs", { patch: "@@ -1,2 +1,2 @@\n fn a() {\n-    1\n+    2" }),
  diff("src/b.rs", { status: "renamed", old_path: "src/b_old.rs" }),
  diff("guide.md", { repo: "acme/docs", status: "added", patch: "@@ -0,0 +1,2 @@\n+# Guide\n+Hi" }),
];

function open(at: DiffTarget) {
  const client = new QueryClient();
  for (const g of groups) {
    for (const f of g.files) {
      const d = diffs.find((x) => x.path === f.path && x.repo === g.repo);
      if (d) client.setQueryData(keys.fileDiff("ses_1", g.repo ?? "", f.path, f.digest ?? ""), d);
    }
  }
  const onSelect = vi.fn();
  const onClose = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = (t: DiffTarget) =>
    act(() =>
      root.render(
        <QueryClientProvider client={client}>
          <DiffDialog
            session="ses_1"
            cliRef="web#12"
            groups={groups}
            current={t}
            onSelect={onSelect}
            onClose={onClose}
          />
        </QueryClientProvider>,
      ),
    );
  render(at);
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  // The newest: a test may open several.
  const dialog = [...document.querySelectorAll("dialog")].pop() as HTMLDialogElement;
  const key = (k: string, target: Element = dialog) =>
    act(() => {
      target.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true }));
    });
  const text = () => dialog.textContent ?? "";
  return { dialog, onSelect, onClose, render, key, text };
}

const settle = () => act(() => new Promise((r) => setTimeout(r, 0)));

describe("a changed file's diff over the page", () => {
  it("shows the file's lines and how it changed, its place among the others", async () => {
    const { dialog } = open({ path: "src/a.rs" });
    await settle();
    expect(dialog.open).toBe(true);
    expect(dialog.querySelector("h2")?.textContent).toBe("src/a.rs");
    expect(dialog.querySelector("header")?.textContent).toContain("Modified+1 −1Rust");
    expect(dialog.querySelector("nav")?.textContent).toContain("5/8");
    expect([...dialog.querySelectorAll(".diff-code")].map((c) => c.textContent)).toEqual([
      "fn a() {",
      "    1",
      "    2",
    ]);
  });

  it("lists the changed files of each repository under their directories, the one shown marked", () => {
    const { dialog } = open({ path: "src/a.rs" });
    const list = dialog.querySelector("aside") as HTMLElement;
    const lines = [...list.querySelectorAll("ul ul > li, ul > li > div")].map((e) => e.textContent?.trim());
    expect(lines).toContain("assets");
    expect(lines).toContain("src");
    expect(list.textContent).toContain("acme/docs");
    const current = list.querySelector('[aria-current="true"]');
    expect(current?.textContent).toContain("a.rs");
    expect(current?.getAttribute("title")).toBe("src/a.rs");
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

  it("steps to the next and previous files with j and k, across repositories", () => {
    const { key, onSelect, render } = open({ path: "src/old.rs" });
    key("j");
    expect(onSelect).toHaveBeenLastCalledWith({ repo: "acme/docs", path: "guide.md" });
    key("k");
    expect(onSelect).toHaveBeenLastCalledWith({ repo: undefined, path: "src/b.rs" });
    // From the last file there is none further.
    render({ repo: "acme/docs", path: "guide.md" });
    onSelect.mockClear();
    key("j");
    expect(onSelect).not.toHaveBeenCalled();
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

  it("filters the list, and Escape clears the filter before it closes", () => {
    const { dialog, onClose } = open({ path: "src/a.rs" });
    const input = dialog.querySelector("input") as HTMLInputElement;
    act(() => {
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      set?.call(input, "src");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const names = () =>
      [...dialog.querySelectorAll("aside button[title]")].map((b) => b.getAttribute("title"));
    expect(names()).toEqual(["src/a.rs", "src/b.rs", "src/old.rs"]);
    const esc = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    act(() => {
      input.dispatchEvent(esc);
    });
    expect(esc.defaultPrevented).toBe(true);
    expect(input.value).toBe("");
    expect(onClose).not.toHaveBeenCalled();
    // Letters in the filter are its own, not shortcuts.
    const j = new KeyboardEvent("keydown", { key: "j", bubbles: true, cancelable: true });
    act(() => {
      input.dispatchEvent(j);
    });
    expect(j.defaultPrevented).toBe(false);
  });

  it("says why it shows no lines", async () => {
    const say = async (path: string, repo?: string) => {
      const { text } = open({ path, repo });
      await settle();
      return text();
    };
    expect(await say("assets/logo.png")).toContain("A binary file");
    const large = await say("package-lock.json");
    expect(large).toContain("This diff is too large to show here");
    expect(large).toContain("900 lines added and 300 taken out");
    expect(large).toContain("fctl diff web#12 package-lock.json");
    const mode = await say("scripts/deploy");
    expect(mode).toContain("Only its mode changed100644 → 100755: executable now.");
    expect(await say("src/b.rs")).toContain("Renamed, its lines as they wereFrom src/b_old.rs.");
    expect(await say("src/old.rs")).toContain("its runner runs an older version of fctl");
    expect(await say("gone.rs")).toContain("No changes in this file any more");
  });

  it("closes with its X", () => {
    const { dialog, onClose } = open({ path: "src/a.rs" });
    act(() => (dialog.querySelector('button[aria-label="Close (Esc)"]') as HTMLButtonElement).click());
    expect(onClose).toHaveBeenCalled();
  });
});
