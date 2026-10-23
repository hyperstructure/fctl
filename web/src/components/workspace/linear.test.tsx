import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { LinearTab } from "./linear";

// No request is answered: the tab shows what the cache holds.
vi.mock("@/lib/api", () => {
  const never = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: never, workspaces: never, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
});

const workspace = (role: G.WorkspaceRole) =>
  ({ id: "wsp_1", slug: "acme", name: "Acme", color: "yellow", role }) as unknown as G.Workspace;

const connection: G.LinearConnection = {
  organization: "3f1d9d7c-1111-4c2e-9a5b-0d2f6c8e7a10",
  name: "Acme Inc",
  url_key: "acme",
  app_handle: "fctl-sh",
  status: "active",
  connected_by: { id: "usr_ada", name: "Ada" },
  created_at: 0,
  projects: [{ slug: "web", name: "Web", team_key: "ENG" }],
};

function tab(linear: G.WorkspaceLinear, role: G.WorkspaceRole = "admin", returned?: "connected") {
  queryClient.setQueryData(keys.workspaceLinear("wsp_1"), linear);
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <LinearTab w={workspace(role)} returned={returned} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

describe("LinearTab", () => {
  it("lists the Linear workspaces it works in, as whom, and the projects that take their tickets", () => {
    const c = tab({ configured: true, webhook: true, connections: [connection] });
    expect(c.textContent).toContain("Acme Inc");
    expect(c.textContent).toContain("linear.app/acme · as @fctl-sh · connected by Ada");
    expect(c.textContent).toContain("WebENG");
    expect(c.textContent).toContain("Connected");
    expect(c.textContent).toContain("Connect another Linear workspace");
  });

  it("has admins connect one, and tells others who does", () => {
    const admin = tab({ configured: true, webhook: true, connections: [] });
    expect(admin.textContent).toContain("Acme works in no Linear workspace yet.");
    expect([...admin.querySelectorAll("button")].some((b) => b.textContent === "Connect Linear")).toBe(true);
    const member = tab({ configured: true, webhook: true, connections: [connection] }, "member");
    expect(member.textContent).toContain("Acme's admins connect Linear workspaces.");
    expect(member.querySelector('[aria-label^="Disconnect"]')).toBeNull();
  });

  it("says what to do when Linear took fctl's access back", () => {
    const c = tab({ configured: true, webhook: true, connections: [{ ...connection, status: "revoked" }] });
    expect(c.textContent).toContain("Connect again");
    expect(c.textContent).not.toContain("Connected");
  });

  it("says when the instance has no Linear app, or takes no webhooks", () => {
    expect(tab({ configured: false, webhook: false, connections: [] }).textContent).toContain(
      "This instance has no Linear app yet.",
    );
    expect(tab({ configured: true, webhook: false, connections: [] }).textContent).toContain(
      "takes no Linear webhooks yet",
    );
  });

  it("confirms a connection on the way back from Linear", () => {
    const c = tab({ configured: true, webhook: true, connections: [connection] }, "admin", "connected");
    expect(c.textContent).toContain("Linear is connected");
  });
});
