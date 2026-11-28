import { QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { queryClient } from "@/lib/queries";
import { DEFAULT_POLICY } from "@/lib/session-policy";
import { Networks, networksLead, parseNetworks } from "./networks";

const updateInstanceSettings = vi.fn();
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  api: { updateInstanceSettings: (b: G.UpdateInstanceSettingsRequest) => updateInstanceSettings(b) },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
beforeEach(() => {
  updateInstanceSettings.mockReset();
  queryClient.clear();
});
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

const overview = (networks: string[]): G.SignInOverview => ({
  providers: [],
  rules: [],
  coverage: [],
  self_sign_up: true,
  active: 3,
  passwords: 3,
  passkeys: 0,
  no_way_in: 0,
  email: false,
  sessions: DEFAULT_POLICY,
  networks,
  address: "203.0.113.9",
});

function open(o: G.SignInOverview) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <Networks overview={o} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const button = (text: string) =>
    [...container.querySelectorAll("button")].find((b) => b.textContent?.trim() === text);
  return { container, button };
}

describe("Admin → Sign-in: networks", () => {
  it("reads ranges as typed, and says where people sign in from", () => {
    expect(parseNetworks("203.0.113.0/24\n 2001:db8::/32, 10.0.0.1")).toEqual([
      "203.0.113.0/24",
      "2001:db8::/32",
      "10.0.0.1",
    ]);
    expect(networksLead([])).toBe("People sign in and send requests from anywhere.");
    expect(networksLead(["203.0.113.0/24", "2001:db8::/32"])).toBe(
      "People sign in and send requests only from 203.0.113.0/24 and 2001:db8::/32. The primary instance owner signs in from anywhere, in a browser or the CLI.",
    );
  });

  it("adds the admin's own address, and saves the ranges", async () => {
    updateInstanceSettings.mockResolvedValue({ networks: ["203.0.113.9/32"] });
    const page = open(overview([]));
    expect(page.container.textContent).toContain("Your address: 203.0.113.9");
    expect(page.button("Save")).toBeUndefined();
    await act(async () => page.button("Add it")?.click());
    expect(page.container.querySelector("textarea")?.value).toBe("203.0.113.9");
    await act(async () => page.button("Save")?.click());
    expect(updateInstanceSettings).toHaveBeenCalledWith({ networks: ["203.0.113.9"] });
  });

  it("shows why the instance refused them", async () => {
    updateInstanceSettings.mockRejectedValue(
      Object.assign(new Error("Your own address, 203.0.113.9, isn't in these networks"), {
        field: undefined,
      }),
    );
    const page = open(overview(["10.0.0.0/8"]));
    const area = page.container.querySelector("textarea");
    expect(area?.value).toBe("10.0.0.0/8");
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(
        area,
        "192.0.2.0/24",
      );
      area?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => page.button("Save")?.click());
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(page.container.textContent).toContain("Your own address, 203.0.113.9, isn't in these networks");
  });
});
