import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { PULLS_QUERY } from "@/lib/pull-list";
import { keys } from "@/lib/queries";
import { keepSidebar } from "@/lib/sidebar";
import { AppShell, Sidebar } from "./app-shell";

// No request is answered: the sidebar shows what the cache holds, and what this browser kept. What
// it asks for sessions with is kept.
const asked = vi.hoisted(() => ({ sessions: [] as unknown[] }));
vi.mock("@/lib/api", () => {
  const pending = new Proxy(
    {},
    {
      get: (_, name) => (q: unknown) => {
        if (name === "sessions") asked.sessions.push(q);
        return new Promise(() => {});
      },
    },
  );
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
  asked.sessions.length = 0;
});

const workspace = (id: string, name: string): G.Workspace =>
  ({ id, slug: name.toLowerCase(), name, color: "blue", role: "owner", blocked: false }) as G.Workspace;
const project = (name: string, w: G.Workspace, active_sessions = 0): G.Project =>
  ({ id: `prj_${name}`, slug: name, name, workspace: { id: w.id }, active_sessions }) as G.Project;

const acme = workspace("ws_acme", "Acme");
const side = workspace("ws_side", "Side");
const workspaces: G.WorkspaceList = { workspaces: [acme, side], invitations: [], limit: 5 };
const projects: G.ProjectList = { projects: [project("web", acme, 2), project("api", acme)] };

/** What `el` says, without what it hides from screen readers: the palette's keycaps. */
function shownText(el: Element): string {
  return [...el.childNodes]
    .map((n) =>
      n instanceof Element
        ? n.getAttribute("aria-hidden") === "true"
          ? ""
          : shownText(n)
        : (n.textContent ?? ""),
    )
    .join("");
}

/**
 * The sidebar as a reload shows it, for Ada, on the page at `path`: the control plane's lists come
 * when the test says, and `cached` is in the cache already.
 */
function reload(
  path = "/",
  cached: (client: QueryClient) => void = () => {},
  more: { folded?: boolean; onFold?: (f: boolean) => void; scope?: string } = {},
) {
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
        <Sidebar onPalette={() => {}} onNavigate={() => {}} onClose={() => {}} path={path} {...more} />
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
        [...container.querySelectorAll("section ul[aria-label]")].map((ul) => [
          ul.getAttribute("aria-label"),
          [...ul.querySelectorAll("a")].map((a) => [a.getAttribute("href"), a.textContent]),
        ]),
      ),
    container,
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

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const NOW = Date.UTC(2026, 10, 5, 14);

const session = (
  n: number,
  title: string,
  status: G.SessionStatus,
  project = "prj_web",
  more: Partial<G.Session> = {},
): G.Session =>
  ({
    id: `ses_${n}`,
    project_id: project,
    project_slug: project.replace("prj_", ""),
    number: n,
    title,
    status,
    created_at: n,
    archived: false,
    ...more,
  }) as G.Session;

// The sessions the sidebar asks for, and the Agent Floor's lists.
const recent = { since: "1h", limit: 100 };
const working = { status: "active", limit: 50 };
const ended = { status: "ended", limit: 12 };
/** What Needs you looks through: the sessions Ada started or follows, from the last day. */
const yours = { yours: true, since: "1d", limit: 200 };

