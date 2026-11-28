import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { coverage, covered, SignInBlocked, WorkspaceSignIn, wayOut } from "./sign-in";

const update = vi.fn();
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  api: { instance: () => new Promise(() => {}) },
  workspaces: { update: (id: string, b: G.UpdateWorkspaceRequest) => update(id, b) },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
beforeEach(() => update.mockReset());
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

const acme: G.Workspace = {
  id: "ws_acme",
  slug: "acme",
  name: "Acme",
  color: "blue",
  role: "admin",
  email: "ada@acme.com",
  base_role: "editor",
  require_two_factor: false,
  instance_requires_two_factor: false,
  blocked: false,
  members: 3,
  projects: 1,
  runners: 0,
  runners_online: 0,
  github: [],
  skip_invites: false,
  own_runners: false,
  autopilots: false,
  created_at: 0,
};

const member = (name: string, email: string): G.WorkspaceMember => ({
  user: { id: name.toLowerCase(), name },
  email,
  role: "member",
  joined_at: 0,
});
const members = [
  member("Ada", "ada@acme.com"),
  member("Bob", "bob@ACME.com"),
  member("Cy", "cy@contractor.io"),
];

function render(el: ReactElement, google = true) {
  const client = new QueryClient();
  client.setQueryData<G.InstanceInfo>(keys.instance, {
    name: "fctl",
    version: "1.0.0",
    setup_required: false,
    features: { google, email: true, github: true, publish_screenshots: false, early_access: false },
    sign_in: {
      providers: google ? [{ id: "google", name: "Google", start: "/api/v1/auth/google/start" }] : [],
      rules: [],
      self_sign_up: true,
    },
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<QueryClientProvider client={client}>{el}</QueryClientProvider>));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const button = (text: string) =>
    [...container.querySelectorAll("button")].find((b) => b.textContent?.trim() === text);
  return { container, button };
}

describe("a workspace's own sign-in requirement", () => {
  it("covers the members in it as an address at its domains", () => {
    expect(covered(members, ["acme.com"]).map((m) => m.user.name)).toEqual(["Ada", "Bob"]);
    expect(coverage("Acme", members, ["acme.com"])).toEqual([
      "2 people are in Acme as an address at acme.com: it opens to them only once they've signed in with Google as that address. Their other workspaces don't change.",
      "1 person in it as other addresses signs in as before.",
    ]);
    expect(coverage("Acme", members, ["acme.dev"])[0]).toContain(
      "No member is in Acme as an address at acme.dev yet",
    );
  });

  it("says whom it covers, then requires Google", async () => {
    update.mockResolvedValue(acme);
    const page = render(<WorkspaceSignIn w={acme} members={members} onSaved={() => {}} />);
    await act(async () => page.button("Require Google")?.click());
    // The admin's own domain comes first.
    expect(page.container.querySelector("input")?.value).toBe("acme.com");
    expect(page.container.textContent).toContain("2 people are in Acme as an address at acme.com");
    const submit = [...page.container.querySelectorAll("button[type=submit]")][0] as HTMLButtonElement;
    await act(async () => submit.click());
    expect(update).toHaveBeenCalledWith("ws_acme", {
      sign_in: { domains: ["acme.com"], provider: "google", passkeys: false },
    });
  });

  it("offers the way out when it would close the workspace to the admin", () => {
    const google = { id: "google", name: "Google", start: "/api/v1/auth/google/start" };
    expect(wayOut({ code: "conflict" }, acme, google)).toBe(
      "/api/v1/auth/google/start?intent=login&return_to=%2Fw%2Facme%3Ftab%3Dsettings&login_hint=ada%40acme.com",
    );
    expect(wayOut({ code: "validation" }, acme, google)).toBeUndefined();
    expect(wayOut({ code: "conflict" }, acme, undefined)).toBeUndefined();
    expect(wayOut(null, acme, google)).toBeUndefined();
  });

  it("stops requiring it", async () => {
    update.mockResolvedValue(acme);
    const required = { ...acme, sign_in: { domains: ["acme.com"], provider: "google", passkeys: false } };
    const page = render(<WorkspaceSignIn w={required} members={members} onSaved={() => {}} />);
    expect(page.container.textContent).toContain("Members in Acme as an address at acme.com open it only");
    await act(async () => page.button("Stop requiring it")?.click());
    expect(update).toHaveBeenCalledWith("ws_acme", {
      sign_in: { domains: [], provider: "google", passkeys: false },
    });
  });

  it("waits for the instance to sign people in with Google", () => {
    const page = render(<WorkspaceSignIn w={acme} members={members} onSaved={() => {}} />, false);
    expect(page.container.textContent).toContain("This instance doesn't sign people in with Google");
    expect(page.button("Require Google")).toBeUndefined();
  });

  it("lets a rule be stopped where nobody signs in with Google any more", async () => {
    update.mockResolvedValue(acme);
    const required = { ...acme, sign_in: { domains: ["acme.com"], provider: "google", passkeys: false } };
    const page = render(<WorkspaceSignIn w={required} members={members} onSaved={() => {}} />, false);
    expect(page.container.textContent).toContain("nobody it covers can open Acme");
    expect(page.button("Change domains")).toBeUndefined();
    await act(async () => page.button("Stop requiring it")?.click());
    expect(update).toHaveBeenCalledWith("ws_acme", {
      sign_in: { domains: [], provider: "google", passkeys: false },
    });
  });

  it("lets its primary admin stop requiring it while it closes it to them", async () => {
    update.mockResolvedValue(acme);
    const closed = {
      ...acme,
      role: "owner" as const,
      blocked: true,
      sign_in_blocked: true,
      sign_in: { domains: ["acme.com"], provider: "google", passkeys: false },
    };
    const page = render(<SignInBlocked w={closed} />, false);
    // Nobody signs in with Google here now: no link that goes nowhere.
    expect(page.container.querySelector("a")).toBeNull();
    expect(page.container.textContent).toContain("doesn't sign people in with Google any more");
    await act(async () => page.button("Stop requiring Google")?.click());
    expect(update).toHaveBeenCalledWith("ws_acme", {
      sign_in: { domains: [], provider: "google", passkeys: false },
    });
    // A member isn't offered it.
    const member = render(<SignInBlocked w={{ ...closed, role: "member" }} />);
    expect(member.button("Stop requiring Google")).toBeUndefined();
  });

  it("tells a member it closes to why, and the way in", () => {
    const closed = {
      ...acme,
      blocked: true,
      sign_in_blocked: true,
      sign_in: { domains: ["acme.com"], provider: "google", passkeys: false },
    };
    const page = render(<SignInBlocked w={closed} />);
    expect(page.container.textContent).toContain(
      "Acme requires its members at acme.com to sign in with Google as that address, and you signed in another way.",
    );
    expect(page.container.querySelector("a")?.getAttribute("href")).toBe(
      "/api/v1/auth/google/start?intent=login&return_to=%2Fw%2Facme&login_hint=ada%40acme.com",
    );
  });
});
