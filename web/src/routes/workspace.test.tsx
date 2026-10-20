import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { WorkspacePage } from "./workspace";

const route = vi.hoisted(() => ({
  tab: "runners" as string | undefined,
  // Back from GitHub.
  github: undefined as string | undefined,
  account: undefined as string | undefined,
  reason: undefined as string | undefined,
}));
const update = vi.hoisted(() => vi.fn());
const connectGithub = vi.hoisted(() => vi.fn());
const chooseGithub = vi.hoisted(() => vi.fn());
const dismissGithubOffer = vi.hoisted(() => vi.fn());
const navigate = vi.hoisted(() => vi.fn());

// No request is answered but saving the workspace and connecting GitHub: the page shows what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  const answered: Record<string | symbol, unknown> = {
    update,
    connectGithub,
    chooseGithub,
    dismissGithubOffer,
  };
  const workspaces = new Proxy({}, { get: (_, k) => answered[k] ?? (() => new Promise(() => {})) });
  return { api: pending, workspaces, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ slug: "side-project" }),
  useSearch: () => ({ tab: route.tab, github: route.github, account: route.account, reason: route.reason }),
  useNavigate: () => navigate,
  Link: ({ children }: { children: ReactNode }) => <a href="/w/side-project">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  update.mockReset();
  connectGithub.mockReset();
  chooseGithub.mockReset();
  dismissGithubOffer.mockReset();
  navigate.mockReset();
  vi.restoreAllMocks();
  Object.assign(route, { tab: "runners", github: undefined, account: undefined, reason: undefined });
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
    can_connect: true,
    offers: [],
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

