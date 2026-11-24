import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { LoginPage } from "./auth";

const login = vi.fn();
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  api: {
    login: (b: unknown) => login(b),
    instance: () => new Promise(() => {}),
    passkeySignInOptions: () => new Promise(() => {}),
  },
}));
const search = vi.hoisted(() => ({}) as Record<string, string | undefined>);
vi.mock("@tanstack/react-router", () => ({
  useSearch: () => search,
  useNavigate: () => () => {},
  Link: ({ children }: { children: ReactNode }) => <a href="/early-access">{children}</a>,
}));
const went = vi.hoisted(() => ({ to: [] as string[] }));
vi.mock("@/lib/history", async (original) => ({
  ...(await original<typeof import("@/lib/history")>()),
  goTo: (to: string) => went.to.push(to),
}));
// The scene behind the page, without WebGL.
vi.mock("@/components/ui/gl-canvas", () => ({ GLCanvas: () => null }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const google: G.SignInProvider = { id: "google", name: "Google", start: "/api/v1/auth/google/start" };

function instance(rules: G.SignInRule[], self_sign_up = true): G.InstanceInfo {
  return {
    name: "Acme factory",
    version: "1.0.0",
    setup_required: false,
    features: { google: true, email: true, github: true, publish_screenshots: false, early_access: false },
    sign_in: { providers: [google], rules, self_sign_up },
  };
}

const roots: (() => void)[] = [];
beforeEach(() => {
  login.mockReset();
  went.to.length = 0;
  for (const k of Object.keys(search)) delete search[k];
});
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function open(info: G.InstanceInfo) {
  const client = new QueryClient();
  client.setQueryData(keys.instance, info);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <LoginPage />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const type = (text: string) =>
    act(() => {
      const input = container.querySelector<HTMLInputElement>("input[type=email]");
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });
  const submit = () =>
    act(async () => {
      container
        .querySelector("form")
        ?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
  const password = () => container.querySelector("input[type=password]");
  const button = (text: string) =>
    [...container.querySelectorAll("button")].find((b) => b.textContent?.trim().startsWith(text));
  return { container, type, submit, password, button };
}

describe("the sign-in page where a domain signs in with a provider", () => {
  const acme = instance([{ domains: ["acme.com", "acme.dev"], provider: "google", passkeys: false }]);

  it("says which domains sign in with it, under its button", () => {
    const page = open(acme);
    expect(page.container.textContent).toContain("acme.com and acme.dev sign in with Google.");
    expect(page.container.querySelector("a[href='/api/v1/auth/google/start?intent=login']")).not.toBeNull();
  });

  it("turns the form into the provider's button for an address there, and sends them on to it", async () => {
    const page = open(acme);
    page.type("ada@acme.dev");
    expect(page.password()).toBeNull();
    const via = page.container.querySelector("button[type=submit]");
    expect(via?.getAttribute("aria-label")).toBe("Continue with Google as ada@acme.dev");
    expect(page.container.textContent).toContain("acme.com and acme.dev sign in with Google");
    await page.submit();
    expect(login).not.toHaveBeenCalled();
    expect(went.to).toEqual(["/api/v1/auth/google/start?intent=login&login_hint=ada%40acme.dev"]);
  });

  it("keeps the password for other addresses", async () => {
    login.mockReturnValue(new Promise(() => {}));
    const page = open(acme);
    page.type("cy@contractor.io");
    expect(page.password()).not.toBeNull();
    await page.submit();
    expect(login).toHaveBeenCalledWith({ email: "cy@contractor.io", password: "" });
  });

  it("lets the primary instance owner sign in with their password, and says only theirs does", () => {
    const page = open(acme);
    page.type("ada@acme.com");
    act(() => page.button("Primary instance owner?")?.click());
    expect(page.password()).not.toBeNull();
    expect(page.container.textContent).toContain(
      "Only the primary instance owner's password signs in at these domains, for when Google is down.",
    );
    act(() => page.button("Continue with Google instead")?.click());
    expect(page.password()).toBeNull();
  });

  it("names the domains to sign in as when Google came back with another account", () => {
    search.error = "google_wrong_account";
    search.domains = "acme.com";
    const page = open(acme);
    expect(page.container.textContent).toContain(
      "Your account signs in with Google as your acme.com address.",
    );
  });

  it("tells people without an account where accounts come from, where signing in makes none", () => {
    const page = open(instance([], false));
    expect(page.container.textContent).toContain("Need an account? Your organization makes them here");
    // Without rules, the form is as it was.
    page.type("ada@acme.com");
    expect(page.password()).not.toBeNull();
  });
});
