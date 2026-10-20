import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { WorkspacePage } from "./workspace";

// No request is answered: the page shows what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: pending, workspaces: pending, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ slug: "side-project" }),
  useSearch: () => ({ tab: "runners" }),
  useNavigate: () => () => {},
  Link: ({ children }: { children: ReactNode }) => <a href="/w/side-project">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

const side: G.Workspace = {
  id: "ws_side",
  slug: "side-project",
  name: "Side project",
  color: "pink",
  role: "owner",
  email: "ada@example.com",
  base_role: "editor",
  require_two_factor: false,
  blocked: false,
  primary: { id: "usr_ada", name: "Ada Lovelace" },
  members: 2,
  projects: 1,
  runners: 0,
  runners_online: 0,
  github: [],
  created_at: 0,
};

/** The workspace's Runners tab, without runners, as `workspace` sees it. */
function runnersTab(workspace: G.Workspace) {
  const client = new QueryClient();
  client.setQueryData(keys.workspace(workspace.slug), workspace);
  client.setQueryData<G.RunnerList>(keys.workspaceRunners(workspace.id), { runners: [] });
  client.setQueryData<G.RunnerList>(keys.runners, { runners: [] });
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <WorkspacePage />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container.textContent ?? "";
}

describe("a workspace without runners", () => {
  it("shows its admins how to bring a machine online", () => {
    const text = runnersTab(side);
    expect(text).toContain("Side project's sessions wait until one of its runners is online.");
    expect(text).toContain("Choose a machine that stays on");
    expect(text).toContain(
      "install Apple's container on macOS 26 or later, Lima on older macOS, or Docker on Linux",
    );
    expect(text).toContain("Install fctl on it");
    expect(text).toContain("curl -fsSL https://fctl.sh/install | sh");
    expect(text).toContain("brew install hyperstructure/tap/fctl");
    expect(text).toContain("Register it for Side project");
    expect(text).toContain(`fctl login ${location.origin}`);
    expect(text).toContain("fctl host install --workspace side-project");
    expect(text).toContain("Name it above and choose Register for a command with a token.");
  });

  it("tells members whom to ask for one", () => {
    const text = runnersTab({ ...side, role: "member", email: "grace@example.com" });
    expect(text).toContain("No runners yet");
    expect(text).toContain("Ask Ada Lovelace, its primary admin, to add one.");
    expect(text).not.toContain("Choose a machine that stays on");
  });
});
