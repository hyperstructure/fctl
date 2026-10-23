import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { TicketsTab } from "./tickets";

// No request is answered: the tab shows what the cache holds.
vi.mock("@/lib/api", () => {
  const never = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return {
    api: never,
    workspaces: never,
    linearLinkUrl: (project?: string) => `/api/v1/linear/link${project ? `?project=${project}` : ""}`,
    ApiFailure: class extends Error {},
  };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(<QueryClientProvider client={queryClient}>{node}</QueryClientProvider>));
  roots.push(() => act(() => root.unmount()));
  return container;
}

const settings: G.ProjectSettings = {
  auto_pull_request: true,
  remote_control: true,
  claude_ai_connectors: true,
  credit_people: true,
  credit_claude: true,
  publish_screenshots: false,
  package_caches: false,
  project_memory: true,
  retrospectives: true,
  env: {},
};
const workspace: G.WorkspaceRef = { id: "wsp_1", slug: "acme", name: "Acme", color: "yellow" };
const ORG = "3f1d9d7c-1111-4c2e-9a5b-0d2f6c8e7a10";
const TEAM = "8b2a4c6e-2222-4d3f-8e1a-1c3e5a7b9d20";
const linear: G.LinearTickets = { organization: ORG, team: TEAM, team_key: "ENG", team_name: "Engineering" };

