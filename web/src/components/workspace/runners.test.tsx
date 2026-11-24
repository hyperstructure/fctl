import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { EnrollmentTokens, WaitingRunners } from "./runners";

// No request is answered but making a pool: the sections show what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  const workspaces = new Proxy(
    {},
    {
      get: (_, name) =>
        name === "createEnrollmentToken"
          ? (_id: string, b: G.CreateEnrollmentTokenRequest) =>
              Promise.resolve({
                token: {
                  id: "enr_new",
                  name: b.name,
                  personal: false,
                  approval: false,
                  created_at: 0,
                  runners: 0,
                  pending: 0,
                  labels: b.labels,
                  placement: b.placement,
                  max_runners: b.max_runners,
                  remove_offline_after_minutes: b.remove_offline_after_minutes,
                },
                secret: "fctl_et_shown_once",
              })
          : () => new Promise(() => {}),
    },
  );
  return { api: pending, workspaces, ApiFailure: class extends Error {} };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  document.body.innerHTML = "";
});

function show(node: React.ReactNode, tokens?: G.EnrollmentToken[], demand?: G.WorkspaceDemand) {
  const client = new QueryClient();
  if (tokens) client.setQueryData<G.EnrollmentTokenList>(keys.enrollmentTokens(acme.id), { tokens });
  if (demand) client.setQueryData<G.WorkspaceDemand>(keys.workspaceDemand(acme.id), demand);
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

  it("say so when they are yours, enrolled for your sessions", () => {
    const ada = { id: "usr_ada", name: "Ada" } as G.UserRef;
    const c = show(
      <WaitingRunners
        w={{ ...acme, role: "member" }}
        runners={[waiting({ id: "rnr_1", name: "pod-7", created_by: ada, yours: true })]}
      />,
    );
    expect(c.textContent).toContain(
      "It enrolled with a token you made for your sessions. Until you approve it, it runs nothing and gets none of your sessions' code, secrets or tokens.",
    );
  });
});

const token = (t: Partial<G.EnrollmentToken> & { id: string }): G.EnrollmentToken => ({
  name: t.id,
  personal: false,
  approval: false,
  created_at: now - 86_400_000,
  runners: 0,
  pending: 0,
  ...t,
});

