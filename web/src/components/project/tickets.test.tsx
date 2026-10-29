import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import type { LinearOutcome } from "@/lib/tickets";
import { TicketsTab } from "./tickets";

// No request is answered: the tab shows what the cache holds. Each call is noted.
const calls = vi.hoisted(() => [] as { name: string; args: unknown[] }[]);
vi.mock("@/lib/api", () => {
  const never = new Proxy(
    {},
    {
      get:
        (_, name) =>
        (...args: unknown[]) => {
          calls.push({ name: String(name), args });
          return new Promise(() => {});
        },
    },
  );
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
  calls.splice(0);
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  const root = createRoot(container);
  const show = (n: ReactNode) =>
    act(() => root.render(<QueryClientProvider client={queryClient}>{n}</QueryClientProvider>));
  show(node);
  roots.push(() => act(() => root.unmount()));
  return Object.assign(container, { show });
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
const DESIGN = "8b2a4c6e-2222-4d3f-8e1a-1c3e5a7b9d30";
const WEBSITE = "c0000000-0000-4000-8000-000000000101";
const POSTERS = "c0000000-0000-4000-8000-000000000201";
const linear: G.LinearTickets = { organization: ORG, team: TEAM, team_key: "ENG", team_name: "Engineering" };
/** Engineering's tickets, and Design's of its Posters project. */
const both: G.LinearTickets = {
  ...linear,
  more_teams: [
    { team: DESIGN, team_key: "DES", team_name: "Design", project: POSTERS, project_name: "Posters" },
  ],
};
const engineering: G.LinearTeam = {
  id: TEAM,
  key: "ENG",
  name: "Engineering",
  color: "#5e6ad2",
  states: [],
  start_state: "In Progress",
  review_state: "Code review",
  done_state: "Shipped",
  projects: [{ id: WEBSITE, name: "Website" }],
};
const PLATFORM = "8b2a4c6e-2222-4d3f-8e1a-1c3e5a7b9d40";
const platform: G.LinearTeam = {
  id: PLATFORM,
  key: "PLT",
  name: "Platform",
  states: [],
  projects: [],
};
/** Engineering's, Design's and Platform's tickets. */
const three: G.LinearTickets = {
  ...linear,
  more_teams: [
    { team: DESIGN, team_key: "DES", team_name: "Design" },
    { team: PLATFORM, team_key: "PLT", team_name: "Platform" },
  ],
};
const design: G.LinearTeam = {
  id: DESIGN,
  key: "DES",
  name: "Design",
  color: "#f2994a",
  states: [],
  start_state: "In Progress",
  review_state: "In Review",
  done_state: "Done",
  projects: [{ id: POSTERS, name: "Posters" }],
};

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
    teams = [engineering],
    returned,
  }: {
    status?: Partial<G.ProjectTickets>;
    role?: G.WorkspaceRole;
    connections?: G.LinearConnection[];
    /** The teams fctl's app reaches, as the cache holds them; `null`: none listed. */
    teams?: G.LinearTeam[] | null;
    returned?: LinearOutcome;
  } = {},
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
  if (teams) queryClient.setQueryData<G.LinearTeamList>(keys.linearTeams(workspace.id, ORG), { teams });
  queryClient.setQueryData<G.ProjectTickets>(keys.projectTickets(p.id), {
    tracker: p.settings.tickets?.tracker ?? "github",
    scope: "GitHub issues of acme/web",
    via: "gh",
    workspace: workspace.id,
    writable: [{ owner: "acme", name: "web" }],
    ...status,
  });
  return render(<TicketsTab project={p} returned={returned} />);
}

const radios = (c: HTMLElement) => [...c.querySelectorAll<HTMLInputElement>('input[type="radio"]')];
const checked = (c: HTMLElement) => radios(c).find((r) => r.checked)?.value;

/** A Linear project of the tab's, as it saves it. */
const linearProject = (l: G.LinearTickets = linear, role: G.ProjectRole = "maintainer") =>
  project({ role, settings: { ...settings, tickets: { tracker: "linear", linear: l } } });
const linearStatus: Partial<G.ProjectTickets> = {
  tracker: "linear",
  scope: "Linear team ENG (Acme)",
  via: "fctl",
  linear: connection,
  linked: true,
};

