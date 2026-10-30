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
    params?: Record<string, string>;
    search?: unknown;
    onClick?: () => void;
    children: ReactNode;
  } & Record<string, unknown>) => (
    <a
      href={to.replace(/\$(\w+)/g, (_, k: string) => params?.[k] ?? "")}
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

/**
 * The sidebar as a reload shows it, for Ada, on the page at `path`: the control plane's lists come
 * when the test says, and `cached` is in the cache already.
 */
function reload(path = "/", cached: (client: QueryClient) => void = () => {}) {
  const client = new QueryClient();
  client.setQueryData(keys.me, {
    user: { id: "usr_ada", name: "Ada Lovelace", role: "member" },
    claude: [],
  } as unknown as G.Me);
  cached(client);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <Sidebar onPalette={() => {}} onNavigate={() => {}} onClose={() => {}} path={path} />
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
    /** The sessions listed under each project, with their statuses. */
    sessions: () =>
      Object.fromEntries(
        [...container.querySelectorAll("ul[aria-label]")].map((ul) => [
          ul.getAttribute("aria-label"),
          [...ul.querySelectorAll("a")].map((a) => [a.getAttribute("href"), a.textContent]),
        ]),
      ),
    come: (lists: {
      workspaces?: G.WorkspaceList;
      projects?: G.ProjectList;
      /** A session's summary, or a list of sessions under its query. */
      session?: G.Session;
      sessions?: [object, G.Session[]];
    }) =>
      act(async () => {
        if (lists.workspaces) client.setQueryData(keys.workspaces, lists.workspaces);
        if (lists.projects) client.setQueryData(keys.projects, lists.projects);
        if (lists.session) client.setQueryData(keys.session(lists.session.id), lists.session);
        if (lists.sessions)
          client.setQueryData(keys.sessions(lists.sessions[0]), { sessions: lists.sessions[1] });
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

const session = (n: number, title: string, status: G.SessionStatus, project = "prj_web"): G.Session =>
  ({
    id: `ses_${n}`,
    project_id: project,
    project_slug: "web",
    number: n,
    title,
    status,
    created_at: n,
  }) as G.Session;

// The list of a project's sessions that haven't ended, as the sidebar asks for it, and the Agent Floor's.
const live = (project: string) => ({ project, status: "active", limit: 50 });
const floor = { status: "active", limit: 50 };

describe("a session's page", () => {
  const building = session(3, "Fix the build", "running");
  const waiting = session(2, "Add dark mode", "idle");
  const paused = session(1, "Export audit logs", "paused");

  it("lists the sessions of its project that haven't ended under the project", async () => {
    const sidebar = reload("/s/ses_2");
    await sidebar.come({ workspaces, projects, session: waiting });
    expect(sidebar.sessions()).toEqual({});
    await sidebar.come({
      sessions: [live("prj_web"), [building, waiting, session(4, "Retry uploads", "stopped"), paused]],
    });
    expect(sidebar.sessions()).toEqual({
      "web's active sessions": [
        ["/s/ses_3", "Fix the build: Working"],
        ["/s/ses_2", "Add dark mode: Waiting"],
        ["/s/ses_1", "Export audit logs: Paused"],
      ],
    });
    // The feed says one ended, before the list comes again.
    await sidebar.come({
      sessions: [live("prj_web"), [{ ...building, status: "stopped" }, waiting, paused]],
    });
    expect(sidebar.sessions()["web's active sessions"]).toEqual([
      ["/s/ses_2", "Add dark mode: Waiting"],
      ["/s/ses_1", "Export audit logs: Paused"],
    ]);
  });

  it("lists them at once for a session opened from a list, as the lists in the cache have them", () => {
    const sidebar = reload("/s/ses_3", (client) => {
      client.setQueryData(keys.workspaces, workspaces);
      client.setQueryData(keys.projects, projects);
      client.setQueryData(keys.sessions(floor), {
        sessions: [building, session(9, "Write the docs", "running", "prj_api"), paused],
      });
    });
    expect(sidebar.sessions()).toEqual({
      "web's active sessions": [
        ["/s/ses_3", "Fix the build: Working"],
        ["/s/ses_1", "Export audit logs: Paused"],
      ],
    });
  });

  it("lists none for a session of a project the sidebar doesn't show, or on other pages", async () => {
    const shared = reload("/s/ses_8");
    await shared.come({ workspaces, projects, session: session(8, "Shared with Ada", "running", "prj_x") });
    await shared.come({ sessions: [live("prj_x"), [session(8, "Shared with Ada", "running", "prj_x")]] });
    expect(shared.sessions()).toEqual({});
    const home = reload("/");
    await home.come({ workspaces, projects, sessions: [live("prj_web"), [building]] });
    expect(home.sessions()).toEqual({});
  });
});

describe("a project's page", () => {
  it("lists the project's sessions that haven't ended under it, by its slug or its id", async () => {
    const web = reload("/p/web");
    await web.come({ workspaces, projects });
    await web.come({
      sessions: [
        live("prj_web"),
        [session(3, "Fix the build", "running"), session(1, "Retry uploads", "failed")],
      ],
    });
    expect(web.sessions()).toEqual({ "web's active sessions": [["/s/ses_3", "Fix the build: Working"]] });
    const api = reload("/p/prj_api");
    await api.come({ workspaces, projects });
    await api.come({ sessions: [live("prj_api"), [session(9, "Write the docs", "idle", "prj_api")]] });
    expect(api.sessions()).toEqual({ "api's active sessions": [["/s/ses_9", "Write the docs: Waiting"]] });
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
    /** The palette, opened from the drawer's search field. */
    search: () =>
      click([...drawer().querySelectorAll("button")].find((b) => b.textContent?.includes("Search"))),
    pick: (name: string) =>
      click([...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === name)),
    escape: () =>
      act(() => {
        document
          .querySelector('input[placeholder^="Search"]')
          ?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      }),
    palette: () => !!document.querySelector('input[placeholder^="Search"]'),
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

  it("goes at once when the palette opened from it opens a page", () => {
    const shell = phone();
    shell.open();
    shell.search();
    expect(shell.palette()).toBe(true);
    shell.pick("Settings");
    expect(shell.palette()).toBe(false);
    expect(shell.state()).toEqual({ open: false, slides: false, shade: false });
  });

  it("stays when the palette opened from it closes without opening a page", () => {
    const shell = phone();
    shell.open();
    shell.search();
    shell.escape();
    expect(shell.palette()).toBe(false);
    expect(shell.state()).toEqual({ open: true, slides: true, shade: true });
    shell.search();
    cleanups.push(() => delete document.documentElement.dataset.theme);
    shell.pick("Toggle theme");
    expect(shell.palette()).toBe(false);
    expect(shell.state()).toEqual({ open: true, slides: true, shade: true });
  });
});
