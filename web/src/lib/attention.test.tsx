import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { clearAttention, setPageTitle, useAttention } from "./attention";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const badge = { setAppBadge: vi.fn(async () => {}), clearAppBadge: vi.fn(async () => {}) };

afterEach(() => {
  vi.unstubAllGlobals();
  badge.setAppBadge.mockClear();
  badge.clearAppBadge.mockClear();
  setPageTitle(undefined);
});

function Count({ n }: { n: number }) {
  useAttention(n);
  return null;
}

function render(n: number) {
  const host = document.createElement("div");
  const root = createRoot(host);
  act(() => root.render(<Count n={n} />));
  return {
    again: (m: number) => act(() => root.render(<Count n={m} />)),
    unmount: () => act(() => root.unmount()),
  };
}

describe("what needs the person, outside the page", () => {
  it("counts in the title, in front of the page's own, and on the app's icon", () => {
    vi.stubGlobal("navigator", { ...navigator, ...badge });
    const shown = render(3);
    expect(document.title).toBe("(3) fctl");
    expect(badge.setAppBadge).toHaveBeenLastCalledWith(3);
    setPageTitle("Add Danish to the shop");
    expect(document.title).toBe("(3) Add Danish to the shop");
    shown.again(0);
    expect(document.title).toBe("Add Danish to the shop");
    expect(badge.clearAppBadge).toHaveBeenCalled();
    shown.unmount();
  });

  it("goes with the sidebar, and as the person signs out", () => {
    vi.stubGlobal("navigator", { ...navigator, ...badge });
    const shown = render(2);
    expect(document.title).toBe("(2) fctl");
    shown.unmount();
    expect(document.title).toBe("fctl");
    expect(badge.clearAppBadge).toHaveBeenCalledTimes(1);

    // Signing out leaves the page without unmounting anything.
    render(4);
    expect(document.title).toBe("(4) fctl");
    clearAttention();
    expect(document.title).toBe("fctl");
    expect(badge.clearAppBadge).toHaveBeenCalledTimes(2);
  });

  it("says nothing where browsers have no badges", () => {
    const shown = render(1);
    expect(document.title).toBe("(1) fctl");
    shown.unmount();
    expect(document.title).toBe("fctl");
  });
});