const project = (over: Partial<G.Project> = {}): G.Project => ({
  id: "prj_1",
  slug: "web",
  name: "Web",
  workspace,
  repo: { owner: "acme", name: "web" },
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

const instance: G.InstanceInfo = {
  name: "fctl",
  version: "0.22.0",
  setup_required: false,
  features: {
    google: false,
    email: false,
    github: true,
    publish_screenshots: false,
    early_access: false,
    linear: true,
  },
};

const connection: G.LinearConnection = {
  organization: ORG,
  name: "Acme",
  url_key: "acme",
  status: "active",
  connected_by: { id: "usr_ada", name: "Ada" },
  created_at: 0,
};

function tab(
  p: G.Project,
  {
    status = {},
    role = "member",
    connections = [connection],
  }: { status?: Partial<G.ProjectTickets>; role?: G.WorkspaceRole; connections?: G.LinearConnection[] } = {},
) {
  queryClient.setQueryData(keys.instance, instance);
  queryClient.setQueryData<G.WorkspaceList>(keys.workspaces, {
    workspaces: [{ ...workspace, role, members: 2, runners: 1, runners_online: 1 } as unknown as G.Workspace],
    invitations: [],
    limit: 5,
  });
  queryClient.setQueryData<G.WorkspaceLinear>(keys.workspaceLinear(workspace.id), {
    configured: true,
    webhook: true,
    connections,
  });
  queryClient.setQueryData<G.LinearTeamList>(keys.linearTeams(workspace.id, ORG), {
    teams: [
      {
        id: TEAM,
        key: "ENG",
        name: "Engineering",
        states: [],
        start_state: "In Progress",
        projects: [{ id: "c0000000-0000-4000-8000-000000000101", name: "Website" }],
      },
    ],
  });
  queryClient.setQueryData<G.ProjectTickets>(keys.projectTickets(p.id), {
    tracker: p.settings.tickets?.tracker ?? "github",
    scope: "GitHub issues of acme/web",
    via: "gh",
    workspace: workspace.id,
    writable: [{ owner: "acme", name: "web" }],
    ...status,
  });
  return render(<TicketsTab project={p} />);
}

const radios = (c: HTMLElement) => [...c.querySelectorAll<HTMLInputElement>('input[type="radio"]')];
const checked = (c: HTMLElement) => radios(c).find((r) => r.checked)?.value;

describe("the Tickets tab", () => {
  it("offers the three trackers, with GitHub Issues for a project from before them", () => {
    const c = tab(project());
    expect(radios(c).map((r) => r.value)).toEqual(["none", "github", "linear"]);
    expect(checked(c)).toBe("github");
    expect(c.textContent).toContain("gh issue");
    expect(c.textContent).not.toContain("Save");
  });

  it("lets only maintainers choose", () => {
    const c = tab(project({ role: "editor" }));
    expect(radios(c).every((r) => r.disabled || r.checked)).toBe(true);
    expect(c.textContent).toContain("The project's owners and maintainers choose its tracker.");
  });

  it("asks to save a choice, and says what none means", () => {
    const c = tab(project());
    act(() => radios(c)[0]?.click());
    expect(checked(c)).toBe("none");
    expect(c.textContent).toContain("Their GitHub tokens carry no issues permission");
    expect(c.textContent).toContain("Save");
  });

  it("keeps agents to one repository's issues through fctl when it writes to several", () => {
    const c = tab(
      project({
        settings: { ...settings, tickets: { tracker: "github", repo: { owner: "acme", name: "web" } } },
      }),
      {
        status: {
          writable: [
            { owner: "acme", name: "web" },
            { owner: "acme", name: "lib" },
          ],
          via: "fctl",
        },
      },
    );
    expect(c.textContent).toContain("Only acme/web");
    expect(c.textContent).toContain("fctl ticket");
    expect(c.textContent).toContain("Their own tokens reach no issues at all.");
  });

  it("has admins connect Linear, and tells others whom to ask", () => {
    const linearProject = project({ settings: { ...settings, tickets: { tracker: "linear" } } });
    const admin = tab(linearProject, { role: "admin", connections: [] });
    expect(admin.textContent).toContain("Connect a Linear workspace");
    expect([...admin.querySelectorAll("button")].some((b) => b.textContent === "Connect Linear")).toBe(true);
    act(() => {
      for (const unmount of roots.splice(0)) unmount();
    });
    const member = tab(linearProject, { role: "member", connections: [] });
    expect(member.textContent).toContain("Acme's admins connect Linear");
    expect([...member.querySelectorAll("button")].some((b) => b.textContent === "Connect Linear")).toBe(
      false,
    );
  });

  it("shows the team, how to delegate a ticket, and asks people to link their Linear account", () => {
    const c = tab(project({ settings: { ...settings, tickets: { tracker: "linear", linear } } }), {
      status: {
        tracker: "linear",
        scope: "Linear team ENG (Acme)",
        via: "fctl",
        linear: connection,
        linked: false,
      },
    });
    expect(c.textContent).toContain("Engineering");
    expect(c.textContent).toContain("moves to In Progress");
    expect(c.textContent).toContain("A ticket labelled explore gets an exploration");
    expect(c.textContent).toContain("assign one of ENG's issues to fctl");
    const link = c.querySelector<HTMLAnchorElement>('a[href="/api/v1/linear/link?project=web"]');
    expect(link?.textContent).toContain("Link Linear account");
  });

  it("says when fctl knows the person in Linear already", () => {
    const c = tab(project({ settings: { ...settings, tickets: { tracker: "linear", linear } } }), {
      status: {
        tracker: "linear",
        scope: "Linear team ENG (Acme)",
        via: "fctl",
        linear: connection,
        linked: true,
      },
    });
    expect(c.textContent).toContain("fctl knows your Linear account in Acme");
    expect(c.querySelector('a[href^="/api/v1/linear/link"]')).toBeNull();
  });

  it("names fctl's app as people find it in Linear", () => {
    const c = tab(project({ settings: { ...settings, tickets: { tracker: "linear", linear } } }), {
      status: {
        tracker: "linear",
        scope: "Linear team ENG (Acme)",
        via: "fctl",
        linear: { ...connection, app_name: "fctl-sh", app_handle: "fctl-sh" },
        linked: true,
      },
    });
    expect(c.textContent).toContain("assign one of ENG's issues to fctl-sh, or mention @fctl-sh");
  });

  it("lists the sessions tickets started lately", () => {
    const c = tab(project({ settings: { ...settings, tickets: { tracker: "linear", linear } } }), {
      status: {
        tracker: "linear",
        scope: "Linear team ENG (Acme)",
        via: "fctl",
        linear: connection,
        linked: true,
        recent: [
          {
            ticket: {
              tracker: "linear",
              id: "i1",
              key: "ENG-7",
              title: "Fix the login",
              url: "https://linear.app/acme/issue/ENG-7",
            },
            session: { id: "ses_7", number: 7, title: "Fix the login redirect loop" },
            status: "running",
            by: { id: "usr_ada", name: "Ada" },
            at: 0,
          },
        ],
      },
    });
    expect(c.textContent).toContain("Tickets lately");
    expect(c.textContent).toContain("ENG-7");
    expect(c.textContent).toContain("Fix the login redirect loop");
  });
});
