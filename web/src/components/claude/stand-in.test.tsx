import { QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { KeepTokenOut, STAND_IN_LABEL, standInHint } from "./stand-in";

const updateMe = vi.fn((b: G.UpdateMeRequest) =>
  Promise.resolve(me({ claude_stand_in: b.claude_stand_in ?? false })),
);

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, k) => (k === "updateMe" ? updateMe : () => new Promise(() => {})) }),
  ApiFailure: class extends Error {},
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  updateMe.mockClear();
});

const login = { id: "crd_login", kind: "oauth" } as G.ClaudeCredentialInfo;
const token = { id: "crd_token", kind: "setup_token" } as G.ClaudeCredentialInfo;

function me(m: Partial<G.Me> = {}): G.Me {
  return { claude: [token], claude_stand_in: false, ...m } as G.Me;
}

function render(m: G.Me) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <KeepTokenOut me={m} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

describe("keeping your Claude token out of sandboxes", () => {
  it("is off until you turn it on, and says what that means", () => {
    const view = render(me());
    const toggle = view.querySelector<HTMLButtonElement>('[role="switch"]');
    expect(toggle?.getAttribute("aria-label")).toBe(STAND_IN_LABEL);
    expect(toggle?.getAttribute("aria-checked")).toBe("false");
    expect(view.textContent).toContain("Experimental");
    expect(view.textContent).toContain("where their agents can read it");
  });

  it("turns on, and the account says so", async () => {
    const view = render(me());
    await act(async () => view.querySelector<HTMLButtonElement>('[role="switch"]')?.click());
    expect(updateMe).toHaveBeenCalledWith({ claude_stand_in: true });
    expect(queryClient.getQueryData<G.Me>(keys.me)?.claude_stand_in).toBe(true);
  });

  it("says what it does once on, and what a full login gives up", () => {
    expect(standInHint(me({ claude_stand_in: true }))).toContain(
      "The runner puts your token in as each of Claude's requests leaves for the Anthropic API",
    );
    expect(standInHint(me({ claude_stand_in: true }))).not.toContain("Remote Control");
    expect(standInHint(me({ claude_stand_in: true, claude: [token, login] }))).toContain(
      "Remote Control and claude.ai connectors may not work while it's on",
    );
  });
});
