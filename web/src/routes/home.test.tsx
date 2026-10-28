import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { draftId, saveText } from "@/lib/draft";
import { keys } from "@/lib/queries";
import { Home } from "./home";

// No request is answered: the page shows what the cache holds, and a session it starts goes on starting.
vi.mock("@/lib/api", async (original) => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { ...(await original<typeof import("@/lib/api")>()), api: pending, workspaces: pending };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children }: { to: string; children: ReactNode }) => <a href={to}>{children}</a>,
  useNavigate: () => () => {},
}));
// The page without the live feed and WebGL.
vi.mock("@/lib/feed", () => ({
  useFeed: () => ({ connected: false, runners: [], sessions: new Map() }),
  feed: { onPulse: () => () => {} },
}));
vi.mock("@hyperstructure/design/gl/foundry", () => ({
  LANES: 8,
  foundry: () => ({ scene: {}, setLanes() {}, pulse() {}, pointer() {}, pointerLeave() {} }),
}));
vi.mock("@/components/ui/gl-canvas", () => ({ GLCanvas: () => null }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  localStorage.clear();
  vi.useRealTimers();
});

const workspace = (id: string, name: string): G.Workspace =>
  ({ id, slug: name.toLowerCase(), name, color: "blue", role: "member", blocked: false }) as G.Workspace;
const project = (slug: string, w: G.Workspace, role: G.ProjectRole = "editor"): G.Project =>
  ({
    id: `prj_${slug}`,
    slug,
    name: slug,
    workspace: { id: w.id, slug: w.slug, name: w.name },
    role,
    settings: {},
  }) as G.Project;

const frog = workspace("ws_frog", "Frog");
const kind = workspace("ws_kind", "Kind Machine");
// Where Ada only watches.
const watched = workspace("ws_watched", "Watched");
const projects = [
  project("cook-it", frog),
  project("frog-api", frog),
  project("kind-docs", kind, "viewer"),
  project("kind-web", kind),
  project("kind-app", kind),
  project("watched-web", watched, "viewer"),
];

/** The Agent Floor, for Ada in Frog, Kind Machine and Watched. */
function open() {
  const client = new QueryClient();
  client.setQueryData(keys.me, {
    user: { id: "usr_ada", name: "Ada Lovelace", role: "member" },
    claude: [],
  } as unknown as G.Me);
  client.setQueryData(keys.workspaces, { workspaces: [frog, kind, watched], invitations: [], limit: 5 });
  client.setQueryData(keys.projects, { projects });
  client.setQueryData(keys.runners, { runners: [] });
  client.setQueryData(keys.sessions({ status: "active", limit: 50 }), { sessions: [] });
  client.setQueryData(keys.sessions({ status: "ended", limit: 12 }), { sessions: [] });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <Home />
      </QueryClientProvider>,
    ),
  );
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const picker = () =>
    container.querySelector<HTMLElement>('[role=combobox][aria-label="Workspace and project"]');
  return {
    /** The project the composer starts a session in, as its picker says it. */
    project: () =>
      picker()?.querySelector(":scope > .grid:not([aria-hidden]) > span:not([aria-hidden])")?.textContent,
    /** The workspace whose work the page shows. */
    showing: () =>
      container.querySelector('fieldset[aria-label="Show the work of"] [aria-pressed=true]')?.textContent,
    /** Shows a workspace's work, or every workspace's. */
    show: (name: string) => {
      const chip = [
        ...container.querySelectorAll<HTMLElement>('fieldset[aria-label="Show the work of"] button'),
      ].find((b) => b.textContent?.includes(name));
      if (!chip) throw new Error(`no ${name}`);
      act(() => chip.click());
    },
    /** Picks a project in the composer, and lets its list close (on fake timers). */
    pick: (slug: string) => {
      act(() => picker()?.click());
      const option = [...container.querySelectorAll<HTMLElement>("[role=option]")].find(
        (o) => o.querySelector(".truncate")?.textContent === slug,
      );
      if (!option) throw new Error(`no ${slug}`);
      act(() => option.click());
      act(() => vi.advanceTimersByTime(1000));
    },
    start: () =>
      act(async () => {
        container.querySelector("form")?.requestSubmit();
      }),
  };
}

const lastTargets = () => JSON.parse(localStorage.getItem("fctl:last-targets") ?? "{}");

describe("the Agent Floor's composer", () => {
  it("starts in the project picked there last", () => {
    localStorage.setItem("fctl:last-target", "frog-api");
    expect(open().project()).toBe("Frog / frog-api");
  });

  it("goes to the workspace whose work the page shows, to the project it had there last", () => {
    localStorage.setItem("fctl:last-target", "frog-api");
    localStorage.setItem("fctl:last-targets", JSON.stringify({ ws_kind: "kind-app" }));
    const floor = open();
    expect(floor.project()).toBe("Frog / frog-api");
    floor.show("Kind Machine");
    expect(floor.showing()).toContain("Kind Machine");
    expect(floor.project()).toBe("Kind Machine / kind-app");
    floor.show("Frog");
    expect(floor.project()).toBe("Frog / frog-api");
    // The next page starts where this one is.
    expect(localStorage.getItem("fctl:last-target")).toBe("frog-api");
    expect(lastTargets()).toEqual({ ws_frog: "frog-api", ws_kind: "kind-app" });
  });

  it("goes to a workspace's first project you start sessions in, when it had none there", () => {
    const floor = open();
    expect(floor.project()).toBe("Frog / cook-it");
    // Ada only watches kind-docs.
    floor.show("Kind Machine");
    expect(floor.project()).toBe("Kind Machine / kind-web");
    expect(localStorage.getItem("fctl:last-target")).toBe("kind-web");
  });

  it("stays where it is when the page shows every workspace, or one where it can't start sessions", () => {
    const floor = open();
    floor.show("Kind Machine");
    floor.show("All workspaces");
    expect(floor.project()).toBe("Kind Machine / kind-web");
    floor.show("Watched");
    expect(floor.project()).toBe("Kind Machine / kind-web");
  });

  it("keeps a project picked in another workspace, and goes back to it in its workspace", () => {
    vi.useFakeTimers();
    const floor = open();
    floor.show("Kind Machine");
    floor.pick("kind-app");
    floor.pick("cook-it");
    expect(floor.project()).toBe("Frog / cook-it");
    expect(floor.showing()).toContain("Kind Machine");
    floor.show("All workspaces");
    floor.show("Kind Machine");
    expect(floor.project()).toBe("Kind Machine / kind-app");
  });

  it("stays in the project a session is starting in", async () => {
    saveText(draftId("usr_ada"), { prompt: "Make the soup", project: "cook-it" });
    const floor = open();
    expect(floor.project()).toBe("Frog / cook-it");
    await floor.start();
    floor.show("Kind Machine");
    expect(floor.project()).toBe("Frog / cook-it");
  });
});
