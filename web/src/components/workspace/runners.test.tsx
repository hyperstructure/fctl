import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { EnrollmentTokens, WaitingRunners } from "./runners";

// No request is answered: the sections show what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: pending, workspaces: pending, ApiFailure: class extends Error {} };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  document.body.innerHTML = "";
});

function show(node: React.ReactNode, tokens?: G.EnrollmentToken[]) {
  const client = new QueryClient();
  if (tokens) client.setQueryData<G.EnrollmentTokenList>(keys.enrollmentTokens(acme.id), { tokens });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<QueryClientProvider client={client}>{node}</QueryClientProvider>));
  roots.push(() => act(() => root.unmount()));
  return container;
}

const acme = { id: "wsp_acme", slug: "acme", name: "Acme", role: "admin" } as G.Workspace;
const now = Date.now();

const waiting = (r: Partial<G.Runner> & { id: string }): G.Runner =>
  ({
    name: r.id,
    status: "offline",
    drivers: [],
    capacity: 4,
    active_sessions: 0,
    labels: [],
    created_at: now - 5 * 60_000,
    pending_until: now + 6 * 86_400_000,
    workspace: acme,
    ...r,
  }) as G.Runner;

const button = (label: string) => {
  const b = [...document.querySelectorAll("button")].find((x) => x.textContent === label);
  if (!b) throw new Error(`no button ${label}`);
  return b;
};

describe("runners that wait for approval", () => {
  it("say what they are, where they enrolled from, and when they go", () => {
    const c = show(
      <WaitingRunners
        w={acme}
        runners={[
          waiting({
            id: "rnr_01ABC",
            name: "build-7",
            hostname: "ip-10-0-3-17",
            os: "linux",
            arch: "x86_64",
            driver: "docker",
            version: "0.28.0",
            status: "online",
            enrollment: { token_name: "Build fleet", address: "203.0.113.7", location: "Stockholm, SE" },
          }),
        ]}
      />,
    );
    const text = c.textContent ?? "";
    expect(text).toContain("A runner waits for your approval");
    expect(text).toContain("build-7connected");
    expect(text).toContain("ip-10-0-3-17");
    expect(text).toContain("linux/x86_64");
    // The id the machine's `fctl host status` shows.
    expect(text).toContain("rnr_01ABC");
    expect(text).toContain("with “Build fleet” from 203.0.113.7 (Stockholm, SE).");
    expect(text).toContain("Removed in 6 days unless approved.");
    expect(button("Approve")).toBeTruthy();
  });

  it("ask before one is rejected", () => {
    show(<WaitingRunners w={acme} runners={[waiting({ id: "rnr_1", name: "build-7" })]} />);
    act(() => button("Reject").click());
    expect(document.body.textContent).toContain("Reject build-7?");
    expect(button("Reject it")).toBeTruthy();
  });

  it("show nothing while none waits", () => {
    expect(show(<WaitingRunners w={acme} runners={[]} />).textContent).toBe("");
  });
});

const token = (t: Partial<G.EnrollmentToken> & { id: string }): G.EnrollmentToken => ({
  name: t.id,
  approval: false,
  created_at: now - 86_400_000,
  runners: 0,
  pending: 0,
  ...t,
});

describe("enrollment tokens", () => {
  it("say what each does, how full it is, and keep the ended ones folded", () => {
    const c = show(<EnrollmentTokens w={acme} />, [
      token({
        id: "enr_1",
        name: "Autoscaling group",
        max_runners: 20,
        runners: 3,
        remove_offline_after_minutes: 5,
        expires_at: now + 30 * 86_400_000,
        created_by: { id: "usr_ada", name: "Ada Lovelace" } as G.UserRef,
      }),
      token({ id: "enr_2", name: "Laptops", approval: true, runners: 2, pending: 1 }),
      token({ id: "enr_3", name: "Old fleet", revoked_at: now - 1000 }),
    ]);
    const text = c.textContent ?? "";
    expect(text).toContain(
      "Enrolls up to 20 runners at a time, which run everyone's sessions at once and are removed once they have been offline for 5 minutes.",
    );
    expect(text).toContain("3 of 20 runners");
    expect(text).toContain("1 waiting");
    expect(text).toContain("made by Ada Lovelace");
    expect(text).not.toContain("Old fleet");
    act(() => button("Show 1 revoked or expired").click());
    expect(c.textContent).toContain("Old fleetRevoked");
  });

  it("make a token from a form that says what it will do", () => {
    show(<EnrollmentTokens w={acme} />, []);
    act(() => button("New enrollment token").click());
    const create = button("Create token");
    expect(create.disabled).toBe(true);
    const summary = () => document.querySelector("dialog [aria-live]")?.textContent ?? "";
    expect(summary()).toContain("Enrolls any number of runners, which run everyone's sessions at once.");
    act(() => (document.querySelector('[role="switch"]') as HTMLButtonElement).click());
    expect(summary()).toContain("which wait for an admin's approval before they run anything");
  });
});