describe("the sidebar's sessions", () => {
  // The sidebar's clock, which moves on every minute.
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
    vi.setSystemTime(NOW);
  });
  afterEach(() => vi.useRealTimers());

  const building = session(6, "Fix the build", "running");
  const waiting = session(5, "Add dark mode", "idle");
  const docs = session(4, "Write the docs", "paused", "prj_api");
  const retried = session(3, "Retry uploads", "stopped", "prj_web", { ended_at: NOW - 20 * MINUTE });
  const broke = session(2, "Bump the SDK", "failed", "prj_api", { ended_at: NOW - 59 * MINUTE });
  const exported = session(1, "Export audit logs", "stopped", "prj_web", { ended_at: NOW - 2 * HOUR });

  it("lists under each project on every page its sessions at work and those that ended in the last hour", async () => {
    for (const path of ["/", "/settings", "/w/acme", "/p/web", "/s/ses_5"]) {
      const sidebar = reload(path);
      await sidebar.come({
        workspaces,
        projects,
        sessions: [recent, [building, waiting, docs, retried, broke, exported]],
      });
      expect(sidebar.sessions()).toEqual({
        "web's recent sessions": [
          ["/s/ses_6", "Fix the build: Working"],
          ["/s/ses_5", "Add dark mode: Waiting"],
          ["/s/ses_3", "Retry uploads: Stopped"],
        ],
        "api's recent sessions": [
          ["/s/ses_4", "Write the docs: Paused"],
          ["/s/ses_2", "Bump the SDK: Failed"],
        ],
      });
    }
    // Those the control plane has from an hour back.
    expect(asked.sessions).toContainEqual({ activeSince: NOW - HOUR, limit: 100 });
  });

  it("goes on listing one that ended for an hour", async () => {
    const sidebar = reload("/s/ses_6");
    await sidebar.come({ workspaces, projects, sessions: [recent, [building, waiting]] });
    // The feed says it ended, before the list comes again.
    await sidebar.come({
      sessions: [recent, [{ ...building, status: "stopped", ended_at: NOW }, waiting]],
    });
    const web = () => sidebar.sessions()["web's recent sessions"];
    expect(web()).toEqual([
      ["/s/ses_6", "Fix the build: Stopped"],
      ["/s/ses_5", "Add dark mode: Waiting"],
    ]);
    act(() => vi.advanceTimersByTime(HOUR));
    expect(web()).toHaveLength(2);
    act(() => vi.advanceTimersByTime(MINUTE));
    expect(web()).toEqual([["/s/ses_5", "Add dark mode: Waiting"]]);
  });

  it("leaves out archived sessions, and those of projects it doesn't show", async () => {
    const sidebar = reload("/s/ses_8");
    await sidebar.come({
      workspaces,
      projects,
      sessions: [
        recent,
        [
          session(8, "Shared with Ada", "running", "prj_x"),
          { ...waiting, archived: true },
          { ...retried, archived: true },
          building,
        ],
      ],
    });
    expect(sidebar.sessions()).toEqual({ "web's recent sessions": [["/s/ses_6", "Fix the build: Working"]] });
  });

  it("lists them at once as the lists in the cache have them, until its own comes", async () => {
    const sidebar = reload("/s/ses_6", (client) => {
      client.setQueryData(keys.workspaces, workspaces);
      client.setQueryData(keys.projects, projects);
      client.setQueryData(keys.sessions(working), { sessions: [building, docs] });
      client.setQueryData(keys.sessions(ended), { sessions: [retried, exported] });
    });
    expect(sidebar.sessions()).toEqual({
      "web's recent sessions": [
        ["/s/ses_6", "Fix the build: Working"],
        ["/s/ses_3", "Retry uploads: Stopped"],
      ],
      "api's recent sessions": [["/s/ses_4", "Write the docs: Paused"]],
    });
    await sidebar.come({ sessions: [recent, [waiting]] });
    expect(sidebar.sessions()).toEqual({ "web's recent sessions": [["/s/ses_5", "Add dark mode: Waiting"]] });
  });

  it("lists none in a folded workspace", async () => {
    localStorage.setItem("fctl:folded-workspaces", JSON.stringify(["ws_acme"]));
    const sidebar = reload("/");
    await sidebar.come({ workspaces, projects, sessions: [recent, [building]] });
    expect(sidebar.sessions()).toEqual({});
  });
});

