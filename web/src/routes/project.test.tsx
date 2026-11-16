import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { queryClient } from "@/lib/queries";
import { Sessions } from "./project";

const sessions = vi.hoisted(() => vi.fn());

// Only the lists of sessions are answered; anything else the tab asks for stays pending.
vi.mock("@/lib/api", () => {
  const pending = () => new Promise(() => {});
  const api = new Proxy({}, { get: (_, k) => (k === "sessions" ? sessions : pending) });
  const workspaces = new Proxy({}, { get: () => pending });
  return { api, workspaces, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useRouter: () => ({ navigate: vi.fn() }),
  Link: ({ children }: { children: ReactNode }) => <a href="/s">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  sessions.mockReset();
});

const project = { id: "prj_web", slug: "web", name: "Web", role: "viewer" } as G.Project;
// Ended three days ago and more, so that they list under "Older".
const NOW = Date.now();
const ended = (n: number): G.Session =>
  ({
    id: `ses_${n}`,
    number: n,
    title: `Session ${n}`,
    status: "stopped",
    project_id: project.id,
    project_slug: project.slug,
    created_at: NOW - 3 * 86_400_000 - (1000 - n) * 60_000,
    stats: { files_changed: 0, additions: 0, deletions: 0 },
    pull_requests: [],
    created_by: { id: "usr_ada", name: "Ada Lovelace" },
  }) as unknown as G.Session;

async function render() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <Sessions project={project} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

/** Lets the queries' answers arrive and the tab show them. */
const settle = () => act(() => new Promise((r) => setTimeout(r, 20)));

const more = (c: HTMLElement) =>
  [...c.querySelectorAll("button")].find((b) => b.textContent === "Show older sessions");

describe("a project's sessions", () => {
  it("goes on to older ones a page at a time, until there are none", async () => {
    sessions.mockImplementation(async (q: { status?: string; before?: number }) => {
      if (q.status === "active") return { sessions: [] };
      if (q.before === undefined) return { sessions: [ended(3), ended(2)], next_before: ended(2).created_at };
      return { sessions: [ended(1)] };
    });
    const c = await render();
    await settle();
    expect(c.textContent).toContain("Session 2");
    expect(c.textContent).not.toContain("Session 1");

    await act(async () => more(c)?.click());
    await settle();
    expect(sessions).toHaveBeenLastCalledWith(
      expect.objectContaining({ project: project.id, status: "ended", before: ended(2).created_at }),
    );
    expect(c.textContent).toContain("Session 1");
    // The last page says nothing older is left.
    expect(more(c)).toBeUndefined();
  });

  it("offers older ones under Recently when every session on the first page moved in the last day", async () => {
    // A busy project: its newest sessions all ended within the day, so none lists under "Older".
    const today = (n: number) => ({ ...ended(n), created_at: NOW - 3_600_000 + n * 60_000 });
    sessions.mockImplementation(async (q: { status?: string; before?: number }) => {
      if (q.status === "active") return { sessions: [] };
      if (q.before === undefined) return { sessions: [today(3), today(2)], next_before: today(2).created_at };
      return { sessions: [ended(1)] };
    });
    const c = await render();
    await settle();
    expect(c.textContent).toContain("Recently");
    expect(c.textContent).not.toContain("Older");
    await act(async () => more(c)?.click());
    await settle();
    expect(c.textContent).toContain("Session 1");
    expect(more(c)).toBeUndefined();
  });

  it("offers no older ones when the first page holds them all", async () => {
    sessions.mockImplementation(async (q: { status?: string }) =>
      q.status === "active" ? { sessions: [] } : { sessions: [ended(1)] },
    );
    const c = await render();
    await settle();
    expect(c.textContent).toContain("Session 1");
    expect(more(c)).toBeUndefined();
  });
});
