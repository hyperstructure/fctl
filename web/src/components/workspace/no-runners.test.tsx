import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { NoRunnerNote } from "./no-runners";

// No request is answered: the note shows what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: pending, workspaces: pending, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, params, search }: { children: ReactNode; params: { slug: string }; search: object }) => (
    <a href={`/w/${params.slug}?${new URLSearchParams(search as Record<string, string>)}`}>{children}</a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

/** The note for `workspace`, with these runners in the cache as the API or the feed put them there. */
function note(workspace: G.Workspace, runners?: G.Runner[]) {
  const client = new QueryClient();
  if (runners) client.setQueryData<G.RunnerList>(keys.runners, { runners });
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <NoRunnerNote workspace={workspace} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

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
const member: G.Workspace = { ...side, role: "member", email: "grace@example.com" };

const runner = (r: Partial<G.Runner> & { id: string }): G.Runner => ({
  name: r.id,
  status: "online",
  drivers: [],
  capacity: 4,
  active_sessions: 0,
  labels: [],
  created_at: 0,
  workspaces: [{ id: side.id, slug: side.slug, name: side.name, color: side.color }],
  yours: true,
  ...r,
});
// Online, but for another workspace only.
const elsewhere = runner({
  id: "studio",
  workspaces: [{ id: "ws_work", slug: "work", name: "Work", color: "blue" }],
});

describe("the note under a start form", () => {
  it("sends a workspace's admins to add its first runner", () => {
    const text = note(side, [elsewhere]);
    expect(text.textContent).toBe(
      "Side project has no runners yet: sessions you start here wait until one is online. Add a runner",
    );
    expect(text.querySelector("a")?.getAttribute("href")).toBe("/w/side-project?tab=runners");
  });

  it("tells members whom to ask", () => {
    const text = note(member, []);
    expect(text.textContent).toBe(
      "Side project has no runners yet: sessions you start here wait until its admins add one. Ask Ada Lovelace, its primary admin.",
    );
    expect(text.querySelector("a")).toBeNull();
  });

  it("says when none of its runners is online", () => {
    const one = note(member, [runner({ id: "mini", status: "offline" }), elsewhere]);
    expect(one.textContent).toBe(
      "Side project's runner isn't online: sessions you start here wait until it is. See its runner",
    );
    const two = note(side, [
      runner({ id: "mini", status: "offline" }),
      runner({ id: "old", status: "disabled" }),
    ]);
    expect(two.textContent).toBe(
      "None of Side project's runners is online: sessions you start here wait until one is. See its runners",
    );
  });

  it("says nothing while one is online, or before the runners load", () => {
    expect(
      note(side, [runner({ id: "mini", status: "offline" }), runner({ id: "studio" })]).textContent,
    ).toBe("");
    expect(note(side).textContent).toBe("");
  });
});
