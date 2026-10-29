import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { AuthenticatorSetup } from "./setup";

const server = vi.hoisted(() => ({ confirmAuthenticator: vi.fn(), startAuthenticator: vi.fn() }));
vi.mock("@/lib/api", async () => {
  const real = await vi.importActual<typeof import("@/lib/api")>("@/lib/api");
  return { ...real, api: server };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  vi.resetAllMocks();
});

const started: G.AuthenticatorSetup = {
  secret: "JBSWY3DPEHPK3PXP",
  uri: "otpauth://totp/fctl:ada%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=fctl",
  qr: ["1"],
  setup: "v1.sealed",
  expires_at: Date.now() + 15 * 60_000,
  password_required: false,
};

const me = { user: { id: "usr_ada", email: "ada@example.com", name: "Ada" }, emails: [] } as unknown as G.Me;

describe("setting up an authenticator app", () => {
  it("says why a confirmation was refused when the form has no field for it", async () => {
    const { ApiFailure } = await vi.importActual<typeof import("@/lib/api")>("@/lib/api");
    server.confirmAuthenticator.mockRejectedValue(
      new ApiFailure(422, {
        code: "validation",
        field: "sign_in",
        message: "Setting up two-factor sign-in takes a sign-in from the last 15 minutes.",
      }),
    );
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const container = document.createElement("div");
    const root = createRoot(container);
    act(() =>
      root.render(
        <QueryClientProvider client={client}>
          <AuthenticatorSetup me={me} started={started} onDone={() => {}} />
        </QueryClientProvider>,
      ),
    );
    roots.push(() => act(() => root.unmount()));
    const input = container.querySelector<HTMLInputElement>('input[autocomplete="one-time-code"]');
    if (!input) throw new Error("no code input");
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, "123456");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {});
    expect(server.confirmAuthenticator).toHaveBeenCalled();
    expect(container.textContent).toContain("takes a sign-in from the last 15 minutes");
  });
});
