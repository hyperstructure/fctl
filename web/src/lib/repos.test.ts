import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { parseRepo, repoAdditions, repoChoices, repoLabel, repoSummary } from "./repos";

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

describe("a repository typed by hand", () => {
  it("is owner/name, or owner/* for every repository of the account", () => {
    expect(parseRepo("hyperstructure/fctl")).toEqual({ owner: "hyperstructure", name: "fctl" });
    expect(parseRepo("  hyperstructure/fctl-site/ ")).toEqual({ owner: "hyperstructure", name: "fctl-site" });
    expect(parseRepo("hyperstructure/*")).toEqual({ owner: "hyperstructure" });
    expect(parseRepo("hyperstructure/fctl.git")).toEqual({ owner: "hyperstructure", name: "fctl" });
  });

  it("is its address on github.com, of something in it too", () => {
    expect(parseRepo("https://github.com/hyperstructure/design")).toEqual({
      owner: "hyperstructure",
      name: "design",
    });
    expect(parseRepo("github.com/hyperstructure/design.git")?.name).toBe("design");
    expect(parseRepo("https://github.com/hyperstructure/fctl/pull/295#discussion")).toEqual({
      owner: "hyperstructure",
      name: "fctl",
    });
    expect(parseRepo("https://github.com/hyperstructure/fctl?tab=readme")?.name).toBe("fctl");
  });

  it("isn't a word, an account alone, or a path", () => {
    for (const s of [
      "",
      "fctl",
      "hyperstructure/",
      "/fctl",
      "a/b/c",
      "a b/c",
      "a/..",
      "a/.git",
      "github.com/a",
    ])
      expect(parseRepo(s), s).toBeUndefined();
  });
});

const repo = (owner: string, name: string, more: Partial<G.GithubRepo> = {}): G.GithubRepo => ({
  owner,
  name,
  full_name: `${owner}/${name}`,
  private: false,
  default_branch: "main",
  ...more,
});
const listed = (owner: string, name?: string, primary = false): G.ProjectRepo => ({
  owner,
  name,
  access: primary ? "write" : "read",
  primary,
});
// The workspace's repositories, newest push first.
const available = [
  repo("hyperstructure", "fctl"),
  repo("hyperstructure", "homebrew-tap"),
  repo("hyperstructure", "design", { private: true }),
  repo("hyperstructure", "fctl-site"),
];
const project = [listed("hyperstructure", "fctl", true), listed("hyperstructure", "design")];
const values = (cs: { value: string }[]) => cs.map((c) => c.value);