describe("a workspace's GitHub tab", () => {
  const github: G.WorkspaceGithub = {
    configured: true,
    app_slug: "fctl-sh",
    install_url: "https://fctl.test/api/v1/github/install?state=signed",
    connections: [],
    approval_required: false,
    can_connect: true,
    offers: [],
  };

  const acme: G.GithubConnection = {
    account: "acme",
    repos: ["web", "design"],
    status: "active",
    requested_by: { id: "usr_ada", name: "Ada Lovelace" },
    github_user: "ada",
    created_at: 0,
  };

  /** What Ada's GitHub sign-in showed her she may connect of acme. */
  const offer: G.GithubOffer = {
    account: "acme",
    account_type: "Organization",
    github_user: "ada",
    all: false,
    repos: ["api", "design", "web"],
    others: 2,
    expires_at: Date.now() + 600_000,
  };

  /** The tab as `workspace` sees it, with `g` from the control plane. */
  function githubTab(workspace: G.Workspace, g: G.WorkspaceGithub) {
    route.tab = "github";
    queryClient.setQueryData(keys.workspace(workspace.slug), workspace);
    queryClient.setQueryData<G.WorkspaceGithub>(keys.workspaceGithub(workspace.id), g);
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

  /** The button that says `text`. */
  const button = (page: HTMLElement, text: string) =>
    [...page.querySelectorAll("button")].find((b) => b.textContent?.trim() === text);
  const click = async (el: Element | null | undefined) => {
    await act(async () => {
      el?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await act(() => new Promise((done) => setTimeout(done, 10)));
  };
  /** The repositories checked in the choice. */
  const checked = (page: HTMLElement) =>
    [...page.querySelectorAll<HTMLInputElement>("input[type=checkbox]")]
      .filter((c) => c.checked)
      .map((c) => c.closest("label")?.textContent);

  it("connects accounts through a GitHub sign-in, without names to type", async () => {
    connectGithub.mockResolvedValue({
      url: "https://fctl.test/api/v1/github/connect?ticket=t",
      expires_at: 1,
    });
    const assign = vi.spyOn(window.location, "assign").mockImplementation(() => {});
    const page = githubTab(side, github);
    expect(page.textContent).toContain("Connecting an account takes signing in to GitHub");
    expect(page.querySelector("input")).toBeNull();
    // Installing goes through the control plane, which keeps where it started.
    expect(page.querySelector("a[href^='https://fctl.test/api/v1/github/install']")?.textContent).toBe(
      "Install fctl-sh on GitHub",
    );
    await click(button(page, "Connect with GitHub"));
    expect(connectGithub).toHaveBeenCalledWith("ws_side", { account: undefined });
    expect(assign).toHaveBeenCalledWith("https://fctl.test/api/v1/github/connect?ticket=t");
  });

  it("chooses again which repositories of a connected account it works in", async () => {
    connectGithub.mockResolvedValue({
      url: "https://fctl.test/api/v1/github/connect?ticket=t",
      expires_at: 1,
    });
    vi.spyOn(window.location, "assign").mockImplementation(() => {});
    const page = githubTab(side, { ...github, connections: [acme] });
    await click(
      page.querySelector("button[aria-label='Choose which repositories of acme Side project works in']"),
    );
    expect(connectGithub).toHaveBeenCalledWith("ws_side", { account: "acme" });
  });

  it("offers the repositories GitHub showed, all of them to begin with", async () => {
    Object.assign(route, { github: "choose", account: "acme" });
    chooseGithub.mockResolvedValue({ ...github, connections: [{ ...acme, repos: ["api", "web"] }] });
    const page = githubTab(side, { ...github, offers: [offer] });
    expect(page.textContent).toContain("Choose what Side project works in on acme");
    expect(page.textContent).toContain("signed in as @ada");
    expect(page.textContent).toContain(
      "The app reaches 2 more repositories there that you can see but aren't an admin of",
    );
    // An organization it doesn't reach all of: no "every repository".
    expect(page.querySelector("input[type=radio]")).toBeNull();
    expect(checked(page)).toEqual(["api", "design", "web"]);
    const design = [...page.querySelectorAll<HTMLInputElement>("input[type=checkbox]")][1];
    await click(design);
    expect(checked(page)).toEqual(["api", "web"]);
    await click(button(page, "Connect acme"));
    expect(chooseGithub).toHaveBeenCalledWith("ws_side", "acme", { repos: ["api", "web"] });
    expect(navigate).toHaveBeenCalledWith(
      expect.objectContaining({
        search: { tab: "github", github: "connected", account: "acme" },
        replace: true,
      }),
    );
  });

  it("offers every repository where the person may give it, or some", async () => {
    chooseGithub.mockResolvedValue({ ...github, connections: [{ ...acme, repos: undefined }] });
    const page = githubTab(side, { ...github, offers: [{ ...offer, all: true, others: 0 }] });
    const [every, some] = [...page.querySelectorAll<HTMLInputElement>("input[type=radio]")];
    expect(every?.checked).toBe(true);
    expect(page.textContent).toContain("It reaches api, design, web now.");
    // The list shows once they choose some.
    expect(page.querySelector("input[type=checkbox]")).toBeNull();
    await click(some);
    expect(checked(page)).toEqual(["api", "design", "web"]);
    await click(every);
    await click(button(page, "Connect acme"));
    expect(chooseGithub).toHaveBeenCalledWith("ws_side", "acme", { repos: undefined });
  });

  it("starts from what it works in there now", () => {
    const page = githubTab(side, {
      ...github,
      connections: [{ ...acme, repos: ["Web"] }],
      offers: [{ ...offer, all: true }],
    });
    const [every] = [...page.querySelectorAll<HTMLInputElement>("input[type=radio]")];
    expect(every?.checked).toBe(false);
    expect(checked(page)).toEqual(["web"]);
  });

  it("connects nothing when they'd rather not", async () => {
    Object.assign(route, { github: "choose", account: "acme" });
    dismissGithubOffer.mockResolvedValue(github);
    const page = githubTab(side, { ...github, offers: [offer] });
    await click(button(page, "Not now"));
    expect(dismissGithubOffer).toHaveBeenCalledWith("ws_side", "acme");
    expect(chooseGithub).not.toHaveBeenCalled();
    expect(navigate).toHaveBeenCalledWith(expect.objectContaining({ search: { tab: "github" } }));
  });

  it("says when the choice ran out", () => {
    Object.assign(route, { github: "choose", account: "acme" });
    expect(githubTab(side, github).textContent).toContain(
      "What GitHub showed you waits 10 minutes for you to choose from, and that ran out.",
    );
  });

  it("says what GitHub let it connect, and as whom", () => {
    Object.assign(route, { github: "connected", account: "acme" });
    const page = githubTab(side, { ...github, connections: [acme] });
    expect(page.textContent).toContain("Side project works in acme now: web, design.");
    expect(page.textContent).toContain("connected by Ada Lovelace as @ada on GitHub");
  });

  it("explains why GitHub's answer connected nothing", () => {
    Object.assign(route, { github: "refused", account: "acme", reason: "not_admin" });
    expect(githubTab(side, github).textContent).toContain(
      "GitHub says you aren't an admin of every repository you named in acme.",
    );
    Object.assign(route, { reason: undefined });
    expect(githubTab(side, github).textContent).toContain(
      "GitHub says you aren't an admin of any repository of acme the app reaches, so you can't connect it.",
    );
    // The same whether the app isn't installed there or reaches nothing they can see.
    Object.assign(route, { reason: "no_installation" });
    expect(githubTab(side, github).textContent).toContain(
      "GitHub shows you no installation of the fctl-sh app on acme: it isn't installed there, or it reaches no repository you can see.",
    );
    // Without an account: none of the ones GitHub showed.
    Object.assign(route, { account: undefined, reason: "no_admin" });
    expect(githubTab(side, github).textContent).toContain(
      "GitHub says you aren't an admin of any repository the fctl-sh app reaches on the accounts it shows you",
    );
  });

  it("asks people back from GitHub to connect from here", () => {
    Object.assign(route, { github: "installed" });
    const page = githubTab(side, github);
    expect(page.textContent).toContain(
      "The fctl-sh app changed on GitHub. To choose what Side project works in",
    );
    expect(button(page, "Connect with GitHub")).toBeDefined();
  });

  it("offers nothing to connect where the instance can't sign people in to GitHub", () => {
    const page = githubTab(side, {
      ...github,
      install_url: undefined,
      can_connect: false,
      connections: [acme],
    });
    expect(page.textContent).toContain("which this instance isn't set up for");
    expect(page.textContent).not.toContain("Connect with GitHub");
    expect(page.querySelector("button[aria-label^='Choose which repositories']")).toBeNull();
  });

  it("shows members the accounts, not the forms or anyone's choices", () => {
    const page = githubTab(
      { ...side, role: "member" },
      { ...github, install_url: undefined, offers: [offer] },
    );
    expect(page.textContent).toContain("Side project works in no GitHub account yet.");
    expect(page.textContent).not.toContain("Connect with GitHub");
    expect(page.textContent).not.toContain("Choose what");
  });
});
