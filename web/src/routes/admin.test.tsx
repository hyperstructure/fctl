import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { Admin } from "./admin";

// No request is answered: the page shows what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: pending, workspaces: pending, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  useSearch: () => ({ tab: "runners" }),
  Link: ({ children }: { children: ReactNode }) => <a href="/admin">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

/** The Runners tab, with these runners in the cache as the API or the feed put them there. */
function runnersTab(runners: G.Runner[]) {
  const client = new QueryClient();
  client.setQueryData<G.RunnerList>(keys.runners, { runners });
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <Admin />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

/** A runner as control planes before workspaces send it: without `workspaces` or `yours`. */
const before = JSON.parse(
  '{"id":"run_1","name":"studio mac","status":"online","version":"0.8.0","hostname":"studio","drivers":[],"capacity":4,"active_sessions":1,"labels":[],"created_at":1760940000000}',
) as G.Runner;

const workspace: G.WorkspaceRef = {
  id: "ws_1",
  slug: "hyperstructure",
  name: "hyperstructure",
  color: "blue",
};

describe("the admin's runners", () => {
  it("lists a runner from a control plane before workspaces", () => {
    const page = runnersTab([before]);
    expect(page.textContent).toContain("studio mac");
    expect(page.textContent).toContain("1/4 sessions");
    // It doesn't say which workspaces it runs, so the page doesn't either.
    expect(page.textContent).not.toContain("Runs no workspace's sessions yet");
  });

  it("names the workspaces a runner runs, or says it runs none", () => {
    const page = runnersTab([
      { ...before, id: "run_1", name: "studio mac", workspaces: [workspace], yours: true },
      { ...before, id: "run_2", name: "spare mini", workspaces: [], yours: true },
    ]);
    const [studio, spare] = [...page.querySelectorAll("li")].map((li) => li.textContent);
    expect(studio).toContain("hyperstructure");
    expect(studio).not.toContain("Runs no workspace's sessions yet");
    expect(spare).toContain("Runs no workspace's sessions yet");
  });
});