/** The list of teams: each row's words, as a person reads them. */
const rows = (c: HTMLElement) =>
  [...c.querySelectorAll<HTMLElement>('ul[aria-label="Teams"] > li')].map((li) => {
    const [name, note] = [...li.querySelectorAll("p")];
    const words = [...(name?.children ?? [])].map((w) => w.textContent).join(" ");
    return note ? `${words} · ${note.textContent}` : words;
  });
const combobox = (c: HTMLElement, label: string) =>
  c.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${label}"]`);
/** Opens the select called `label` and picks its option that reads `text`. */
function pick(c: HTMLElement, label: string, text: string) {
  act(() => combobox(c, label)?.click());
  const option = [...c.querySelectorAll<HTMLElement>('[role="option"]')].find(
    (o) => o.querySelector(".truncate")?.textContent === text,
  );
  if (!option) throw new Error(`${label} has no option ${text}`);
  act(() => option.click());
}
const button = (c: HTMLElement, label: string) =>
  [...c.querySelectorAll<HTMLButtonElement>("button")].find(
    (b) => b.getAttribute("aria-label") === label || b.textContent === label,
  );
/** What the tab saved: the tickets of the settings it sent. */
const sent = () =>
  (calls.find((x) => x.name === "updateProject")?.args[1] as { settings: G.ProjectSettings } | undefined)
    ?.settings.tickets;

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

  it("keeps a choice until what is stored changes", () => {
    const c = tab(project());
    act(() => radios(c)[0]?.click());
    expect(checked(c)).toBe("none");
    c.show(<TicketsTab project={project({ total_sessions: 4 })} />);
    expect(checked(c)).toBe("none");
    c.show(
      <TicketsTab project={project({ settings: { ...settings, tickets: { tracker: "linear", linear } } })} />,
    );
    expect(checked(c)).toBe("linear");
    expect(c.textContent).not.toContain("Save");
  });

  it("comes back from connecting Linear with Linear chosen, for the team next", () => {
    const c = tab(project(), { role: "admin", returned: "connected" });
    expect(c.textContent).toContain("Linear is connected");
    expect(checked(c)).toBe("linear");
    expect(c.textContent).toContain("Choose a team");
    expect(c.textContent).toContain("Choose a team first.");
  });

  it("comes back from a Linear sign-in that failed with Linear still chosen, to try again", () => {
    const c = tab(project(), { role: "admin", connections: [], returned: "denied" });
    expect(checked(c)).toBe("linear");
    expect([...c.querySelectorAll("button")].some((b) => b.textContent === "Connect Linear")).toBe(true);
  });

  it("shows those who can't choose what is stored, back from Linear too", () => {
    const c = tab(project({ role: "editor" }), { role: "admin", returned: "connected" });
    expect(checked(c)).toBe("github");
    expect(c.textContent).not.toContain("Save");
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
    expect(c.textContent).toContain("A delegated ticket at once, as Linear asks of agents");
    expect(c.textContent).toContain(
      "The ticket moves to In Review when its pull request is up, and to Done once a pull request that fixes it merges.",
    );
    expect(c.textContent).toContain("A ticket labelled explore gets an exploration");
    expect(c.textContent).toContain("assign one of ENG's issues to fctl");
    const link = c.querySelector<HTMLAnchorElement>('a[href="/api/v1/linear/link?project=web"]');
    expect(link?.textContent).toContain("Link Linear account");
  });

  it("moves tickets through the team's states, each with a switch", () => {
    const c = tab(
      project({
        settings: { ...settings, tickets: { tracker: "linear", linear: { ...linear, move_done: false } } },
      }),
      {
        status: {
          tracker: "linear",
          scope: "Linear team ENG (Acme)",
          via: "fctl",
          linear: connection,
          linked: true,
        },
      },
    );
    const switches = () => [...c.querySelectorAll<HTMLButtonElement>('button[role="switch"]')];
    const states = () =>
      switches().map((s) => [s.getAttribute("aria-label"), s.getAttribute("aria-checked")]);
    expect(states()).toEqual([
      ["Move a ticket to In Progress as its session starts", "true"],
      ["Move a ticket to Code review when its pull request is up", "true"],
      ["Move a ticket to Shipped once its pull requests merge", "false"],
    ]);
    expect(c.textContent).toContain("through Engineering's workflow, never back");
    expect(c.textContent).toContain("The ticket moves to In Review when its pull request is up.");
    expect(c.textContent).not.toContain("Save");
    act(() => switches()[2]?.click());
    expect(states()[2]).toEqual(["Move a ticket to Shipped once its pull requests merge", "true"]);
    act(() => switches()[0]?.click());
    expect(states()[0]).toEqual(["Move a ticket to In Progress as its session starts", "false"]);
    expect(c.textContent).toContain("Save");
  });

  it("shows a viewer the team's states as what they are, without its list of teams", () => {
    // Only the workspace's admins and the project's maintainers list them.
    const c = tab(
      project({ role: "viewer", settings: { ...settings, tickets: { tracker: "linear", linear } } }),
      {
        status: {
          tracker: "linear",
          scope: "Linear team ENG (Acme)",
          via: "fctl",
          linear: connection,
          linked: true,
        },
        teams: null,
      },
    );
    expect(c.textContent).toContain("Engineering");
    const labels = [...c.querySelectorAll<HTMLButtonElement>('button[role="switch"]')].map((s) =>
      s.getAttribute("aria-label"),
    );
    expect(labels).toEqual([
      "Move a ticket to its first started state as its session starts",
      "Move a ticket to its review state when its pull request is up",
      "Move a ticket to its first completed state once its pull requests merge",
    ]);
    expect(c.textContent).not.toContain("no longer reaches it");
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

  it("lists the project's teams, each with the issues of it the project takes", () => {
    const c = tab(linearProject(both), { status: linearStatus, teams: [engineering, design] });
    expect(rows(c)).toEqual(["Engineering ENG", "Design DES"]);
    expect(combobox(c, "Issues of Engineering")?.textContent).toContain("Every issue");
    expect(combobox(c, "Issues of Design")?.textContent).toContain("Only Posters");
    expect(combobox(c, "Team agents file new tickets in")?.textContent).toContain("Engineering");
    expect(c.textContent).toContain("Teams");
    expect(c.textContent).toContain("assign one of ENG's or DES's issues to fctl");
    expect(c.textContent).not.toContain("Save");
  });

  it("adds a team, marked new until it is saved, and saves every team", async () => {
    const c = tab(linearProject(), { status: linearStatus, teams: [engineering, design] });
    expect(rows(c)).toEqual(["Engineering ENG"]);
    expect(combobox(c, "Team agents file new tickets in")).toBeNull();
    pick(c, "Add a team", "Design");
    expect(rows(c)).toEqual(["Engineering ENG", "Design DES New"]);
    // Every team fctl's app reaches is on the list now.
    expect(combobox(c, "Add a team")).toBeNull();
    expect(c.textContent).toContain("fctl's app reaches no other team.");
    pick(c, "Issues of Design", "Posters");
    await act(async () => button(c, "Save")?.click());
    expect(sent()?.linear).toMatchObject({
      team: TEAM,
      more_teams: [
        { team: DESIGN, team_key: "DES", team_name: "Design", project: POSTERS, project_name: "Posters" },
      ],
    });
  });

  it("takes a team off when it is saved, which Undo brings back as it was", async () => {
    const c = tab(linearProject(both), { status: linearStatus, teams: [engineering, design] });
    // In the page, where buttons take focus.
    document.body.append(c);
    roots.push(() => c.remove());
    act(() => button(c, "Take Design off")?.focus());
    act(() => button(c, "Take Design off")?.click());
    expect(rows(c)).toEqual([
      "Engineering ENG",
      "Design DES · Its tickets start no sessions here once you save",
    ]);
    // The keys go on from the row's Undo.
    expect(document.activeElement?.textContent).toBe("Undo");
    expect(c.textContent).toContain("Save");
    await act(async () => button(c, "Save")?.click());
    expect(sent()?.linear?.more_teams).toEqual([]);
    act(() => button(c, "Undo")?.click());
    expect(rows(c)).toEqual(["Engineering ENG", "Design DES"]);
    expect(combobox(c, "Issues of Design")?.textContent).toContain("Only Posters");
    expect(document.activeElement?.getAttribute("aria-label")).toBe("Take Design off");
  });

  it("comes back to the saved teams when every change is undone, in any order", () => {
    const c = tab(linearProject(three), { status: linearStatus, teams: [engineering, design, platform] });
    for (const order of [
      ["Engineering", "Design"],
      ["Design", "Engineering"],
    ]) {
      act(() => button(c, "Take Engineering off")?.click());
      act(() => button(c, "Take Design off")?.click());
      expect(c.textContent).toContain("Save");
      for (const name of order) {
        const row = [...c.querySelectorAll('ul[aria-label="Teams"] > li')].find((li) =>
          li.textContent?.startsWith(name),
        );
        act(() => row?.querySelector<HTMLButtonElement>("[data-row-action]")?.click());
      }
      expect(rows(c)).toEqual(["Engineering ENG", "Design DES", "Platform PLT"]);
      expect(c.textContent).not.toContain("Save");
    }
  });

  it("counts only which team comes first, not the order of the others", async () => {
    const c = tab(linearProject(three), { status: linearStatus, teams: [engineering, design, platform] });
    pick(c, "Team agents file new tickets in", "Platform");
    expect(rows(c)).toEqual(["Platform PLT", "Engineering ENG", "Design DES"]);
    expect(c.textContent).toContain("Save");
    // Once the list has closed.
    await act(() => new Promise((done) => setTimeout(done, 500)));
    pick(c, "Team agents file new tickets in", "Engineering");
    expect(rows(c)).toEqual(["Engineering ENG", "Platform PLT", "Design DES"]);
    expect(c.textContent).not.toContain("Save");
  });

  it("asks for a team before it saves one taken off as the only one", () => {
    const c = tab(linearProject(), { status: linearStatus, teams: [engineering, design] });
    act(() => button(c, "Take Engineering off")?.click());
    expect(c.textContent).toContain("Choose a team first.");
    expect(button(c, "Save")?.disabled).toBe(true);
    // Another team, or Engineering again with Undo.
    pick(c, "Choose a team", "Design");
    expect(rows(c)).toEqual([
      "Design DES New",
      "Engineering ENG · Its tickets start no sessions here once you save",
    ]);
    expect(button(c, "Save")?.disabled).toBe(false);
  });

  it("files new tickets in the team people choose, which goes first", async () => {
    const c = tab(linearProject(both), { status: linearStatus, teams: [engineering, design] });
    pick(c, "Team agents file new tickets in", "Design");
    expect(rows(c)).toEqual(["Design DES", "Engineering ENG"]);
    await act(async () => button(c, "Save")?.click());
    expect(sent()?.linear).toMatchObject({
      team: DESIGN,
      team_key: "DES",
      project: POSTERS,
      more_teams: [{ team: TEAM, team_key: "ENG", team_name: "Engineering" }],
    });
  });

  it("names the states each team's workflow has for a move", () => {
    const c = tab(linearProject(both), { status: linearStatus, teams: [engineering, design] });
    const labels = [...c.querySelectorAll('button[role="switch"]')].map((s) => s.getAttribute("aria-label"));
    expect(labels).toEqual([
      "Move a ticket to In Progress as its session starts",
      "Move a ticket to Code review or In Review when its pull request is up",
      "Move a ticket to Shipped or Done once its pull requests merge",
    ]);
    expect(c.textContent).toContain("through their team's workflow, never back");
    expect(c.textContent).toContain("does for these teams already");
  });

  it("says which team fctl's app no longer reaches", () => {
    const c = tab(linearProject(both), { status: linearStatus, teams: [engineering] });
    expect(rows(c)[1]).toBe(
      "Design DES · fctl's app no longer reaches it: take it off, or give the app the team in Linear's settings",
    );
    expect(combobox(c, "Issues of Design")?.disabled).toBe(true);
  });

  it("shows the teams as saved to those who don't choose, without asking Linear for its teams", () => {
    const c = tab(linearProject(both, "editor"), { status: linearStatus, teams: null });
    expect(rows(c)).toEqual(["Engineering ENG", "Design DES"]);
    expect(combobox(c, "Issues of Design")?.textContent).toContain("Only Posters");
    expect(combobox(c, "Issues of Design")?.disabled).toBe(true);
    expect(button(c, "Take Design off")).toBeUndefined();
    expect(combobox(c, "Add a team")).toBeNull();
    expect(calls.some((x) => x.name === "linearTeams")).toBe(false);
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
