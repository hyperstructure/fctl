import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiFailure } from "@/lib/api";
import { loadSent, saveSent } from "@/lib/early-access";
import { EarlyAccessPage } from "./early-access";

const requestAccess = vi.fn();
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  api: {
    requestAccess: (b: unknown) => requestAccess(b),
    instance: () => new Promise(() => {}),
  },
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/login">{children}</a>,
}));
// The foundry behind the page, without WebGL: what the page asks of its threads.
const lanes = vi.fn();
const pulse = vi.fn();
vi.mock("@hyperstructure/design/gl/foundry", () => ({
  foundry: () => ({
    scene: {},
    setLanes: (m: Map<number, unknown>) => lanes(new Map(m)),
    pulse: (lane: number, weight: number) => pulse(lane, weight),
    pointer() {},
    pointerLeave() {},
  }),
}));
vi.mock("@/components/ui/gl-canvas", async () => {
  const { useEffect } = await import("react");
  return {
    GLCanvas: ({ scene }: { scene: () => unknown }) => {
      useEffect(() => {
        scene();
      }, [scene]);
      return null;
    },
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
beforeEach(() => {
  requestAccess.mockReset();
  lanes.mockReset();
  pulse.mockReset();
});
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  localStorage.clear();
});

function open() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={new QueryClient()}>
        <EarlyAccessPage />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const type = (text: string) =>
    act(() => {
      const input = container.querySelector("input");
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });
  const submit = () =>
    act(async () => {
      container
        .querySelector("form")
        ?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
  /** The last lit lanes, by number. */
  const lit = () => [...((lanes.mock.lastCall?.[0] as Map<number, { lit: number }>) ?? new Map()).keys()];
  return { container, type, submit, lit };
}

describe("asking for early access", () => {
  it("puts them on the list, lights their thread and remembers it", async () => {
    requestAccess.mockResolvedValue({ ok: true });
    const page = open();
    expect(page.container.textContent).toContain("Request early access.");
    expect(page.lit()).toEqual([]);
    page.type("Ada@Example.com ");
    // Their thread shows once what they type could be an address.
    expect(page.lit()).toHaveLength(1);
    await page.submit();
    expect(requestAccess).toHaveBeenCalledWith({ email: "Ada@Example.com" });
    expect(page.container.textContent).toContain("You're on the list.");
    expect(page.container.textContent).toContain("ada@example.com");
    expect(page.container.textContent).toContain("While you wait, have these ready");
    expect(pulse).toHaveBeenCalledTimes(1);
    expect(page.lit()).toHaveLength(1);
    expect(loadSent()?.email).toBe("ada@example.com");
  });

  it("says what was wrong with the address, under the field", async () => {
    requestAccess.mockRejectedValue(
      new ApiFailure(400, { code: "validation", field: "email", message: "Enter a valid email address" }),
    );
    const page = open();
    page.type("ada@example");
    await page.submit();
    expect(page.container.querySelector("[role=alert]")?.textContent).toBe("Enter a valid email address");
    expect(page.container.querySelector("input")?.getAttribute("aria-invalid")).toBe("true");
    expect(page.container.textContent).not.toContain("You're on the list.");
    expect(pulse).not.toHaveBeenCalled();
    expect(loadSent()).toBeUndefined();
  });

  it("says they are on the list when they come back, and lets them ask with another address", () => {
    saveSent({ email: "ada@example.com", at: Date.now() - 86_400_000 });
    const page = open();
    expect(page.container.textContent).toContain("You're on the list.");
    expect(page.container.textContent).toContain("ada@example.com");
    // Their thread is lit already, with no packet: that was when they asked.
    expect(page.lit()).toHaveLength(1);
    expect(pulse).not.toHaveBeenCalled();
    const another = [...page.container.querySelectorAll("button")].find(
      (b) => b.textContent === "Ask with another address",
    );
    act(() => another?.click());
    expect(page.container.textContent).toContain("Request early access.");
    expect(loadSent()).toBeUndefined();
  });
});
