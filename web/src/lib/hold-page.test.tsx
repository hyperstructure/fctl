import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { useHoldPage } from "./hold-page";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: Root[] = [];
afterEach(() => {
  for (const r of roots.splice(0)) act(() => r.unmount());
  document.documentElement.removeAttribute("style");
});

function Holds() {
  useHoldPage();
  return null;
}

/** Something over the page that holds it: `let go` unmounts it. */
function hold() {
  const root = createRoot(document.createElement("div"));
  roots.push(root);
  act(() => root.render(<Holds />));
  return () => act(() => root.unmount());
}

/** One over another: the outer one's effects let go before the inner one's when both go at once. */
function Nested({ inner }: { inner: boolean }) {
  useHoldPage();
  return inner ? <Holds /> : null;
}

describe("useHoldPage", () => {
  const html = document.documentElement;

  it("holds the page while it is there, then gives back its own style", () => {
    html.style.overflow = "clip";
    const letGo = hold();
    expect(html.style.overflow).toBe("hidden");
    letGo();
    expect(html.style.overflow).toBe("clip");
  });

  it("lets go once the last of several does, whatever the order", () => {
    const first = hold();
    const second = hold();
    first();
    expect(html.style.overflow).toBe("hidden");
    second();
    expect(html.style.overflow).toBe("");
  });

  it("lets go when one over another goes with it", () => {
    // A dialog opened from a sheet, when the page goes: React lets go of the sheet first.
    const root = createRoot(document.createElement("div"));
    roots.push(root);
    act(() => root.render(<Nested inner={false} />));
    act(() => root.render(<Nested inner />));
    expect(html.style.overflow).toBe("hidden");
    act(() => root.render(null));
    expect(html.style.overflow).toBe("");
  });
});