describe("what needs you", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
    vi.setSystemTime(NOW);
  });
  afterEach(() => vi.useRealTimers());

  const ada = { id: "usr_ada", name: "Ada Lovelace" };
  const pr = (number: number): G.PullRequestRef => ({
    number,
    url: `https://github.com/acme/web/pull/${number}`,
    state: "open",
  });

  it("lists what its agents ask first, then pull requests they finished and replies they wait for", async () => {
    const asks = session(7, "Round the prices", "idle", "prj_web", {
      created_by: ada,
      updated_at: 7,
      question: { text: "Whole kroner, or øre?", options: ["Whole kroner", "Øre"], asked_at: NOW - MINUTE },
    });
    const finished = session(8, "Add Danish", "stopped", "prj_web", {
      created_by: ada,
      updated_at: 8,
      ended_at: NOW - HOUR,
      pull_requests: [pr(12)],
    });
    const waits = session(9, "Fix the build", "idle", "prj_api", {
      created_by: ada,
      updated_at: 9,
      status_reason: "Waiting for a message",
    });
    const theirs = session(10, "Grace's", "idle", "prj_api", {
      created_by: { id: "usr_grace", name: "Grace" },
      updated_at: 10,
      status_reason: "Waiting for a message",
    });
    const sidebar = reload("/");
    await sidebar.come({ workspaces, projects, sessions: [working, [asks, waits, theirs]] });
    await sidebar.come({ sessions: [ended, [finished]] });
    const needs = () =>
      [...sidebar.container.querySelectorAll('ul[aria-label="What needs you"] a')].map((a) =>
        a.getAttribute("title"),
      );
    expect(needs()).toEqual([
      "Round the prices\nWhole kroner, or øre?",
      "Add Danish\n#12 is ready for you",
      "Fix the build\nWaiting for your reply",
    ]);
    // Its count is in the tab's title too.
    expect(document.title).toBe("(3) fctl");
  });

  /** Ada's two open pull requests, one ready for review, and one of Grace's that is. */
  const opened = [
    session(3, "Ready", "stopped", "prj_web", { pull_requests: [pr(3)], created_by: ada }),
    session(4, "At work", "running", "prj_web", { pull_requests: [pr(4)], created_by: ada }),
    session(2, "Grace's", "stopped", "prj_web", {
      pull_requests: [pr(2)],
      created_by: { id: "usr_grace", name: "Grace" },
    }),
  ];

  it("counts the pull requests of your sessions ready for review, as the page lists them at first", async () => {
    const sidebar = reload("/");
    await sidebar.come({ workspaces, projects, sessions: [PULLS_QUERY, opened] });
    // The page's own request, which the server narrows to Ada's before its limit: it reaches an
    // older one of hers that everyone's newest sessions leave out.
    const older = session(1, "Older", "stopped", "prj_web", { pull_requests: [pr(1)], created_by: ada });
    const hers = opened.filter((s) => s.created_by.id === ada.id);
    await sidebar.come({ sessions: [{ ...PULLS_QUERY, mine: true }, [...hers, older]] });
    const link = sidebar.container.querySelector('a[href="/pulls"]');
    expect(link?.textContent).toBe("Pull requests2");
    expect(link?.getAttribute("title")).toBe("2 ready for review, of 3 open from your sessions");
  });

  it("counts everyone's once Mine is off on the pull requests page", async () => {
    localStorage.setItem("fctl:pulls-mine", "0");
    const sidebar = reload("/");
    await sidebar.come({ workspaces, projects, sessions: [PULLS_QUERY, opened] });
    const link = sidebar.container.querySelector('a[href="/pulls"]');
    expect(link?.textContent).toBe("Pull requests2");
    expect(link?.getAttribute("title")).toBe("2 ready for review, of 3 open");
  });

  it("counts as the pull requests page does: not those whose agent asks, nor those a newer session says merged", async () => {
    const sidebar = reload("/");
    const merged = { ...pr(5), state: "merged", updated_at: 20 };
    await sidebar.come({
      workspaces,
      projects,
      sessions: [
        { ...PULLS_QUERY, mine: true },
        [
          // Newest first, as the API lists them.
          session(8, "Merged since", "stopped", "prj_web", { pull_requests: [merged] }),
          session(7, "Asks", "idle", "prj_web", {
            pull_requests: [pr(6)],
            question: { text: "Keep øre?", asked_at: NOW },
          }),
          session(6, "Ready", "stopped", "prj_web", { pull_requests: [pr(7)] }),
          session(5, "Said open", "stopped", "prj_web", { pull_requests: [{ ...pr(5), updated_at: 10 }] }),
        ],
      ],
    });
    const link = sidebar.container.querySelector('a[href="/pulls"]');
    expect(link?.textContent).toBe("Pull requests1");
    expect(link?.getAttribute("title")).toBe("1 ready for review, of 2 open from your sessions");
  });

  it("looks through every session you started or follow from the last day", async () => {
    const failed = session(11, "Migrate the orders", "failed", "prj_api", {
      created_by: ada,
      updated_at: 11,
      ended_at: NOW - 5 * HOUR,
    });
    const sidebar = reload("/");
    // Twelve others ended since: the Agent Floor's list of ended sessions no longer has it.
    await sidebar.come({ workspaces, projects, sessions: [yours, [failed]] });
    expect(asked.sessions).toContainEqual({ yours: true, activeSince: NOW - 24 * HOUR, limit: 200 });
    const needs = [...sidebar.container.querySelectorAll('ul[aria-label="What needs you"] a')];
    expect(needs.map((a) => a.getAttribute("title"))).toEqual(["Migrate the orders\nFailed"]);
  });

  it("keeps its links out of Tab's way while it is closed", async () => {
    const waits = session(9, "Fix the build", "idle", "prj_api", {
      created_by: ada,
      updated_at: 9,
      status_reason: "Waiting for a message",
    });
    const sidebar = reload("/");
    await sidebar.come({ workspaces, projects, sessions: [working, [waits]] });
    const list = () => sidebar.container.querySelector('ul[aria-label="What needs you"]');
    expect(list()?.closest("[inert]")).toBeNull();
    const toggle = [...sidebar.container.querySelectorAll("button")].find((b) =>
      b.textContent?.startsWith("Needs you"),
    );
    act(() => toggle?.click());
    expect(toggle?.getAttribute("aria-expanded")).toBe("false");
    expect(list()?.closest("[inert]")).not.toBeNull();
  });

  it("folds into a rail of the projects' icons, and unfolds from it", async () => {
    const onFold = vi.fn();
    const sidebar = reload("/", () => {}, { folded: true, onFold });
    await sidebar.come({ workspaces, projects });
    const names = [...sidebar.container.querySelectorAll('a[href^="/p/"]')].map((a) =>
      a.getAttribute("aria-label"),
    );
    expect(names).toEqual(["web", "api"]);
    act(() => sidebar.container.querySelector<HTMLButtonElement>('button[aria-label^="Unfold"]')?.click());
    expect(onFold).toHaveBeenCalledWith(false);
  });

  it("keeps Settings in the rail, and Admin for admins", async () => {
    const member = reload("/", () => {}, { folded: true });
    await member.come({ workspaces, projects });
    const links = (c: HTMLElement) =>
      [...c.querySelectorAll('a[href="/settings"], a[href="/admin"]')].map((a) =>
        a.getAttribute("aria-label"),
      );
    expect(links(member.container)).toEqual(["Settings"]);
    const admin = reload(
      "/admin",
      (client) =>
        client.setQueryData(keys.me, {
          user: { id: "usr_ada", name: "Ada Lovelace", role: "admin" },
          claude: [],
        } as unknown as G.Me),
      { folded: true },
    );
    await admin.come({ workspaces, projects });
    expect(links(admin.container)).toEqual(["Admin", "Settings"]);
    expect(admin.container.querySelector('a[href="/admin"]')?.getAttribute("aria-current")).toBe("page");
  });
});

