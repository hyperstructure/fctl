import { QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { queryClient } from "@/lib/queries";
import { DEFAULT_POLICY } from "@/lib/session-policy";
import { SessionPolicies } from "./sessions";

const sessionPolicyPreview = vi.fn();
const updateInstanceSettings = vi.fn();
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  api: {
    sessionPolicyPreview: (p: G.SessionPolicy) => sessionPolicyPreview(p),
    updateInstanceSettings: (b: G.UpdateInstanceSettingsRequest) => updateInstanceSettings(b),
  },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
beforeEach(() => {
  sessionPolicyPreview.mockReset();
  updateInstanceSettings.mockReset();
  queryClient.clear();
});
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

const settle = () =>
  act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });

function open(policy: G.SessionPolicy) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <SessionPolicies policy={policy} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const button = (text: string) =>
    [...container.querySelectorAll("button")].find((b) => b.textContent?.trim() === text);
  const choose = async (label: string, text: string) => {
    const list = container.querySelector<HTMLElement>(`[aria-label="${label}"]`);
    await act(async () => list?.click());
    // The list just opened: one opened before may linger as it closes.
    const option = [...document.body.querySelectorAll<HTMLElement>('[role="option"]')].findLast((o) =>
      o.textContent?.startsWith(text),
    );
    expect(option, text).toBeTruthy();
    await act(async () => option?.click());
    // The list flashes the pick, then closes, before it opens again.
    await act(async () => {
      await new Promise((r) => setTimeout(r, 600));
    });
    await settle();
  };
  return { container, button, choose };
}

describe("Admin → Sign-in: staying signed in", () => {
  it("says what it doesn't hold while nothing changes", () => {
    const page = open(DEFAULT_POLICY);
    expect(page.container.textContent).toContain(
      "Admin tokens and service accounts' secrets last as long as their makers chose",
    );
    expect(page.button("Save")).toBeUndefined();
    expect(sessionPolicyPreview).not.toHaveBeenCalled();
  });

  it("shows what a change ends before it saves it", async () => {
    sessionPolicyPreview.mockResolvedValue({
      browsers: 14,
      clis: 0,
      access_tokens: 2,
      member_tokens: 0,
      you: true,
    } satisfies G.SessionPolicyPreview);
    updateInstanceSettings.mockResolvedValue({});
    const page = open(DEFAULT_POLICY);
    await page.choose("Browsers sign in again after", "12 hours");
    expect(sessionPolicyPreview).toHaveBeenLastCalledWith({ ...DEFAULT_POLICY, browser_max_hours: 12 });
    expect(page.container.textContent).toContain(
      "Saving signs out 14 browsers at once, yours among them: their people sign in again.",
    );
    expect(page.container.textContent).toContain(
      "It ends 2 access tokens: scripts and CI that use them need new ones.",
    );
    await act(async () => page.button("Save")?.click());
    expect(updateInstanceSettings).toHaveBeenCalledWith({ ...DEFAULT_POLICY, browser_max_hours: 12 });
  });

  it("waits for what the draft as it is ends before it saves", async () => {
    let answer: (p: G.SessionPolicyPreview) => void = () => {};
    sessionPolicyPreview.mockResolvedValueOnce({
      browsers: 1,
      clis: 0,
      access_tokens: 0,
      member_tokens: 0,
      you: false,
    } satisfies G.SessionPolicyPreview);
    const page = open(DEFAULT_POLICY);
    await page.choose("Browsers sign in again after", "A week");
    expect(page.button("Save")?.disabled).toBe(false);
    // Another change: the first draft's counts show, faded, and Save waits.
    sessionPolicyPreview.mockReturnValueOnce(new Promise((r) => (answer = r)));
    await page.choose("Browsers sign in again after", "12 hours");
    expect(page.container.textContent).toContain("Saving signs out 1 browser at once");
    expect(page.button("Save")?.disabled).toBe(true);
    await act(async () => answer({ browsers: 9, clis: 0, access_tokens: 0, member_tokens: 0, you: false }));
    await settle();
    expect(page.container.textContent).toContain("Saving signs out 9 browsers at once");
    expect(page.button("Save")?.disabled).toBe(false);
  });

  it("puts the saved policy back on cancel", async () => {
    sessionPolicyPreview.mockResolvedValue({
      browsers: 0,
      clis: 0,
      access_tokens: 3,
      member_tokens: 3,
      you: false,
    } satisfies G.SessionPolicyPreview);
    const page = open(DEFAULT_POLICY);
    const off = page.container.querySelector<HTMLButtonElement>("[role=switch]");
    await act(async () => off?.click());
    await settle();
    expect(page.container.textContent).toContain(
      "Only admins make them, and members' stop working: members use the CLI, signed in as themselves.",
    );
    expect(page.container.textContent).toContain("It ends 3 access tokens, all of them members'");
    await act(async () => page.button("Cancel")?.click());
    expect(page.button("Save")).toBeUndefined();
    expect(page.container.textContent).toContain(
      "Everyone makes tokens for scripts, CI and bots in Settings.",
    );
    expect(updateInstanceSettings).not.toHaveBeenCalled();
  });
});
