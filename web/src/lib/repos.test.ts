import { describe, expect, it } from "vitest";
import { repoLabel, repoSummary, repoTitle } from "./repos";

const fctl = { owner: "hyperstructure", name: "fctl" };

describe("a project's repositories", () => {
  it("names the primary repository alone", () => {
    expect(repoSummary({ repo: fctl })).toEqual({ name: "hyperstructure/fctl", more: undefined });
    expect(repoSummary({ repo: fctl, other_repos: 0 })?.more).toBeUndefined();
    expect(repoLabel({ repo: fctl })).toBe("hyperstructure/fctl");
    expect(repoTitle({ repo: fctl }, "Hyperstructure")).toBe(
      "Agents work in hyperstructure/fctl, in Hyperstructure",
    );
  });

  it("counts the others", () => {
    const p = { repo: fctl, other_repos: 3 };
    expect(repoLabel(p)).toBe("hyperstructure/fctl (+3 more)");
    expect(repoTitle(p)).toBe("Agents work in hyperstructure/fctl, and may also use 3 more repositories");
    expect(repoTitle({ repo: fctl, other_repos: 1 }, "Hyperstructure")).toBe(
      "Agents work in hyperstructure/fctl, in Hyperstructure, and may also use 1 more repository",
    );
  });

  it("says when agents may read the entire org", () => {
    // Named ones are in the org too.
    const p = { repo: fctl, other_repos: 2, all_repos_of: "hyperstructure" };
    expect(repoSummary(p)).toEqual({ name: "hyperstructure/fctl", more: "(+entire org)" });
    expect(repoLabel(p)).toBe("hyperstructure/fctl (+entire org)");
    expect(repoTitle(p)).toBe(
      "Agents work in hyperstructure/fctl, and may also read every repository of hyperstructure",
    );
  });

  it("describes the others without a primary repository", () => {
    expect(repoLabel({ other_repos: 2 })).toBe("2 repositories");
    expect(repoLabel({ other_repos: 1 })).toBe("1 repository");
    expect(repoTitle({ other_repos: 2 })).toBe(
      "Agents start in an empty directory, and may use 2 repositories",
    );
    expect(repoLabel({ all_repos_of: "hyperstructure" })).toBe("hyperstructure (entire org)");
    expect(repoSummary({})).toBeUndefined();
    expect(repoLabel({ other_repos: 0 })).toBeUndefined();
    expect(repoTitle({})).toBeUndefined();
  });
});