describe("the project a page is about", () => {
  /** The links under each project: its own pages, and its sessions. */
  const links = (c: HTMLElement) =>
    [...c.querySelectorAll("section a[aria-current]")].map((a) => a.textContent?.trim());

  it("keeps its own links on its pull requests page, the one it is on lit", async () => {
    const sidebar = reload("/pulls", () => {}, { scope: "p:api" });
    await sidebar.come({ workspaces, projects });
    const pulls = [...sidebar.container.querySelectorAll('a[href="/pulls"]')];
    // The one at the top, and the project's own.
    expect(pulls).toHaveLength(2);
    expect(pulls[0]?.getAttribute("aria-current")).toBeNull();
    expect(links(sidebar.container)).toEqual(["Pull requests"]);
    expect(sidebar.container.textContent).not.toContain("Tickets");
  });

  it("lights the pull requests at the top when the page lists every one", async () => {
    const sidebar = reload("/pulls");
    await sidebar.come({ workspaces, projects });
    const pulls = [...sidebar.container.querySelectorAll('a[href="/pulls"]')];
    expect(pulls).toHaveLength(1);
    expect(pulls[0]?.getAttribute("aria-current")).toBe("page");
  });

  it("is the project of a session no list has, as its page loaded it", async () => {
    const old = session(40, "Last month's", "stopped", "prj_api", { ended_at: NOW - 30 * 24 * HOUR });
    const sidebar = reload(`/s/${old.id}`);
    await sidebar.come({ workspaces, projects, session: old });
    const open = [...sidebar.container.querySelectorAll('a[href="/pulls"]')];
    expect(open).toHaveLength(2);
  });
});