describe("enrollment tokens", () => {
  it("say what each does, how full it is, and keep the ended ones folded", () => {
    const c = show(<EnrollmentTokens w={acme} admin onRunners={false} />, [
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
    show(<EnrollmentTokens w={acme} admin onRunners={false} />, []);
    act(() => button("New pool").click());
    const create = button("Create pool");
    expect(create.disabled).toBe(true);
    const summary = () => document.querySelector("dialog [aria-live]")?.textContent ?? "";
    expect(summary()).toContain("Enrolls any number of runners, which run everyone's sessions at once.");
    act(() => (document.querySelector('[role="switch"]') as HTMLButtonElement).click());
    expect(summary()).toContain("which wait for an admin's approval before they run anything");
  });

  it("are a member's own: for their sessions, which they approve", () => {
    const member = { ...acme, role: "member" } as G.Workspace;
    const c = show(<EnrollmentTokens w={member} admin={false} onRunners />, [
      token({
        id: "enr_9",
        name: "My cluster",
        personal: true,
        remove_offline_after_minutes: 5,
        created_by: { id: "usr_bo", name: "Bo" } as G.UserRef,
      }),
    ]);
    expect(c.textContent).toContain("They are yours and run only your sessions in Acme");
    // Whose it is, as someone else (an admin, say) reads it.
    expect(c.textContent).toContain("Only Bo's sessions");
    expect(c.textContent).toContain("which run Bo's sessions at once");
    act(() => button("New pool").click());
    const summary = () => document.querySelector("dialog [aria-live]")?.textContent ?? "";
    // No choice of whose: a member's are for their own sessions.
    expect(document.body.textContent).not.toContain("Everyone's sessions");
    expect(summary()).toContain("Enrolls any number of runners, which run your sessions at once.");
    act(() => (document.querySelector('[role="switch"]') as HTMLButtonElement).click());
    expect(summary()).toContain("which wait for your approval before they run anything");
  });
});

describe("pools", () => {
  const pool = (p: Partial<G.PoolDemand> & { id: string; name: string }): G.PoolDemand => ({
    pool: { id: p.id, name: p.name, labels: ["aws", "large"], placement: "packed", max_runners: 20 },
    runners: { online: 2, busy: 2, draining: 0, pending: 0, starting: 1, offline: 0 },
    slots: { capacity: 8, busy: 7, free: 1 },
    waiting: { sessions: 3, since: now - 125_000 },
    wanted: 3,
    at: now,
    ...p,
  });

  it("show what each needs now, its labels and placement, and the runners it enrolled", () => {
    const enrolled = waiting({ id: "rnr_7", name: "aws-7", status: "online" });
    const c = show(
      <EnrollmentTokens
        w={acme}
        admin
        onRunners={false}
        runners={[
          {
            ...enrolled,
            pending_until: undefined,
            enrollment: { token_id: "enr_1", token_name: "AWS large" },
          },
          { ...enrolled, id: "rnr_mac", name: "studio-mac", pending_until: undefined },
        ]}
        row={(r) => <li key={r.id}>runner {r.name}</li>}
      />,
      [
        token({
          id: "enr_1",
          name: "AWS large",
          labels: ["aws", "large"],
          placement: "packed",
          max_runners: 20,
          runners: 3,
          remove_offline_after_minutes: 5,
        }),
      ],
      { pools: [pool({ id: "enr_1", name: "AWS large" })], waiting: { sessions: 3 }, unpooled: 0, at: now },
    );
    const text = c.textContent ?? "";
    expect(text).toContain("Pools");
    expect([...c.querySelectorAll("[aria-label=Labels] li")].map((l) => l.textContent)).toEqual([
      "aws",
      "large",
    ]);
    expect(text).toContain("Growing: 1 runner is starting for 3 sessions that wait.");
    expect(text).toContain("2 of 2 runners busy");
    expect(text).toContain("7 of 8 sessions");
    expect(text).toContain("longest 2m");
    expect(text).toContain("Packed: fills one runner before the next, so that idle ones can go.");
    // Its runners under it; others aren't its.
    expect(text).toContain("runner aws-7");
    expect(text).not.toContain("runner studio-mac");
  });

  it("are edited as they run: labels, placement and limit", () => {
    show(<EnrollmentTokens w={acme} admin onRunners={false} />, [
      token({ id: "enr_1", name: "AWS large", labels: ["aws"], placement: "spread" }),
    ]);
    act(() => button("").click());
    const edit = [...document.querySelectorAll("[role=menuitem]")].find((x) => x.textContent === "Edit…");
    act(() => (edit as HTMLElement).click());
    const dialog = document.querySelector("dialog[open]");
    expect(dialog?.textContent).toContain("Edit AWS large");
    expect(dialog?.querySelector<HTMLInputElement>("input.font-mono")?.value).toBe("aws");
  });

  it("ask for labels and a placement when made, packing one that removes offline runners", () => {
    show(<EnrollmentTokens w={acme} admin onRunners={false} />, []);
    act(() => button("New pool").click());
    const summary = () => document.querySelector("dialog [aria-live]")?.textContent ?? "";
    expect(summary()).toContain(
      "No labels: projects that ask for none run on its runners. Spread: each session goes to the least busy runner.",
    );
  });
});

describe("making a pool", () => {
  const type = (input: HTMLInputElement, text: string) =>
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  const field = (label: string) => {
    const l = [...document.querySelectorAll("dialog label")].find((e) => e.textContent === label);
    return document.querySelector(`[id="${(l as HTMLLabelElement | undefined)?.htmlFor}"]`) as HTMLElement;
  };

  it("goes step by step, and a cloud brings what its machines need", async () => {
    vi.useFakeTimers();
    try {
      show(<EnrollmentTokens w={acme} admin onRunners={false} />, []);
      act(() => button("New pool").click());
      const dialog = () => document.querySelector("dialog[open]")?.textContent ?? "";
      expect(dialog()).toContain("Name it");
      expect(dialog()).toContain("Choose its labels and machines");
      expect(dialog()).toContain("Choose where it runs");
      // A preview of the pool once a runner is online, named as the form says.
      type(field("Name") as HTMLInputElement, "AWS large");
      type(field("Labels") as HTMLInputElement, "aws, large");
      expect(dialog()).toContain("Once its first runner is online:");
      expect(dialog()).toContain("Idle: its runner runs nothing.");
      // A cloud's machines go without uninstalling: they're removed when offline, and packed.
      act(() => field("Its machines").click());
      const aws = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find((o) =>
        o.textContent?.startsWith("AWS"),
      );
      act(() => aws?.click());
      act(() => vi.advanceTimersByTime(1000));
      const summary = document.querySelector("dialog [aria-live]")?.textContent ?? "";
      expect(summary).toContain("are removed once they have been offline for 5 minutes");
      expect(summary).toContain(
        "Its runners carry the labels aws and large. Packed: fills one runner before the next, so that idle ones can go.",
      );
      await act(async () => button("Create pool").click());
      expect(dialog()).toContain("AWS large is ready");
      expect(dialog()).toContain("Bring it online on AWS");
      expect(dialog()).toContain("Add fctl's module to your Terraform");
      const code = [...document.querySelectorAll("dialog code")].map((c) => c.textContent).join("\n");
      expect(code).toContain("fctl_et_shown_once");
      expect(code).toContain('module "fctl_aws_large"');
    } finally {
      vi.useRealTimers();
    }
  });
});
