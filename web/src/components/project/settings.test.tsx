import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import type { ProjectView } from "@/lib/project-page";
import { keys, queryClient } from "@/lib/queries";
import { ProjectSettings } from "./settings";

// No request is answered, but a project's update, which answers with what it was sent: the pages
// show what the cache holds. Each update is noted.
const updates = vi.hoisted(() => [] as { id: string; body: G.UpdateProjectRequest }[]);
const blocker = vi.hoisted(
  () =>
    ({ disabled: [] }) as {
      disabled: boolean[];
      opts?: {
        shouldBlockFn: (a: {
          current: { pathname: string; search: unknown };
          next: { pathname: string; search: unknown };
        }) => boolean;
      };
    },
);
vi.mock("@/lib/api", async (real) => {
  const pending = new Proxy(
    {},
    {
      get: (_, name) =>
        name === "updateProject"
          ? (id: string, body: G.UpdateProjectRequest) => {
              updates.push({ id, body });
              const before = queryClient.getQueryData<G.Project>(keys.project("shop"));
              // As the control plane stores it: its text trimmed.
              const name = body.name?.trim();
              return Promise.resolve({ ...before, ...body, ...(name !== undefined && { name }) });
            }
          : () => new Promise(() => {}),
    },
  );
  return { ...(await real<typeof import("@/lib/api")>()), api: pending, workspaces: pending };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    search,
    children,
    className,
    ...rest
  }: {
    search?: { tab?: string };
    children: ReactNode;
    className?: string;
  }) => (
    <a href={search?.tab ? `/p/shop?tab=${search.tab}` : "/p/shop"} className={className} {...rest}>
      {children}
    </a>
  ),
  useNavigate: () => vi.fn(),
  useBlocker: (opts: { disabled?: boolean } & NonNullable<typeof blocker.opts>) => {
    blocker.disabled.push(!!opts.disabled);
    blocker.opts = opts;
    return { status: "idle" };
  },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  updates.splice(0);
  blocker.disabled.splice(0);
});

const settings: G.ProjectSettings = {
  auto_pull_request: true,
  remote_control: false,
  claude_ai_connectors: false,
  credit_people: true,
  credit_claude: true,
  publish_screenshots: false,
  package_caches: false,
  project_memory: true,
  retrospectives: true,
  instructions: "Run the tests first.",
  env: { NODE_ENV: "development" },
};

const project = (over: Partial<G.Project> = {}): G.Project => ({
  id: "prj_1",
  slug: "shop",
  name: "Shop",
  default_branch: "main",
  settings,
  role: "maintainer",
  created_by: { id: "usr_ada", name: "Ada" },
  created_at: 0,
  updated_at: 0,
  active_sessions: 0,
  total_sessions: 3,
  ...over,
});

/** The project's Settings at the address `view`, as `p` has it for its caller. */
function open(view: ProjectView, p = project()) {
  queryClient.setQueryData(keys.project(p.slug), p);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const show = (q: G.Project) =>
    act(() =>
      root.render(
        <QueryClientProvider client={queryClient}>
          <ProjectSettings project={q} view={view} />
        </QueryClientProvider>,
      ),
    );
  show(p);
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  /** Where the navigation beside the page goes, and which page it calls current. */
  const nav = () => {
    const list = container.querySelector('nav[aria-label="Shop\'s settings"]');
    return [...(list?.querySelectorAll("a") ?? [])].map(
      (a) => `${a.textContent}${a.getAttribute("aria-current") ? " (open)" : ""}`,
    );
  };
  const heading = () => container.querySelector("h2")?.textContent;
  const button = (text: string) =>
    [...container.querySelectorAll("button")].find((b) => b.textContent === text) as HTMLButtonElement;
  const toggle = (label: string) =>
    container.querySelector(`button[role="switch"][aria-label="${label}"]`) as HTMLButtonElement;
  return { container, nav, heading, button, toggle, show };
}

