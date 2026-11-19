import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { PathText, splitPath } from "./path";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("splitPath", () => {
  it("keeps the slash with the directory", () => {
    expect(splitPath("web/src/components/session/tool.tsx")).toEqual({
      dir: "web/src/components/session/",
      name: "tool.tsx",
    });
    expect(splitPath("/etc/hosts")).toEqual({ dir: "/etc/", name: "hosts" });
  });

  it("has no directory for a name on its own", () => {
    expect(splitPath("README.md")).toEqual({ dir: "", name: "README.md" });
  });
});

describe("PathText", () => {
  let host: HTMLElement | undefined;
  afterEach(() => host?.remove());

  const render = (path: string) => {
    host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    act(() => root.render(<PathText path={path} data-find="line:k1" nameClassName="text-ink-3" />));
    const line = host.firstElementChild as HTMLElement;
    return { line, parts: [...line.children] as HTMLElement[] };
  };

  it("shows the directory dimmer, before the name", () => {
    const { line, parts } = render("crates/fctl-control/src/api/admin/reset_link.rs");
    expect(line.dataset.find).toBe("line:k1");
    expect(parts.map((p) => p.textContent)).toEqual(["crates/fctl-control/src/api/admin/", "reset_link.rs"]);
    expect(parts[0]?.className).toContain("text-ink-4");
    expect(parts[1]?.className).toContain("text-ink-3");
  });

  it("lets only the directory shrink: the name is capped at the line instead", () => {
    // Given any share of the shrinking, Safari took a fraction of a pixel from the name and drew its
    // ellipsis over its last letters ("tool.t…").
    const { parts } = render("web/src/components/session/tool.tsx");
    expect(parts[0]?.classList).toContain("min-w-0");
    expect(parts[1]?.classList).toContain("shrink-0");
    expect(parts[1]?.classList).toContain("max-w-full");
    expect(parts[1]?.classList).toContain("truncate");
  });

  it("shows a name on its own without a directory", () => {
    const { parts } = render("README.md");
    expect(parts.map((p) => p.textContent)).toEqual(["README.md"]);
  });
});
