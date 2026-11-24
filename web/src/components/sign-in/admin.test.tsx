import { type QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { SignInAdmin } from "./admin";

const signInPreview = vi.fn();
const updateInstanceSettings = vi.fn();
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  api: {
    signIn: () => new Promise(() => {}),
    signInPreview: (rules: G.SignInRule[]) => signInPreview(rules),
    updateInstanceSettings: (b: G.UpdateInstanceSettingsRequest) => updateInstanceSettings(b),
    me: () => new Promise(() => {}),
    instance: () => new Promise(() => {}),
  },
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/settings">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const google: G.SignInProviderStatus = {
  id: "google",
  name: "Google",
  configured: true,
  setup: "GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET",
  redirect_url: "https://fctl.acme.dev/api/v1/auth/google/callback",
  people: 12,
};

const person = (name: string): G.SignInPerson => ({
  id: name.toLowerCase(),
  name,
  email: `${name.toLowerCase()}@acme.com`,
});

const overview = (fields: Partial<G.SignInOverview> = {}): G.SignInOverview => ({
  providers: [google],
  rules: [],
  coverage: [],
  self_sign_up: true,
  active: 52,
  passwords: 38,
  passkeys: 9,
  no_way_in: 3,
  owner: person("Ada"),
  method: "password",
  email: true,
  ...fields,
});

const me = {
  user: { id: "bob", email: "bob@acme.com", name: "Bob", role: "admin" },
  identities: [{ id: "idn_1", provider: "google", email: "bob@acme.com", created_at: 0 }],
} as unknown as G.Me;

const roots: (() => void)[] = [];
beforeEach(() => {
  signInPreview.mockReset();
  updateInstanceSettings.mockReset();
  queryClient.clear();
});
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function open(o: G.SignInOverview) {
  // The tab's dialogs use the app's client, as the page's queries do.
  const client: QueryClient = queryClient;
  client.setQueryData(keys.signIn, o);
  client.setQueryData(keys.me, me);
  client.setQueryData<G.InstanceInfo>(keys.instance, {
    name: "Acme factory",
    version: "1.0.0",
    setup_required: false,
    features: { google: true, email: true, github: true, publish_screenshots: false, early_access: false },
    sign_in: {
      providers: [{ id: "google", name: "Google", start: "/api/v1/auth/google/start" }],
      rules: o.rules,
      self_sign_up: o.self_sign_up,
    },
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <SignInAdmin save={() => {}} saving={false} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const button = (text: string) =>
    [...document.querySelectorAll("button")].find((b) => b.textContent?.trim().startsWith(text));
  const type = async (text: string) => {
    await act(async () => {
      const input = document.querySelector<HTMLInputElement>("dialog input");
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    // What was typed settles, then the preview comes.
    await act(async () => {
      await new Promise((r) => setTimeout(r, 300));
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
  };
  return { container, button, type };
}

const preview = (fields: Partial<G.SignInPreview> = {}): G.SignInPreview => ({
  covered: 42,
  passwords: [person("Cy"), person("Dee")],
  password_count: 38,
  passkeys: [],
  passkey_count: 0,
  access_tokens: 0,
  admin_tokens: 0,
  linked: 3,
  first: 0,
  released: 0,
  others: 4,
  owner: person("Ada"),
  you: "meets",
  email: true,
  ...fields,
});

describe("Admin → Sign-in", () => {
  it("says how people sign in today, and whom each domain's rule covers", () => {
    const page = open(
      overview({
        rules: [{ domains: ["acme.com", "acme.dev"], provider: "google", passkeys: false }],
        coverage: [{ people: 40, linked: 12, passwords: 26 }],
      }),
    );
    const text = page.container.textContent;
    expect(text).toContain(
      "52 people have an account: 38 with a password, 12 with Google and 9 with a passkey. 3 people haven't signed in yet.",
    );
    expect(text).toContain("acme.com and acme.dev sign in with Google only.");
    expect(text).toContain(
      "40 people · 12 with Google · 26 still have a password, which goes at their next sign-in",
    );
    expect(text).toContain("Ada, the primary instance owner, keeps their password and passkeys");
    expect(text).toContain("https://fctl.acme.dev/api/v1/auth/google/callback");
  });

  it("shows whom requiring a provider concerns before it saves it", async () => {
    signInPreview.mockResolvedValue(preview());
    updateInstanceSettings.mockResolvedValue({ sign_in: [] });
    const page = open(overview());
    act(() => page.button("Require a provider")?.click());
    await page.type("ACME.com");
    expect(signInPreview).toHaveBeenLastCalledWith([
      { domains: ["acme.com"], provider: "google", passkeys: false },
    ]);
    const dialog = document.querySelector("dialog")?.textContent ?? "";
    expect(dialog).toContain(
      "38 people sign in with a password today: their next sign-in goes through Google, and their passwords stop working.",
    );
    expect(dialog).toContain("fctl emails each of them what changes.");
    const save = page.button("Require Google");
    expect(save?.disabled).toBe(false);
    await act(async () => save?.click());
    expect(updateInstanceSettings).toHaveBeenCalledWith({
      sign_in: [{ domains: ["acme.com"], provider: "google", passkeys: false }],
    });
  });

  it("waits until an admin it would sign out signs in with the provider", async () => {
    signInPreview.mockResolvedValue(preview({ you: "signs_out" }));
    const page = open(overview());
    act(() => page.button("Require a provider")?.click());
    await page.type("acme.com");
    const dialog = document.querySelector("dialog");
    expect(dialog?.textContent).toContain("This would sign you out: you signed in another way.");
    expect(dialog?.querySelector("a[href^='/api/v1/auth/google/start']")?.getAttribute("href")).toBe(
      "/api/v1/auth/google/start?intent=login&return_to=%2Fadmin%3Ftab%3Dsign-in",
    );
    expect(page.button("Require Google")?.disabled).toBe(true);
  });

  it("says what isn't a domain, and asks the server nothing for it", async () => {
    const page = open(overview());
    act(() => page.button("Require a provider")?.click());
    await page.type("acme");
    expect(document.querySelector("dialog [role=alert]")?.textContent).toBe(
      "acme isn't a domain: give one like acme.com",
    );
    expect(signInPreview).not.toHaveBeenCalled();
  });

  it("says who sets a provider up, and requires none until one is", () => {
    const page = open(overview({ providers: [{ ...google, configured: false, people: 0 }] }));
    const text = page.container.textContent;
    expect(text).toContain("The deployment sets it up with GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET.");
    expect(page.button("Require a provider")?.disabled).toBe(true);
    expect(text).toContain("Set up a provider first: domains require one that is.");
  });
});