describe("a project's Settings", () => {
  it("lists every page for its maintainers, in groups, with General beside them", () => {
    const s = open("settings");
    expect(s.nav()).toEqual(["General (open)", "Agents", "Pull requests", "Tickets", "Sandbox", "Secrets"]);
    expect(s.heading()).toBe("General");
    // In groups: the project, how its agents work, where they run.
    expect(
      s.container.querySelector('nav[aria-label="Shop\'s settings"]')?.querySelectorAll("ul").length,
    ).toBe(3);
  });

  it("shows the rest of the project the one page everyone reads, alone", () => {
    const s = open("settings", project({ role: "editor" }));
    expect(s.nav()).toEqual([]);
    expect(s.heading()).toBe("Tickets");
    expect([...s.container.querySelectorAll("a")].some((a) => a.textContent === "Settings")).toBe(false);
  });

  it("opens the page its address names, with the way back to the list on a phone", () => {
    const s = open("sandbox");
    expect(s.heading()).toBe("Sandbox");
    expect(s.nav()).toContain("Sandbox (open)");
    const back = [...s.container.querySelectorAll("a")].find((a) => a.textContent === "Settings");
    expect(back?.getAttribute("href")).toBe("/p/shop?tab=settings");
  });

  it("says on a phone what each page holds now", () => {
    const s = open("settings", project({ settings: { ...settings, model: "opus", effort: "high" } }));
    const list = [...s.container.querySelectorAll("nav")].find((n) => n.className.includes("@2xl:hidden"));
    expect(list?.textContent).toContain("GeneralSessions start from main");
    expect(list?.textContent).toContain("AgentsOpus · high effort");
    expect(list?.textContent).toContain("TicketsGitHub Issues");
  });

  it("offers to save only once something changed, and asks before leaving with it unsaved", () => {
    const s = open("agents");
    expect(s.button("Save")).toBeUndefined();
    expect(blocker.disabled.at(-1)).toBe(true);
    act(() => s.toggle("Remote Control").click());
    expect(s.button("Save")).toBeDefined();
    expect(s.container.textContent).toContain("Sessions that start from now on take it.");
    expect(blocker.disabled.at(-1)).toBe(false);
    // Turned back, there's nothing to save.
    act(() => s.toggle("Remote Control").click());
    expect(s.button("Save")).toBeUndefined();
  });

  it("saves a page's changes with the rest of the project's settings as they are", async () => {
    const s = open("pull-requests");
    act(() => s.toggle("Failed checks").click());
    expect(s.container.textContent).toContain("Follow-ups change at once");
    await act(async () => s.button("Save").click());
    expect(updates).toHaveLength(1);
    const sent = updates[0]?.body.settings;
    expect(sent?.follow_ups?.checks).toBe(true);
    // What other pages edit goes back as it was: a PATCH replaces all of them.
    expect(sent?.instructions).toBe("Run the tests first.");
    expect(sent?.env).toEqual({ NODE_ENV: "development" });
  });

  it("has nothing left to save once saved, though the control plane tidied what it stored", async () => {
    const s = open("general");
    const name = s.container.querySelector("input") as HTMLInputElement;
    act(() => {
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      set?.call(name, "Shop ");
      name.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(s.button("Save")).toBeDefined();
    await act(async () => s.button("Save").click());
    expect(updates[0]?.body.name).toBe("Shop ");
    // It stored "Shop", as it was: the page shows that, and asks nothing before leaving.
    expect(name.value).toBe("Shop");
    expect(s.button("Save")).toBeUndefined();
    expect(blocker.disabled.at(-1)).toBe(true);
  });

  it("asks before leaving for another page, not for an address that shows this one", () => {
    open("settings");
    const leaves = (search: unknown, pathname = "/p/shop") =>
      blocker.opts?.shouldBlockFn({
        current: { pathname: "/p/shop", search: { tab: "settings" } },
        next: { pathname, search },
      });
    // Settings itself shows General: its own entry in the list stays on it.
    expect(leaves({ tab: "general" })).toBe(false);
    expect(leaves({ tab: "agents" })).toBe(true);
    expect(leaves({ tab: "memory" })).toBe(true);
    expect(leaves({}, "/")).toBe(true);
  });

  it("puts the model chosen back with the rest of the draft when someone saves elsewhere", () => {
    vi.useFakeTimers();
    try {
      const s = open("agents");
      const model = s.container.querySelector('[role="combobox"]') as HTMLButtonElement;
      // What the Model list shows chosen (ui/select.test.tsx): its trigger holds the options too.
      const shown = () =>
        model.querySelector("span:not([aria-hidden]) > span:not([aria-hidden])")?.textContent;
      expect(shown()).toBe("Claude's default");
      act(() => model.click());
      const sonnet = [...s.container.querySelectorAll<HTMLElement>('[role="option"]')].find((o) =>
        o.textContent?.startsWith("Sonnet"),
      );
      act(() => sonnet?.click());
      act(() => vi.advanceTimersByTime(1000));
      expect(shown()).toBe("Sonnet");
      expect(s.button("Save")).toBeDefined();
      // Someone saves Remote Control meanwhile: the page starts again from what is saved, its model too.
      s.show(project({ settings: { ...settings, remote_control: true } }));
      act(() => vi.advanceTimersByTime(1000));
      expect(shown()).toBe("Claude's default");
      expect(s.button("Save")).toBeUndefined();
    } finally {
      vi.useRealTimers();
    }
  });

  it("drops the changes Cancel throws away", () => {
    const s = open("agents");
    act(() => s.toggle("Project memory").click());
    expect(s.toggle("Project memory").getAttribute("aria-checked")).toBe("false");
    act(() => s.button("Cancel").click());
    expect(s.toggle("Project memory").getAttribute("aria-checked")).toBe("true");
    expect(s.button("Save")).toBeUndefined();
  });

  it("takes a change saved elsewhere in place of what it showed", () => {
    const s = open("agents");
    s.show(project({ settings: { ...settings, remote_control: true } }));
    expect(s.toggle("Remote Control").getAttribute("aria-checked")).toBe("true");
    expect(s.button("Save")).toBeUndefined();
  });

  it("leaves the environment unchanged by lines that set nothing", () => {
    const s = open("sandbox");
    const env = s.container.querySelector(
      "textarea[placeholder='NODE_ENV=development']",
    ) as HTMLTextAreaElement;
    const type = (text: string) =>
      act(() => {
        const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
        set?.call(env, text);
        env.dispatchEvent(new Event("input", { bubbles: true }));
      });
    type("NODE_ENV=development\n\n# for the tests");
    expect(s.button("Save")).toBeUndefined();
    type("NODE_ENV=production");
    expect(s.button("Save")).toBeDefined();
  });
});
