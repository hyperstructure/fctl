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
const toast = vi.hoisted(() => vi.fn());
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
vi.mock("@/components/ui", async (actual) => ({
  ...(await actual<typeof import("@/components/ui")>()),
  toast,
}));
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
  id_tokens: true,
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
    expect(s.nav()).toEqual([
      "General (open)",
      "Agents",
      "Pull requests",
      "Tickets",
      "Runners",
      "Sandbox",
      "Secrets",
      "Identity OIDC",
      "Webhooks",
    ]);
    expect(s.heading()).toBe("General");
    // In groups: the project, how its agents work, where they run, what it tells other tools.
    expect(
      s.container.querySelector('nav[aria-label="Shop\'s settings"]')?.querySelectorAll("ul").length,
    ).toBe(4);
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

  it("saves where sessions compact: Claude's default, a size offered, or one typed", async () => {
    vi.useFakeTimers();
    try {
      const s = open("agents");
      const field = (label: string) => {
        const l = [...s.container.querySelectorAll("label")].find((e) => e.textContent === label);
        return s.container.querySelector(`[id="${l?.htmlFor}"]`) as HTMLElement | null;
      };
      const choose = (text: string) => {
        act(() => field("Compact near")?.click());
        const option = [...s.container.querySelectorAll<HTMLElement>('[role="option"]')].find((o) =>
          o.textContent?.startsWith(text),
        );
        act(() => option?.click());
        act(() => vi.advanceTimersByTime(1000));
      };
      const type = (text: string) =>
        act(() => {
          const input = field("Tokens") as HTMLInputElement;
          Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
          input.dispatchEvent(new Event("input", { bubbles: true }));
        });
      // The mutation tells the page on a timer; the page then gets the project as saved, as the
      // route gives it from the cache.
      const save = async () => {
        await act(async () => s.button("Save").click());
        await act(async () => vi.advanceTimersByTime(1000));
        const saved = queryClient.getQueryData<G.Project>(keys.project("shop"));
        if (saved) s.show(saved);
      };
      expect(field("Tokens")).toBeNull();
      choose("Another size");
      // Nothing typed yet: it says so rather than save.
      await save();
      expect(updates).toHaveLength(0);
      expect(s.container.textContent).toContain("Type a size from 100K to 1M tokens, like 300K");
      type("300k");
      await save();
      expect(updates.at(-1)?.body.settings?.compact_window).toBe(300_000);
      expect(updates.at(-1)?.body.settings?.instructions).toBe("Run the tests first.");
      // Saved, the field shows it as it'd be typed.
      expect((field("Tokens") as HTMLInputElement).value).toBe("300K");
      // Another choice and back leaves nothing to save, and the size typed comes back with it.
      choose("400K");
      choose("Another size");
      expect((field("Tokens") as HTMLInputElement).value).toBe("300K");
      expect(s.button("Save")).toBeUndefined();
      type("350k");
      choose("Claude's default");
      choose("Another size");
      expect((field("Tokens") as HTMLInputElement).value).toBe("350k");
      choose("Another size");
      choose("400K");
      await save();
      expect(updates.at(-1)?.body.settings?.compact_window).toBe(400_000);
      // A size typed, then the saved choice again: nothing to save.
      choose("Another size");
      type("500k");
      expect(s.button("Save")).toBeDefined();
      choose("400K");
      expect(s.button("Save")).toBeUndefined();
      choose("Claude's default");
      await save();
      // Sent as null, which the control plane reads as Claude's default rather than as unsent.
      expect(updates.at(-1)?.body.settings?.compact_window).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("saves how long stale notes stay: a number of days, or never, sent as null", async () => {
    vi.useFakeTimers();
    try {
      const s = open("agents", project({ settings: { ...settings, stale_note_days: 45 } }));
      const field = () => {
        const l = [...s.container.querySelectorAll("label")].find(
          (e) => e.textContent === "Delete stale notes",
        );
        return s.container.querySelector(`[id="${l?.htmlFor}"]`) as HTMLElement;
      };
      const options = () => [
        ...s.container.querySelectorAll<HTMLElement>('[role="listbox"] [role="option"]'),
      ];
      const choose = (text: string) => {
        act(() => field().click());
        const option = [...s.container.querySelectorAll<HTMLElement>('[role="option"]')].find(
          (o) => o.textContent === text,
        );
        act(() => option?.click());
        act(() => vi.advanceTimersByTime(1000));
      };
      const save = async () => {
        await act(async () => s.button("Save").click());
        await act(async () => vi.advanceTimersByTime(1000));
        const saved = queryClient.getQueryData<G.Project>(keys.project("shop"));
        if (saved) s.show(saved);
      };
      // A number set from the CLI shows among those offered, in order.
      act(() => field().click());
      expect(options().map((o) => o.textContent)).toEqual([
        "Never: they stay until someone deletes them",
        "After a day stale",
        "After 3 days stale",
        "After 7 days stale",
        "After 14 days stale",
        "After 30 days stale",
        "After 45 days stale",
        "After 90 days stale",
        "After 365 days stale",
      ]);
      act(() => field().click());
      choose("After 7 days stale");
      await save();
      expect(updates.at(-1)?.body.settings?.stale_note_days).toBe(7);
      expect(updates.at(-1)?.body.settings?.instructions).toBe("Run the tests first.");
      choose("Never: they stay until someone deletes them");
      await save();
      // Sent as null, which the control plane reads as never rather than as unsent.
      expect(updates.at(-1)?.body.settings).toHaveProperty("stale_note_days", null);
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

describe("a project's Runners page", () => {
  const acme = { id: "wsp_acme", slug: "acme", name: "Acme" } as G.WorkspaceRef;
  const runner = (r: Partial<G.Runner> & { id: string }): G.Runner =>
    ({
      name: r.id,
      status: "online",
      drivers: [],
      capacity: 4,
      active_sessions: 0,
      labels: [],
      created_at: 0,
      workspaces: [],
      yours: false,
      personal: [],
      ...r,
    }) as G.Runner;
  const pool: G.PoolDemand = {
    pool: { id: "enr_large", name: "AWS large", labels: ["aws", "large"], placement: "packed" },
    runners: { online: 4, busy: 2, draining: 0, pending: 0, starting: 0, offline: 0 },
    slots: { capacity: 16, busy: 5, free: 11 },
    waiting: { sessions: 0 },
    wanted: 2,
    shape: { capacity: 4, cpus: 16, memory_mb: 65536, driver: "docker", version: "0.38.0", seen_at: 0 },
    at: 0,
  };

  // Grace's own pool, and her runner in it, which run only her sessions.
  const graces: G.PoolDemand = {
    ...pool,
    pool: {
      id: "enr_grace",
      name: "Grace's GPUs",
      labels: ["gpu"],
      placement: "packed",
      owner: { id: "usr_grace", name: "Grace" },
    },
  };

  /**
   * The page, with Acme's runners and pools as its admins read them, or without the pools while
   * they load (`pools: false`).
   */
  function runners(labels: string[], fallback?: number, pools = true) {
    queryClient.setQueryData(keys.workspace("wsp_acme"), {
      ...acme,
      primary: { id: "usr_ada", name: "Ada" },
    } as G.Workspace);
    queryClient.setQueryData<G.RunnerList>(keys.workspaceRunners("wsp_acme"), {
      runners: [
        runner({ id: "rnr_1", labels: ["aws", "large"], pool: { id: "enr_large", name: "AWS large" } }),
        runner({ id: "rnr_mac", name: "studio-mac" }),
        runner({
          id: "rnr_grace",
          labels: ["gpu"],
          pool: { id: "enr_grace", name: "Grace's GPUs" },
          personal: ["wsp_acme"],
        }),
      ],
    });
    queryClient.removeQueries({ queryKey: keys.workspaceDemand("wsp_acme") });
    if (pools)
      queryClient.setQueryData<G.WorkspaceDemand>(keys.workspaceDemand("wsp_acme"), {
        pools: [pool, graces],
        waiting: { sessions: 0 },
        unpooled: 0,
        at: 0,
      });
    return open(
      "runners",
      project({
        workspace: acme,
        settings: { ...settings, runner_labels: labels, runner_fallback_minutes: fallback ?? null },
      }),
    );
  }
  const shown = (s: ReturnType<typeof open>, n = 0) =>
    s.container
      .querySelectorAll('[role="combobox"]')
      [n]?.querySelector("span:not([aria-hidden]) > span:not([aria-hidden])")?.textContent;

  it("offers any runner, or a pool by its name with what its runners give", () => {
    vi.useFakeTimers();
    try {
      const s = runners([]);
      expect(s.heading()).toBe("Runners");
      expect(shown(s)).toBe("Any runner");
      act(() => (s.container.querySelector('[role="combobox"]') as HTMLElement).click());
      const options = [...s.container.querySelectorAll<HTMLElement>('[role="option"]')].map(
        (o) => o.textContent,
      );
      expect(options).toContain("AWS large4 runners with 16 CPUs and 64 GiB · aws, large");
      // A member's own pool runs only their sessions: no project chooses it.
      expect(options.some((o) => o?.startsWith("Grace's GPUs"))).toBe(false);
      const large = [...s.container.querySelectorAll<HTMLElement>('[role="option"]')].find((o) =>
        o.textContent?.startsWith("AWS large"),
      );
      act(() => large?.click());
      act(() => vi.advanceTimersByTime(1000));
      expect((s.container.querySelector("input.font-mono") as HTMLInputElement).value).toBe("aws, large");
      expect(s.button("Save")).toBeDefined();
    } finally {
      vi.useRealTimers();
    }
  });

  it("names the pool a project's labels choose, and how long its sessions wait for it", () => {
    const s = runners(["large", "aws"], 15);
    expect(shown(s)).toBe("AWS large");
    expect(shown(s, 1)).toBe("After 15 minutes, any runner");
    expect(s.container.textContent).toContain(
      "Its sessions wait for one of those runners, then go to any of Acme's.",
    );
    expect(s.container.querySelector(".text-amber")).toBeNull();
  });

  it("says in amber when no runner has its labels, and who adds one", () => {
    // Grace's own pool and runner have them, but run only her sessions.
    const s = runners(["gpu"]);
    expect(shown(s)).toBe("Runners with labels you choose");
    expect(s.container.querySelector(".text-amber")?.textContent).toBe(
      "No runner of Acme has the label gpu: its sessions wait until one does. Ada and its other admins add runners and pools on its Runners tab.",
    );
  });

  it("says nothing of labels no runner has until it knows the pools", () => {
    const s = runners(["arm"], undefined, false);
    expect(s.container.querySelector(".text-amber")).toBeNull();
  });

  it("saves the labels and the wait, for sessions that start from then on", async () => {
    const s = runners(["gpu"]);
    const input = s.container.querySelector("input.font-mono") as HTMLInputElement;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, "gpu, Large");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    toast.mockClear();
    await act(async () => s.button("Save").click());
    expect(updates.at(-1)?.body.settings).toMatchObject({
      runner_labels: ["gpu", "large"],
      runner_fallback_minutes: null,
    });
    // One toast, which says where its sessions run now.
    expect(toast.mock.calls).toEqual([["Sessions that start from now on run on runners with gpu, large"]]);
  });
});
