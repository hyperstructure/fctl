import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  agentsGist,
  PROJECT_VIEWS,
  pullRequestsGist,
  sandboxGist,
  settingsGists,
  settingsPageOf,
  settingsPagesFor,
  shownView,
  tabOf,
  ticketsGist,
} from "./project-page";

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
  env: {},
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

describe("a project's page", () => {
  it("puts Memory's retrospectives under Memory, and Settings' pages under Settings", () => {
    expect(tabOf("sessions")).toBe("sessions");
    expect(tabOf("memory")).toBe("memory");
    expect(tabOf("retrospectives")).toBe("memory");
    expect(tabOf("settings")).toBe("settings");
    expect(tabOf("tickets")).toBe("settings");
    expect(tabOf("secrets")).toBe("settings");
    expect(tabOf("pull-requests")).toBe("settings");
  });

  it("still opens what the tabs it had are called: they're pages of Memory and Settings now", () => {
    for (const old of [
      "tickets",
      "retrospectives",
      "repositories",
      "members",
      "secrets",
      "settings",
      "memory",
      "schedules",
      "reviews",
      "autopilot",
    ])
      expect(PROJECT_VIEWS).toContain(old);
  });

  it("opens what the address asks for, where the person may see it", () => {
    const p = project();
    expect(shownView(undefined, p)).toBe("sessions");
    expect(shownView("secrets", p)).toBe("secrets");
    expect(shownView("retrospectives", p)).toBe("retrospectives");
    // Below maintainer, Settings opens with what they may read.
    const editor = project({ role: "editor" });
    expect(shownView("secrets", editor)).toBe("settings");
    expect(shownView("agents", editor)).toBe("settings");
    expect(shownView("tickets", editor)).toBe("tickets");
    expect(shownView("members", editor)).toBe("members");
    // Where the workspace lets it have no autopilot, Sessions.
    expect(shownView("autopilot", p)).toBe("sessions");
    expect(shownView("autopilot", project({ autopilot: { state: "off" } as G.ProjectAutopilot }))).toBe(
      "autopilot",
    );
  });

  it("shows owners and maintainers every page of Settings, and the rest what everyone reads", () => {
    expect(settingsPagesFor("owner")).toEqual([
      "general",
      "repositories",
      "members",
      "agents",
      "pull-requests",
      "tickets",
      "sandbox",
      "secrets",
    ]);
    expect(settingsPagesFor("maintainer")).toEqual(settingsPagesFor("owner"));
    expect(settingsPagesFor("editor")).toEqual(["repositories", "members", "tickets"]);
    expect(settingsPagesFor("viewer")).toEqual(["repositories", "members", "tickets"]);
    // Settings itself opens its first page beside the list.
    expect(settingsPageOf("settings", "maintainer")).toBe("general");
    expect(settingsPageOf("settings", "viewer")).toBe("repositories");
    expect(settingsPageOf("sandbox", "maintainer")).toBe("sandbox");
  });
});

describe("what the list of Settings' pages says", () => {
  it("names the model and effort agents work with", () => {
    expect(agentsGist(settings)).toBe("Claude's default model");
    expect(agentsGist({ ...settings, model: "opus", effort: "xhigh" })).toBe("Opus · extra high effort");
    expect(agentsGist({ ...settings, effort: "low" })).toBe("Claude's default model · low effort");
    expect(agentsGist({ ...settings, model: "claude-opus-5-5" })).toBe("claude-opus-5-5");
  });

  it("says whether agents open pull requests, and what brings them back", () => {
    expect(pullRequestsGist(settings)).toBe("Opened when done · no follow-ups");
    expect(
      pullRequestsGist({
        ...settings,
        auto_pull_request: false,
        follow_ups: { comments: true, checks: false, closed: false, conflicts: false },
      }),
    ).toBe("Opened when asked · 1 follow-up");
    expect(
      pullRequestsGist({
        ...settings,
        follow_ups: { comments: true, checks: true, closed: true, conflicts: true },
      }),
    ).toBe("Opened when done · 4 follow-ups");
  });

  it("names the tracker, and Linear's first team", () => {
    expect(ticketsGist(settings)).toBe("GitHub Issues");
    expect(ticketsGist({ ...settings, tickets: { tracker: "none" } })).toBe("None");
    const linear = { organization: "org", team: "t1", team_key: "ENG" };
    expect(ticketsGist({ ...settings, tickets: { tracker: "linear", linear } })).toBe("Linear · ENG");
    expect(
      ticketsGist({
        ...settings,
        tickets: { tracker: "linear", linear: { ...linear, more_teams: [{ team: "t2", team_key: "DES" }] } },
      }),
    ).toBe("Linear · ENG and 1 more");
  });

  it("says what sandboxes start from", () => {
    expect(sandboxGist(settings)).toBe("Each runner's own image");
    expect(sandboxGist({ ...settings, image: "ghcr.io/acme/agent:1" })).toBe("ghcr.io/acme/agent:1");
    expect(sandboxGist({ ...settings, image_build: { file: "Dockerfile.agent" } })).toBe(
      "Built from Dockerfile.agent",
    );
  });

  it("counts members and secrets once they're loaded", () => {
    const loading = settingsGists(project());
    expect(loading.members).toBeUndefined();
    expect(loading.secrets).toBeUndefined();
    expect(loading.general).toBe("Sessions start from main");
    expect(loading.repositories).toBe("No repository");
    const loaded = settingsGists(project({ repo: { owner: "acme", name: "shop" }, other_repos: 2 }), {
      members: 1,
      secrets: 0,
    });
    expect(loaded.members).toBe("1 member");
    expect(loaded.secrets).toBe("None yet");
    expect(loaded.repositories).toBe("acme/shop (+2 more)");
  });
});
