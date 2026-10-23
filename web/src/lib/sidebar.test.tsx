import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { ApiFailure } from "./api";
import { fetchMe } from "./queries";
import { forgetSidebar, keepSidebar, keptSidebar, type SidebarLists, useSidebarLists } from "./sidebar";

const { me } = vi.hoisted(() => ({ me: vi.fn() }));
vi.mock("./api", async (original) => ({ ...(await original<typeof import("./api")>()), api: { me } }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  localStorage.clear();
  me.mockReset();
});

const workspace = (w: Partial<G.Workspace> & Pick<G.Workspace, "id" | "name">): G.Workspace => ({
  slug: w.name.toLowerCase(),
  color: "blue",
  role: "owner",
  email: "ada@acme.example",
  base_role: "editor",
  require_two_factor: false,
  instance_requires_two_factor: false,
  blocked: false,
  primary: { id: "usr_ada", name: "Ada Lovelace" },
  members: 3,
  projects: 2,
  runners: 1,
  runners_online: 1,
  github: ["acme"],
  skip_invites: false,
  own_runners: false,
  created_at: 0,
  ...w,
});

const project = (name: string, w: G.Workspace, active_sessions = 0): G.Project => ({
  id: `prj_${name}`,
  slug: name,
  name,
  workspace: { id: w.id, slug: w.slug, name: w.name, color: w.color },
  repo: { owner: "acme", name },
  default_branch: "main",
  settings: {
    instructions: "Never mention the launch date",
    setup_script: "npm ci",
    env: { REGION: "eu-west-1" },
    auto_pull_request: true,
    remote_control: false,
    claude_ai_connectors: false,
    credit_people: true,
    credit_claude: true,
    publish_screenshots: false,
    package_caches: false,
    project_memory: true,
    retrospectives: true,
  },
  role: "owner",
  created_by: { id: "usr_ada", name: "Ada Lovelace" },
  created_at: 0,
  updated_at: 0,
  active_sessions,
  total_sessions: 7,
});

const acme = workspace({ id: "ws_acme", name: "Acme", avatar_url: "/api/v1/workspaces/ws_acme/avatar?v=2" });
const side = workspace({ id: "ws_side", name: "Side", color: "pink", role: "member", blocked: true });
const invitation: G.WorkspaceInvitation = {
  id: "inv_1",
  workspace: { id: "ws_lab", slug: "lab", name: "Lab", color: "yellow" },
  email: "ada@lab.example",
  role: "member",
  invited_by: { id: "usr_grace", name: "Grace Hopper" },
  created_at: 0,
  expires_at: 0,
  needs_two_factor: false,
};
const workspaces: G.WorkspaceList = { workspaces: [acme, side], invitations: [invitation], limit: 5 };
const projects: G.ProjectList = { projects: [project("web", acme, 2), project("api", acme)] };

