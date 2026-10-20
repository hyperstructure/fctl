import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { WorkspacePage } from "./workspace";

const route = vi.hoisted(() => ({ tab: "runners" as string | undefined }));
const update = vi.hoisted(() => vi.fn());

// No request is answered but saving the workspace: the page shows what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  const workspaces = new Proxy(
    {},
    { get: (_, k) => (k === "update" ? update : () => new Promise(() => {})) },
  );
  return { api: pending, workspaces, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ slug: "side-project" }),
  useSearch: () => ({ tab: route.tab }),
  useNavigate: () => () => {},
  Link: ({ children }: { children: ReactNode }) => <a href="/w/side-project">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  update.mockReset();
  route.tab = "runners";
});

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
  skip_invites: false,
  created_at: 0,
};

/** The workspace's Runners tab, without runners, as `workspace` sees it. */
function runnersTab(workspace: G.Workspace) {
  const client = new QueryClient();
  client.setQueryData(keys.workspace(workspace.slug), workspace);
  client.setQueryData<G.RunnerList>(keys.workspaceRunners(workspace.id), { runners: [] });
  client.setQueryData<G.RunnerList>(keys.runners, { runners: [] });
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <WorkspacePage />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container.textContent ?? "";
}

describe("a workspace without runners", () => {
  it("shows its admins how to bring a machine online", () => {
    const text = runnersTab(side);
    expect(text).toContain("Side project's sessions wait until one of its runners is online.");
    expect(text).toContain("Choose a machine that stays on");
    expect(text).toContain(
      "install Apple's container on macOS 26 or later, Lima on older macOS, or Docker on Linux",
    );
    expect(text).toContain("Install fctl on it");
    expect(text).toContain("curl -fsSL https://fctl.sh/install | sh");
    expect(text).toContain("brew install hyperstructure/tap/fctl");
    expect(text).toContain("Register it for Side project");
    expect(text).toContain(`fctl login ${location.origin}`);
    expect(text).toContain("fctl host install --workspace side-project");
    expect(text).toContain("Name it above and choose Register for a command with a token.");
  });

  it("tells members whom to ask for one", () => {
    const text = runnersTab({ ...side, role: "member", email: "grace@example.com" });
    expect(text).toContain("No runners yet");
    expect(text).toContain("Ask Ada Lovelace, its primary admin, to add one.");
    expect(text).not.toContain("Choose a machine that stays on");
  });
});

const notes = {
  id: "prj_notes",
  workspace: { id: side.id, slug: side.slug, name: side.name, color: side.color },
  slug: "notes",
  name: "Notes",
  default_branch: "main",
  active_sessions: 0,
  total_sessions: 0,
  created_at: 0,
  updated_at: 0,
} as G.Project;

/** The workspace's Projects tab as `workspace` sees it, with these projects, on the app's own client. */
function projectsTab(workspace: G.Workspace, projects: G.Project[] = []) {
  route.tab = undefined;
  queryClient.setQueryData(keys.workspace(workspace.slug), workspace);
  queryClient.setQueryData<G.WorkspaceGithub>(keys.workspaceGithub(workspace.id), {
    configured: true,
    connections: [],
    approval_required: false,
  });
  queryClient.setQueryData<G.ProjectList>(keys.projects, { projects });
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <WorkspacePage />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

/** The step of the setup checklist with this title. */
const step = (page: HTMLElement, title: string) =>
  [...page.querySelectorAll("li")].find((li) => li.textContent?.startsWith(title));

describe("setting up a workspace for one", () => {
  const solo: G.Workspace = {
    ...side,
    members: 1,
    runners: 1,
    runners_online: 1,
    github: ["hyperstructure"],
  };

  it("lets its admin skip inviting people, which finishes it", async () => {
    update.mockResolvedValue({ ...solo, skip_invites: true });
    const page = projectsTab(solo, [notes]);
    expect(page.textContent).toContain("Set up Side project");
    expect(page.textContent).toContain("3 of 4 done");
    const skip = [...(step(page, "Invite the people you work with")?.querySelectorAll("button") ?? [])].find(
      (b) => b.textContent === "Skip for now",
    );
    expect(skip).toBeDefined();
    await act(async () => skip?.click());
    // The saved workspace comes back, and the page shows it.
    await act(() => new Promise((done) => setTimeout(done, 10)));
    expect(update).toHaveBeenCalledWith("ws_side", { skip_invites: true });
    expect(page.textContent).not.toContain("Set up Side project");
  });

  it("leaves inviting people out once it is skipped", () => {
    const page = projectsTab({ ...solo, runners: 0, runners_online: 0, skip_invites: true });
    expect(page.textContent).toContain("Bring a machine online");
    expect(page.textContent).not.toContain("Invite the people you work with");
    expect(page.textContent).toContain("1 of 3 done");
  });

  it("counts people who joined after all", () => {
    const page = projectsTab({ ...solo, members: 2, runners: 0, runners_online: 0, skip_invites: true });
    expect(step(page, "Invite the people you work with")?.textContent).not.toContain("Skip for now");
    expect(page.textContent).toContain("2 of 4 done");
  });

  it("offers no skipping for the steps every workspace needs", () => {
    const page = projectsTab({ ...solo, runners: 0, runners_online: 0, github: [] });
    for (const title of ["Bring a machine online", "Connect a GitHub account", "Create a project"])
      expect(step(page, title)?.textContent ?? "").not.toContain("Skip for now");
    expect(step(page, "Invite the people you work with")?.textContent).toContain("Skip for now");
  });
});
