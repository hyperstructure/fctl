import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FloorLink, laneOf, litLanes } from "./floor-link";

// The conduit without WebGL: what the row asks of it.
const { calls, pulses } = vi.hoisted(() => ({
  calls: [] as unknown[][],
  pulses: new Set<(p: { sessionId: string; kind: string; weight: number }) => void>(),
}));
vi.mock("@hyperstructure/design/gl/conduit", () => ({
  LANES: 8,
  DISSOLVE_MS: 220,
  conduit: () => ({
    scene: {},
    show: (on: boolean) => calls.push(["show", on]),
    setLanes: (m: Map<number, unknown>) => calls.push(["lanes", [...m.keys()]]),
    pulse: (lane: number, weight: number) => calls.push(["pulse", lane, weight]),
    burst: () => calls.push(["burst"]),
    pointer: () => calls.push(["pointer"]),
    pointerLeave: () => calls.push(["pointerLeave"]),
  }),
}));
// Like the real one, it makes its scene once, when it mounts.
vi.mock("@/components/ui/gl-canvas", async () => {
  const { useEffect, useState } = await import("react");
  return {
    GLCanvas: ({ scene }: { scene: () => unknown }) => {
      const [make] = useState(() => scene);
      useEffect(() => {
        make();
      }, [make]);
      return <canvas />;
    },
  };
});
vi.mock("@/lib/feed", () => ({
  feed: {
    onPulse: (fn: (p: { sessionId: string; kind: string; weight: number }) => void) => {
      pulses.add(fn);
      return () => pulses.delete(fn);
    },
  },
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children, ...props }: { to: string; children: ReactNode }) => (
    <a href={to} {...props}>
      {children}
    </a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
beforeEach(() => {
  calls.length = 0;
});
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  vi.useRealTimers();
});

function row(working = 0) {
  const onClick = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = (n: number) =>
    act(() => root.render(<FloorLink active={false} working={n} onClick={onClick} />));
  render(working);
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const link = container.querySelector("a");
  if (!link) throw new Error("no link");
  // React makes pointerenter and pointerleave from pointerover and pointerout.
  const enter = (pointerType: string) =>
    act(() => {
      link.dispatchEvent(new PointerEvent("pointerover", { bubbles: true, pointerType }));
    });
  const leave = () =>
    act(() => {
      link.dispatchEvent(
        new PointerEvent("pointerout", { bubbles: true, pointerType: "mouse", relatedTarget: document.body }),
      );
    });
  const pulse = (sessionId: string, kind = "tool", weight = 100) =>
    act(() => {
      for (const fn of pulses) fn({ sessionId, kind, weight });
    });
  return { link, onClick, enter, leave, pulse, render, scene: () => link.querySelector("canvas") };
}

describe("the Agent Floor's row", () => {
  it("links to the Agent Floor", () => {
    const { link } = row();
    expect(link.getAttribute("href")).toBe("/");
    expect(link.textContent).toBe("Agent Floor");
  });

  it("powers on under a mouse, and stays a plain row under a finger", () => {
    const { link, enter, scene } = row(2);
    enter("touch");
    expect(scene()).toBeNull();
    expect(link.dataset.lit).toBeUndefined();
    enter("mouse");
    expect(scene()).not.toBeNull();
    expect(link.dataset.lit).toBe("true");
    // Made with the threads of the sessions at work, then powered on.
    expect(calls).toEqual([
      ["lanes", [2, 6]],
      ["show", true],
    ]);
  });

  it("dissolves when the pointer leaves, then goes out of the page", () => {
    vi.useFakeTimers();
    const { link, enter, leave, scene } = row();
    enter("mouse");
    leave();
    expect(calls.slice(-2)).toEqual([["pointerLeave"], ["show", false]]);
    expect(link.dataset.lit).toBeUndefined();
    // Still drawing while it dissolves.
    expect(scene()?.parentElement?.classList).not.toContain("hidden");
    act(() => {
      vi.advanceTimersByTime(300);
    });
    expect(scene()?.parentElement?.classList).toContain("hidden");
    // Back under the pointer, it shows again, made once.
    enter("mouse");
    expect(scene()?.parentElement?.classList).not.toContain("hidden");
    expect(calls.filter((c) => c[0] === "lanes")).toHaveLength(1);
    expect(calls.at(-1)).toEqual(["show", true]);
  });

  it("lights a thread for each session at work and sends what their agents do down them", () => {
    const { enter, pulse, render } = row(3);
    enter("mouse");
    pulse("ses_a", "tool", 300);
    const lane = laneOf("ses_a", [1, 4, 6]);
    expect(calls.at(-1)).toEqual(["pulse", lane, 300]);
    // Text as it streams is no packet, as on the floor itself.
    pulse("ses_a", "delta");
    expect(calls.at(-1)?.[0]).toBe("pulse");
    expect(calls.filter((c) => c[0] === "pulse")).toHaveLength(1);
    render(1);
    expect(calls.at(-1)).toEqual(["lanes", [4]]);
  });

  it("sends a burst when it is clicked", () => {
    const { link, onClick, enter } = row();
    enter("mouse");
    act(() => link.click());
    expect(calls.at(-1)).toEqual(["burst"]);
    expect(onClick).toHaveBeenCalled();
  });
});

describe("litLanes", () => {
  it("spreads the threads of sessions at work over the bar", () => {
    expect(litLanes(0)).toEqual([]);
    expect(litLanes(1)).toEqual([4]);
    expect(litLanes(3)).toEqual([1, 4, 6]);
    // As many as there are lanes, at most.
    expect(litLanes(20)).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
  });
});

describe("laneOf", () => {
  it("sends a session's packets down the same lit thread each time", () => {
    const lanes = [1, 4, 6];
    const lane = laneOf("ses_01M5JHQREMSNJFWWHA6CJ783Q8", lanes);
    expect(lanes).toContain(lane);
    expect(laneOf("ses_01M5JHQREMSNJFWWHA6CJ783Q8", lanes)).toBe(lane);
    expect(laneOf("ses_a", [])).toBeUndefined();
  });
});