/** The shell at a phone's width, where the sidebar is a drawer (or `wide`, from 768 px, beside the page). */
function phone({ wide = false } = {}) {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: wide && query.includes("min-width"),
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
  cleanups.push(() => vi.unstubAllGlobals());
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
  // happy-dom lays nothing out: a drawer 244 px wide.
  Object.defineProperty(drawer(), "offsetWidth", { value: 244 });
  const click = (el: Element | null | undefined) => act(() => (el as HTMLElement).click());
  const closeButtons = () => [...container.querySelectorAll('button[aria-label="Close menu"]')];
  return {
    /** Whether it is open, and whether it slides when it opens or closes. */
    state: () => ({
      open: drawer().classList.contains("translate-x-0"),
      slides: drawer().classList.contains("transition-[translate,display]"),
      shade: closeButtons().some((b) => !drawer().contains(b) && !b.classList.contains("invisible")),
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
      click([...document.querySelectorAll("button")].find((b) => shownText(b).trim() === name)),
    escape: () =>
      act(() => {
        document
          .querySelector('input[placeholder^="Search"]')
          ?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      }),
    palette: () => !!document.querySelector('input[placeholder^="Search"]'),
    drawer,
    /** The handle on its edge, beside the page. */
    edge: () => drawer().querySelector('[role="separator"]') as HTMLElement,
    /** How wide it shows, whether it animates as that changes, and what this browser keeps. */
    layout: () => ({
      shows: container.querySelector<HTMLElement>(":scope > div")?.style.getPropertyValue("--sidebar-w"),
      animates: drawer().classList.contains("md:transition-[width]"),
      folded: localStorage.getItem("fctl:sidebar-folded") === "1",
      width: localStorage.getItem("fctl:sidebar-width"),
    }),
    /** The shade over the page while it is open. */
    overlay: () => closeButtons().find((b) => !drawer().contains(b)) as HTMLElement | undefined,
    /** Something on the page, beside the drawer. */
    page: () => container.querySelector("main") as HTMLElement,
    /**
     * A finger on `el` at (`x0`, `y0`) that moves to each of `to`, 50 ms apart, then lets go
     * 10 ms later (or is `cancel`led), and what `during` sees after each move. Which moves
     * kept the page from scrolling, and whether the touch clicks as it ends.
     */
    drag(
      el: Element,
      [x0, y0]: [number, number],
      to: [number, number][],
      { end = "touchend", during }: { end?: "touchend" | "touchcancel"; during?: () => void } = {},
    ) {
      const touch = (x: number, y: number) =>
        new Touch({ identifier: 1, target: el, clientX: x, clientY: y });
      const at = <E extends Event>(e: E, ms: number) => Object.defineProperty(e, "timeStamp", { value: ms });
      const kept: boolean[] = [];
      act(() => {
        el.dispatchEvent(at(new TouchEvent("touchstart", { bubbles: true, touches: [touch(x0, y0)] }), 0));
      });
      to.forEach(([x, y], i) => {
        act(() => {
          const move = new TouchEvent("touchmove", {
            bubbles: true,
            cancelable: true,
            touches: [touch(x, y)],
          });
          el.dispatchEvent(at(move, (i + 1) * 50));
          kept.push(move.defaultPrevented);
        });
        during?.();
      });
      const last = at(
        new TouchEvent(end, { bubbles: true, cancelable: true, touches: [] }),
        to.length * 50 + 10,
      );
      act(() => {
        el.dispatchEvent(last);
      });
      return { kept, clicks: !last.defaultPrevented };
    },
  };
}

describe("folding the sidebar", () => {
  const fold = (key = "b") =>
    act(() => {
      document.body.dispatchEvent(
        new KeyboardEvent("keydown", { key, metaKey: true, ctrlKey: true, bubbles: true }),
      );
    });

  it("folds and unfolds it with ⌘B beside the page, and says so", () => {
    phone({ wide: true });
    expect(document.querySelector("button[aria-label='Fold the sidebar']")?.getAttribute("title")).toBe(
      "Fold the sidebar (Ctrl+B)",
    );
    fold();
    expect(localStorage.getItem("fctl:sidebar-folded")).toBe("1");
    fold();
    expect(localStorage.getItem("fctl:sidebar-folded")).toBeNull();
  });

  it("folds and unfolds it with ⌘\\ too, where \\ is a key of its own", () => {
    phone({ wide: true });
    fold("\\");
    expect(localStorage.getItem("fctl:sidebar-folded")).toBe("1");
    fold("\\");
    expect(localStorage.getItem("fctl:sidebar-folded")).toBeNull();
  });

  it("leaves it as it was where it is a drawer, which doesn't fold", () => {
    phone();
    fold();
    expect(localStorage.getItem("fctl:sidebar-folded")).toBeNull();
  });

  /** A mouse that presses on `el` at `x`, moves to each of `to` and lets go, and what `during` sees after each move. */
  const drag = <T,>(el: HTMLElement, x: number, to: number[], during: () => T) => {
    const pointer = (type: string, clientX: number) =>
      act(() => {
        el.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerId: 1, clientX }));
      });
    pointer("pointerdown", x);
    const seen = to.map((x) => {
      pointer("pointermove", x);
      return during();
    });
    pointer("pointerup", to.at(-1) ?? x);
    return seen;
  };

  it("drags it out of its rail by its edge: it slides out past halfway to its least width, then follows", () => {
    localStorage.setItem("fctl:sidebar-folded", "1");
    localStorage.setItem("fctl:sidebar-width", "320");
    const shell = phone({ wide: true });
    expect(shell.layout()).toEqual({ shows: "60px", animates: true, folded: true, width: "320" });
    expect(shell.edge().getAttribute("aria-valuetext")).toBe("Folded");
    const seen = drag(shell.edge(), 58, [140, 160, 300], shell.layout);
    expect(seen).toEqual([
      { shows: "60px", animates: true, folded: true, width: "320" },
      { shows: "240px", animates: true, folded: false, width: "240" },
      { shows: "302px", animates: false, folded: false, width: "302" },
    ]);
    expect(shell.layout()).toEqual({ shows: "302px", animates: true, folded: false, width: "302" });
    expect(shell.edge().getAttribute("aria-valuenow")).toBe("302");
    expect(shell.edge().hasAttribute("aria-valuetext")).toBe(false);
  });

  it("folds it into its rail as its edge passes halfway there, with the width it had before", () => {
    localStorage.setItem("fctl:sidebar-width", "320");
    const shell = phone({ wide: true });
    const seen = drag(shell.edge(), 320, [250, 160, 140, 100, 200, 260], shell.layout);
    expect(seen).toEqual([
      { shows: "250px", animates: false, folded: false, width: "250" },
      { shows: "240px", animates: false, folded: false, width: "240" },
      { shows: "60px", animates: true, folded: true, width: "320" },
      { shows: "60px", animates: true, folded: true, width: "320" },
      { shows: "240px", animates: true, folded: false, width: "240" },
      { shows: "260px", animates: false, folded: false, width: "260" },
    ]);
  });

  it("folds and unfolds it from its edge with the keys, and unfolds it as it was at first with a double click", () => {
    localStorage.setItem("fctl:sidebar-width", "256");
    const shell = phone({ wide: true });
    const key = (key: string) =>
      act(() => {
        shell.edge().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
      });
    key("ArrowLeft");
    expect(shell.layout()).toMatchObject({ folded: false, width: "240" });
    key("ArrowLeft");
    expect(shell.layout()).toMatchObject({ folded: true, width: "240" });
    key("ArrowLeft");
    expect(shell.layout()).toMatchObject({ folded: true, width: "240" });
    key("ArrowRight");
    expect(shell.layout()).toMatchObject({ shows: "240px", folded: false });
    key("Enter");
    expect(shell.layout()).toMatchObject({ shows: "60px", folded: true });
    key("Enter");
    expect(shell.layout()).toMatchObject({ shows: "240px", folded: false });
    key("Enter");
    act(() => {
      shell.edge().dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });
    expect(shell.layout()).toEqual({ shows: "288px", animates: true, folded: false, width: null });
  });
});

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

