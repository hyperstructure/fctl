import { describe, expect, it } from "vitest";
import { repoLabel, repoSummary } from "./repos";

const fctl = { owner: "hyperstructure", name: "fctl" };

describe("a project's repositories", () => {
  it("names the primary repository alone", () => {
    expect(repoSummary({ repo: fctl })).toEqual({ name: "hyperstructure/fctl", more: undefined });
    expect(repoSummary({ repo: fctl, other_repos: 0 })?.more).toBeUndefined();
    expect(repoLabel({ repo: fctl })).toBe("hyperstructure/fctl");
  });

  it("counts the others", () => {
    expect(repoLabel({ repo: fctl, other_repos: 3 })).toBe("hyperstructure/fctl (+3 more)");
  });

  it("says when agents may read the entire org", () => {
    // Named ones are in the org too.
    const p = { repo: fctl, other_repos: 2, all_repos_of: "hyperstructure" };
    expect(repoSummary(p)).toEqual({ name: "hyperstructure/fctl", more: "(+entire org)" });
    expect(repoLabel(p)).toBe("hyperstructure/fctl (+entire org)");
  });

  it("describes the others without a primary repository", () => {
    expect(repoLabel({ other_repos: 2 })).toBe("2 repositories");
    expect(repoLabel({ other_repos: 1 })).toBe("1 repository");
    expect(repoLabel({ all_repos_of: "hyperstructure" })).toBe("hyperstructure (entire org)");
    expect(repoSummary({})).toBeUndefined();
    expect(repoLabel({ other_repos: 0 })).toBeUndefined();
  });
});
