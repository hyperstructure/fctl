import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  GITHUB_ACCESS,
  githubOffers,
  lowerAccess,
  sessionGithub,
  tracksTickets,
  within,
} from "./github-access";

const project = (settings: Partial<G.ProjectSettings>) => ({ settings: settings as G.ProjectSettings });

describe("what agents may do on GitHub", () => {
  it("goes from pushing down to triage and reading", () => {
    expect(GITHUB_ACCESS.map((l) => l.value)).toEqual(["push", "comment", "triage", "read"]);
    expect(within("triage", "comment")).toBe(true);
    expect(within("comment", "triage")).toBe(false);
    expect(within("read", "triage")).toBe(true);
    expect(lowerAccess("push", "triage")).toBe("triage");
    expect(lowerAccess("triage", "read")).toBe("read");
    // A project held to triage holds its sessions there too.
    expect(sessionGithub({ github_access: "comment" }, project({ github_access: "triage" }))).toBe("triage");
  });

  it("offers triage within the project's setting, as its tracker makes it", () => {
    expect(githubOffers(project({ github_access: "triage" })).map((l) => l.value)).toEqual([
      "triage",
      "read",
    ]);
    expect(githubOffers(project({ github_access: "read" })).map((l) => l.value)).toEqual(["read"]);
    const triage = (settings: Partial<G.ProjectSettings>) =>
      githubOffers(project(settings)).find((l) => l.value === "triage")?.description;
    expect(triage({})).toBe(
      "Labels issues and comments on them, and nothing else: no code, no pull requests, no reviews.",
    );
    expect(triage({ tickets: { tracker: "linear" } as G.Tickets })).toContain("the project's Linear issues");
    expect(triage({ tickets: { tracker: "none" } as G.Tickets })).toContain("its agent only reads");
  });

  it("knows which projects have tickets to triage", () => {
    expect(tracksTickets(project({}))).toBe(true);
    expect(tracksTickets(project({ tickets: { tracker: "linear" } as G.Tickets }))).toBe(true);
    expect(tracksTickets(project({ tickets: { tracker: "none" } as G.Tickets }))).toBe(false);
  });
});
