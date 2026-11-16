import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { Button, buttonClass } from "./button";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(node));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container.querySelector("button") as HTMLButtonElement;
}

describe("Button", () => {
  it("looks disabled while it can't be used, and keeps its look while it loads", () => {
    const idle = render(
      <Button variant="primary" disabled>
        Invite
      </Button>,
    );
    expect(idle.disabled).toBe(true);
    expect(idle.hasAttribute("data-loading")).toBe(false);

    const loading = render(
      <Button variant="primary" loading>
        Invite
      </Button>,
    );
    // Disabled, so that a second click doesn't send it again, but marked as loading, which the
    // disabled look leaves out.
    expect(loading.disabled).toBe(true);
    expect(loading.hasAttribute("data-loading")).toBe(true);
    expect(loading.className).toContain("data-loading:cursor-progress");
  });

  it("gives a disabled primary a neutral look rather than fading its yellow", () => {
    const primary = buttonClass({ variant: "primary" });
    expect(primary).toContain("disabled:not-data-loading:bg-raised");
    expect(primary).toContain("disabled:not-data-loading:text-ink-4");
    expect(primary).not.toContain("disabled:opacity");
  });

  it("responds to hover only while it can be used", () => {
    for (const variant of ["primary", "secondary", "ghost", "danger", "quiet"] as const) {
      const hovers = buttonClass({ variant })
        .split(" ")
        .filter((c) => c.includes("hover:"));
      expect(hovers.length, variant).toBeGreaterThan(0);
      for (const c of hovers) expect(c, variant).toMatch(/^not-disabled:hover:/);
    }
  });
});
