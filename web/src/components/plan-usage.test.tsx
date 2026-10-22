import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { PlanUsageStrip } from "./plan-usage";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, className }: { children: ReactNode; className?: string }) => (
    <a href="/settings" className={className}>
      {children}
    </a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function strip(usage: G.PlanLimit[]) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(<PlanUsageStrip usage={usage} />));
  roots.push(() => act(() => root.unmount()));
  return container;
}

/** What the strip says under its meters, and in which tone. */
const notice = (c: HTMLElement) => {
  const p = c.querySelector("p");
  return p && { text: p.textContent, tone: p.className };
};

describe("the home page's plan usage", () => {
  const hour = 3_600_000;
  const limit = (window: string, utilization: number, more: Partial<G.PlanLimit> = {}): G.PlanLimit => ({
    window,
    utilization,
    resets_at: Date.now() + hour,
    observed_at: Date.now(),
    ...more,
  });

  it("says nothing of Claude's warning a quarter into the week", () => {
    const c = strip([
      limit("five_hour", 0.12),
      limit("seven_day", 0.25, { status: "allowed_warning", resets_at: Date.now() + 6 * 24 * hour }),
    ]);
    expect(notice(c)).toBeNull();
    expect(c.textContent).toContain("25%");
    expect(c.querySelector(".text-amber, .text-danger")).toBeNull();
  });

  it("warns from 80 %, and in red from 90 %", () => {
    const close = notice(strip([limit("five_hour", 0.31), limit("seven_day", 0.85)]));
    expect(close?.text).toMatch(/^85% of your weekly limit is used\. It resets .+\.$/);
    expect(close?.tone).toBe("text-amber");
    const critical = notice(strip([limit("five_hour", 0.31), limit("seven_day", 0.93)]));
    expect(critical?.text).toMatch(/^93% of your weekly limit is used\./);
    expect(critical?.tone).toBe("text-danger");
  });

  it("says when a limit is used up", () => {
    const reached = notice(strip([limit("five_hour", 1, { status: "rejected" }), limit("seven_day", 0.93)]));
    expect(reached?.text).toMatch(
      /^Your 5-hour limit is used up\. Sessions you start now pause until it resets/,
    );
    expect(reached?.tone).toBe("text-danger");
  });
});