describe("the repositories to add to a project", () => {
  it("offers the account's every repository, then its others, then those on the project", () => {
    const cs = repoChoices({
      listed: project,
      available,
      accounts: ["hyperstructure"],
      chosen: [],
      query: "",
    });
    expect(values(cs)).toEqual([
      "hyperstructure/*",
      "hyperstructure/homebrew-tap",
      "hyperstructure/fctl-site",
      "hyperstructure/fctl",
      "hyperstructure/design",
    ]);
    expect(cs.map((c) => c.on)).toEqual([undefined, undefined, undefined, "primary", "added"]);
    expect(cs[0]?.name).toBeUndefined();
    expect(cs[4]?.private).toBe(true);
    expect(cs.some((c) => c.elsewhere)).toBe(false);
  });

  it("knows the account's every repository is on the project", () => {
    const cs = repoChoices({
      listed: [...project, listed("hyperstructure")],
      available,
      accounts: ["hyperstructure"],
      chosen: [],
      query: "",
    });
    expect(values(cs)[0]).toBe("hyperstructure/homebrew-tap");
    expect(cs.find((c) => c.value === "hyperstructure/*")?.on).toBe("added");
  });

  it("keeps to the project's account", () => {
    const cs = repoChoices({
      listed: project,
      available: [...available, repo("romanesco", "web")],
      accounts: ["hyperstructure", "romanesco"],
      chosen: [],
      query: "",
    });
    expect(values(cs).some((v) => v.startsWith("romanesco/"))).toBe(false);
  });

  it("offers every account of the workspace to a project without repositories, until one is chosen", () => {
    const workspace = [...available, repo("romanesco", "web")];
    const accounts = ["romanesco", "hyperstructure"];
    const none = repoChoices({ listed: [], available: workspace, accounts, chosen: [], query: "" });
    expect(values(none).slice(0, 3)).toEqual(["romanesco/*", "romanesco/web", "hyperstructure/*"]);
    expect(none.some((c) => c.elsewhere)).toBe(false);
    const some = repoChoices({
      listed: [],
      available: workspace,
      accounts,
      chosen: ["romanesco/web"],
      query: "",
    });
    expect(some.find((c) => c.value === "romanesco/*")?.elsewhere).toBeUndefined();
    expect(some.find((c) => c.value === "hyperstructure/fctl")?.elsewhere).toBe("romanesco");
  });

  it("narrows them down to what's typed, closest first", () => {
    const find = (query: string) =>
      values(repoChoices({ listed: project, available, accounts: [], chosen: [], query }));
    expect(find("fctl")).toEqual(["hyperstructure/fctl", "hyperstructure/fctl-site"]);
    expect(find("site")).toEqual(["hyperstructure/fctl-site"]);
    expect(find("TAP")).toEqual(["hyperstructure/homebrew-tap"]);
    expect(find("*")).toEqual(["hyperstructure/*"]);
    expect(find("https://github.com/hyperstructure/fctl-site/pulls")).toEqual(["hyperstructure/fctl-site"]);
    expect(find("nothing")).toEqual([]);
  });

  it("offers a repository typed whole that isn't on the list, after those that are", () => {
    const find = (query: string, chosen: string[] = []) =>
      repoChoices({ listed: project, available, accounts: [], chosen, query });
    // Among those as close, the ones to add come first.
    expect(values(find("hyperstructure/fc"))).toEqual([
      "hyperstructure/fctl-site",
      "hyperstructure/fctl",
      "hyperstructure/fc",
    ]);
    expect(find("hyperstructure/fc").at(-1)?.typed).toBe(true);
    // Not twice, whatever its case.
    expect(values(find("Hyperstructure/Homebrew-Tap"))).toEqual(["hyperstructure/homebrew-tap"]);
    // Another account's can't join the project's.
    expect(find("romanesco/web")[0]).toMatchObject({
      value: "romanesco/web",
      typed: true,
      elsewhere: "hyperstructure",
    });
    // Chosen, it stays on the list when the field is cleared, ticked.
    expect(values(find("", ["hyperstructure/sandbox"])).slice(0, 2)).toEqual([
      "hyperstructure/sandbox",
      "hyperstructure/*",
    ]);
    expect(values(find("sand", ["hyperstructure/sandbox"]))).toEqual(["hyperstructure/sandbox"]);
  });
});

describe("what the field adds to a project", () => {
  const chosen = ["hyperstructure/homebrew-tap"];

  it("adds what's chosen, and a repository typed whole besides", () => {
    expect(repoAdditions(chosen, "", project)).toEqual(chosen);
    expect(repoAdditions(chosen, "hyperstructure/sandbox", project)).toEqual([
      "hyperstructure/homebrew-tap",
      "hyperstructure/sandbox",
    ]);
    expect(repoAdditions([], "https://github.com/hyperstructure/sandbox/pulls", project)).toEqual([
      "hyperstructure/sandbox",
    ]);
    // What narrows the list down isn't a repository.
    expect(repoAdditions(chosen, "tap", project)).toEqual(chosen);
    expect(repoAdditions(chosen, "Hyperstructure/Homebrew-Tap", project)).toEqual(chosen);
  });

  it("leaves out one typed that the project has, which would only change its access", () => {
    expect(repoAdditions(chosen, "hyperstructure/design", project)).toEqual(chosen);
    expect(repoAdditions([], "HYPERSTRUCTURE/fctl", project)).toEqual([]);
    expect(repoAdditions([], "hyperstructure/*", [...project, listed("hyperstructure")])).toEqual([]);
    expect(repoAdditions([], "hyperstructure/*", project)).toEqual(["hyperstructure/*"]);
  });
});
