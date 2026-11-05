import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { RunnerApprovals } from "./runner-approvals";

// No request is answered: the notice shows what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: pending, workspaces: pending, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    params,
    search,
  }: {
    children: ReactNode;
    to: string;
    params?: { slug: string };
    search: Record<string, string>;
  }) => (
    <a href={`${params ? to.replace("$slug", params.slug) : to}?${new URLSearchParams(search)}`}>
      {children}
    </a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function notice(runners: G.Runner[], workspaces: G.Workspace[]) {
  const client = new QueryClient();
  client.setQueryData<G.RunnerList>(keys.runners, { runners });
  client.setQueryData(keys.workspaces, { workspaces, invitations: [] });
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <RunnerApprovals />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

const acme = { id: "wsp_acme", slug: "acme", name: "Acme" } as G.WorkspaceRef;
const side = { id: "wsp_side", slug: "side", name: "Side project" } as G.WorkspaceRef;
const ws = (ref: G.WorkspaceRef, role: G.WorkspaceRole) => ({ ...ref, role, blocked: false }) as G.Workspace;
const runner = (r: Partial<G.Runner> & { id: string }): G.Runner =>
  ({
    name: r.id,
    status: "online",
    drivers: [],
    capacity: 4,
    active_sessions: 0,
    labels: [],
    created_at: 0,
    ...r,
  }) as G.Runner;

describe("the notice of runners that wait for approval", () => {
  it("says how many wait where, and goes to the Runners tab", () => {
    const c = notice(
      [
        runner({ id: "rnr_1", name: "build-7", workspace: acme, pending_until: 9 }),
        runner({ id: "rnr_2", name: "build-8", workspace: acme, pending_until: 9 }),
        runner({ id: "rnr_3", name: "approved", workspace: acme }),
      ],
      [ws(acme, "admin")],
    );
    expect(c.querySelector("section")?.textContent).toContain("2 runners wait for your approval");
    expect(c.textContent).toContain("build-7 and build-8 in Acme: they run nothing until approved");
    expect(c.querySelector("a")?.getAttribute("href")).toBe("/w/acme?tab=runners");
  });

  it("is only for the admins of the workspaces where they wait", () => {
    const waiting = [runner({ id: "rnr_1", workspace: side, pending_until: 9 })];
    // An instance admin sees every runner, but approves as a workspace's admin.
    expect(notice(waiting, [ws(side, "member"), ws(acme, "owner")]).textContent).toBe("");
    expect(notice([], [ws(acme, "owner")]).textContent).toBe("");
    expect(notice(waiting, [ws(side, "owner")]).textContent).toContain("A runner waits for your approval");
  });
});
