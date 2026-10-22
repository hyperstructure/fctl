import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { VerifyDialog } from "./verify";

const server = vi.hoisted(() => ({
  verification: vi.fn(),
  verify: vi.fn(),
  verifyPasskeyOptions: vi.fn(),
  confirm: undefined as undefined | (() => Promise<boolean>),
}));
vi.mock("@/lib/api", () => ({
  api: server,
  ApiFailure: class extends Error {},
  setConfirmer: (c: () => Promise<boolean>) => {
    server.confirm = c;
    return () => {
      if (server.confirm === c) server.confirm = undefined;
    };
  },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const status = (methods: G.SecondFactor[]): G.Verification => ({ required: true, verified: false, methods });

const roots: (() => void)[] = [];
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  vi.useRealTimers();
  vi.resetAllMocks();
});

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <VerifyDialog />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const dialog = () => document.querySelector("dialog");
  /** A request that waits for the person to confirm it's them: what they answered. */
  const ask = () => {
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = server.confirm?.();
    });
    return answer;
  };
  const settle = () =>
    act(async () => {
      for (let i = 0; i < 5; i++) await Promise.resolve();
    });
  const enter = (text: string) =>
    act(() => {
      const input = dialog()?.querySelector("input");
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      set?.call(input, text);
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });
  // "Lost your phone? Use a recovery code" is the button for "Use a recovery code".
  const button = (name: string) =>
    [...(dialog()?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.trim().endsWith(name));
  return { dialog, ask, settle, enter, button };
}

describe("confirming it's you", () => {
  it("asks for a code, and answers the request once the code checks out", async () => {
    server.verification.mockResolvedValue(status(["totp", "recovery_code"]));
    server.verify.mockResolvedValue({ ...status(["totp"]), verified: true });
    const s = setup();
    const answer = s.ask();
    await s.settle();
    expect(s.dialog()?.textContent).toContain("Confirm it's you");
    expect(s.dialog()?.textContent).toContain("This takes the code your authenticator app shows.");
    s.enter("123456");
    await s.settle();
    expect(server.verify).toHaveBeenCalledWith({ code: "123456" });
    act(() => vi.runAllTimers());
    await expect(answer).resolves.toBe(true);
  });

  it("answers no when the person cancels, and nothing is sent", async () => {
    server.verification.mockResolvedValue(status(["totp"]));
    const s = setup();
    const answer = s.ask();
    await s.settle();
    act(() => s.button("Cancel")?.click());
    await expect(answer).resolves.toBe(false);
    expect(server.verify).not.toHaveBeenCalled();
  });

  it("answers every request that waits at once", async () => {
    server.verification.mockResolvedValue(status(["totp"]));
    server.verify.mockResolvedValue({ ...status(["totp"]), verified: true });
    const s = setup();
    const first = s.ask();
    const second = s.ask();
    await s.settle();
    expect(document.querySelectorAll("dialog")).toHaveLength(1);
    s.enter("654321");
    await s.settle();
    act(() => vi.runAllTimers());
    await expect(first).resolves.toBe(true);
    await expect(second).resolves.toBe(true);
    expect(server.verify).toHaveBeenCalledTimes(1);
  });

  it("says when this browser can't use the passkey that is all the person has", async () => {
    // happy-dom has no WebAuthn, as a browser without passkeys.
    server.verification.mockResolvedValue(status(["passkey"]));
    const s = setup();
    s.ask();
    await s.settle();
    expect(s.dialog()?.textContent).toContain("this browser can't use passkeys");
    expect(server.verifyPasskeyOptions).not.toHaveBeenCalled();
  });

  it("takes a recovery code instead of the app's", async () => {
    server.verification.mockResolvedValue(status(["totp", "recovery_code"]));
    server.verify.mockResolvedValue({ ...status(["totp"]), verified: true });
    const s = setup();
    const answer = s.ask();
    await s.settle();
    act(() => s.button("Use a recovery code")?.click());
    expect(s.dialog()?.textContent).toContain("This takes one of your recovery codes.");
    s.enter("kftwq xpdmb");
    act(() => s.button("Confirm")?.click());
    await s.settle();
    expect(server.verify).toHaveBeenCalledWith({ code: "KFTWQ-XPDMB" });
    act(() => vi.runAllTimers());
    await expect(answer).resolves.toBe(true);
  });
});
