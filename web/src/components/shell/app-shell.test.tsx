import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { keepSidebar } from "@/lib/sidebar";
import { AppShell, Sidebar } from "./app-shell";

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
    onClick,
    children,
    ...rest
  }: {
    to: string;
    params?: { slug: string };
    search?: unknown;
    onClick?: () => void;
    children: ReactNode;
  } & Record<string, unknown>) => (
    <a
      href={params ? to.replace("$slug", params.slug) : to}
      onClick={(e) => {
        e.preventDefault();
        onClick?.();
      }}
      {...rest}
    >
      {children}
    </a>
  ),
  Outlet: () => null,
  useNavigate: () => () => {},
  useRouter: () => ({ history: { push: () => {} } }),
  useRouterState: ({ select }: { select: (s: { location: { pathname: string } }) => string }) =>
    select({ location: { pathname: "/" } }),
}));
// The shell without the live feed and notifications.
vi.mock("@/lib/feed", () => ({ useFeed: () => {}, feed: { onPulse: () => () => {} } }));
vi.mock("@/lib/push", () => ({
  syncPush: async () => {},
  onPushOpen: () => () => {},
  forgetPush: async () => {},
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
        <Sidebar onPalette={() => {}} onNavigate={() => {}} onClose={() => {}} path="/" />
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

/** The shell at a phone's width, where the sidebar is a drawer. */
function phone() {
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
        <AppShell />
      </QueryClientProvider>,
    ),
  );
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const drawer = () => container.querySelector("aside") as HTMLElement;
  const click = (el: Element | null | undefined) => act(() => (el as HTMLElement).click());
  const closeButtons = () => [...container.querySelectorAll('button[aria-label="Close menu"]')];
  return {
    /** Whether it is open, and whether it slides when it opens or closes. */
    state: () => ({
      open: drawer().classList.contains("translate-x-0"),
      slides: drawer().classList.contains("transition-[translate,display]"),
      shade: closeButtons().some((b) => !drawer().contains(b)),
    }),
    open: () => click(container.querySelector('button[aria-label="Open menu"]')),
    close: () => click(closeButtons().find((b) => drawer().contains(b))),
    shade: () => click(closeButtons().find((b) => !drawer().contains(b))),
    follow: (name: string) =>
      click([...drawer().querySelectorAll("a")].find((a) => a.textContent?.trim() === name)),
  };
}

describe("the drawer", () => {
  it("slides in, and away when its close button or the shade closes it", () => {
    const shell = phone();
    expect(shell.state()).toEqual({ open: false, slides: true, shade: false });
    shell.open();
    expect(shell.state()).toEqual({ open: true, slides: true, shade: true });
    shell.close();
    expect(shell.state()).toEqual({ open: false, slides: true, shade: false });
    shell.open();
    shell.shade();
    expect(shell.state()).toEqual({ open: false, slides: true, shade: false });
  });

  it("goes at once when one of its links opens a page, and slides in again", () => {
    const shell = phone();
    shell.open();
    shell.follow("Settings");
    expect(shell.state()).toEqual({ open: false, slides: false, shade: false });
    shell.open();
    expect(shell.state()).toEqual({ open: true, slides: true, shade: true });
  });
});
