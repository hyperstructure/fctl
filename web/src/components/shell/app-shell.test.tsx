import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { keepSidebar } from "@/lib/sidebar";
import { Sidebar } from "./app-shell";

// No request is answered: the sidebar shows what the cache holds, and what this browser kept.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: pending, workspaces: pending, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    params,
    search: _,
    children,
    ...rest
  }: { to: string; params?: { slug: string }; search?: unknown; children: ReactNode } & Record<
    string,
    unknown
  >) => (
    <a href={params ? to.replace("$slug", params.slug) : to} {...rest}>
      {children}
    </a>
  ),
  useNavigate: () => () => {},
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  localStorage.clear();
});

const workspace = (id: string, name: string): G.Workspace =>
  ({ id, slug: name.toLowerCase(), name, color: "blue", role: "owner", blocked: false }) as G.Workspace;
const project = (name: string, w: G.Workspace, active_sessions = 0): G.Project =>
  ({ id: `prj_${name}`, slug: name, name, workspace: { id: w.id }, active_sessions }) as G.Project;

const acme = workspace("ws_acme", "Acme");
const side = workspace("ws_side", "Side");
const workspaces: G.WorkspaceList = { workspaces: [acme, side], invitations: [], limit: 5 };
const projects: G.ProjectList = { projects: [project("web", acme, 2), project("api", acme)] };

/** The sidebar as a reload shows it, for Ada: the control plane's lists come when the test says. */
function reload() {
  const client = new QueryClient();
  client.setQueryData(keys.me, {
    user: { id: "usr_ada", name: "Ada Lovelace", role: "member" },
    claude: [],
  } as unknown as G.Me);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <Sidebar onPalette={() => {}} onNavigate={() => {}} path="/" />
      </QueryClientProvider>,
    ),
  );
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return {
    /** Each workspace's projects, with their counts of active sessions, and the offer to create one. */
    shown: () =>
      Object.fromEntries(
        [...container.querySelectorAll("section")].map((s) => [
          s.getAttribute("aria-label"),
          [...s.querySelectorAll<HTMLAnchorElement>('a[href^="/p/"], a[href="/new"]')]
            .map((a) => a.textContent?.trim())
            .filter(Boolean),
        ]),
      ),
    come: (lists: { workspaces?: G.WorkspaceList; projects?: G.ProjectList }) =>
      act(async () => {
        if (lists.workspaces) client.setQueryData(keys.workspaces, lists.workspaces);
        if (lists.projects) client.setQueryData(keys.projects, lists.projects);
        // The cache tells the sidebar in a timeout.
        await new Promise((done) => setTimeout(done, 0));
      }),
  };
}

describe("the sidebar", () => {
  it("shows the workspaces and projects this browser kept at once, until the control plane's come", async () => {
    keepSidebar("usr_ada", workspaces, projects);
    const sidebar = reload();
    expect(sidebar.shown()).toEqual({ Acme: ["web2", "api"], Side: ["Create a project"] });
    const docs = project("docs", side);
    await sidebar.come({ workspaces });
    expect(sidebar.shown()).toEqual({ Acme: ["web2", "api"], Side: ["Create a project"] });
    await sidebar.come({ projects: { projects: [project("web", acme), docs] } });
    expect(sidebar.shown()).toEqual({ Acme: ["web"], Side: ["docs"] });
  });

  it("offers to create a project only once it knows there are none", async () => {
    const sidebar = reload();
    expect(sidebar.shown()).toEqual({});
    await sidebar.come({ workspaces });
    expect(sidebar.shown()).toEqual({ Acme: [], Side: [] });
    await sidebar.come({ projects });
    expect(sidebar.shown()).toEqual({ Acme: ["web2", "api"], Side: ["Create a project"] });
  });

  it("shows no one else's lists", () => {
    keepSidebar("usr_grace", workspaces, projects);
    expect(reload().shown()).toEqual({});
  });
});
