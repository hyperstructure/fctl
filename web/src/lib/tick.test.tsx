import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useTickAt } from "./tick";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const DAY = 86_400_000;
let renders = 0;

function Clock({ at }: { at?: number }) {
  useTickAt(at);
  renders += 1;
  return null;
}

describe("useTickAt", () => {
  let unmount: () => void = () => {};
  beforeEach(() => {
    vi.useFakeTimers({ now: Date.UTC(2026, 10, 7, 12) });
    renders = 0;
  });
  afterEach(() => {
    unmount();
    vi.useRealTimers();
  });

  function mount(at?: number) {
    const root = createRoot(document.createElement("div"));
    act(() => root.render(<Clock at={at} />));
    unmount = () => act(() => root.unmount());
  }

  it("renders again once the time has passed", () => {
    mount(Date.now() + 5 * 60_000);
    expect(renders).toBe(1);
    act(() => vi.advanceTimersByTime(4 * 60_000));
    expect(renders).toBe(1);
    act(() => vi.advanceTimersByTime(2 * 60_000));
    expect(renders).toBe(2);
  });

  it("waits out times further than one timer reaches", () => {
    // 30 days: a timer stops at about 24.
    mount(Date.now() + 30 * DAY);
    act(() => vi.advanceTimersByTime(25 * DAY));
    expect(renders).toBe(1);
    act(() => vi.advanceTimersByTime(5 * DAY + 1000));
    expect(renders).toBe(2);
  });

  it("waits for nothing that has passed", () => {
    mount(Date.now() - 1000);
    mount(undefined);
    act(() => vi.advanceTimersByTime(DAY));
    expect(renders).toBe(2);
  });
});
