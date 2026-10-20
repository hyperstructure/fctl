import { afterEach, describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  arrange,
  arrangeIn,
  byWorkspace,
  pending,
  withPendingProjects,
  withPendingWorkspaces,
} from "./order";

const ws = (id: string) => ({ id, slug: id, name: id }) as G.Workspace;
const project = (id: string, workspace: string) =>
  ({ id, slug: id, name: id, workspace: { id: workspace, slug: workspace, name: workspace } }) as G.Project;
const ids = (list: readonly { id: string }[]) => list.map((x) => x.id);

afterEach(() => {
  pending.setWorkspaces(undefined);
  pending.setProjects("acme", undefined);
  pending.setProjects("side", undefined);
});

describe("arrange", () => {
  const items = ["a", "b", "c", "d"].map((id) => ({ id }));

  it("puts the ones named first, in their order, and the others after them as they were", () => {
    expect(ids(arrange(items, ["c", "a"], (x) => x.id))).toEqual(["c", "a", "b", "d"]);
    expect(ids(arrange(items, ["d", "c", "b", "a"], (x) => x.id))).toEqual(["d", "c", "b", "a"]);
  });

  it("leaves out names of nothing, and keeps the list as it was without an order", () => {
    expect(ids(arrange(items, ["gone", "b"], (x) => x.id))).toEqual(["b", "a", "c", "d"]);
    expect(ids(arrange(items, [], (x) => x.id))).toEqual(["a", "b", "c", "d"]);
  });
});

describe("projects in workspaces", () => {
  const list = [
    project("web", "acme"),
    project("api", "acme"),
    project("notes", "side"),
    project("docs", "acme"),
  ];

  it("orders one workspace's projects in the places they had", () => {
    expect(ids(arrangeIn(list, "acme", ["docs", "web", "api"]))).toEqual(["docs", "web", "notes", "api"]);
  });

  it("groups them under their workspaces in order, each one's as they were", () => {
    expect(ids(byWorkspace(list, ["side", "acme"]))).toEqual(["notes", "web", "api", "docs"]);
  });
});

describe("orders being saved", () => {
  it("leave lists as they come when there are none", () => {
    const workspaces = { workspaces: [ws("acme"), ws("side")], invitations: [], limit: 5 };
    expect(withPendingWorkspaces(workspaces)).toBe(workspaces);
    const projects = { projects: [project("web", "acme")] };
    expect(withPendingProjects(projects)).toBe(projects);
  });

  it("keep their order in lists fetched before they are saved", () => {
    pending.setWorkspaces(["side", "acme"]);
    pending.setProjects("acme", ["api", "web"]);
    const workspaces = { workspaces: [ws("acme"), ws("side")], invitations: [], limit: 5 };
    expect(ids(withPendingWorkspaces(workspaces).workspaces)).toEqual(["side", "acme"]);
    const projects = { projects: [project("web", "acme"), project("api", "acme"), project("notes", "side")] };
    expect(ids(withPendingProjects(projects).projects)).toEqual(["notes", "api", "web"]);
  });
});
