import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useMedia } from "./media";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  vi.unstubAllGlobals();
});

function Says({ query, fallback }: { query: string; fallback?: boolean }) {
  return <>{String(useMedia(query, fallback))}</>;
}

function render(query: string, fallback?: boolean) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(<Says query={query} fallback={fallback} />));
  return {
    text: () => container.textContent,
    unmount: () => act(() => root.unmount()),
  };
}

describe("useMedia", () => {
  it("says what the query says, again as the window changes", () => {
    let wide = true;
    const listeners = new Set<() => void>();
    vi.stubGlobal("matchMedia", (media: string) => ({
      media,
      get matches() {
        return wide;
      },
      addEventListener: (_: string, fn: () => void) => listeners.add(fn),
      removeEventListener: (_: string, fn: () => void) => listeners.delete(fn),
    }));
    const r = render("(min-width: 48rem)");
    expect(r.text()).toBe("true");
    wide = false;
    act(() => {
      for (const fn of listeners) fn();
    });
    expect(r.text()).toBe("false");
    r.unmount();
    expect(listeners.size).toBe(0);
  });

  it("falls back where the browser can't say", () => {
    vi.stubGlobal("matchMedia", undefined);
    expect(render("(min-width: 48rem)").text()).toBe("true");
    expect(render("(min-width: 48rem)", false).text()).toBe("false");
  });
});
