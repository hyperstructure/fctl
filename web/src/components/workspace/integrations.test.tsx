import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import type { Integration } from "@/lib/slack";
import { Integrations } from "./integrations";

// No request is answered: the switcher says what the cache holds.
vi.mock("@/lib/api", () => {
  const never = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: never, workspaces: never, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    search,
    "aria-current": current,
  }: {
    children: ReactNode;
    search: { integration: string };
    "aria-current"?: "page";
  }) => (
    <a href={`/w/acme?tab=integrations&integration=${search.integration}`} aria-current={current}>
      {children}
    </a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
});

const w = { id: "wsp_1", slug: "acme", name: "Acme", github: ["acme"] } as unknown as G.Workspace;

function tab(shown: Integration[], open: Integration) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <Integrations w={w} shown={shown} open={open}>
          <p>The open one's page</p>
        </Integrations>
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

describe("a workspace's Integrations", () => {
  it("moves between GitHub, Linear and Slack, each saying how it stands", () => {
    queryClient.setQueryData(keys.instance, { features: { github: true, linear: true, slack: true } });
    queryClient.setQueryData(keys.workspaceLinear("wsp_1"), {
      configured: true,
      webhook: true,
      connections: [],
    });
    queryClient.setQueryData(keys.workspaceSlack("wsp_1"), {
      configured: true,
      events: true,
      socket_mode: false,
      match_email: false,
      instance_admin: false,
      connections: [{ team: "T1", name: "Acme Corp", status: "revoked" }],
    });
    const c = tab(["github", "linear", "slack"], "slack");
    const links = [...c.querySelectorAll("nav[aria-label='Integrations'] a")];
    expect(links.map((a) => a.getAttribute("href"))).toEqual([
      "/w/acme?tab=integrations&integration=github",
      "/w/acme?tab=integrations&integration=linear",
      "/w/acme?tab=integrations&integration=slack",
    ]);
    expect(links[0]?.textContent).toContain("Connected to acme");
    expect(links[1]?.textContent).toContain("Not connected yet");
    expect(links[2]?.textContent).toContain("Access revoked");
    // A word or two where a phone leaves little room.
    expect(links[2]?.textContent).toContain("Revoked");
    expect(links[2]?.getAttribute("aria-current")).toBe("page");
    expect(links[0]?.getAttribute("aria-current")).toBeNull();
    expect(c.textContent).toContain("The open one's page");
  });

  it("holds a place for how one stands until it's known", () => {
    const c = tab(["github", "slack"], "github");
    const slack = [...c.querySelectorAll("nav a")][1];
    expect(slack?.textContent).toBe("Slack");
    expect(slack?.querySelector(".animate-shimmer")).not.toBeNull();
  });

  it("shows the page alone where there is nothing to move to", () => {
    const c = tab(["github"], "github");
    expect(c.querySelector("nav")).toBeNull();
    expect(c.textContent).toBe("The open one's page");
  });
});