describe("the sidebar kept in this browser", () => {
  it("keeps only what the sidebar shows, for the person signed in", () => {
    keepSidebar("usr_ada", workspaces, projects);
    expect(keptSidebar("usr_ada")).toEqual({
      workspaces: {
        workspaces: [
          {
            id: "ws_acme",
            slug: "acme",
            name: "Acme",
            color: "blue",
            avatar_url: "/api/v1/workspaces/ws_acme/avatar?v=2",
            role: "owner",
            blocked: false,
          },
          { id: "ws_side", slug: "side", name: "Side", color: "pink", role: "member", blocked: true },
        ],
        invitations: [
          { id: "inv_1", workspace: { id: "ws_lab", slug: "lab", name: "Lab", color: "yellow" } },
        ],
        limit: 5,
      },
      projects: [
        { id: "prj_web", slug: "web", name: "web", active_sessions: 2, workspace: { id: "ws_acme" } },
        { id: "prj_api", slug: "api", name: "api", active_sessions: 0, workspace: { id: "ws_acme" } },
      ],
    });
    const kept = localStorage.getItem("fctl:sidebar") ?? "";
    for (const left of ["example", "Lovelace", "Hopper", "launch date", "npm ci", "eu-west-1", "total"])
      expect(kept).not.toContain(left);
    expect(keptSidebar("usr_grace")).toBeUndefined();
  });

  it("ignores what it can't read", () => {
    keepSidebar("usr_ada", workspaces, projects);
    const good = JSON.parse(localStorage.getItem("fctl:sidebar") ?? "");
    const keep = (change: (d: typeof good) => void) => {
      const d = structuredClone(good);
      change(d);
      localStorage.setItem("fctl:sidebar", JSON.stringify(d));
      return keptSidebar("usr_ada");
    };
    expect(keep(() => {})).toBeDefined();
    expect(keep((d) => (d.version = 2))).toBeUndefined();
    expect(keep((d) => (d.projects[1].name = 7))).toBeUndefined();
    expect(keep((d) => (d.projects[0].active_sessions = "2"))).toBeUndefined();
    expect(keep((d) => (d.projects[0].workspace = "ws_acme"))).toBeUndefined();
    expect(keep((d) => delete d.workspaces.workspaces[0].role)).toBeUndefined();
    expect(keep((d) => (d.workspaces.invitations[0].workspace = null))).toBeUndefined();
    expect(keep((d) => delete d.workspaces.limit)).toBeUndefined();
    expect(keep((d) => (d.projects = {}))).toBeUndefined();
    localStorage.setItem("fctl:sidebar", "{not json");
    expect(keptSidebar("usr_ada")).toBeUndefined();
  });

  it("forgets them when you sign out, or the control plane says you aren't signed in", async () => {
    keepSidebar("usr_ada", workspaces, projects);
    forgetSidebar();
    expect(keptSidebar("usr_ada")).toBeUndefined();

    keepSidebar("usr_ada", workspaces, projects);
    me.mockRejectedValueOnce(
      new ApiFailure(0, { code: "unavailable", message: "You appear to be offline." }),
    );
    await expect(fetchMe()).rejects.toThrow("offline");
    expect(keptSidebar("usr_ada")).toBeDefined();
    me.mockRejectedValueOnce(new ApiFailure(401, { code: "unauthorized", message: "Sign in first" }));
    await expect(fetchMe()).resolves.toBeNull();
    expect(keptSidebar("usr_ada")).toBeUndefined();
  });
});

describe("what the sidebar shows", () => {
  /** The hook as the sidebar calls it, with the lists the control plane has sent so far. */
  function sidebar() {
    let shown: SidebarLists | undefined;
    function Probe(props: { workspaces?: G.WorkspaceList; projects?: G.ProjectList }) {
      shown = useSidebarLists("usr_ada", props.workspaces, props.projects);
      return null;
    }
    const container = document.createElement("div");
    const root = createRoot(container);
    cleanups.push(() => act(() => root.unmount()));
    return (props: { workspaces?: G.WorkspaceList; projects?: G.ProjectList } = {}) => {
      act(() => root.render(<Probe {...props} />));
      return shown;
    };
  }

  const renamed: G.WorkspaceList = { ...workspaces, workspaces: [{ ...acme, name: "Acme Inc" }] };
  const more: G.ProjectList = { projects: [...projects.projects, project("docs", acme)] };

  it("shows the lists it kept until the control plane's have both come, then keeps those", () => {
    keepSidebar("usr_ada", workspaces, projects);
    const show = sidebar();
    expect(show()).toEqual({ ...keptSidebar("usr_ada"), kept: true });
    // The workspaces alone don't take the place of the lists kept: they go together.
    expect(show({ workspaces: renamed })).toEqual({ ...keptSidebar("usr_ada"), kept: true });
    expect(show({ workspaces: renamed, projects: more })).toEqual({
      workspaces: renamed,
      projects: more.projects,
      kept: false,
    });
    expect(keptSidebar("usr_ada")?.workspaces?.workspaces.map((w) => w.name)).toEqual(["Acme Inc"]);
    expect(keptSidebar("usr_ada")?.projects?.map((p) => p.name)).toEqual(["web", "api", "docs"]);
  });

  it("shows each list as it comes when none were kept", () => {
    keepSidebar("usr_grace", workspaces, projects);
    const show = sidebar();
    expect(show()).toEqual({ workspaces: undefined, projects: undefined, kept: false });
    expect(show({ workspaces })).toEqual({ workspaces, projects: undefined, kept: false });
    expect(show({ workspaces, projects })).toEqual({ workspaces, projects: projects.projects, kept: false });
  });
});