describe("the drawer under a finger", () => {
  it("comes in from the left edge of the screen as far as the finger drags it, and stays open past halfway", () => {
    const shell = phone();
    const seen: { open: boolean; shade: boolean; at: string; shown: number }[] = [];
    const { kept, clicks } = shell.drag(
      shell.page(),
      [10, 300],
      [
        [14, 301],
        [30, 302],
        [200, 305],
      ],
      {
        during: () =>
          seen.push({
            open: shell.state().open,
            shade: shell.state().shade,
            at: shell.drawer().style.translate,
            shown: Number(shell.overlay()?.style.opacity ?? 0),
          }),
      },
    );
    expect(seen).toEqual([
      // Not yet a drag: it could be a tap.
      { open: false, shade: false, at: "", shown: 0 },
      { open: true, shade: true, at: "-224px 0", shown: expect.closeTo(20 / 244) },
      { open: true, shade: true, at: "-54px 0", shown: expect.closeTo(190 / 244) },
    ]);
    expect(kept).toEqual([false, true, true]);
    // It slides the rest of the way, at its own pace, and the touch clicks nothing.
    expect(clicks).toBe(false);
    expect(shell.state()).toEqual({ open: true, slides: true, shade: true });
    expect(shell.drawer().style.translate).toBe("");
    expect(shell.drawer().style.transition).toBe("");
    expect(shell.overlay()?.style.opacity).toBe("");
  });

  it("goes back out when the finger flicks it back before letting go, even past halfway", () => {
    const shell = phone();
    shell.drag(
      shell.page(),
      [10, 300],
      [
        [60, 300],
        [250, 300],
        [200, 300],
      ],
    );
    expect(shell.state()).toEqual({ open: false, slides: true, shade: false });
    expect(shell.drawer().style.translate).toBe("");
  });

  it("goes back to where it was when the browser takes the touch", () => {
    const shell = phone();
    shell.drag(
      shell.page(),
      [10, 300],
      [
        [60, 300],
        [200, 300],
      ],
      { end: "touchcancel" },
    );
    expect(shell.state()).toEqual({ open: false, slides: true, shade: false });
    shell.open();
    shell.drag(
      shell.drawer(),
      [200, 300],
      [
        [150, 300],
        [20, 300],
      ],
      { end: "touchcancel" },
    );
    expect(shell.state()).toEqual({ open: true, slides: true, shade: true });
  });

  it("goes out as a finger drags it by itself or the shade to the left", () => {
    const shell = phone();
    shell.open();
    const { kept, clicks } = shell.drag(
      shell.drawer(),
      [200, 300],
      [
        [190, 300],
        [120, 302],
        [20, 305],
      ],
    );
    expect(kept).toEqual([true, true, true]);
    expect(clicks).toBe(false);
    expect(shell.state()).toEqual({ open: false, slides: true, shade: false });
    shell.open();
    const byShade = shell.drag(
      shell.overlay() as HTMLElement,
      [300, 300],
      [
        [280, 300],
        [100, 300],
      ],
    );
    expect(byShade.clicks).toBe(false);
    expect(shell.state()).toEqual({ open: false, slides: true, shade: false });
  });

  it("leaves to the page a finger that scrolls, or moves away from where the drawer goes", () => {
    const shell = phone();
    expect(
      shell.drag(
        shell.page(),
        [10, 300],
        [
          [14, 320],
          [16, 400],
        ],
      ).kept,
    ).toEqual([false, false]);
    expect(shell.state().open).toBe(false);
    shell.open();
    // Up and down its list, or to the right.
    expect(
      shell.drag(
        shell.drawer(),
        [100, 300],
        [
          [104, 320],
          [90, 400],
        ],
      ).kept,
    ).toEqual([false, false]);
    expect(
      shell.drag(
        shell.drawer(),
        [100, 300],
        [
          [130, 300],
          [20, 300],
        ],
      ).kept,
    ).toEqual([false, false]);
    expect(shell.state().open).toBe(true);
  });

  it("comes in only from the edge of the screen", () => {
    const shell = phone();
    const { kept, clicks } = shell.drag(
      shell.page(),
      [80, 300],
      [
        [120, 300],
        [300, 300],
      ],
    );
    expect(kept).toEqual([false, false]);
    expect(clicks).toBe(true);
    expect(shell.state().open).toBe(false);
  });

  it("stays beside the page from 768 px up", () => {
    const shell = phone({ wide: true });
    expect(
      shell.drag(
        shell.page(),
        [10, 300],
        [
          [60, 300],
          [200, 300],
        ],
      ).kept,
    ).toEqual([false, false]);
    expect(shell.state().open).toBe(false);
  });

  it("follows a flick short of halfway, but not a finger that stopped before it let go", () => {
    const shell = phone();
    // 90 px of 244, quickly.
    shell.drag(
      shell.page(),
      [10, 300],
      [
        [20, 300],
        [60, 300],
        [100, 300],
      ],
    );
    expect(shell.state().open).toBe(true);
    shell.close();
    // As far, then still for 100 ms.
    shell.drag(
      shell.page(),
      [10, 300],
      [
        [20, 300],
        [60, 300],
        [100, 300],
        [100, 300],
        [100, 300],
      ],
    );
    expect(shell.state().open).toBe(false);
  });

  it("doesn't come in under the palette", () => {
    const shell = phone();
    shell.open();
    shell.search();
    shell.shade();
    expect(shell.palette()).toBe(true);
    const palette = document.querySelector('input[placeholder^="Search"]') as HTMLElement;
    expect(
      shell.drag(
        palette,
        [10, 300],
        [
          [60, 300],
          [200, 300],
        ],
      ).kept,
    ).toEqual([false, false]);
    expect(shell.state().open).toBe(false);
  });

  it("doesn't come in under a finger scrolling back a row that scrolls sideways", () => {
    const shell = phone();
    const row = document.createElement("nav");
    const tab = document.createElement("a");
    row.append(tab);
    shell.page().append(row);
    Object.defineProperty(row, "scrollLeft", { value: 40 });
    expect(
      shell.drag(
        tab,
        [10, 300],
        [
          [60, 300],
          [200, 300],
        ],
      ).kept,
    ).toEqual([false, false]);
    expect(shell.state().open).toBe(false);
  });

  it("comes in from past the notch, in landscape", () => {
    const shell = phone();
    shell.drawer().style.paddingLeft = "44px";
    shell.drag(
      shell.page(),
      [70, 300],
      [
        [80, 300],
        [300, 300],
        [300, 300],
        [300, 300],
      ],
    );
    expect(shell.state().open).toBe(true);
  });

  it("goes on following its finger when another one lets go", () => {
    const shell = phone();
    const el = shell.page();
    const touch = (id: number, x: number) =>
      new Touch({ identifier: id, target: el, clientX: x, clientY: 300 });
    const send = (type: string, touches: Touch[]) =>
      act(() => {
        el.dispatchEvent(new TouchEvent(type, { bubbles: true, cancelable: true, touches }));
      });
    send("touchstart", [touch(1, 10)]);
    send("touchmove", [touch(1, 60)]);
    // A second finger taps elsewhere: it touches, and lets go.
    act(() => {
      document.body.dispatchEvent(
        new TouchEvent("touchstart", { bubbles: true, touches: [touch(1, 60), touch(2, 300)] }),
      );
    });
    send("touchend", [touch(1, 60)]);
    expect(shell.drawer().style.translate).toBe("-194px 0");
    send("touchmove", [touch(1, 120)]);
    expect(shell.drawer().style.translate).toBe("-134px 0");
  });

  it("lets go of a finger whose touch ended off the page, and of one whose end never came", () => {
    const shell = phone();
    // What the finger touched leaves the page as it drags: its end still comes to it.
    const gone = document.createElement("span");
    shell.page().append(gone);
    const touch = (x: number) => new Touch({ identifier: 1, target: gone, clientX: x, clientY: 300 });
    act(() => {
      gone.dispatchEvent(new TouchEvent("touchstart", { bubbles: true, touches: [touch(10)] }));
      gone.dispatchEvent(
        new TouchEvent("touchmove", { bubbles: true, cancelable: true, touches: [touch(60)] }),
      );
    });
    act(() => gone.remove());
    act(() => {
      gone.dispatchEvent(
        new TouchEvent("touchmove", { bubbles: true, cancelable: true, touches: [touch(240)] }),
      );
      gone.dispatchEvent(new TouchEvent("touchend", { bubbles: true, cancelable: true, touches: [] }));
    });
    expect(shell.state()).toEqual({ open: true, slides: true, shade: true });
    expect(shell.drawer().style.translate).toBe("");
    // A drag whose end never came: the next touch puts the drawer back, and drags it anew.
    shell.close();
    const lost = new Touch({ identifier: 3, target: shell.page(), clientX: 10, clientY: 300 });
    act(() => {
      shell.page().dispatchEvent(new TouchEvent("touchstart", { bubbles: true, touches: [lost] }));
      shell.page().dispatchEvent(
        new TouchEvent("touchmove", {
          bubbles: true,
          cancelable: true,
          touches: [new Touch({ identifier: 3, target: shell.page(), clientX: 100, clientY: 300 })],
        }),
      );
    });
    expect(shell.drawer().style.translate).toBe("-154px 0");
    shell.drag(
      shell.page(),
      [10, 300],
      [
        [60, 300],
        [240, 300],
        [240, 300],
        [240, 300],
      ],
    );
    expect(shell.state()).toEqual({ open: true, slides: true, shade: true });
    expect(shell.drawer().style.translate).toBe("");
  });

  it("fades the shade with the drawer, and neither moves for people who reduce motion", () => {
    const shell = phone();
    const shade = shell.overlay() as HTMLElement;
    expect(shade.className).toContain("transition-[opacity,visibility]");
    expect(shade.className).toContain("motion-reduce:transition-none");
    expect(shell.drawer().className).toContain("motion-reduce:transition-none");
    shell.open();
    shell.close();
    // Still there as it fades out, but out of reach.
    expect(shell.overlay()).toBe(shade);
    expect(shade.classList).toContain("invisible");
  });
});
