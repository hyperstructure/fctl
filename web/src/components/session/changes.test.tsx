import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { type RepoChanges, repoChanges } from "@/lib/git";
import { Changes } from "./changes";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  vi.restoreAllMocks();
});

/** A branch two commits ahead that changed `n` files, `src/00.rs` on. */
function repo(n: number): RepoChanges {
  const files = Array.from(
    { length: n },
    (_, i): G.FileChange => ({
      path: `src/${String(i).padStart(2, "0")}.rs`,
      status: "modified",
      additions: 2,
      deletions: 1,
    }),
  );
  const [primary] = repoChanges({ ahead: 2, behind: 0, files, uncommitted: 0, pushed: false });
  if (!primary) throw new Error("no primary repository");
  return primary;
}

function changes(r: RepoChanges) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Changes repo={r} showTotals />));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

const paths = (c: HTMLElement) => [...c.querySelectorAll("li")].map((li) => li.textContent?.slice(1));

describe("a repository's changes", () => {
  it("lists the first ten files, and the rest a click away", () => {
    // Folding the list keeps its button in sight.
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((f) => {
      f(0);
      return 0;
    });
    const scrolled = vi.spyOn(Element.prototype, "scrollIntoView").mockImplementation(() => {});
    const c = changes(repo(34));
    expect(paths(c)).toEqual(repo(10).files.map((f) => f.path));
    const button = c.querySelector("button");
    if (!button) throw new Error("no button");
    expect(button.textContent).toBe("Show 24 more files");
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(c.querySelector(`#${CSS.escape(button.getAttribute("aria-controls") ?? "")}`)?.tagName).toBe("UL");
    // The totals count every file, shown or not.
    expect(c.textContent).toContain("+68 −342 commitsnot pushed yet");

    act(() => button.click());
    expect(paths(c)).toEqual(repo(34).files.map((f) => f.path));
    expect(button.textContent).toBe("Show fewer files");
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(scrolled).not.toHaveBeenCalled();

    act(() => button.click());
    expect(paths(c)).toHaveLength(10);
    expect(button.textContent).toBe("Show 24 more files");
    expect(scrolled.mock.contexts).toEqual([button]);
    expect(scrolled).toHaveBeenCalledWith({ block: "nearest" });
  });

  it("lists all of a few files, and eleven: a button in place of one file would save no room", () => {
    for (const n of [1, 10, 11]) {
      const c = changes(repo(n));
      expect(paths(c)).toHaveLength(n);
      expect(c.querySelector("button")).toBeNull();
    }
    expect(paths(changes(repo(12)))).toHaveLength(10);
  });

  it("says when there are no changes yet", () => {
    const c = changes(repo(0));
    expect(c.querySelector("ul")).toBeNull();
    expect(c.textContent).toContain("No changes yet.");
    expect(c.querySelector("button")).toBeNull();
  });
});
